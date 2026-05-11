//! Error types for Server Manager
//! See design doc Section 2.4.

use llamaforge_config::{GpuId, ServerId};

/// Errors that can occur within the Server Manager component.
/// See design doc Section 2.4.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ServerManagerError {
    #[error("Server not found: {id}")]
    ServerNotFound { id: ServerId },

    #[error("Insufficient VRAM on GPU {gpu:?}: needed {needed_mb} MB, free {free_mb} MB")]
    InsufficientVram { gpu: GpuId, needed_mb: u64, free_mb: u64 },

    #[error("Server {id} failed to start: {reason}")]
    StartFailed { id: ServerId, reason: String },

    #[error("Server {id} exceeded maximum restart attempts ({max})")]
    MaxRestartsExceeded { id: ServerId, max: u32 },

    #[error("Failed to spawn process: {source}")]
    SpawnError { source: std::io::Error },

    #[error("Invalid configuration: {message}")]
    ConfigInvalid { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_formatting() {
        let err = ServerManagerError::ServerNotFound { id: ServerId("srv1".into()) };
        assert_eq!(err.to_string(), "Server not found: srv1");

        let err = ServerManagerError::ConfigInvalid { message: "bad path".into() };
        assert_eq!(err.to_string(), "Invalid configuration: bad path");
    }
}
