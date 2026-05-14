use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("Bootstrap error: {0}")]
    Bootstrap(String),
    #[error("Server task failed: {0}")]
    ServerTaskFailed(#[source] tokio::task::JoinError),
}

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("Invalid body: {0}")]
    InvalidBody(String),
    #[error("Auth failure")]
    AuthFailure,
    #[error("Model not found")]
    ModelNotFound,
    #[error("Queue full")]
    QueueFull,
    #[error("Backend error")]
    BackendError,
    #[error("Timeout")]
    Timeout,
    #[error("Internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicStreamPhase {
    Init,
    PreambleSent,
    ContentBlock,
    BlockStopped,
    MessageDone,
    Complete,
}

#[derive(Debug, Error)]
pub enum AnthropicProtocolError {
    #[error("Invalid transition from {from:?} to {to:?}")]
    InvalidTransition { from: AnthropicStreamPhase, to: AnthropicStreamPhase },
}
