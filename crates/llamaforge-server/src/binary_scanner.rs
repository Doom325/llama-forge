//! `llama-server` binary scanner for the Setup Wizard.
//!
//! Searches `$PATH` plus common install locations for a `llama-server`
//! executable, then runs `llama-server --version` (5 s timeout) to extract the
//! version string and supported backends. Implements `08_SETUP_WIZARD.md`
//! Section 11.2 (Binary Scanner API contract) and the Appendix implementation
//! notes.
//!
//! A binary that is present but does not answer `--version` within the timeout
//! (or exits non-zero) is reported as `found: true` with
//! `version: "found but unresponsive to --version"`, so the user can still
//! confirm or reject it — it is never silently downgraded to "not found".
//!
//! Pure domain logic: no axum, no `tracing`. The PR5 handler adds the
//! `"version": 1` envelope and span instrumentation.
//!
//! Note on the wire field name: the found variant's version serializes as
//! `binary_version` (Section 11.2 JSON example), **not** `version`. Every
//! `/api/setup/*` response flattens the `"version": 1` API envelope into the
//! top-level object, and binary scan is the only payload carrying a
//! version-like field — naming it `version` would collide with the envelope
//! integer once the PR5 handler adds it. (The Appendix TS type and Section 11.2
//! prose say `version`; that is the buggy side of the doc's own contradiction,
//! corrected alongside this change.)

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

/// Executable we are looking for.
const BINARY_NAME: &str = "llama-server";

/// Timeout for the `--version` probe.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// Reported version when the binary is present but unresponsive to `--version`.
const UNRESPONSIVE: &str = "found but unresponsive to --version";

/// Common install locations searched in addition to `$PATH`.
const COMMON_LOCATIONS: &[&str] = &["/usr/local/bin", "/opt/homebrew/bin", "/usr/bin"];

/// Backend keywords recognized in `--version` output.
const KNOWN_BACKENDS: &[&str] = &["cuda", "metal", "vulkan", "rocm", "hip", "blas", "cpu"];

// ---------------------------------------------------------------------------
// Request / response types.
// ---------------------------------------------------------------------------

/// A binary scan request (Section 11.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryScanRequest {
    /// Extra directories to search first, ahead of `$PATH` and common
    /// locations.
    #[serde(default)]
    pub extra_paths: Vec<String>,
}

/// Result of a binary scan — a discriminated union on `found` (Section 11.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BinaryScanResult {
    Found {
        /// Always `true`; present so the JSON matches the `found` discriminator.
        found: BinaryFoundFlag<true>,
        path: String,
        /// The binary's own version string. Serialized as `binary_version` to
        /// avoid colliding with the `"version": 1` envelope (see module note).
        #[serde(rename = "binary_version")]
        version: String,
        backends: Vec<String>,
    },
    NotFound {
        /// Always `false`.
        found: BinaryFoundFlag<false>,
        searched_paths: Vec<String>,
    },
}

impl BinaryScanResult {
    fn found(path: String, version: String, backends: Vec<String>) -> Self {
        Self::Found { found: BinaryFoundFlag, path, version, backends }
    }

    fn not_found(searched_paths: Vec<String>) -> Self {
        Self::NotFound { found: BinaryFoundFlag, searched_paths }
    }
}

/// Const-generic marker that serializes to the boolean literal `FOUND`, so the
/// `found` discriminator is fixed per variant rather than a free-floating bool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinaryFoundFlag<const FOUND: bool>;

impl<const FOUND: bool> Serialize for BinaryFoundFlag<FOUND> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(FOUND)
    }
}

impl<'de, const FOUND: bool> Deserialize<'de> for BinaryFoundFlag<FOUND> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = bool::deserialize(deserializer)?;
        if value == FOUND {
            Ok(BinaryFoundFlag)
        } else {
            Err(serde::de::Error::custom("found discriminator mismatch"))
        }
    }
}

