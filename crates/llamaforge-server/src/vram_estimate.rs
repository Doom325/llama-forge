//! VRAM estimation and GGUF header parsing.
//! See design doc Section 3.3.

use crate::types::ModelArchitecture;
use llamaforge_core::config::ServerConfig;
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

#[allow(dead_code)]
fn sync_parse(_path: &Path) -> Result<GgufMetadata, GgufError> {
    // Stubbed until real KV parser is implemented in a future PR
    Err(GgufError::Malformed)
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

    fn make_tiny_gguf(tmp: &TempDir, _arch: &str, _quant: &str) -> std::path::PathBuf {
        use std::io::Write;
        let path = tmp.path().join("model.gguf");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"GGUF").unwrap();
        file.write_all(&3_u32.to_le_bytes()).unwrap();
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
    async fn test_parse_gguf() {
        let tmp = TempDir::new().unwrap();

        let ok_path = make_tiny_gguf(&tmp, "llama", "Q4_K_M");
        let result = parse_gguf(&ok_path).await;
        // Due to PR3 reviewer feedback, parse_gguf is hard-stubbed to return Malformed
        // so downstream can't consume fake data until a real parser lands.
        assert!(matches!(result, Err(GgufError::Malformed)));

        let malformed_path = make_malformed_gguf(&tmp);
        let result = parse_gguf(&malformed_path).await;
        assert!(matches!(result, Err(GgufError::Malformed)));

        let unsupported_path = make_unsupported_gguf(&tmp);
        let result = parse_gguf(&unsupported_path).await;
        // Stubbed function doesn't check versions anymore.
        assert!(matches!(result, Err(GgufError::Malformed)));
    }
}
