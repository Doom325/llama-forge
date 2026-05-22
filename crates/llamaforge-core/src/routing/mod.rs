//! Smart Routing — picks the best `llama-server` per request based on
//! slot availability and KV-cache (prompt prefix) affinity.
//!
//! Implements design.md §10.2 (Routing Algorithm) and §10.3 (Prompt
//! Cache Affinity). Component spec:
//! `docs/component-designs/05_SMART_ROUTING.md`.
//!
//! Provides the core routing logic, constraint filtering, session affinity,
//! and scoring algorithms for selecting the best server for a request.

pub mod affinity;
pub mod config;
pub mod error;
pub mod router;
pub mod types;
pub mod weights;

use crate::types::ModelTarget;

/// A read-only view of a request used by the routing layer.
/// This prevents a bidirectional dependency or forcing the routing layer to know
/// about the full `QueuedRequest` type from `llamaforge-queue`.
pub trait RequestView: Send + Sync {
    /// The target model for this request.
    fn model_target(&self) -> &ModelTarget;

    /// The capabilities required by this request.
    fn capabilities(&self) -> &RequestCapabilities;

    /// The session ID for session affinity (if any).
    fn session_id(&self) -> Option<&crate::types::SessionId>;
}

/// Breaks the bidirectional dependency between routing and dispatch crates.
/// See design doc Section 2.1a.
pub trait DispatchCountView: Send + Sync {
    /// Returns the number of in-flight requests for a given server.
    fn in_flight_count(&self, server_id: &crate::types::ServerId) -> usize;
}

/// Used by Request Queue's admission gate (Component 04) and dispatch
/// iterator. Split surface — `serves_model` answers the admission
/// question ("is this model knowable?"), `has_idle_slot` answers the
/// dispatch question ("can I forward right now?"). Conflating them
/// causes the queue to synchronously reject burst traffic the moment
/// all slots are busy.
/// This trait is intentionally open (unsealed) so that external test crates can implement it.
pub trait CapacityChecker: Send + Sync {
    /// True if any *configured* server matches the model + capability
    /// set, regardless of lifecycle (Running, Starting, etc.) or slot
    /// occupancy. The admission controller calls this — a server that
    /// is busy now will be free later, so admission should queue, not
    /// reject.
    fn serves_model(&self, target: &ModelTarget, capabilities: &RequestCapabilities) -> bool;

    /// True if at least one matching server is Running *and* has a
    /// free slot. The queue's dispatch iterator calls this before
    /// asking the router to actually pick a server; it's the
    /// fast-fail check that avoids a wasted `select_server` call.
    fn has_idle_slot(&self, target: &ModelTarget, capabilities: &RequestCapabilities) -> bool;
}

/// Used by Dispatch & Proxy (Component 06) to select a server after dequeue.
pub trait ServerSelector: Send + Sync {
    /// Selects the best server for a given request.
    fn select_server(&self, request: &dyn RequestView) -> Result<RoutingDecision, RoutingError>;
}

// Re-export key types from submodules so consumers can import from `crate::routing::*`
pub use error::RoutingError;
pub use router::Router;
pub use types::{RequestCapabilities, RoutingDecision};
