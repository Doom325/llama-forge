//! VRAM estimation and GGUF header parsing.
//! See design doc Section 3.3.

use crate::types::ModelArchitecture;
use llamaforge_core::config::ServerConfig;
use std::fs::File;
use std::io::{BufReader, ErrorKind, Read};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum GgufError {
    #[error("Malformed GGUF header")]
    Malformed,
    #[error("I/O Error: {0}")]
    Io(#[from] std::io::Error),
    #[allow(dead_code)]
    #[error("Unsupported GGUF version: {0}")]
    UnsupportedVersion(u32),
}

#[non_exhaustive]
#[derive(Debug)]
#[allow(non_camel_case_types)]
pub enum Quantization {
    Q4_K_M,
    Q5_K_M,
    Q6_K,
    Q8_0,
    F16,
    F32,
    Other(String),
}

#[derive(Debug)]
pub struct GgufMetadata {
    pub architecture: ModelArchitecture,
    pub block_count: u32,
    pub embedding_length: u32,
    pub head_count: u32,
    pub head_count_kv: u32,
    pub quantization: Quantization,
    pub file_size_bytes: u64,
}

#[allow(dead_code)]
pub async fn parse_gguf(path: &Path) -> Result<GgufMetadata, GgufError> {
    let path_buf = path.to_path_buf();
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || {
        let _enter = span.enter();
        sync_parse(&path_buf)
    })
    .await
    .unwrap_or_else(|e| Err(GgufError::Io(std::io::Error::other(e.to_string()))))
}

// GGUF metadata value types (gguf.md spec).
const GGUF_UINT8: u32 = 0;
const GGUF_INT8: u32 = 1;
const GGUF_UINT16: u32 = 2;
const GGUF_INT16: u32 = 3;
const GGUF_UINT32: u32 = 4;
const GGUF_INT32: u32 = 5;
const GGUF_FLOAT32: u32 = 6;
const GGUF_BOOL: u32 = 7;
const GGUF_STRING: u32 = 8;
const GGUF_ARRAY: u32 = 9;
const GGUF_UINT64: u32 = 10;
const GGUF_INT64: u32 = 11;
const GGUF_FLOAT64: u32 = 12;

/// Sanity caps so a corrupt header can't make us allocate/loop unboundedly.
const MAX_GGUF_STRING: u64 = 1 << 20; // 1 MiB
const MAX_GGUF_COUNT: u64 = 1 << 22; // 4M KV pairs / array elements

/// `UnexpectedEof` (a truncated/garbage header) is a *malformed* model the caller
/// can fall back from; other I/O errors mean the file itself is unreadable.
fn map_io(e: std::io::Error) -> GgufError {
    if e.kind() == ErrorKind::UnexpectedEof {
        GgufError::Malformed
    } else {
        GgufError::Io(e)
    }
}

fn read_exact_n<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<(), GgufError> {
    r.read_exact(buf).map_err(map_io)
}

