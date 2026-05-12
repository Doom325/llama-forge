//! Module: types
use llamaforge_types::ServerId;

/// Represents capabilities required by a request.
///
/// See design doc Section 2.2.
pub struct RequestCapabilities;

/// The routing decision made by the ServerSelector.
///
/// See design doc Section 2.2.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    // Stub field
    server_id: ServerId,
}

impl RoutingDecision {
    /// Creates a new RoutingDecision.
    pub fn new(server_id: ServerId) -> Self {
        Self { server_id }
    }

    /// Returns the selected server ID.
    pub fn server_id(&self) -> &ServerId {
        &self.server_id
    }
}
