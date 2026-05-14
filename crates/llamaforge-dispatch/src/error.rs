use std::error::Error;
use std::fmt;
use thiserror::Error;

/// HTTP Status code domain wrapper
///
/// See design doc Section 2.6
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpStatus(pub u16);

impl HttpStatus {
    pub fn is_server_error(&self) -> bool {
        self.0 >= 500 && self.0 < 600
    }

    pub fn is_client_error(&self) -> bool {
        self.0 >= 400 && self.0 < 500
    }
}

impl fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The phase during which a proxy task was cancelled.
///
/// See design doc Section 2.6
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationPhase {
    PreConnect,
    PreStream,
    Streaming,
    Cleanup,
}

impl fmt::Display for CancellationPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreConnect => write!(f, "pre-connect"),
            Self::PreStream => write!(f, "pre-stream"),
            Self::Streaming => write!(f, "streaming"),
            Self::Cleanup => write!(f, "cleanup"),
        }
    }
}

/// High-level errors during the initial dispatch connect phase.
///
/// See design doc Section 2.6.
#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(#[source] Box<dyn Error + Send + Sync>),

    #[error("Backend returned error status: {status}")]
    BackendError { status: HttpStatus },

    #[error("Request cancelled during phase: {phase}")]
    Cancelled { phase: CancellationPhase },

    #[error("Connection timed out")]
    Timeout,
}

/// Errors that can occur while streaming response chunks.
///
/// See design doc Section 2.6.
#[derive(Debug, Error)]
pub enum StreamError {
    #[error("Backend disconnected unexpectedly")]
    BackendDisconnect,

    #[error("Timed out waiting for chunk")]
    Timeout,

    #[error("Request cancelled")]
    Cancelled,

    #[error("Failed to parse chunk: {reason}")]
    ChunkParse { reason: String },

    #[error("Response channel closed")]
    ChannelClosed,

    #[error("Internal error: {0}")]
    Internal(#[source] Box<dyn Error + Send + Sync>),
}

impl From<DispatchError> for StreamError {
    fn from(e: DispatchError) -> Self {
        match e {
            DispatchError::Cancelled { .. } => StreamError::Cancelled,
            DispatchError::Timeout => StreamError::Timeout,
            other => StreamError::Internal(Box::new(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_status_helpers() {
        assert!(HttpStatus(500).is_server_error());
        assert!(!HttpStatus(500).is_client_error());

        assert!(HttpStatus(404).is_client_error());
        assert!(!HttpStatus(404).is_server_error());

        assert!(!HttpStatus(200).is_server_error());
        assert!(!HttpStatus(200).is_client_error());
    }

    #[test]
    fn test_cancellation_phase_display() {
        assert_eq!(CancellationPhase::PreConnect.to_string(), "pre-connect");
        assert_eq!(CancellationPhase::PreStream.to_string(), "pre-stream");
        assert_eq!(CancellationPhase::Streaming.to_string(), "streaming");
        assert_eq!(CancellationPhase::Cleanup.to_string(), "cleanup");
    }

    #[test]
    fn test_dispatch_error_display() {
        let err1 = DispatchError::Timeout;
        assert_eq!(err1.to_string(), "Connection timed out");

        let err2 = DispatchError::Cancelled { phase: CancellationPhase::PreConnect };
        assert_eq!(err2.to_string(), "Request cancelled during phase: pre-connect");

        let err3 = DispatchError::BackendError { status: HttpStatus(503) };
        assert_eq!(err3.to_string(), "Backend returned error status: 503");

        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        let err4 = DispatchError::ConnectionFailed(Box::new(io_err));
        assert_eq!(err4.to_string(), "Connection failed: refused");
        assert!(err4.source().is_some());
    }

    #[test]
    fn test_stream_error_display() {
        let err1 = StreamError::BackendDisconnect;
        assert_eq!(err1.to_string(), "Backend disconnected unexpectedly");

        let err2 = StreamError::Timeout;
        assert_eq!(err2.to_string(), "Timed out waiting for chunk");

        let err3 = StreamError::Cancelled;
        assert_eq!(err3.to_string(), "Request cancelled");

        let err4 = StreamError::ChunkParse { reason: "invalid utf8".into() };
        assert_eq!(err4.to_string(), "Failed to parse chunk: invalid utf8");

        let err5 = StreamError::ChannelClosed;
        assert_eq!(err5.to_string(), "Response channel closed");

        let inner_err = std::io::Error::other("broken");
        let err6 = StreamError::Internal(Box::new(inner_err));
        assert_eq!(err6.to_string(), "Internal error: broken");
        assert!(err6.source().is_some());
    }
}