fn read_u32<R: Read>(r: &mut R) -> Result<u32, GgufError> {
    let mut b = [0u8; 4];
    read_exact_n(r, &mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u64<R: Read>(r: &mut R) -> Result<u64, GgufError> {
    let mut b = [0u8; 8];
    read_exact_n(r, &mut b)?;
    Ok(u64::from_le_bytes(b))
}

fn read_gguf_string<R: Read>(r: &mut R) -> Result<String, GgufError> {
    let len = read_u64(r)?;
    if len > MAX_GGUF_STRING {
        return Err(GgufError::Malformed);
    }
    let mut buf = vec![0u8; len as usize];
    read_exact_n(r, &mut buf)?;
    String::from_utf8(buf).map_err(|_| GgufError::Malformed)
}

fn is_int_type(t: u32) -> bool {
    matches!(
        t,
        GGUF_UINT8
            | GGUF_INT8
            | GGUF_UINT16
            | GGUF_INT16
            | GGUF_UINT32
            | GGUF_INT32
            | GGUF_UINT64
            | GGUF_INT64
    )
}

/// Read any integer-typed value, widened to u64 (metadata counts are positive).
fn read_int_as_u64<R: Read>(r: &mut R, t: u32) -> Result<u64, GgufError> {
    let n = match t {
        GGUF_UINT8 | GGUF_INT8 => 1,
        GGUF_UINT16 | GGUF_INT16 => 2,
        GGUF_UINT32 | GGUF_INT32 => 4,
        GGUF_UINT64 | GGUF_INT64 => 8,
        _ => return Err(GgufError::Malformed),
    };
    let mut b = [0u8; 8];
    read_exact_n(r, &mut b[..n])?;
    Ok(u64::from_le_bytes(b)) // little-endian; unread high bytes stay zero
}

fn skip_bytes<R: Read>(r: &mut R, mut n: u64) -> Result<(), GgufError> {
    let mut scratch = [0u8; 4096];
    while n > 0 {
        let take = n.min(scratch.len() as u64) as usize;
        read_exact_n(r, &mut scratch[..take])?;
        n -= take as u64;
    }
    Ok(())
}

/// Advance past a metadata value of any type (including nested arrays) so the
/// next key can be read.
fn skip_value<R: Read>(r: &mut R, t: u32) -> Result<(), GgufError> {
    match t {
        GGUF_STRING => {
            let len = read_u64(r)?;
            if len > MAX_GGUF_STRING {
                return Err(GgufError::Malformed);
            }
            skip_bytes(r, len)
        }
        GGUF_ARRAY => {
            let elem = read_u32(r)?;
            // GGUF arrays hold scalars/strings, never nested arrays (llama.cpp).
            // Refusing array-of-array bounds skip_value's recursion to one level —
            // otherwise a crafted/corrupt header could nest arrays unboundedly and
            // overflow the stack (a hard crash, not the intended Malformed fallback).
            if elem == GGUF_ARRAY {
                return Err(GgufError::Malformed);
            }
            let count = read_u64(r)?;
            if count > MAX_GGUF_COUNT {
                return Err(GgufError::Malformed);
            }
            for _ in 0..count {
                skip_value(r, elem)?;
            }
            Ok(())
        }
        GGUF_BOOL | GGUF_UINT8 | GGUF_INT8 => skip_bytes(r, 1),
        GGUF_UINT16 | GGUF_INT16 => skip_bytes(r, 2),
        GGUF_UINT32 | GGUF_INT32 | GGUF_FLOAT32 => skip_bytes(r, 4),
        GGUF_UINT64 | GGUF_INT64 | GGUF_FLOAT64 => skip_bytes(r, 8),
        _ => Err(GgufError::Malformed),
    }
}

fn map_architecture(arch: &str) -> ModelArchitecture {
    match arch {
        "llama" => ModelArchitecture::Llama,
        "mistral" => ModelArchitecture::Mistral,
        "mixtral" => ModelArchitecture::Mixtral,
        "qwen" | "qwen2" | "qwen2moe" => ModelArchitecture::Qwen,
        "gemma" | "gemma2" => ModelArchitecture::Gemma,
        "phi2" | "phi3" => ModelArchitecture::Phi,
        "deepseek" | "deepseek2" => ModelArchitecture::DeepSeek,
        other => ModelArchitecture::Other(other.to_string()),
    }
}

/// Map the `general.file_type` enum (llama.cpp `LLAMA_FTYPE`) to a quantization.
fn map_file_type(ft: u64) -> Quantization {
    match ft {
        0 => Quantization::F32,
        1 => Quantization::F16,
        7 => Quantization::Q8_0,
        15 => Quantization::Q4_K_M,
        17 => Quantization::Q5_K_M,
        18 => Quantization::Q6_K,
        other => Quantization::Other(format!("file_type={other}")),
    }
}

/// Parse the GGUF header + metadata KV block. Only the fields needed for VRAM
/// estimation are extracted; everything else (tokenizer arrays, etc.) is skipped.
fn sync_parse(path: &Path) -> Result<GgufMetadata, GgufError> {
    let file = File::open(path)?;
    let file_size_bytes = file.metadata()?.len();
    let mut r = BufReader::new(file);

    let mut magic = [0u8; 4];
    read_exact_n(&mut r, &mut magic)?;
    if &magic != b"GGUF" {
        return Err(GgufError::Malformed);
    }
    let version = read_u32(&mut r)?;
    if version != 2 && version != 3 {
        return Err(GgufError::UnsupportedVersion(version));
    }
    let _tensor_count = read_u64(&mut r)?;
    let kv_count = read_u64(&mut r)?;
    if kv_count > MAX_GGUF_COUNT {
        return Err(GgufError::Malformed);
    }

    let mut architecture: Option<String> = None;
    let mut file_type: Option<u64> = None;
    let mut block_count: Option<u64> = None;
    let mut embedding_length: Option<u64> = None;
    let mut head_count: Option<u64> = None;
    let mut head_count_kv: Option<u64> = None;

    // Match arch-specific keys by suffix (e.g. `llama.block_count`) so we don't
    // need to have seen `general.architecture` first.
    for _ in 0..kv_count {
        let key = read_gguf_string(&mut r)?;
        let vtype = read_u32(&mut r)?;
        if key == "general.architecture" && vtype == GGUF_STRING {
            architecture = Some(read_gguf_string(&mut r)?);
        } else if key == "general.file_type" && is_int_type(vtype) {
            file_type = Some(read_int_as_u64(&mut r, vtype)?);
        } else if key.ends_with(".block_count") && is_int_type(vtype) {
            block_count = Some(read_int_as_u64(&mut r, vtype)?);
        } else if key.ends_with(".embedding_length") && is_int_type(vtype) {
            embedding_length = Some(read_int_as_u64(&mut r, vtype)?);
        } else if key.ends_with(".attention.head_count_kv") && is_int_type(vtype) {
            head_count_kv = Some(read_int_as_u64(&mut r, vtype)?);
        } else if key.ends_with(".attention.head_count") && is_int_type(vtype) {
            head_count = Some(read_int_as_u64(&mut r, vtype)?);
        } else {
            skip_value(&mut r, vtype)?;
        }
    }

    Ok(GgufMetadata {
        architecture: architecture.as_deref().map(map_architecture).unwrap_or_default(),
        block_count: block_count.unwrap_or(0) as u32,
        embedding_length: embedding_length.unwrap_or(0) as u32,
        head_count: head_count.unwrap_or(0) as u32,
        head_count_kv: head_count_kv.unwrap_or(0) as u32,
        quantization: file_type
            .map(map_file_type)
            .unwrap_or_else(|| Quantization::Other("unknown".into())),
        file_size_bytes,
    })
}

pub fn compute_vram_estimate(config: &ServerConfig, meta: &GgufMetadata) -> u64 {
    if let Some(override_mb) = config.estimated_vram_mb {
        return override_mb * 1024 * 1024;
    }

    let bits_per_param = match meta.quantization {
        Quantization::Q4_K_M => 4.5,
        Quantization::Q5_K_M => 5.5,
        Quantization::Q6_K => 6.6,
        Quantization::Q8_0 => 8.5,
        Quantization::F16 => 16.0,
        Quantization::F32 => 32.0,
        Quantization::Other(_) => 8.0, // fallback
    };

    let param_bytes = (meta.file_size_bytes as f64 * bits_per_param / 8.0) as u64;

    let bytes_per_elem = 2; // f16 typically
    let ctx_size = if config.ctx_size == 0 {
        // Fallback for VRAM estimation when auto-context is used.
        // We assume 8192 if we don't have better info to avoid underestimating.
        8192
    } else {
        config.ctx_size as u64
    };

    // KV cache
    let embedding_per_head = meta.embedding_length / meta.head_count;
    let kv_cache_bytes = 2
        * (meta.block_count as u64)
        * (meta.head_count_kv as u64)
        * (embedding_per_head as u64)
        * ctx_size
        * bytes_per_elem;

    let total = param_bytes + kv_cache_bytes;

    // Add 10% safety margin
    (total as f64 * 1.1) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A metadata KV pair for the test GGUF builder.
    enum Kv {
        Str(&'static str, &'static str),
        U32(&'static str, u32),
        /// An array of u32 — used to exercise the array-skipping path.
        ArrU32(&'static str, &'static [u32]),
        /// An array whose declared element type is itself ARRAY — illegal in
        /// GGUF; used to prove the parser rejects it instead of recursing.
        NestedArray(&'static str),
    }

    fn push_str(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
    }

    impl Kv {
        fn write(&self, buf: &mut Vec<u8>) {
            match self {
                Kv::Str(k, v) => {
                    push_str(buf, k);
                    buf.extend_from_slice(&8u32.to_le_bytes()); // STRING
                    push_str(buf, v);
                }
                Kv::U32(k, v) => {
                    push_str(buf, k);
                    buf.extend_from_slice(&4u32.to_le_bytes()); // UINT32
                    buf.extend_from_slice(&v.to_le_bytes());
                }
                Kv::ArrU32(k, vs) => {
                    push_str(buf, k);
                    buf.extend_from_slice(&9u32.to_le_bytes()); // ARRAY
                    buf.extend_from_slice(&4u32.to_le_bytes()); // elem type UINT32
                    buf.extend_from_slice(&(vs.len() as u64).to_le_bytes());
                    for v in *vs {
                        buf.extend_from_slice(&v.to_le_bytes());
                    }
                }
                Kv::NestedArray(k) => {
                    push_str(buf, k);
                    buf.extend_from_slice(&9u32.to_le_bytes()); // ARRAY
                    buf.extend_from_slice(&9u32.to_le_bytes()); // elem type = ARRAY (illegal)
                    buf.extend_from_slice(&1u64.to_le_bytes()); // count
                                                                // A would-be inner array; the parser must reject before reading it.
                    buf.extend_from_slice(&4u32.to_le_bytes());
                    buf.extend_from_slice(&0u64.to_le_bytes());
                }
            }
        }
    }

    fn build_gguf(tmp: &TempDir, name: &str, version: u32, kvs: &[Kv]) -> std::path::PathBuf {
        let mut out = Vec::new();
        out.extend_from_slice(b"GGUF");
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        out.extend_from_slice(&(kvs.len() as u64).to_le_bytes());
        for kv in kvs {
            kv.write(&mut out);
        }
        let path = tmp.path().join(name);
        std::fs::write(&path, &out).unwrap();
        path
    }

    fn make_malformed_gguf(tmp: &TempDir) -> std::path::PathBuf {
        use std::io::Write;
        let path = tmp.path().join("malformed.gguf");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"BAD!").unwrap();
        file.write_all(&3_u32.to_le_bytes()).unwrap();
        path
    }

    fn make_unsupported_gguf(tmp: &TempDir) -> std::path::PathBuf {
        use std::io::Write;
        let path = tmp.path().join("unsupported.gguf");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"GGUF").unwrap();
        file.write_all(&1_u32.to_le_bytes()).unwrap();
        path
    }

    #[test]
    fn test_compute_vram_estimate() {
        let mut config = ServerConfig { ctx_size: 4096, parallel_slots: 1, ..Default::default() };

        let meta = GgufMetadata {
            architecture: ModelArchitecture::Llama,
            block_count: 32,
            embedding_length: 4096,
            head_count: 32,
            head_count_kv: 32,
            quantization: Quantization::Q4_K_M,
            file_size_bytes: 4_000_000_000,
        };

        let est = compute_vram_estimate(&config, &meta);
        let expected = ((2_250_000_000u64 + 2_147_483_648u64) as f64 * 1.1) as u64;
        assert_eq!(est, expected);

        config.estimated_vram_mb = Some(1024);
        assert_eq!(compute_vram_estimate(&config, &meta), 1024 * 1024 * 1024);
    }

    #[tokio::test]
    async fn parse_gguf_extracts_metadata() {
        let tmp = TempDir::new().unwrap();
        let path = build_gguf(
            &tmp,
            "llama.gguf",
            3,
            &[
                Kv::Str("general.architecture", "llama"),
                Kv::Str("general.name", "Test Model"), // unrelated string → skipped
                Kv::U32("general.file_type", 15),      // MOSTLY_Q4_K_M
                Kv::ArrU32("tokenizer.ggml.token_type", &[1, 2, 3]), // array → skipped
                Kv::U32("llama.block_count", 32),
                Kv::U32("llama.embedding_length", 4096),
                Kv::U32("llama.attention.head_count", 32),
                Kv::U32("llama.attention.head_count_kv", 8),
            ],
        );

        let meta = parse_gguf(&path).await.unwrap();
        assert!(matches!(meta.architecture, ModelArchitecture::Llama));
        assert_eq!(meta.block_count, 32);
        assert_eq!(meta.embedding_length, 4096);
        assert_eq!(meta.head_count, 32);
        assert_eq!(meta.head_count_kv, 8); // distinct from head_count (GQA)
        assert!(matches!(meta.quantization, Quantization::Q4_K_M));
        assert!(meta.file_size_bytes > 0);
    }

    #[tokio::test]
    async fn parse_gguf_unknown_architecture_falls_back_to_other() {
        let tmp = TempDir::new().unwrap();
        let path = build_gguf(
            &tmp,
            "mystery.gguf",
            3,
            &[
                Kv::Str("general.architecture", "mystery-net"),
                Kv::U32("mystery-net.block_count", 4),
            ],
        );
        let meta = parse_gguf(&path).await.unwrap();
        match meta.architecture {
            ModelArchitecture::Other(name) => assert_eq!(name, "mystery-net"),
            other => panic!("expected Other, got {other:?}"),
        }
        assert_eq!(meta.block_count, 4);
    }

    #[tokio::test]
    async fn parse_gguf_rejects_bad_magic_and_unsupported_version() {
        let tmp = TempDir::new().unwrap();

        // Truncated (magic + version, no counts) → malformed.
        let truncated = build_gguf(&tmp, "trunc.gguf", 3, &[]);
        let mut bytes = std::fs::read(&truncated).unwrap();
        bytes.truncate(8);
        std::fs::write(&truncated, &bytes).unwrap();
        assert!(matches!(parse_gguf(&truncated).await, Err(GgufError::Malformed)));

        assert!(matches!(parse_gguf(&make_malformed_gguf(&tmp)).await, Err(GgufError::Malformed)));

        assert!(matches!(
            parse_gguf(&make_unsupported_gguf(&tmp)).await,
            Err(GgufError::UnsupportedVersion(1))
        ));
    }

    #[tokio::test]
    async fn parse_gguf_rejects_nested_arrays_without_recursing() {
        // A nested-array element type must be refused (Malformed), never followed
        // recursively — otherwise a crafted/corrupt header could overflow the stack.
        let tmp = TempDir::new().unwrap();
        let path = build_gguf(
            &tmp,
            "nested.gguf",
            3,
            &[Kv::Str("general.architecture", "llama"), Kv::NestedArray("tokenizer.ggml.merges")],
        );
        assert!(matches!(parse_gguf(&path).await, Err(GgufError::Malformed)));
    }
}
