//! Local model (`.gguf`) scanner for the Setup Wizard.
//!
//! Recursively walks the requested directories, discovers `.gguf` files, and
//! derives lightweight metadata (quantization / architecture) from filenames —
//! **not** by parsing GGUF headers (Open Question #1: filename heuristics for
//! v1; header parsing is too slow for an interactive scan). Implements
//! `08_SETUP_WIZARD.md` Section 11.3 (Model Scanner API contract) and the
//! Appendix implementation notes.
//!
//! Bounded by `max_depth` (default 5), `max_results` (default 100), and a 30 s
//! deadline. When any limit fires the scan returns the models found so far with
//! `truncated: true` and a `truncation_reason`. Permission errors and symlink
//! cycles are collected as `warnings` and never abort the scan.
//!
//! Pure domain logic: no axum, no `tracing`. The PR5 handler adds the
//! `"version": 1` envelope and span instrumentation.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::{DirEntry, WalkDir};

/// Default recursion depth when the request omits `max_depth`.
const DEFAULT_MAX_DEPTH: usize = 5;
/// Default result cap when the request omits `max_results`.
const DEFAULT_MAX_RESULTS: usize = 100;
/// Maximum directories accepted in a single request.
const MAX_DIRECTORIES: usize = 10;
/// Hard wall-clock budget for one scan.
const SCAN_DEADLINE: Duration = Duration::from_secs(30);
/// Length of the SHA-256 hex prefix used as a stable model `id`.
const ID_PREFIX_LEN: usize = 16;

// ---------------------------------------------------------------------------
// Request / response types.
// ---------------------------------------------------------------------------

/// Model discovery source. Only `local` is implemented; `huggingface` is
/// reserved (design doc Section 11.3, currently NG1) and deserializing it
/// fails, yielding a 400 at the handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelSource {
    Local,
}

/// A model scan request (Section 11.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelScanRequest {
    pub source: ModelSource,
    pub directories: Vec<String>,
    #[serde(default)]
    pub max_depth: Option<usize>,
    #[serde(default)]
    pub max_results: Option<usize>,
}

/// A discovered `.gguf` model with filename-derived metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredModel {
    /// SHA-256 hex prefix of the canonical path — stable across rescans.
    pub id: String,
    pub path: String,
    pub filename: String,
    pub size_bytes: u64,
    pub quantization_hint: String,
    pub architecture_hint: String,
    /// Last-modified time, RFC 3339. `None` if the filesystem doesn't report it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
}

/// Which bound truncated the scan. Serializes to `"timeout"`, `"max_results"`,
/// or `"max_depth"`; absent (`null`) when `truncated` is false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruncationReason {
    Timeout,
    MaxResults,
    MaxDepth,
}

/// Result of a model scan (Section 11.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelScanResult {
    pub models: Vec<DiscoveredModel>,
    pub scan_duration_ms: u64,
    pub files_scanned: u64,
    pub truncated: bool,
    pub truncation_reason: Option<TruncationReason>,
    pub warnings: Vec<String>,
}

/// Validation failure that the handler maps to a 400. Soft problems
/// (permissions, cycles, unreadable dirs) are warnings, not errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelScanError {
    #[error("at least one directory must be provided")]
    NoDirectories,
    #[error("too many directories: {count} provided, maximum is {max}")]
    TooManyDirectories { count: usize, max: usize },
}

// ---------------------------------------------------------------------------
// Public API.
// ---------------------------------------------------------------------------

/// Scan the requested directories for `.gguf` models.
///
/// The blocking filesystem walk runs on a `spawn_blocking` thread so it never
/// stalls the async runtime. Delegates to [`scan_models_with_deadline`] with a
/// deadline of `now + `[`SCAN_DEADLINE`].
pub async fn scan_models(request: ModelScanRequest) -> Result<ModelScanResult, ModelScanError> {
    scan_models_with_deadline(request, Instant::now() + SCAN_DEADLINE).await
}