/// Validation failure mapped to a 400 by the handler.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BinaryScanError {
    #[error("invalid extra_path {path:?}: {reason}")]
    InvalidExtraPath { path: String, reason: String },
}

// ---------------------------------------------------------------------------
// Public API.
// ---------------------------------------------------------------------------

/// Search for `llama-server` and probe its version.
pub async fn scan_binary(request: BinaryScanRequest) -> Result<BinaryScanResult, BinaryScanError> {
    let search_dirs = build_search_dirs(&request.extra_paths)?;

    match first_executable(&search_dirs) {
        Some(path) => {
            let (version, backends) = match probe_version(&path).await {
                VersionProbe::Ok { version, backends } => (version, backends),
                VersionProbe::Unresponsive => (UNRESPONSIVE.to_string(), Vec::new()),
            };
            Ok(BinaryScanResult::found(path.to_string_lossy().into_owned(), version, backends))
        }
        None => Ok(BinaryScanResult::not_found(
            search_dirs.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        )),
    }
}

/// Assemble the ordered, de-duplicated search directory list:
/// `extra_paths` (validated) → `$PATH` entries → common locations.
fn build_search_dirs(extra_paths: &[String]) -> Result<Vec<PathBuf>, BinaryScanError> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let push_unique = |dir: PathBuf, dirs: &mut Vec<PathBuf>| {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };

    for raw in extra_paths {
        if raw.contains('\0') {
            return Err(BinaryScanError::InvalidExtraPath {
                path: raw.clone(),
                reason: "path contains a null byte".to_string(),
            });
        }
        if raw.trim().is_empty() {
            return Err(BinaryScanError::InvalidExtraPath {
                path: raw.clone(),
                reason: "path is empty".to_string(),
            });
        }
        push_unique(PathBuf::from(raw), &mut dirs);
    }

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            push_unique(dir, &mut dirs);
        }
    }

    for location in COMMON_LOCATIONS {
        push_unique(PathBuf::from(location), &mut dirs);
    }
    if let Some(home) = std::env::var_os("HOME") {
        push_unique(PathBuf::from(home).join(".local/bin"), &mut dirs);
    }

    Ok(dirs)
}

/// Return the first `<dir>/llama-server` that exists and is executable.
fn first_executable(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().map(|dir| dir.join(BINARY_NAME)).find(|candidate| is_executable_file(candidate))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// Outcome of the `--version` probe.
enum VersionProbe {
    Ok { version: String, backends: Vec<String> },
    Unresponsive,
}

/// Run `<path> --version` (5 s timeout) and parse the output. A timeout,
/// non-zero exit, spawn failure, or empty output is reported as unresponsive.
async fn probe_version(path: &Path) -> VersionProbe {
    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return VersionProbe::Unresponsive,
    };

    let output = match tokio::time::timeout(VERSION_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) if output.status.success() => output,
        _ => return VersionProbe::Unresponsive,
    };

    // llama.cpp prints `--version` to stderr; accept either stream.
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push('\n');
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    match parse_version(&combined) {
        Some(version) => VersionProbe::Ok { version, backends: parse_backends(&combined) },
        None => VersionProbe::Unresponsive,
    }
}

// ---------------------------------------------------------------------------
// Output parsing (pure).
// ---------------------------------------------------------------------------

/// Extract a version string from `--version` output: the first non-empty line,
/// with a leading `version:` / `version` label stripped.
pub(crate) fn parse_version(output: &str) -> Option<String> {
    let line = output.lines().map(str::trim).find(|l| !l.is_empty())?;
    let trimmed = line
        .strip_prefix("version:")
        .or_else(|| line.strip_prefix("version"))
        .unwrap_or(line)
        .trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Detect supported backends by scanning the output for known keywords.
pub(crate) fn parse_backends(output: &str) -> Vec<String> {
    let lower = output.to_ascii_lowercase();
    KNOWN_BACKENDS
        .iter()
        .filter(|backend| lower.contains(**backend))
        .map(|backend| (*backend).to_string())
        .collect()
}
