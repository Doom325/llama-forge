use crate::{RequestorId, ServerId};
use bytes::Bytes;
use futures_core::Stream;
use std::pin::Pin;
use thiserror::Error;

/// Port traits defining boundaries between components.
///
/// See design doc Section 2.2 and 2.3.
/// A token representing a dispatched request.
pub trait DispatchToken: Send + Sync {
    // Methods to be defined by Component 06.
}

/// A stream of bytes returned from a dispatched request.
pub type ResponseStream = Pin<Box<dyn Stream<Item = Result<Bytes, DispatchError>> + Send>>;

/// Dispatches a request to a server.
///
/// See design doc Section 2.2.
#[async_trait::async_trait]
pub trait SlotDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        server_id: &ServerId,
        // TODO: Prefer a typed payload instead of `Arc<dyn Any>` when payload types are strictly defined
        request: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    ) -> Result<(ResponseStream, Box<dyn DispatchToken>), DispatchError>;
}

/// Tracks in-flight request counts for admission control.
///
/// See design doc Section 2.2.
pub trait AdmissionAccounting: Send + Sync {
    fn record_completion(&self, requestor: &RequestorId);
    fn record_dequeue(&self, requestor: &RequestorId, reason: DequeueReason);
}

/// Notifies the scheduler when a slot becomes available.
///
/// See design doc Section 2.2.
pub trait SlotNotifier: Send + Sync {
    fn notify_slot_available(&self);
}

/// Provides the true count of active (dispatched) requests per requestor.
///
/// Used for periodic reconciliation sweeps. See design doc Section 3.10.
pub trait ActiveRequestProvider: Send + Sync {
    fn active_requests(&self) -> std::collections::HashMap<RequestorId, u32>;
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("Dispatch failed")]
    Failed,
}

/// Reason for a request being dequeued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DequeueReason {
    Dispatched,
    Cancelled,
    TimedOut,
    ShuttingDown,
}

// --- Added for Transport Layer (PR1) ---

/// Summary of a server's state.
/// See design doc §5.3.1.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerSummary {
    /// Server unique ID. See design doc §5.3.1.
    pub id: crate::ServerId,
    /// Model alias loaded by this server. See design doc §5.3.1.
    pub alias: crate::ModelAlias,
    /// Lifecycle state of the server. See design doc §5.3.1.
    pub state: crate::ServerLifecycle,
    /// Port the server listens on. See design doc §5.3.1.
    pub port: u16,
    /// Total slots on the server. See design doc §5.3.1.
    pub slots_total: u32,
    /// Currently busy slots. See design doc §5.3.1.
    pub slots_busy: u32,
    /// When the server first started. See design doc §5.3.1.
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    /// When the server was last restarted. See design doc §5.3.1.
    pub last_restart_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Server not found: {id}")]
    NotFound { id: crate::ServerId },
    #[error("Server already running: {id}")]
    AlreadyRunning { id: crate::ServerId },
    #[error("Server spawn failed: {reason}")]
    SpawnFailed { reason: String },
    #[error("Server unreachable: {id}")]
    Unreachable { id: crate::ServerId },
}

/// A token representing a queued request.
/// Holds backpressure for cancellation.
/// See design doc §5.4.3.
#[derive(Debug)]
pub struct QueueTicket {
    /// Request ID. See design doc §5.4.3.
    pub request_id: crate::RequestId,
    /// Time when the request was admitted to the queue. See design doc §5.4.3.
    pub admitted_at: chrono::DateTime<chrono::Utc>,
    /// Token to signal cancellation of the request. See design doc §5.4.3.
    pub cancel_token: tokio_util::sync::CancellationToken,
    /// Stream of events from the dispatch process. See design doc §5.4.3.
    pub events: tokio::sync::mpsc::Receiver<DispatchEvent>,
}

/// Events emitted during request dispatch.
/// See design doc §5.4.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchEvent {
    Queued { position: u32 },
    Dispatched { server_id: crate::ServerId, slot_id: crate::SlotId },
    FirstToken { ttft_ms: u64 },
    Completed { tokens: u32 },
    Failed { error: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueueSnapshot {
    pub depth: u32,
    pub estimated_throughput_req_sec: f32,
}

#[derive(Debug, Error)]
pub enum AdmissionError {
    #[error("Queue full")]
    QueueFull,
    #[error("Cancelled")]
    Cancelled,
    #[error("Invalid request: {reason}")]
    InvalidRequest { reason: String },
    #[error("Rate limited. Retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u32 },
    /// No configured server can serve the requested model alias /
    /// capability set. Distinct from `QueueFull`: this is a permanent
    /// rejection for the given model name, not a transient
    /// backpressure signal.
    #[error("No server for model: {model}")]
    NoServerForModel { model: crate::ModelAlias },
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("Request not found: {id}")]
    NotFound { id: crate::RequestId },
    #[error("Already cancelled")]
    AlreadyCancelled,
    #[error("Internal error: {detail}")]
    Internal { detail: String },
}

/// An incoming request to be queued and dispatched.
/// See design doc §5.4.2.
#[derive(Debug, Clone)]
pub struct IncomingRequest {
    /// Unique ID for this request. See design doc §5.4.2.
    pub request_id: crate::RequestId,
    /// ID of the caller/user. See design doc §5.4.2.
    pub requestor_id: crate::RequestorId,
    /// Requestor class for quota rules. See design doc §5.4.2.
    pub requestor_class: crate::RequestorClass,
    /// Model targeted by this request. See design doc §5.4.2.
    pub model_target: crate::ModelTarget,
    /// Special capabilities required (e.g. multimodal). See design doc §5.4.2.
    pub capabilities: crate::RequestCapabilities,
    /// Messages making up the chat history. See design doc §5.4.2.
    pub messages: Vec<crate::Message>,
    /// Generation parameters (temp, max_tokens, etc). See design doc §5.4.2.
    pub params: crate::GenerationParams,
    /// Whether the response should be streamed. See design doc §5.4.2.
    pub stream: bool,
    /// Optional session affinity ID. See design doc §5.4.2.
    pub session_id: Option<crate::SessionId>,
    /// When the request was submitted. See design doc §5.4.2.
    pub submitted_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("Store unavailable")]
    StoreUnavailable,
    #[error("Internal error: {detail}")]
    Internal { detail: String },
}

/// Open trait. ServerControl
#[async_trait::async_trait]
pub trait ServerControl: Send + Sync {
    async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError>;
    async fn start_server(&self, id: &crate::ServerId) -> Result<(), ServerError>;
    async fn stop_server(&self, id: &crate::ServerId) -> Result<(), ServerError>;
}

/// Open trait. RouteValidator
///
/// Takes the destructured `(model, capabilities)` tuple rather than the full
/// `IncomingRequest`: the transport layer already has these fields and the
/// validator only ever consumes those two. Callers destructure at the call
/// site (one line) instead of forcing an adapter to wrap the whole request.
pub trait RouteValidator: Send + Sync {
    fn validate_routable(
        &self,
        model: &crate::ModelTarget,
        capabilities: &crate::RequestCapabilities,
    ) -> Result<(), AdmissionError>;
}

/// Open trait. QueueSubmitter
pub trait QueueSubmitter: Send + Sync {
    fn submit(&self, req: IncomingRequest) -> Result<QueueTicket, AdmissionError>;
    fn cancel(&self, id: &crate::RequestId) -> Result<(), QueueError>;
    fn status(&self) -> Result<QueueSnapshot, QueueError>;
}

/// Open trait. MetricsExporter
pub trait MetricsExporter: Send + Sync {
    fn gather_text(&self) -> String;
}
