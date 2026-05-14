use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
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

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match &self {
            ApiError::InvalidBody(msg) => (StatusCode::BAD_REQUEST, "invalid_body", msg.clone()),
            ApiError::AuthFailure => (StatusCode::UNAUTHORIZED, "auth_failure", "Authentication failed".to_string()),
            ApiError::ModelNotFound => (StatusCode::NOT_FOUND, "model_not_found", "Model not found".to_string()),
            ApiError::QueueFull => (StatusCode::TOO_MANY_REQUESTS, "queue_full", "Queue is full".to_string()),
            ApiError::BackendError => (StatusCode::BAD_GATEWAY, "backend_error", "Backend error".to_string()),
            ApiError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "timeout", "Request timed out".to_string()),
            ApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg.clone()),
        };

        let body = Json(json!({
            "error": error_code,
            "message": message,
        }));

        let mut response = (status, body).into_response();

        if let ApiError::QueueFull = self {
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from_static("30"),
            );
        }

        response
    }
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
