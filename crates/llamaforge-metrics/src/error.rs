//! Error types

use std::time::Duration;
use thiserror::Error;

/// Structured error for storage operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error("storage unavailable: {message}")]
    Unavailable { message: String },

    #[error("operation timed out after {elapsed:?}")]
    Timeout { elapsed: Duration },

    #[error("internal storage error: {source}")]
    Internal { source: Box<dyn std::error::Error + Send + Sync> },
}

/// Error during audit record send.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AuditSendError {
    #[error("audit channel closed (writer thread dead)")]
    ChannelClosed,

    #[error("audit send timed out (10s) — record dropped")]
    Timeout,

    #[error("audit semaphore exhausted")]
    SemaphoreExhausted,
}

/// Error converting between SQLite rows and domain types.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RowConversionError {
    #[error("failed to parse JSON: {0}")]
    ParseJson(#[from] serde_json::Error),

    #[error("failed to parse datetime: {0}")]
    ParseDate(#[from] chrono::ParseError),
}
