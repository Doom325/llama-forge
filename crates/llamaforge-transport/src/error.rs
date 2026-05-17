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
    QueueFull { retry_after_secs: u32 },
    #[error("Backend error")]
    BackendError,
    #[error("Timeout")]
    Timeout,
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Fallback `Retry-After` (seconds) for the `QueueFull` branch when
/// the conversion has no queue context. The HTTP handlers override
/// this with `compute_retry_after` (depth / throughput) before
/// returning — see `crate::http::handle_openai_request`. Callers
/// using the `?` operator on `AdmissionError` get this conservative
/// default instead of `0`.
const QUEUE_FULL_FALLBACK_RETRY_SECS: u32 = 30;

impl From<llamaforge_types::traits::AdmissionError> for ApiError {
    fn from(err: llamaforge_types::traits::AdmissionError) -> Self {
        match err {
            llamaforge_types::traits::AdmissionError::QueueFull => {
                ApiError::QueueFull { retry_after_secs: QUEUE_FULL_FALLBACK_RETRY_SECS }
            }
            llamaforge_types::traits::AdmissionError::RateLimited { retry_after_secs } => {
                ApiError::QueueFull { retry_after_secs }
            }
            llamaforge_types::traits::AdmissionError::InvalidRequest { reason } => {
                ApiError::InvalidBody(reason)
            }
            llamaforge_types::traits::AdmissionError::Cancelled => {
                ApiError::Internal("Request cancelled during admission".to_string())
            }
            llamaforge_types::traits::AdmissionError::NoServerForModel { .. } => {
                ApiError::ModelNotFound
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match &self {
            ApiError::InvalidBody(msg) => (StatusCode::BAD_REQUEST, "invalid_body", msg.clone()),
            ApiError::AuthFailure => {
                (StatusCode::UNAUTHORIZED, "auth_failure", "Authentication failed".to_string())
            }
            ApiError::ModelNotFound => {
                (StatusCode::NOT_FOUND, "model_not_found", "Model not found".to_string())
            }
            ApiError::QueueFull { .. } => {
                (StatusCode::TOO_MANY_REQUESTS, "queue_full", "Queue is full".to_string())
            }
            ApiError::BackendError => {
                (StatusCode::BAD_GATEWAY, "backend_error", "Backend error".to_string())
            }
            ApiError::Timeout => {
                (StatusCode::GATEWAY_TIMEOUT, "timeout", "Request timed out".to_string())
            }
            ApiError::Internal(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg.clone())
            }
        };

        let body = Json(json!({
            "error": error_code,
            "message": message,
        }));

        let mut response = (status, body).into_response();

        if let ApiError::QueueFull { retry_after_secs } = self {
            if let Ok(header_val) = axum::http::HeaderValue::from_str(&retry_after_secs.to_string())
            {
                response.headers_mut().insert(axum::http::header::RETRY_AFTER, header_val);
            }
        }

        response
    }
}

impl From<ApiError> for tonic::Status {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::InvalidBody(msg) => tonic::Status::invalid_argument(msg),
            ApiError::AuthFailure => tonic::Status::unauthenticated("Authentication failed"),
            ApiError::ModelNotFound => tonic::Status::not_found("Model not found"),
            ApiError::QueueFull { .. } => tonic::Status::resource_exhausted("Queue is full"),
            ApiError::BackendError => tonic::Status::internal("Backend error"),
            ApiError::Timeout => tonic::Status::deadline_exceeded("Request timed out"),
            ApiError::Internal(msg) => tonic::Status::internal(msg),
        }
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
