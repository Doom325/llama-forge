//! Config Manager error types.

use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error("config file not found: {path}")]
    NotFound { path: PathBuf },

    #[error("config file is empty (possible crash during writeback): {path}")]
    EmptyFile { path: PathBuf },

    #[error("parse error in {path}: {source}")]
    ParseError {
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("validation failed: {message}")]
    ValidationError { message: String },

    #[error("I/O error reading {path}: {source}")]
    IoError {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub enum ConfigReloadError {
    #[error("reload failed: {0}")]
    LoadError(#[from] ConfigLoadError),

    #[error("diff computation failed: {message}")]
    DiffError { message: String },
}

#[derive(Debug, Error)]
pub enum WritebackError {
    #[error("config file is read-only (Kubernetes ConfigMap?): {path}")]
    ReadOnly { path: PathBuf },

    #[error("flock acquisition timed out after {elapsed:?}")]
    LockTimeout { elapsed: Duration },

    #[error("atomic rename failed (EXDEV), fallback also failed: {source}")]
    WriteFailed { source: std::io::Error },

    #[error("serialization failed: {source}")]
    SerializeError {
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}
