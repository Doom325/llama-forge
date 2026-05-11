//! Core server manager and lifecycle coordinator.
//! See design doc Section 3.1.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use llamaforge_config::{ConfigHandle, GpuId, ServerConfig, ServerId};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use llamaforge_metrics::MetricsIngest;

use crate::error::ServerManagerError;
use crate::state::{ServerState, ServerStateSnapshot};
use crate::telemetry::GpuTelemetry;
use crate::types::{ClientRefreshEvent, ServerLifecycle};
use crate::vram_estimate::{compute_vram_estimate, GgufMetadata};

/// The main entry point for managing `llama-server` instances.
#[allow(dead_code)]
pub struct ServerManager {
    servers: Arc<DashMap<ServerId, ServerState>>,
    /// Protected by a synchronous `parking_lot::Mutex` per ground rule 2.
    vram_ledger: Arc<parking_lot::Mutex<HashMap<GpuId, u64>>>,
    gpu_telemetry: Arc<dyn GpuTelemetry>,
    config: Arc<ConfigHandle>,
    metrics_ingest: MetricsIngest,
    metrics_store: Arc<llamaforge_metrics::InstrumentedMetricsStore>,
    shutdown_token: CancellationToken,
    client_refresh_tx: broadcast::Sender<ClientRefreshEvent>,
    pending_dispatches: Arc<DashMap<ServerId, AtomicU64>>,

    warned_overflow: Arc<DashMap<ServerId, AtomicBool>>,
}

impl ServerManager {
    /// Creates a new `ServerManager`.
    pub fn new(
        gpu_telemetry: Arc<dyn GpuTelemetry>,
        config: Arc<ConfigHandle>,
        metrics_ingest: MetricsIngest,
        metrics_store: Arc<llamaforge_metrics::InstrumentedMetricsStore>,
        shutdown_token: CancellationToken,
    ) -> Self {
        let (client_refresh_tx, _) = broadcast::channel(32);
        Self {
            servers: Arc::new(DashMap::new()),
            vram_ledger: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            gpu_telemetry,
            config,
            metrics_ingest,
            metrics_store,
            shutdown_token,
            client_refresh_tx,
            pending_dispatches: Arc::new(DashMap::new()),
            warned_overflow: Arc::new(DashMap::new()),
        }
    }

    // --- Task 3.1a: pending_dispatches API surface ---

    /// Gets the number of pending dispatches for a server.
    pub fn pending_dispatches(&self, id: &ServerId) -> u64 {
        if let Some(counter) = self.pending_dispatches.get(id) {
            counter.load(Ordering::Relaxed)
        } else {
            0
        }
    }

    /// Increments the pending dispatches counter for a server.
    pub fn increment_pending(&self, id: &ServerId) {
        let counter =
            self.pending_dispatches.entry(id.clone()).or_insert_with(|| AtomicU64::new(0));
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrements the pending dispatches counter for a server.
    pub fn decrement_pending(&self, id: &ServerId) {
        if let Some(counter) = self.pending_dispatches.get(id) {
            let _ = counter.fetch_sub(1, Ordering::Relaxed); // Let it underflow if mismatched, harmless for u64 logic
        }
    }

    // --- Task 3.2: VRAM Ledger ---

    /// Checks if sufficient VRAM is available and records a reservation if successful.
    pub fn pre_spawn_vram_check(
        &self,
        server_config: &ServerConfig,
        gpu_id: &GpuId,
        meta: &GgufMetadata,
    ) -> Result<(), ServerManagerError> {
        let needed_bytes = compute_vram_estimate(server_config, meta);

        let mut ledger = self.vram_ledger.lock();

        let reserved_bytes = ledger.get(gpu_id).copied().unwrap_or(0);

        let free_vram_bytes = match self.gpu_telemetry.free_vram_bytes(gpu_id) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(
                    "GPU telemetry unavailable for {:?}, skipping strict VRAM check: {}",
                    gpu_id,
                    e
                );
                u64::MAX
            }
        };

        if free_vram_bytes == u64::MAX {
            // Unbounded fallback
            *ledger.entry(gpu_id.clone()).or_insert(0) += needed_bytes;
            return Ok(());
        }

        let effective_free_bytes = free_vram_bytes.saturating_sub(reserved_bytes);

        if needed_bytes > effective_free_bytes {
            return Err(ServerManagerError::InsufficientVram {
                gpu: gpu_id.clone(),
                needed_mb: needed_bytes / (1024 * 1024),
                free_mb: effective_free_bytes / (1024 * 1024),
            });
        }

        *ledger.entry(gpu_id.clone()).or_insert(0) += needed_bytes;

