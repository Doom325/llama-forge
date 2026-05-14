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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerSummary {}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Server error")]
    Failed,
}

#[derive(Debug, Clone)]
pub struct QueueTicket {}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueueSnapshot {}

#[derive(Debug, Error)]
pub enum AdmissionError {
    #[error("Queue full")]
    QueueFull,
}

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("Queue error")]
    Failed,
}

#[derive(Debug, Clone)]
pub struct IncomingRequest {}

#[derive(Debug, Error)]
pub enum MetricsError {
    #[error("Metrics error")]
    Failed,
}

/// Open trait. ConfigReader
pub trait ConfigReader: Send + Sync {}

/// Open trait. ConfigWriter
#[async_trait::async_trait]
pub trait ConfigWriter: Send + Sync {
    async fn reload(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Open trait. ServerControl
#[async_trait::async_trait]
pub trait ServerControl: Send + Sync {
    async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError>;
    async fn start_server(&self, id: &crate::ServerId) -> Result<(), ServerError>;
    async fn stop_server(&self, id: &crate::ServerId) -> Result<(), ServerError>;
}

/// Open trait. RouteValidator
pub trait RouteValidator: Send + Sync {
    fn validate_routable(&self, req: &IncomingRequest) -> Result<(), AdmissionError>;
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
