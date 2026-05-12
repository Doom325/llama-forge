//! Module: types
use llamaforge_types::{ServerId, SessionId, SlotId};

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
