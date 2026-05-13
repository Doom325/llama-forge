//! Smart Routing module for LlamaForge.
//!
//! Provides the core routing logic, constraint filtering, session affinity,
//! and scoring algorithms for selecting the best server for a request.

pub mod affinity;
pub mod config;
pub mod error;
pub mod router;
pub mod types;
pub mod weights;

use llamaforge_types::ModelTarget;

/// A read-only view of a request used by the routing layer.
/// This prevents a bidirectional dependency or forcing the routing layer to know
/// about the full `QueuedRequest` type from `llamaforge-queue`.
pub trait RequestView: Send + Sync {
    /// The target model for this request.
    fn model_target(&self) -> &ModelTarget;

    /// The capabilities required by this request.
    fn capabilities(&self) -> &RequestCapabilities;

    /// The session ID for session affinity (if any).
    fn session_id(&self) -> Option<&llamaforge_types::SessionId>;
}

/// Breaks the bidirectional dependency between routing and dispatch crates.
/// See design doc Section 2.1a.
pub trait DispatchCountView: Send + Sync {
    /// Returns the number of in-flight requests for a given server.
    fn in_flight_count(&self, server_id: &llamaforge_types::ServerId) -> usize;
}

/// Used by Request Queue's dispatch iterator (Component 04).
/// This trait is intentionally open (unsealed) so that external test crates can implement it.
pub trait CapacityChecker: Send + Sync {
    /// Checks if there is any configured capacity for a given target.
    fn has_capacity(&self, target: &ModelTarget) -> bool;
}

/// Used by Dispatch & Proxy (Component 06) to select a server after dequeue.
pub trait ServerSelector: Send + Sync {
    /// Selects the best server for a given request.
    fn select_server(&self, request: &dyn RequestView) -> Result<RoutingDecision, RoutingError>;
}

/// Used by Transport Layer (Component 07) at admission time.
pub trait RouteValidator: Send + Sync {
    /// Validates if a request is routable based on constraints.
    fn validate_routable(
        &self,
        model: &ModelTarget,
        capabilities: &RequestCapabilities,
    ) -> Result<(), RoutingError>;
}

// Re-export key types from submodules so consumers can import from `llamaforge_routing::*`
pub use error::RoutingError;
pub use router::Router;
pub use types::{RequestCapabilities, RoutingDecision};
