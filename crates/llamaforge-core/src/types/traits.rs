use crate::types::{RequestorId, ServerId};
use bytes::Bytes;
use futures_core::Stream;
use std::pin::Pin;
use thiserror::Error;

/// Port traits defining boundaries between components.
///
/// See design doc Section 2.2 and 2.3.
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
    ) -> Result<(ResponseStream, Box<dyn std::any::Any + Send + Sync>), DispatchError>;
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

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("Queue full")]
    QueueFull,
    #[error("No server for model: {model}")]
    NoServerForModel { model: String },
    #[error("Rate limited, retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u32 },
    #[error("Dispatch failed: {reason}")]
    Internal { reason: String },
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
    pub id: crate::types::ServerId,
    /// Model alias loaded by this server. See design doc §5.3.1.
    pub alias: crate::types::ModelAlias,
    /// Lifecycle state of the server. See design doc §5.3.1.
    pub state: crate::types::ServerLifecycle,
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
    NotFound { id: crate::types::ServerId },
    #[error("Server already running: {id}")]
    AlreadyRunning { id: crate::types::ServerId },
    #[error("Server spawn failed: {reason}")]
    SpawnFailed { reason: String },
    #[error("Server unreachable: {id}")]
    Unreachable { id: crate::types::ServerId },
}

/// A token representing a queued request.
/// Holds backpressure for cancellation.
/// See design doc §5.4.3.
#[derive(Debug)]
pub struct QueueTicket {
    /// Request ID. See design doc §5.4.3.
    pub request_id: crate::types::RequestId,
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
    Dispatched { server_id: crate::types::ServerId, slot_id: crate::types::SlotId },
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
    NoServerForModel { model: crate::types::ModelAlias },
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("Request not found: {id}")]
    NotFound { id: crate::types::RequestId },
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
    pub request_id: crate::types::RequestId,
    /// ID of the caller/user. See design doc §5.4.2.
    pub requestor_id: crate::types::RequestorId,
    /// Requestor class for quota rules. See design doc §5.4.2.
    pub requestor_class: crate::types::RequestorClass,
    /// Model targeted by this request. See design doc §5.4.2.
    pub model_target: crate::types::ModelTarget,
    /// Special capabilities required (e.g. multimodal). See design doc §5.4.2.
    pub capabilities: crate::types::RequestCapabilities,
    /// Messages making up the chat history. See design doc §5.4.2.
    pub messages: Vec<crate::types::Message>,
    /// Generation parameters (temp, max_tokens, etc). See design doc §5.4.2.
    pub params: crate::types::GenerationParams,
    /// Whether the response should be streamed. See design doc §5.4.2.
    pub stream: bool,
    /// Optional session affinity ID. See design doc §5.4.2.
    pub session_id: Option<crate::types::SessionId>,
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
    async fn start_server(&self, id: &crate::types::ServerId) -> Result<(), ServerError>;
    async fn stop_server(&self, id: &crate::types::ServerId) -> Result<(), ServerError>;
    
    /// Returns the current cluster state (0=Unknown, 1=Healthy, 2=Degraded, 3=Failed).
    fn cluster_state(&self) -> u8;
    
    /// Returns true if the gateway has completed initial startup.
    fn is_gateway_initialized(&self) -> bool;
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
        model: &crate::types::ModelTarget,
        capabilities: &crate::types::RequestCapabilities,
    ) -> Result<(), AdmissionError>;
}

/// Open trait. QueueSubmitter
pub trait QueueSubmitter: Send + Sync {
    fn submit(&self, req: IncomingRequest) -> Result<QueueTicket, AdmissionError>;
    fn cancel(&self, id: &crate::types::RequestId) -> Result<(), QueueError>;
    fn status(&self) -> Result<QueueSnapshot, QueueError>;
}

/// Open trait. MetricsExporter
pub trait MetricsExporter: Send + Sync {
    fn gather_text(&self) -> String;
}
