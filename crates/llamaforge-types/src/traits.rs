use crate::{ModelTarget, ModelAlias, RequestorId, ServerId};
use bytes::Bytes;
use futures_core::Stream;
use std::pin::Pin;
use thiserror::Error;

/// Port traits defining boundaries between components.
///
/// See design doc Section 2.2 and 2.3.
/// Checks if there is any configured capacity for a given target.
///
/// See design doc Section 2.2.
pub trait CapacityChecker: Send + Sync {
    /// Returns true if at least one server is configured that could potentially
    /// serve this target model. Does not check if the server is currently idle.
    fn has_capacity(&self, target: &ModelTarget) -> bool;
}

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

/// Selects the best server for a given request.
///
/// See design doc Section 2.2.
pub trait ServerSelector: Send + Sync {
    fn select_server(&self, target: &ModelTarget) -> Option<RoutingDecision>;
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

/// Result of a routing decision.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub server_id: ServerId,
    pub model_alias: ModelAlias,
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
