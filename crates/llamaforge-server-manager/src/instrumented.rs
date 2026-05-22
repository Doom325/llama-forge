//! Decorator for tracing and observability boundaries.
//! See design doc Section 7.

use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::instrument;

use crate::error::ServerManagerError;
use crate::manager::ServerManager;
use crate::state::ServerStateSnapshot;
use crate::types::{ClientRefreshEvent, SlotState};
use llamaforge_config::{GpuId, ServerConfig};
use llamaforge_types::ServerId;

/// Observability decorator around `ServerManager`.
/// Enforces R26: Domain logic is free of telemetry.
#[derive(Clone)]
pub struct InstrumentedServerManager {
    inner: Arc<ServerManager>,
}

impl InstrumentedServerManager {
    pub fn new(inner: Arc<ServerManager>) -> Self {
        Self { inner }
    }

    #[instrument(skip(self), level = "debug")]
    pub fn pending_dispatches(&self, id: &ServerId) -> u64 {
        let count = self.inner.pending_dispatches(id);
        metrics::gauge!("llamaforge_pending_dispatches", "server_id" => id.to_string())
            .set(count as f64);
        count
    }

    #[instrument(skip(self), level = "debug")]
    pub fn increment_pending(&self, id: &ServerId) {
        self.inner.increment_pending(id);
        let count = self.inner.pending_dispatches(id);
        metrics::gauge!("llamaforge_pending_dispatches", "server_id" => id.to_string())
            .set(count as f64);
    }

    #[instrument(skip(self), level = "debug")]
    pub fn decrement_pending(&self, id: &ServerId) {
        self.inner.decrement_pending(id);
        let count = self.inner.pending_dispatches(id);
        metrics::gauge!("llamaforge_pending_dispatches", "server_id" => id.to_string())
            .set(count as f64);
    }

    #[instrument(skip(self, server_config, meta), level = "debug")]
    pub fn pre_spawn_vram_check(
        &self,
        server_id: &ServerId,
        server_config: &ServerConfig,
        gpu_id: &GpuId,
        meta: &crate::vram_estimate::GgufMetadata,
    ) -> Result<(), ServerManagerError> {
        let res = self.inner.pre_spawn_vram_check(server_id, server_config, gpu_id, meta);
        // The ledger metric could be updated here, but the value requires lock access which
        // is inside manager.rs. For exact metrics, we can do it by polling or accessing state.
        res
    }

    #[instrument(skip(self), level = "debug")]
    pub fn release_vram_reservation(&self, gpu_id: &GpuId, bytes: u64) {
        self.inner.release_vram_reservation(gpu_id, bytes);
    }

    #[instrument(skip(self), level = "debug")]
    pub fn clear_vram_reservation(&self, server_id: &ServerId) {
        self.inner.clear_vram_reservation(server_id);
    }

    #[instrument(skip(self), level = "debug")]
    pub fn server_state(&self, id: &ServerId) -> Option<ServerStateSnapshot> {
        self.inner.server_state(id)
    }

    #[instrument(skip(self), level = "debug")]
    pub fn running_servers(&self) -> Vec<(ServerId, ServerStateSnapshot)> {
        self.inner.running_servers()
    }

    #[instrument(skip(self), level = "debug")]
    pub fn slot_states(&self, id: &ServerId) -> Option<Vec<SlotState>> {
        self.inner.slot_states(id)
    }

    #[instrument(skip(self), level = "debug")]
    pub fn cumulative_requests(&self, id: &ServerId) -> u64 {
        self.inner.cumulative_requests(id)
    }

    #[instrument(skip(self), level = "debug")]
    pub fn increment_requests(&self, id: &ServerId) {
        self.inner.increment_requests(id);
        metrics::counter!("llamaforge_server_cumulative_requests", "server_id" => id.to_string())
            .absolute(self.inner.cumulative_requests(id));
    }

    #[instrument(skip(self), level = "debug")]
    pub fn subscribe_client_refresh(&self) -> broadcast::Receiver<ClientRefreshEvent> {
        self.inner.subscribe_client_refresh()
    }
}
