use crate::{ModelTarget, ModelAlias, RequestorId, ServerId};
use bytes::Bytes;
use futures_core::Stream;
use std::pin::Pin;
use thiserror::Error;

/// Port traits defining boundaries between components.
///
/// See design doc Section 2.2 and 2.3.
mod private {
    pub trait Sealed {}
    pub struct Token;
    // impl Sealed for Router is added in llamaforge-routing crate via a re-export or feature gate
    // — Router cannot be referenced from llamaforge-types without a circular dependency.
}

/// Checks if there is any configured capacity for a given target.
///
/// See design doc Section 2.2.
pub trait CapacityChecker: private::Sealed + Send + Sync {
    /// Returns true if at least one server is configured that could potentially
    /// serve this target model. Does not check if the server is currently idle.
    fn has_capacity(&self, target: &ModelTarget) -> bool;

    #[doc(hidden)]
    fn _sealed(&self) -> private::Token;
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
#[allow(async_fn_in_trait)]
pub trait SlotDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        server_id: &ServerId,
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
    fn record_dequeue(&self, requestor: &RequestorId, reason: DequeueReasonPlaceholder);
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

/// Placeholder to avoid circular dependencies if DequeueReason is defined in the queue crate.
/// We'll define a basic enum here or use a u8/type if it is better placed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DequeueReasonPlaceholder {
    Dispatched,
    Cancelled,
    TimedOut,
}
