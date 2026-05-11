//! Server state projection and internals.
//! See design doc Section 2.3.

use std::collections::HashMap;
use std::time::Instant;

use llamaforge_config::{GpuId, ServerId};

use crate::types::{ServerCapabilities, ServerLifecycle, SlotState};

/// Public read-only snapshot of a server's state.
/// See design doc Section 2.3.
#[derive(Debug, Clone)]
pub struct ServerStateSnapshot {
    pub id: ServerId,
    pub lifecycle: ServerLifecycle,
    pub pid: Option<u32>,
    pub port: u16,
    pub model_alias: String,
    pub parallel_slots: u32,
    pub ctx_size: u32,
    pub slots: Vec<SlotState>,
    pub last_health_check: Option<Instant>,
    pub restart_count: u32,
    pub cumulative_requests: u64,
    pub capabilities: ServerCapabilities,
}

/// Internal mutable state for a server, held in a DashMap.
#[allow(dead_code)]
pub(crate) struct ServerState {
    pub id: ServerId,
    pub lifecycle: ServerLifecycle,
    pub pid: Option<u32>,
    pub port: u16,
    pub model_alias: String,
    pub parallel_slots: u32,
    pub ctx_size: u32,
    pub slots: Vec<SlotState>,
    pub last_health_check: Option<Instant>,
    pub restart_count: u32,
    pub cumulative_requests: u64,
    pub capabilities: Option<ServerCapabilities>,

    // Internal bookkeeping
    /// The instant the server first passed its health check. Used by deadlock detection.
    pub first_health_ok_at: Option<Instant>,
    /// Track the decode progress of each slot. Used by deadlock detection.
    pub last_n_decoded: HashMap<u32, u32>,
    /// The GPU assigned to this server.
    pub gpu_id: GpuId,
    // Cached metadata to avoid re-parsing on restarts (added in PR3).
    // pub gguf_metadata: Option<Arc<crate::vram_estimate::GgufMetadata>>,
}

impl ServerState {
    /// Convert the internal state into a public, read-only snapshot.
    #[allow(dead_code)]
    pub(crate) fn to_snapshot(&self) -> ServerStateSnapshot {
        ServerStateSnapshot {
            id: self.id.clone(),
            lifecycle: self.lifecycle,
            pid: self.pid,
            port: self.port,
            model_alias: self.model_alias.clone(),
            parallel_slots: self.parallel_slots,
            ctx_size: self.ctx_size,
            slots: self.slots.clone(),
            last_health_check: self.last_health_check,
            restart_count: self.restart_count,
            cumulative_requests: self.cumulative_requests,
            capabilities: self.capabilities.clone().unwrap_or_default(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ModelArchitecture, SpeculativeMethod};

    #[test]
    fn test_server_state_snapshot_roundtrip() {
        let state = ServerState {
            id: ServerId("srv1".into()),
            lifecycle: ServerLifecycle::Running,
            pid: Some(1234),
            port: 8080,
            model_alias: "test-model".into(),
            parallel_slots: 4,
            ctx_size: 4096,
            slots: vec![],
            last_health_check: Some(Instant::now()),
            restart_count: 2,
            cumulative_requests: 42,
            capabilities: Some(ServerCapabilities {
                models: vec!["test-model".into()],
                lora_adapters: vec![],
                multimodal: false,
                speculative: SpeculativeMethod::None,
                architecture: ModelArchitecture::Llama,
            }),
            first_health_ok_at: Some(Instant::now()),
            last_n_decoded: HashMap::new(),
            gpu_id: GpuId(0),
        };

        let snapshot = state.to_snapshot();

        assert_eq!(snapshot.id, state.id);
        assert_eq!(snapshot.lifecycle, state.lifecycle);
        assert_eq!(snapshot.pid, state.pid);
        assert_eq!(snapshot.port, state.port);
        assert_eq!(snapshot.model_alias, state.model_alias);
        assert_eq!(snapshot.parallel_slots, state.parallel_slots);
        assert_eq!(snapshot.ctx_size, state.ctx_size);
        assert_eq!(snapshot.restart_count, state.restart_count);
        assert_eq!(snapshot.cumulative_requests, state.cumulative_requests);
        assert!(!snapshot.capabilities.models.is_empty());
    }
}