/// Scan with an explicit wall-clock `deadline`, exposed so the timeout
/// truncation path is testable without a real 30 s wait. A deadline at or
/// before "now" yields the partial results collected before the first
/// deadline check, with `truncated: true` and `truncation_reason: Some(Timeout)`.
pub async fn scan_models_with_deadline(
    request: ModelScanRequest,
    deadline: Instant,
) -> Result<ModelScanResult, ModelScanError> {
    if request.directories.is_empty() {
        return Err(ModelScanError::NoDirectories);
    }
    if request.directories.len() > MAX_DIRECTORIES {
        return Err(ModelScanError::TooManyDirectories {
            count: request.directories.len(),
            max: MAX_DIRECTORIES,
        });
    }

    let max_depth = request.max_depth.unwrap_or(DEFAULT_MAX_DEPTH);
    let max_results = request.max_results.unwrap_or(DEFAULT_MAX_RESULTS);
    let directories: Vec<PathBuf> = request.directories.iter().map(PathBuf::from).collect();

    let start = Instant::now();
    let walk = tokio::task::spawn_blocking(move || {
        walk_directories(directories, max_depth, max_results, deadline)
    })
    .await;

    let mut outcome = match walk {
        Ok(outcome) => outcome,
        // The walk never panics by construction; if the join still fails we
        // return an empty result carrying the reason rather than propagating.
        Err(join_err) => WalkOutcome {
            warnings: vec![format!("scan worker terminated unexpectedly: {join_err}")],
            ..WalkOutcome::default()
        },
    };

    Ok(ModelScanResult {
        models: std::mem::take(&mut outcome.models),
        scan_duration_ms: start.elapsed().as_millis() as u64,
        files_scanned: outcome.files_scanned,
        truncated: outcome.truncation.is_some(),
        truncation_reason: outcome.truncation,
        warnings: outcome.warnings,
    })
}

// ---------------------------------------------------------------------------
// Blocking walk.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct WalkOutcome {
    models: Vec<DiscoveredModel>,
    files_scanned: u64,
    truncation: Option<TruncationReason>,
    warnings: Vec<String>,
}

fn walk_directories(
    directories: Vec<PathBuf>,
    max_depth: usize,
    max_results: usize,
    deadline: Instant,
) -> WalkOutcome {
    let mut out = WalkOutcome::default();
    let mut depth_limited = false;

    'dirs: for dir in directories {
        // Resolve to a canonical root; unreadable/missing dirs warn and skip.
        let root = match dir.canonicalize() {
            Ok(root) => root,
            Err(e) => {
                out.warnings.push(format!("Cannot access {}: {}", dir.display(), e));
                continue;
            }
        };

        let walker = WalkDir::new(&root)
            .max_depth(max_depth)
            .follow_links(true)
            .into_iter()
            .filter_entry(is_not_hidden);

        for entry in walker {
            if Instant::now() >= deadline {
                out.truncation = Some(TruncationReason::Timeout);
                break 'dirs;
            }

            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    out.warnings.push(describe_walk_error(&err));
                    continue;
                }
            };

            out.files_scanned += 1;

            // A non-empty directory sitting exactly at the depth limit had its
            // deeper contents pruned by `max_depth`. An empty boundary directory
            // prunes nothing, so it must not flip the truncation flag.
            if entry.depth() == max_depth
                && entry.file_type().is_dir()
                && has_children(entry.path())
            {
                depth_limited = true;
            }

            if entry.file_type().is_file() && has_gguf_extension(entry.path()) {
                if let Some(model) = build_model(entry.path()) {
                    out.models.push(model);
                    if out.models.len() >= max_results {
                        out.truncation = Some(TruncationReason::MaxResults);
                        break 'dirs;
                    }
                }
            }
        }
    }

    if out.truncation.is_none() && depth_limited {
        out.truncation = Some(TruncationReason::MaxDepth);
    }
    out
}

/// `filter_entry` predicate: keep everything except hidden directories/files
/// below the root (a leading `.` in the name). The root itself (depth 0) is
/// always kept even if its name begins with `.`.
fn is_not_hidden(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    !entry.file_name().to_str().map(|name| name.starts_with('.')).unwrap_or(false)
}

