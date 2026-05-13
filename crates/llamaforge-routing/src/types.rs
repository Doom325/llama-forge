//! Module: types
use llamaforge_types::{
    Capability, LoraAdapterId, ModelAlias, ServerId, ServerLifecycle, SessionId, SlotId,
};
use std::collections::HashSet;

/// A snapshot of a server's state, used by the router to make decisions.
///
/// See design doc Section 3.1.
#[derive(Debug, Clone)]
pub struct ServerSnapshot {
    pub id: ServerId,
    pub lifecycle: ServerLifecycle,
    pub models: HashSet<ModelAlias>,
    pub capabilities: HashSet<Capability>,
    pub lora_adapters: HashSet<LoraAdapterId>,
    pub multimodal: bool,
    pub embedding: bool,
    pub reranking: bool,
    pub architecture: llamaforge_types::ModelArchitecture,
    pub total_slots: u32,
    pub idle_slots: u32,
    pub kv_usage: f64,
    pub deferred_ratio: f64,
}

/// Provides read-only snapshots of server state to the routing engine.
///
/// See design doc Section 2.1a and 4.1.
pub trait ServerStateView: Send + Sync {
    /// Returns snapshots of all servers in the system.
    fn all_servers(&self) -> Vec<ServerSnapshot>;
}

/// Target for a specific slot, eliminating the `-1` sentinel for slot assignment.
///
/// See design doc Section 2.2.
#[derive(Debug, Clone, Copy)]
pub enum SlotTarget {
    Auto,
    Pinned(SlotId),
}

/// The reason a server was chosen by the routing engine.
///
/// See design doc Section 2.2.
#[derive(Debug, Clone)]
pub enum RoutingReason {
    HighestScore { score: f64 },
    SessionAffinity { session_id: SessionId },
    RoundRobin { counter: u64 },
}

/// The routing decision made by the ServerSelector.
///
/// See design doc Section 2.2.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    server_id: ServerId,
    reason: RoutingReason,
    #[allow(dead_code)]
    pub(crate) score: f64,
    #[allow(dead_code)]
    pub(crate) slot_target: SlotTarget,
}

impl RoutingDecision {
    #[allow(dead_code)]
    pub(crate) fn new(
        server_id: ServerId,
        reason: RoutingReason,
        score: f64,
        slot_target: SlotTarget,
    ) -> Self {
        Self { server_id, reason, score, slot_target }
    }

    /// Returns the selected server ID.
    pub fn server_id(&self) -> &ServerId {
        &self.server_id
    }

    /// Returns the reason for the routing decision.
    pub fn reason(&self) -> &RoutingReason {
        &self.reason
    }

    /// Returns the target slot if specific, or None if auto.
    pub fn slot_target(&self) -> Option<llamaforge_types::SlotId> {
        match self.slot_target {
            SlotTarget::Pinned(id) => Some(id),
            SlotTarget::Auto => None,
        }
    }

    /// Exposes the internal score for integration tests.
    #[doc(hidden)]
    pub fn score(&self) -> f64 {
        self.score
    }

    /// Consumes the decision and returns the server ID.
    pub fn into_server_id(self) -> ServerId {
        self.server_id
    }

    /// Exposes a constructor for tests in external crates that need a mock `RoutingDecision`.
    #[doc(hidden)]
    pub fn new_for_test(server_id: ServerId) -> Self {
        Self {
            server_id,
            reason: RoutingReason::RoundRobin { counter: 0 },
            score: 1.0,
            slot_target: SlotTarget::Auto,
        }
    }
}

// Ensure RequestCapabilities from llamaforge_types is re-exported from here
pub use llamaforge_types::RequestCapabilities;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routing_decision_encapsulation() {
        let server_id = ServerId::new(uuid::Uuid::now_v7());
        let decision = RoutingDecision::new(
            server_id,
            RoutingReason::RoundRobin { counter: 1 },
            0.95,
            SlotTarget::Auto,
        );

        assert_eq!(decision.server_id(), &server_id);
        assert!(matches!(decision.reason(), RoutingReason::RoundRobin { counter: 1 }));
        assert_eq!(decision.into_server_id(), server_id);
    }
}