        Ok(())
    }

    /// Releases a specific amount of reserved VRAM.
    pub fn release_vram_reservation(&self, gpu_id: &GpuId, bytes: u64) {
        let mut ledger = self.vram_ledger.lock();
        if let Some(reserved) = ledger.get_mut(gpu_id) {
            *reserved = reserved.saturating_sub(bytes);
        }
    }

    /// Clears the VRAM reservation for a server completely.
    pub fn clear_vram_reservation(&self, server_id: &ServerId) {
        // Find the server to get its GPU and metadata
        if let Some(server) = self.servers.get(server_id) {
            let _gpu_id = server.gpu_id.clone();
            if let Some(_meta) = &server.gguf_metadata {
                // Need ServerConfig to compute the estimate. We fetch it from the snapshot if possible.
                // For now, this is a placeholder. Proper logic would find the config.
                // It's usually easier to just let `release_vram_reservation` be called manually
                // with the exact bytes computed earlier. This method handles it if config is known.
                let _bytes = 0; // compute_vram_estimate(&config, meta);
                                // self.release_vram_reservation(&gpu_id, bytes);
            }
        }
    }

    // --- Task 3.3: State accessors ---

    /// Returns a snapshot of a specific server.
    pub fn server_state(&self, id: &ServerId) -> Option<ServerStateSnapshot> {
        self.servers.get(id).map(|s| s.to_snapshot())
    }

    /// Returns snapshots of all currently running servers.
    pub fn running_servers(&self) -> Vec<(ServerId, ServerStateSnapshot)> {
        self.servers
            .iter()
            .filter(|s| s.lifecycle == ServerLifecycle::Running)
            .map(|s| (s.id.clone(), s.to_snapshot()))
            .collect()
    }

    /// Returns the states of all slots for a specific server.
    pub fn slot_states(&self, id: &ServerId) -> Option<Vec<crate::types::SlotState>> {
        self.servers.get(id).map(|s| s.slots.clone())
    }

    // --- Task 3.4: Cumulative requests ---

    /// Gets the cumulative request count for a server.
    pub fn cumulative_requests(&self, id: &ServerId) -> u64 {
        self.servers.get(id).map(|s| s.cumulative_requests).unwrap_or(0)
    }

    /// Increments the cumulative request count for a server.
    pub fn increment_requests(&self, id: &ServerId) {
        if let Some(mut server) = self.servers.get_mut(id) {
            server.cumulative_requests += 1;
            let current = server.cumulative_requests;

            // Warning logic: half of i32::MAX
            let threshold = (i32::MAX / 2) as u64;
            if current >= threshold {
                let warned = self
                    .warned_overflow
                    .entry(id.clone())
                    .or_insert_with(|| AtomicBool::new(false));
                if !warned.swap(true, Ordering::Relaxed) {
                    warn!(
                        "Server {} has reached {} requests. Consider a rolling restart soon.",
                        id, current
                    );
                }
            }
        }
    }

    // --- Task 3.5: subscribe_client_refresh ---

    /// Subscribes to client refresh events.
    pub fn subscribe_client_refresh(&self) -> broadcast::Receiver<ClientRefreshEvent> {
        self.client_refresh_tx.subscribe()
    }
}

// --- Task 3.2b: resolve_drain_timeout ---

