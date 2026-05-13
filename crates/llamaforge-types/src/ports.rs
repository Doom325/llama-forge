use async_trait::async_trait;
use crate::{ServerId, SlotId, RequestId};
use thiserror::Error;

/// View into the dispatch map to resolve slot assignments.
pub trait SlotDispatchView: Send + Sync {
    /// Returns the request currently assigned to the given slot, if any.
    fn get_request_for_slot(&self, server_id: &ServerId, slot_id: &SlotId) -> Option<RequestId>;
    
    /// Returns true if there are any unresolved dispatches for the server.
    fn has_unresolved(&self, server_id: &ServerId) -> bool;
}

/// Interface for forcefully erasing orphaned slots on a server.
#[async_trait]
pub trait SlotErasure: Send + Sync {
    /// Attempt to erase a slot on the given server.
    async fn erase_slot(&self, server_id: &ServerId, slot_id: &SlotId) -> Result<(), SlotError>;
}

/// Errors that can occur during slot erasure.
#[derive(Debug, Error)]
pub enum SlotError {
    #[error("Failed to erase slot {slot_id} on server {server_id}: {reason}")]
    ErasureFailed {
        server_id: ServerId,
        slot_id: SlotId,
        reason: String,
    },
    #[error("Server {server_id} unreachable")]
    ServerUnreachable {
        server_id: ServerId,
    },
}

/// Interface for notifying the scheduler when a slot becomes available.
pub trait SlotNotifier: Send + Sync {
    /// Notify that a slot has become available on the given server.
    fn notify_slot_available(&self, server_id: &ServerId);
}
