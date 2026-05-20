//! Server state projection and internals.
//! See design doc Section 2.3.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
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
    pub resolved_ctx_size: u32,
    pub slots: Vec<SlotState>,
    pub last_health_check: Option<Instant>,
    pub restart_count: u32,
    pub cumulative_requests: u64,
    pub capabilities: ServerCapabilities,
    /// Wall-clock timestamp when this server's *first* spawn task
    /// started. Captured once at registration and preserved across
    /// restarts; mirrors the design.md §5.3.1 `started_at` field on
    /// `ServerSummary`. `None` until the spawn task initialises it.
    pub started_at: Option<DateTime<Utc>>,
    /// Wall-clock timestamp of the most recent *restart* spawn — i.e.
    /// the second and subsequent spawns. `None` while
    /// `restart_count == 0`.
    pub last_restart_at: Option<DateTime<Utc>>,
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
    pub resolved_ctx_size: u32,
    pub slots: Vec<SlotState>,
    pub last_health_check: Option<Instant>,
    pub restart_count: u32,
    pub cumulative_requests: u64,
    pub capabilities: Option<ServerCapabilities>,

    // Internal bookkeeping
    /// The instant the server first passed its health check. Used by deadlock detection.
    pub first_health_ok_at: Option<Instant>,
    /// Track the decode progress of each slot. Used by deadlock detection.
    pub last_n_decoded: HashMap<u32, Option<u32>>,
    /// The GPU assigned to this server.
    pub gpu_id: GpuId,
    /// Cached metadata to avoid re-parsing on restarts.
    pub gguf_metadata: Option<Arc<crate::vram_estimate::GgufMetadata>>,
    /// Amount of VRAM reserved for this server in bytes.
    pub reserved_vram_bytes: u64,
    /// Token used to signal this specific server task to shutdown.
    pub task_shutdown_token: tokio_util::sync::CancellationToken,
    /// First-spawn wall-clock timestamp. Set once when the server task
    /// initialises; preserved across restarts.
    pub started_at: Option<DateTime<Utc>>,
    /// Last restart wall-clock timestamp. Updated on every spawn
    /// after the first.
    pub last_restart_at: Option<DateTime<Utc>>,

    #[cfg(any(test, feature = "test-utils"))]
    pub test_warm_tps_samples: Option<(usize, f64)>,
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
            resolved_ctx_size: self.resolved_ctx_size,
            slots: self.slots.clone(),
            last_health_check: self.last_health_check,
            restart_count: self.restart_count,
            cumulative_requests: self.cumulative_requests,
            capabilities: self.capabilities.clone().unwrap_or_default(),
            started_at: self.started_at,
            last_restart_at: self.last_restart_at,
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
            resolved_ctx_size: 4096,
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
            gguf_metadata: None,
            reserved_vram_bytes: 0,
            task_shutdown_token: tokio_util::sync::CancellationToken::new(),
            started_at: Some(Utc::now()),
            last_restart_at: None,
            #[cfg(any(test, feature = "test-utils"))]
            test_warm_tps_samples: None,
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
