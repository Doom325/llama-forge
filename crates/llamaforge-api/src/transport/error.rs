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
    /// Surface as `404 Not Found`. Used for management lookups of a specific
    /// entity by id (e.g. `GET /api/servers/{id}` for an unknown server).
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Queue full")]
    QueueFull { retry_after_secs: u32 },
    #[error("Backend error")]
    BackendError,
    #[error("Timeout")]
    Timeout,
    /// Surface as `503 Service Unavailable` with a `Retry-After`. Used
    /// for the warming-window guard: a non-streaming response that ends
    /// without ever observing a terminal `Stop` chunk means the backend
    /// went away mid-response (no Running server, or a crash). Returning
    /// a successful empty body would hand the client confident garbage.
    #[error("Service unavailable")]
    ServiceUnavailable { retry_after_secs: u32 },
    /// Surface as `412 Precondition Failed`. Used for management
    /// operations whose semantics require a precondition the caller
    /// must satisfy (e.g. `/api/config/reload` rejecting a diff that
    /// touches restart-required server fields).
    #[error("Precondition failed: {0}")]
    PreconditionFailed(String),
    #[error("Internal error: {0}")]
    Internal(String),
    /// Surface as `403 Forbidden`. CSRF guard: a mutating `/api/*` request
    /// arrived without the `X-LlamaForge-Request` header. Browsers do not attach
    /// custom headers to cross-origin form/`no-cors` requests, so this blocks
    /// CSRF from external origins (FRONTEND_ARCHITECTURE.md §10.2).
    #[error("CSRF validation failed")]
    Csrf,
}

/// Fallback `Retry-After` (seconds) for the `QueueFull` branch when
/// the conversion has no queue context. The HTTP handlers override
/// this with `compute_retry_after` (depth / throughput) before
/// returning — see `super::http::handle_openai_request`. Callers
/// using the `?` operator on `AdmissionError` get this conservative
/// default instead of `0`.
const QUEUE_FULL_FALLBACK_RETRY_SECS: u32 = 30;

impl From<llamaforge_core::types::traits::AdmissionError> for ApiError {
    fn from(err: llamaforge_core::types::traits::AdmissionError) -> Self {
        match err {
            llamaforge_core::types::traits::AdmissionError::QueueFull => {
                ApiError::QueueFull { retry_after_secs: QUEUE_FULL_FALLBACK_RETRY_SECS }
            }
            llamaforge_core::types::traits::AdmissionError::RateLimited { retry_after_secs } => {
                ApiError::QueueFull { retry_after_secs }
            }
            llamaforge_core::types::traits::AdmissionError::InvalidRequest { reason } => {
                ApiError::InvalidBody(reason)
            }
            llamaforge_core::types::traits::AdmissionError::Cancelled => {
                ApiError::Internal("Request cancelled during admission".to_string())
            }
            llamaforge_core::types::traits::AdmissionError::NoServerForModel { .. } => {
                ApiError::ModelNotFound
            }
        }
    }
}

impl From<llamaforge_core::types::traits::DispatchError> for ApiError {
    fn from(err: llamaforge_core::types::traits::DispatchError) -> Self {
        match err {
            llamaforge_core::types::traits::DispatchError::QueueFull => {
                ApiError::QueueFull { retry_after_secs: QUEUE_FULL_FALLBACK_RETRY_SECS }
            }
            llamaforge_core::types::traits::DispatchError::RateLimited { retry_after_secs } => {
                ApiError::QueueFull { retry_after_secs }
            }
            llamaforge_core::types::traits::DispatchError::NoServerForModel { .. } => {
                ApiError::ModelNotFound
            }
            llamaforge_core::types::traits::DispatchError::Internal { reason } => {
                ApiError::Internal(reason)
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
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone()),
            ApiError::QueueFull { .. } => {
                (StatusCode::TOO_MANY_REQUESTS, "queue_full", "Queue is full".to_string())
            }
            ApiError::BackendError => {
                (StatusCode::BAD_GATEWAY, "backend_error", "Backend error".to_string())
            }
            ApiError::Timeout => {
                (StatusCode::GATEWAY_TIMEOUT, "timeout", "Request timed out".to_string())
            }
            ApiError::ServiceUnavailable { .. } => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "Backend warming up or unavailable; retry shortly".to_string(),
            ),
            ApiError::PreconditionFailed(msg) => {
                (StatusCode::PRECONDITION_FAILED, "precondition_failed", msg.clone())
            }
            ApiError::Internal(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg.clone())
            }
            ApiError::Csrf => (
                StatusCode::FORBIDDEN,
                "csrf_failure",
                "Missing or invalid X-LlamaForge-Request header".to_string(),
            ),
        };

        let body = Json(json!({
            "error": error_code,
            "message": message,
        }));

        let mut response = (status, body).into_response();

        // Retry-After header for both transient backpressure paths.
        let retry_after_secs = match self {
            ApiError::QueueFull { retry_after_secs } => Some(retry_after_secs),
            ApiError::ServiceUnavailable { retry_after_secs } => Some(retry_after_secs),
            _ => None,
        };
        if let Some(secs) = retry_after_secs {
            if let Ok(header_val) = axum::http::HeaderValue::from_str(&secs.to_string()) {
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
            ApiError::NotFound(msg) => tonic::Status::not_found(msg),
            ApiError::QueueFull { .. } => tonic::Status::resource_exhausted("Queue is full"),
            ApiError::BackendError => tonic::Status::internal("Backend error"),
            ApiError::Timeout => tonic::Status::deadline_exceeded("Request timed out"),
            ApiError::ServiceUnavailable { .. } => {
                tonic::Status::unavailable("Backend warming up or unavailable; retry shortly")
            }
            ApiError::PreconditionFailed(msg) => tonic::Status::failed_precondition(msg),
            ApiError::Internal(msg) => tonic::Status::internal(msg),
            ApiError::Csrf => tonic::Status::permission_denied("CSRF validation failed"),
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
