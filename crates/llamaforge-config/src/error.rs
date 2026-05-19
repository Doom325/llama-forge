//! Config Manager error types.
//!
//! See design doc Section 2.7 for error definitions and propagation.

use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

/// Error during initial configuration load.
/// See design doc Section 2.7.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigLoadError {
    /// Config file was not found.
    #[error("config file not found: {path}")]
    NotFound { path: PathBuf },

    /// Config file exists but is empty (possible writeback crash).
    #[error("config file is empty (possible crash during writeback): {path}")]
    EmptyFile { path: PathBuf },

    /// Syntax error in the TOML file.
    #[error("parse error in {path}: {source}")]
    ParseError { path: PathBuf, source: Box<dyn std::error::Error + Send + Sync> },

    /// Business rule validation failed (e.g. port conflict).
    #[error("validation failed: {message}")]
    ValidationError { message: String },

    /// Generic filesystem I/O error.
    #[error("I/O error reading {path}: {source}")]
    IoError { path: PathBuf, source: std::io::Error },
}

/// Error during hot-reload triggered by SIGHUP or API.
/// See design doc Section 2.7.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigReloadError {
    /// Reload failed during the load/parse stage.
    #[error("reload failed: {0}")]
    LoadError(#[from] ConfigLoadError),

    /// Diff computation failed (unexpected schema drift).
    #[error("diff computation failed: {message}")]
    DiffError { message: String },

    /// The blocking task responsible for loading the configuration panicked.
    #[error("spawn_blocking task panicked: {message}")]
    LoadTaskJoin { message: String },

    /// The on-disk file changed a server field that requires a
    /// restart to apply. v1.0 doesn't ship a staged-apply protocol;
    /// `ConfigHandle::reload` aborts before swapping the snapshot or
    /// broadcasting a diff, so callers see no half-applied state.
    /// See `docs/v1.1-deferred.md` "Staged-apply protocol".
    #[error("restart required for fields: {fields:?}")]
    RestartRequired { fields: Vec<crate::diff::ServerConfigField> },

    /// A global setting was changed that requires a restart.
    #[error("restart required for global setting: {message}")]
    GlobalRestartRequired { message: String },
}

/// Error during crash-safe writeback.
/// See design doc Section 2.7.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WritebackError {
    /// Target filesystem is read-only (e.g. Kubernetes ConfigMap).
    #[error("config file is read-only (Kubernetes ConfigMap?): {path}")]
    ReadOnly { path: PathBuf },

    /// Acquisition of the file lock timed out.
    #[error("flock acquisition timed out after {elapsed:?}")]
    LockTimeout { elapsed: Duration },

    /// Atomic rename failed (EXDEV) and fallback also failed.
    #[error("atomic rename failed (EXDEV), fallback also failed: {source}")]
    WriteFailed { source: std::io::Error },

    /// Failed to serialize the config to TOML.
    #[error("serialization failed: {source}")]
    SerializeError { source: Box<dyn std::error::Error + Send + Sync> },

    /// Target server was not found.
    #[error("target server not found in configuration")]
    ServerNotFound,

    /// The blocking task responsible for persisting the configuration panicked.
    #[error("spawn_blocking task panicked: {message}")]
    PersistTaskJoin { message: String },

    /// Writeback is not yet implemented.
    #[error("writeback is not yet implemented")]
    NotImplemented,
}