fn describe_walk_error(err: &walkdir::Error) -> String {
    let path =
        err.path().map(|p| p.display().to_string()).unwrap_or_else(|| "<unknown>".to_string());
    if let Some(ancestor) = err.loop_ancestor() {
        return format!("Symlink cycle detected: {path} loops back to {}", ancestor.display());
    }
    match err.io_error().map(std::io::Error::kind) {
        Some(std::io::ErrorKind::PermissionDenied) => format!("Permission denied: {path}"),
        _ => format!("Cannot read {path}: {err}"),
    }
}

/// Whether a directory has at least one entry (used to distinguish a real
/// depth-limit prune from an empty boundary directory).
fn has_children(path: &Path) -> bool {
    std::fs::read_dir(path).map(|mut entries| entries.next().is_some()).unwrap_or(false)
}

fn has_gguf_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("gguf"))
        .unwrap_or(false)
}

fn build_model(path: &Path) -> Option<DiscoveredModel> {
    let metadata = std::fs::metadata(path).ok()?;
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let filename = path.file_name()?.to_string_lossy().into_owned();

    let modified =
        metadata.modified().ok().map(|t: SystemTime| DateTime::<Utc>::from(t).to_rfc3339());

    Some(DiscoveredModel {
        id: path_id(&canonical),
        path: canonical.to_string_lossy().into_owned(),
        quantization_hint: quantization_hint(&filename),
        architecture_hint: architecture_hint(&filename),
        filename,
        size_bytes: metadata.len(),
        modified,
    })
}

/// Stable id: hex SHA-256 of the canonical path, truncated to a prefix.
fn path_id(canonical: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(ID_PREFIX_LEN);
    for byte in digest.iter() {
        if hex.len() >= ID_PREFIX_LEN {
            break;
        }
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.truncate(ID_PREFIX_LEN);
    hex
}

// ---------------------------------------------------------------------------
// Filename heuristics (pure; unit-tested directly).
// ---------------------------------------------------------------------------

/// Known quantization tokens, longest-first so `Q4_K_M` wins over `Q4_0`.
const QUANTIZATIONS: &[&str] = &[
    "IQ1_S", "IQ2_XXS", "IQ2_XS", "IQ3_XXS", "IQ3_S", "IQ4_NL", "IQ4_XS", "Q2_K", "Q3_K_S",
    "Q3_K_M", "Q3_K_L", "Q4_K_S", "Q4_K_M", "Q5_K_S", "Q5_K_M", "Q6_K", "Q4_0", "Q4_1", "Q5_0",
    "Q5_1", "Q8_0", "BF16", "F16", "F32",
];

/// Known architecture tokens, longest-first so e.g. `codellama` wins over
/// `llama` and `qwen2` over `qwen`.
const ARCHITECTURES: &[&str] = &[
    "tinyllama",
    "codellama",
    "stablelm",
    "starcoder",
    "deepseek",
    "openhermes",
    "mixtral",
    "mistral",
    "gemma2",
    "falcon",
    "vicuna",
    "qwen2",
    "llama",
    "gemma",
    "phi3",
    "yi",
    "qwen",
    "phi",
    "mpt",
    "gpt",
    "solar",
];

/// Extract a quantization hint from a GGUF filename (e.g. `Q4_K_M`), or
/// `"unknown"`. Case-insensitive.
pub(crate) fn quantization_hint(filename: &str) -> String {
    let upper = filename.to_ascii_uppercase();
    longest_match(&upper, QUANTIZATIONS).unwrap_or_else(|| "unknown".to_string())
}

/// Extract an architecture hint from a GGUF filename (e.g. `llama`), or
/// `"unknown"`. Case-insensitive.
pub(crate) fn architecture_hint(filename: &str) -> String {
    let lower = filename.to_ascii_lowercase();
    longest_match(&lower, ARCHITECTURES).unwrap_or_else(|| "unknown".to_string())
}

/// Return the longest token from `candidates` contained in `haystack`.
/// Candidates are compared as-is, so callers normalize case beforehand.
fn longest_match(haystack: &str, candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .filter(|token| haystack.contains(**token))
        .max_by_key(|token| token.len())
        .map(|token| (*token).to_string())
}