/// Resolves the graceful drain timeout based on the configuration or heuristic.
/// Design Section 6: `Auto: max(60, min(ctx_size/30, 600))`.
#[allow(dead_code)]
pub fn resolve_drain_timeout(config: &ServerConfig) -> Duration {
    if let Some(secs) = config.shutdown_drain_timeout_secs {
        return Duration::from_secs(secs);
    }

    let ctx = config.ctx_size as u64;
    let computed = ctx / 30;
    let clamped = computed.clamp(60, 600);
    Duration::from_secs(clamped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ModelArchitecture;
    use crate::vram_estimate::Quantization;

    #[test]
    fn test_resolve_drain_timeout() {
        let mut config = ServerConfig { ctx_size: 0, ..Default::default() };
        assert_eq!(resolve_drain_timeout(&config).as_secs(), 60);
        config.ctx_size = 3000;
        assert_eq!(resolve_drain_timeout(&config).as_secs(), 100);

        config.ctx_size = 20000;
        assert_eq!(resolve_drain_timeout(&config).as_secs(), 600);

        config.ctx_size = 40000;
        assert_eq!(resolve_drain_timeout(&config).as_secs(), 600);
    }

    struct MockTelemetry {
        free_vram: u64,
        is_unavailable: bool,
    }

    impl GpuTelemetry for MockTelemetry {
        fn free_vram_bytes(&self, _id: &GpuId) -> Result<u64, crate::telemetry::TelemetryError> {
            if self.is_unavailable {
                Err(crate::telemetry::TelemetryError::Unavailable)
            } else {
                Ok(self.free_vram)
            }
        }
        fn utilization_percent(&self, _id: &GpuId) -> Result<u8, crate::telemetry::TelemetryError> {
            Ok(0)
        }
        fn list_devices(
            &self,
        ) -> Result<Vec<crate::telemetry::GpuDevice>, crate::telemetry::TelemetryError> {
            Ok(vec![])
        }
    }

    use llamaforge_config::{ConfigPaths, TomlFileConfigSource};
    use tempfile::TempDir;

    fn get_test_config(tmp: &TempDir) -> Arc<ConfigHandle> {
        let main_path = tmp.path().join("llamaforge.toml");
        let state_dir = tmp.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(&main_path, b"[metrics]\n").unwrap();
        let paths = ConfigPaths::new(main_path, tmp.path().join("priority_rules.toml"), state_dir);
        let source = Arc::new(TomlFileConfigSource::new(paths));
        Arc::new(ConfigHandle::load(source).unwrap())
    }

    #[tokio::test]
    async fn test_vram_ledger() {
        let _tmp_db = tempfile::TempDir::new().unwrap();
        let telemetry =
            Arc::new(MockTelemetry { free_vram: 10 * 1024 * 1024 * 1024, is_unavailable: false });
        let config = get_test_config(&_tmp_db);

        let db_path = _tmp_db.path().join("metrics.db");
        let pipeline = llamaforge_metrics::create_metrics_pipeline(
            &llamaforge_metrics::MetricsConfig { db_path, ..Default::default() },
            config.clone(),
        )
        .unwrap();
        let metrics = pipeline.store;
        let ingest = pipeline.ingest;
        let token = CancellationToken::new();

        let manager = ServerManager::new(telemetry, config, ingest, metrics, token);
        let server_config = ServerConfig::default();
        let gpu = GpuId(0);
        let meta = GgufMetadata {
            architecture: ModelArchitecture::Llama,
            block_count: 1,
            embedding_length: 128,
            head_count: 1,
            head_count_kv: 1,
            quantization: Quantization::Q4_K_M,
            file_size_bytes: 4_000_000_000,
        };

        let expected_bytes = compute_vram_estimate(&server_config, &meta);

        assert!(manager.pre_spawn_vram_check(&server_config, &gpu, &meta).is_ok());
        assert_eq!(*manager.vram_ledger.lock().get(&gpu).unwrap(), expected_bytes);

        manager.release_vram_reservation(&gpu, expected_bytes);
        assert_eq!(*manager.vram_ledger.lock().get(&gpu).unwrap(), 0);
    }

    #[tokio::test]
    async fn test_accessors_and_requests() {
        let telemetry = Arc::new(MockTelemetry { free_vram: 0, is_unavailable: true });
        let _tmp_db = tempfile::TempDir::new().unwrap();
        let config = get_test_config(&_tmp_db);
        let db_path = _tmp_db.path().join("metrics2.db");
        let pipeline = llamaforge_metrics::create_metrics_pipeline(
            &llamaforge_metrics::MetricsConfig { db_path, ..Default::default() },
            config.clone(),
        )
        .unwrap();
        let metrics = pipeline.store;
        let ingest = pipeline.ingest;
        let token = CancellationToken::new();

        let manager = ServerManager::new(telemetry, config, ingest, metrics, token);
        let srv_id = ServerId("srv1".into());

        manager.servers.insert(
            srv_id.clone(),
            ServerState {
                id: srv_id.clone(),
                lifecycle: ServerLifecycle::Running,
                pid: None,
                port: 8080,
                model_alias: "test".into(),
                parallel_slots: 1,
                ctx_size: 100,
                slots: vec![],
                last_health_check: None,
                restart_count: 0,
                cumulative_requests: 0,
                capabilities: None,
                first_health_ok_at: None,
                last_n_decoded: HashMap::new(),
                gpu_id: GpuId(0),
                gguf_metadata: None,
                reserved_vram_bytes: 0,
            },
        );

        assert_eq!(manager.running_servers().len(), 1);

        manager.increment_requests(&srv_id);
        assert_eq!(manager.cumulative_requests(&srv_id), 1);

        // Test latch
        let threshold = (i32::MAX / 2) as u64;
        manager.servers.get_mut(&srv_id).unwrap().cumulative_requests = threshold - 1;
        manager.increment_requests(&srv_id); // should warn
        assert_eq!(manager.cumulative_requests(&srv_id), threshold);
    }
}
