use crate::types::{ServerId, SlotId};
use async_trait::async_trait;
use thiserror::Error;

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
    ErasureFailed { server_id: ServerId, slot_id: SlotId, reason: String },
    #[error("Server {server_id} unreachable")]
    ServerUnreachable { server_id: ServerId },
}
