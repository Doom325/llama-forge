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

use llamaforge_types::{ModelTarget, QueuedRequest};

// We define our own traits here. If `llamaforge-types` defines similar traits,
// we define these as separate open traits because this crate owns the Smart Routing logic.
// This is done to decouple the port traits per the component design.

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
    fn select_server(&self, request: &QueuedRequest) -> Result<RoutingDecision, RoutingError>;
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
