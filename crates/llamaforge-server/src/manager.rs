//! Core server manager and lifecycle coordinator.
//! See design doc Section 3.1.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use llamaforge_core::config::{ConfigDiff, ConfigHandle, ConfigReader, GpuId, ServerConfig};
use llamaforge_core::types::ServerId;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use llamaforge_metrics::{MetricsIngest, ProcessStore};

use crate::error::ServerManagerError;
use crate::state::{ServerState, ServerStateSnapshot};
use crate::telemetry::GpuTelemetry;
use crate::types::{ClientRefreshEvent, ServerLifecycle};
use crate::vram_estimate::{compute_vram_estimate, GgufMetadata};

/// The main entry point for managing `llama-server` instances.
#[allow(dead_code)]
pub struct ServerManager {
    pub(crate) servers: Arc<DashMap<ServerId, ServerState>>,
    /// Protected by a synchronous `parking_lot::Mutex` per ground rule 2.
    pub(crate) vram_ledger: Arc<parking_lot::Mutex<HashMap<GpuId, u64>>>,
    pub(crate) gpu_telemetry: Arc<dyn GpuTelemetry>,
    pub(crate) config: Arc<ConfigHandle>,
    pub(crate) metrics_ingest: MetricsIngest,
    pub(crate) metrics_store: Arc<llamaforge_metrics::InstrumentedMetricsStore>,
    pub(crate) shutdown_token: CancellationToken,
    pub(crate) client_refresh_tx: broadcast::Sender<ClientRefreshEvent>,
    pub(crate) lifecycle_tx: broadcast::Sender<(ServerId, ServerLifecycle)>,
    pub(crate) pending_dispatches: Arc<DashMap<ServerId, AtomicU64>>,

    pub(crate) warned_overflow: Arc<DashMap<ServerId, AtomicBool>>,
    pub(crate) gpu_semaphores: Arc<DashMap<GpuId, Arc<tokio::sync::Semaphore>>>,
    pub(crate) tasks: tokio::sync::Mutex<tokio::task::JoinSet<()>>,

    /// Fleet readiness state, encoded as `ClusterState::repr`. Read
    /// with `Ordering::Acquire`, written with `Ordering::Release` per
    /// `design.md:4353` so `/ready` stays lock-free.
    pub(crate) cluster_state: Arc<AtomicU8>,
    /// Number of servers currently in the `Failed` bucket. Surfaces in
    /// the `/ready` 503 `failed_servers` field. Read/written alongside
    /// `cluster_state`; not strictly ordered against it because the
    /// count is informational — the gating decision uses `cluster_state`
    /// alone.
    pub(crate) failed_servers: Arc<AtomicU32>,
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
        let (lifecycle_tx, _) = broadcast::channel(128);
        Self {
            servers: Arc::new(DashMap::new()),
            vram_ledger: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            gpu_telemetry,
            config,
            metrics_ingest,
            metrics_store,
            shutdown_token,
            client_refresh_tx,
            lifecycle_tx,
            pending_dispatches: Arc::new(DashMap::new()),
            warned_overflow: Arc::new(DashMap::new()),
            gpu_semaphores: Arc::new(DashMap::new()),
            tasks: tokio::sync::Mutex::new(tokio::task::JoinSet::new()),
            cluster_state: Arc::new(AtomicU8::new(crate::types::ClusterState::SomeStarting as u8)),
            failed_servers: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Returns the current cluster status. Two `Ordering::Acquire`
    /// atomic loads, lock-free per `design.md:4353`.
    pub fn cluster_status(&self) -> crate::types::ClusterStatus {
        let state =
            crate::types::ClusterState::from_repr(self.cluster_state.load(Ordering::Acquire));
        let failed_servers = self.failed_servers.load(Ordering::Acquire);
        crate::types::ClusterStatus { state, failed_servers }
    }

    /// Recalculates the fleet state and updates the readiness atomics.
    /// Called whenever a server transitions state.
    ///
    /// Bucketing:
    /// - `Running` → active (drives `SomeRunning`).
    /// - `Starting` / `Restarting` / `Draining` → starting.
    /// - `Failed` → failed (drives the circuit breaker once nothing
    ///   else is active or starting).
    /// - `Stopped` is **not** counted as `failed`: it is the
    ///   deliberate operator state reached via `POST /api/servers/{id}/stop`.
    ///   Counting it would trip the circuit breaker on an
    ///   operator-stopped fleet — see `design.md:4353` which scopes
    ///   `AllFailed` to "ALL configured servers have permanently failed".
    /// - Empty cluster maps to `SomeStarting` so `/ready` permits queuing
    ///   during 0-to-1 scale-up.
    pub(crate) fn recalculate_cluster_state(&self) {
        use crate::types::{ClusterState, ServerLifecycle};

        let mut active = 0u32;
        let mut starting = 0u32;
        let mut failed = 0u32;

        for entry in self.servers.iter() {
            match entry.value().lifecycle {
                ServerLifecycle::Running => active += 1,
                ServerLifecycle::Starting
                | ServerLifecycle::Restarting
                | ServerLifecycle::Draining => starting += 1,
                ServerLifecycle::Failed => failed += 1,
                ServerLifecycle::Stopped => {}
                // `ServerLifecycle` is `#[non_exhaustive]`; treat any
                // future variant conservatively as "starting" so a
                // new state can't accidentally trip the breaker.
                _ => starting += 1,
            }
        }

        let new_state = if active > 0 {
            ClusterState::SomeRunning
        } else if starting > 0 {
            ClusterState::SomeStarting
        } else if failed > 0 {
            ClusterState::AllFailed
        } else {
            // Empty fleet or all Stopped — keep traffic queueable.
            ClusterState::SomeStarting
        };

        // `failed` first, then `cluster_state` with Release: a reader
        // that observes `AllFailed` is guaranteed to see the matching
        // `failed_servers` value used to populate the 503 body.
        self.failed_servers.store(failed, Ordering::Release);
        self.cluster_state.store(new_state as u8, Ordering::Release);
    }

    // --- Task 4.1: Implementation of start() ---

    pub async fn start(self: Arc<Self>) -> Result<Arc<Self>, ServerManagerError> {
        let app_config = self.config.snapshot();

        // Task 7.4: Orphan Recovery (FM-1)
        let mut adopted_pids = std::collections::HashMap::new();
        if let Ok(procs) = self.metrics_store.list_managed_processes().await {
            match reqwest::Client::builder().timeout(std::time::Duration::from_secs(1)).build() {
                Ok(client) => {
                    for proc in procs {
                        #[cfg(unix)]
                        {
                            use nix::sys::signal::kill;
                            use nix::unistd::Pid;
                            if kill(Pid::from_raw(proc.pid.0 as i32), None).is_err() {
                                // Not alive, delete row
                                let _ = self.metrics_store.remove_managed_process(proc.pid).await;
                                continue;
                            }
                        }

                        // Alive, check health
                        let url = format!("http://127.0.0.1:{}/health", proc.port);
                        let mut adopted = false;
                        if let Ok(resp) = client.get(&url).send().await {
                            if resp.status().is_success() {
                                // Verify it matches a configured server
                                if let Some(server_config) = app_config.servers.get(&proc.server_id)
                                {
                                    let expected_alias =
                                        server_config.alias.as_deref().unwrap_or("default");
                                    if expected_alias == proc.model_alias {
                                        tracing::info!(
                                            server.id = %proc.server_id,
                                            server.orphan_reclaimed = true,
                                            pid = proc.pid.0,
                                            "Adopted orphaned server process"
                                        );
                                        adopted_pids.insert(proc.server_id.clone(), proc.pid.0);
                                        adopted = true;
                                    }
                                }
                            }
                        }

                        if !adopted {
                            tracing::warn!(
                                server.id = %proc.server_id,
                                pid = proc.pid.0,
                                "orphan_unowned: Live process found but health check failed or model mismatched. Leaving untouched."
                            );
                            let _ = self.metrics_store.remove_managed_process(proc.pid).await;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "server_manager.orphan_recovery_client_build_failed");
                }
            }
        }

        for (server_id, server_config) in &app_config.servers {
            let srv = server_id.clone();
            let cfg = server_config.clone();
            let mgr = self.clone();
            let orphan_pid = adopted_pids.get(&srv).copied();

            // Insert initial state
            mgr.servers
                .insert(srv.clone(), ServerState::new_starting(srv.clone(), &cfg, orphan_pid));

            self.tasks.lock().await.spawn(async move {
                crate::task::spawn_server_task(mgr, srv, cfg, orphan_pid).await;
            });
        }

        // Fleet state should be recalculated now that tasks are spawned.
        self.recalculate_cluster_state();

        Ok(self)
    }

    // --- Task 7.1: Implement apply_config_diff ---

    pub async fn apply_config_diff(self: &Arc<Self>, diff: &ConfigDiff) {
        let app_config = self.config.snapshot();

        // Removed servers
        for server_id in diff.servers_removed() {
            let mut rx = self.lifecycle_tx.subscribe();
            self.stop_server(server_id).await;

            let state = self.servers.get(server_id).map(|s| s.lifecycle);
            if state != Some(ServerLifecycle::Stopped) && state.is_some() {
                let _ = tokio::time::timeout(Duration::from_secs(30), async {
                    while let Ok((id, state)) = rx.recv().await {
                        if id == *server_id && state == ServerLifecycle::Stopped {
                            break;
                        }
                    }
                }).await.map_err(|_| {
                    tracing::warn!(server.id = %server_id, "apply_config_diff.timeout_waiting_for_stop");
                });
            }
            self.servers.remove(server_id);
        }

        // Changed servers
        for (server_id, srv_diff) in diff.servers_changed() {
            if srv_diff.requires_restart() {
                let mut rx = self.lifecycle_tx.subscribe();
                self.stop_server(server_id).await;

                let state = self.servers.get(server_id).map(|s| s.lifecycle);
                if state != Some(ServerLifecycle::Stopped) && state.is_some() {
                    let _ = tokio::time::timeout(Duration::from_secs(30), async {
                        while let Ok((id, state)) = rx.recv().await {
                            if id == *server_id && state == ServerLifecycle::Stopped {
                                break;
                            }
                        }
                    }).await.map_err(|_| {
                        tracing::warn!(server.id = %server_id, "apply_config_diff.timeout_waiting_for_stop");
                    });
                }
                if let Some(server_config) = app_config.servers.get(server_id) {
                    let srv = server_id.clone();
                    let cfg = server_config.clone();
                    let mgr = self.clone();

                    mgr.servers
                        .insert(srv.clone(), ServerState::new_starting(srv.clone(), &cfg, None));

                    self.tasks.lock().await.spawn(async move {
                        crate::task::spawn_server_task(mgr, srv, cfg, None).await;
                    });
                }
            } else if srv_diff.hot_reloadable() {
                if let Some(server_config) = app_config.servers.get(server_id) {
                    if let Some(mut state) = self.servers.get_mut(server_id) {
                        state.model_alias =
                            server_config.alias.clone().unwrap_or_else(|| "default".to_string());
                    }
                }
            }
        }

        // Added servers
        for server_id in diff.servers_added() {
            if let Some(server_config) = app_config.servers.get(server_id) {
                let srv = server_id.clone();
                let cfg = server_config.clone();
                let mgr = self.clone();

                mgr.servers.insert(srv.clone(), ServerState::new_starting(srv.clone(), &cfg, None));

                self.tasks.lock().await.spawn(async move {
                    crate::task::spawn_server_task(mgr, srv, cfg, None).await;
                });
            }
        }
    }

    // --- Task 7.2: Implement stop_server ---

    pub async fn stop_server(&self, server_id: &ServerId) {
        if let Some(state) = self.servers.get(server_id) {
            state.task_shutdown_token.cancel();
        }
    }

    // --- Task 7.3: Implement shutdown_all ---

    pub async fn shutdown_all(&self) {
        let server_ids: Vec<ServerId> = self.servers.iter().map(|s| s.id.clone()).collect();

        for id in server_ids {
            // We can't clone self easily here if we take &self. We'll cancel all tokens.
            if let Some(state) = self.servers.get(&id) {
                state.task_shutdown_token.cancel();
            }
        }

        // Cancel the global shutdown token too
        self.shutdown_token.cancel();

        // Wait for all tasks to finish
        let mut tasks = self.tasks.lock().await;
        while tasks.join_next().await.is_some() {}

        // Clear VRAM ledger completely
        self.vram_ledger.lock().clear();

        // Delete all rows from managed_processes
        if let Ok(procs) = self.metrics_store.list_managed_processes().await {
            for proc in procs {
                let _ = self.metrics_store.remove_managed_process(proc.pid).await;
            }
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
        server_id: &ServerId,
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
            drop(ledger);
            if let Some(mut server) = self.servers.get_mut(server_id) {
                server.reserved_vram_bytes = needed_bytes;
            }
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
        drop(ledger);
        if let Some(mut server) = self.servers.get_mut(server_id) {
            server.reserved_vram_bytes = needed_bytes;
        }

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
        let (gpu_id, bytes) = {
            if let Some(server) = self.servers.get(server_id) {
                (server.gpu_id.clone(), server.reserved_vram_bytes)
            } else {
                return;
            }
        };

        if bytes > 0 {
            self.release_vram_reservation(&gpu_id, bytes);
            if let Some(mut server) = self.servers.get_mut(server_id) {
                server.reserved_vram_bytes = 0;
            }
        }
    }

    // --- Task 3.3: State accessors ---

    pub fn config(&self) -> Arc<ConfigHandle> {
        self.config.clone()
    }

    pub fn vram_ledger(&self) -> HashMap<GpuId, u64> {
        self.vram_ledger.lock().clone()
    }

    /// Returns a snapshot of a specific server.
    pub fn server_state(&self, id: &ServerId) -> Option<ServerStateSnapshot> {
        self.servers.get(id).map(|s| s.to_snapshot())
    }

    pub fn all_servers(&self) -> Vec<(ServerId, ServerStateSnapshot)> {
        self.servers.iter().map(|s| (s.id.clone(), s.to_snapshot())).collect()
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

    pub async fn request_start(
        self: &Arc<Self>,
        server_id: &ServerId,
    ) -> Result<(), ServerManagerError> {
        if let Some(state) = self.servers.get(server_id) {
            if state.lifecycle != ServerLifecycle::Stopped
                && state.lifecycle != ServerLifecycle::Failed
            {
                return Err(ServerManagerError::StartFailed {
                    id: server_id.clone(),
                    reason: "Server is already running or starting".into(),
                });
            }
        } else {
            return Err(ServerManagerError::ServerNotFound { id: server_id.clone() });
        }

        let app_config = self.config.snapshot();
        if let Some(server_config) = app_config.servers.get(server_id) {
            let srv = server_id.clone();
            let cfg = server_config.clone();
            let mgr = self.clone();

            mgr.servers.insert(srv.clone(), ServerState::new_starting(srv.clone(), &cfg, None));

            self.tasks.lock().await.spawn(async move {
                crate::task::spawn_server_task(mgr, srv, cfg, None).await;
            });
            return Ok(());
        }
        Err(ServerManagerError::ServerNotFound { id: server_id.clone() })
    }

    pub async fn request_stop(&self, server_id: &ServerId) -> Result<(), ServerManagerError> {
        if !self.servers.contains_key(server_id) {
            return Err(ServerManagerError::ServerNotFound { id: server_id.clone() });
        }
        self.stop_server(server_id).await;
        Ok(())
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

    /// Returns a receiver for client refresh events.
    pub fn subscribe_client_refresh(&self) -> broadcast::Receiver<ClientRefreshEvent> {
        self.client_refresh_tx.subscribe()
    }

    /// Returns a receiver for server lifecycle changes.
    pub fn subscribe_lifecycle(&self) -> broadcast::Receiver<(ServerId, ServerLifecycle)> {
        self.lifecycle_tx.subscribe()
    }

    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn set_cumulative_requests_for_test(&self, id: &ServerId, count: u64) {
        if let Some(mut state) = self.servers.get_mut(id) {
            state.cumulative_requests = count;
        }
    }

    /// Test helper: insert a synthetic server in a given lifecycle
    /// state without running the spawn task, then refresh the
    /// readiness atomics. Lets integration tests drive
    /// `/ready` / `/ready/inference` through the production
    /// `recalculate_cluster_state` path.
    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn insert_synthetic_server_for_test(&self, id: ServerId, lifecycle: ServerLifecycle) {
        let cfg = llamaforge_core::config::ServerConfig::default();
        let mut state = crate::state::ServerState::new_starting(id, &cfg, None);
        state.lifecycle = lifecycle;
        self.servers.insert(id, state);
        self.recalculate_cluster_state();
    }

    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn force_warm_tps_window_for_test(&self, id: &ServerId, samples: usize, baseline: f64) {
        if let Some(mut state) = self.servers.get_mut(id) {
            state.test_warm_tps_samples = Some((samples, baseline));
        }
    }

    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub async fn wait_for_lifecycle(
        &self,
        id: &ServerId,
        target: ServerLifecycle,
        deadline: Duration,
    ) -> bool {
        let mut rx = self.subscribe_lifecycle();

        if let Some(state) = self.server_state(id) {
            if state.lifecycle == target {
                return true;
            }
        }

        tokio::time::timeout(deadline, async {
            while let Ok((event_id, state)) = rx.recv().await {
                if event_id == *id && state == target {
                    return;
                }
            }
        })
        .await
        .is_ok()
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

        // Explicit override wins
        config.shutdown_drain_timeout_secs = Some(42);
        assert_eq!(resolve_drain_timeout(&config).as_secs(), 42);
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

    use llamaforge_core::config::{ConfigPaths, TomlFileConfigSource};
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

        let srv_id = ServerId::from_config_id("srv_vram");
        manager.servers.insert(
            srv_id.clone(),
            ServerState::new_starting(srv_id.clone(), &server_config, None),
        );

        assert!(manager.pre_spawn_vram_check(&srv_id, &server_config, &gpu, &meta).is_ok());
        assert_eq!(*manager.vram_ledger.lock().get(&gpu).unwrap(), expected_bytes);
        assert_eq!(manager.servers.get(&srv_id).unwrap().reserved_vram_bytes, expected_bytes);

        manager.clear_vram_reservation(&srv_id);
        assert_eq!(*manager.vram_ledger.lock().get(&gpu).unwrap(), 0);
        assert_eq!(manager.servers.get(&srv_id).unwrap().reserved_vram_bytes, 0);
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
        let srv_id = ServerId::from_config_id("srv1");

        let server_config = llamaforge_core::config::ServerConfig {
            binary_path: "mock".into(),
            model_path: "mock".into(),
            port: 8080,
            alias: Some("test".into()),
            parallel_slots: 4,
            ctx_size: 4096,
            devices: vec![llamaforge_core::config::DeviceSpec {
                gpu_id: GpuId(0),
                layer_range: None,
            }],
            ..Default::default()
        };

        manager.servers.insert(
            srv_id.clone(),
            ServerState::new_starting(srv_id.clone(), &server_config, Some(1234)),
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

    #[tokio::test]
    async fn test_recalculate_cluster_state() {
        use crate::types::{ClusterState, ServerLifecycle};

        let telemetry = Arc::new(MockTelemetry { free_vram: 0, is_unavailable: true });
        let _tmp_db = tempfile::TempDir::new().unwrap();
        let config = get_test_config(&_tmp_db);
        let db_path = _tmp_db.path().join("metrics3.db");
        let pipeline = llamaforge_metrics::create_metrics_pipeline(
            &llamaforge_metrics::MetricsConfig { db_path, ..Default::default() },
            config.clone(),
        )
        .unwrap();
        let metrics = pipeline.store;
        let ingest = pipeline.ingest;
        let token = CancellationToken::new();

        let manager = ServerManager::new(telemetry, config, ingest, metrics, token);

        // 1. Initial state: SomeStarting (even if empty, design.md:4354)
        manager.recalculate_cluster_state();
        let status = manager.cluster_status();
        assert_eq!(status.state, ClusterState::SomeStarting);
        assert_eq!(status.failed_servers, 0);

        // 2. One server starting: SomeStarting
        let s1 = ServerId::from_config_id("s1");
        let s1_cfg = llamaforge_core::config::ServerConfig::default();
        manager.servers.insert(s1, crate::state::ServerState::new_starting(s1, &s1_cfg, None));
        manager.recalculate_cluster_state();
        assert_eq!(manager.cluster_status().state, ClusterState::SomeStarting);

        // 3. One running: SomeRunning
        manager.servers.get_mut(&s1).unwrap().lifecycle = ServerLifecycle::Running;
        manager.recalculate_cluster_state();
        assert_eq!(manager.cluster_status().state, ClusterState::SomeRunning);

        // 4. One running, one failed: SomeRunning
        let s2 = ServerId::from_config_id("s2");
        let s2_cfg = llamaforge_core::config::ServerConfig::default();
        let mut s2_state = crate::state::ServerState::new_starting(s2, &s2_cfg, None);
        s2_state.lifecycle = ServerLifecycle::Failed;
        manager.servers.insert(s2, s2_state);
        manager.recalculate_cluster_state();
        let status = manager.cluster_status();
        assert_eq!(status.state, ClusterState::SomeRunning);
        assert_eq!(status.failed_servers, 1);

        // 5. All failed: AllFailed
        manager.servers.get_mut(&s1).unwrap().lifecycle = ServerLifecycle::Failed;
        manager.recalculate_cluster_state();
        let status = manager.cluster_status();
        assert_eq!(status.state, ClusterState::AllFailed);
        assert_eq!(status.failed_servers, 2);

        // 6. One stopped, one failed: AllFailed (stopped doesn't count as starting or running)
        manager.servers.get_mut(&s1).unwrap().lifecycle = ServerLifecycle::Stopped;
        manager.recalculate_cluster_state();
        let status = manager.cluster_status();
        assert_eq!(status.state, ClusterState::AllFailed);
        assert_eq!(status.failed_servers, 1);

        // 7. One stopped, one starting: SomeStarting
        manager.servers.get_mut(&s2).unwrap().lifecycle = ServerLifecycle::Starting;
        manager.recalculate_cluster_state();
        assert_eq!(manager.cluster_status().state, ClusterState::SomeStarting);

        // 8. ALL stopped: must NOT trip the circuit breaker. `Stopped`
        //    is the deliberate operator state — `design.md:4353`
        //    reserves `AllFailed` for "ALL configured servers have
        //    permanently failed", not "operator deliberately stopped
        //    every backend". If this bucketed to `AllFailed`, an
        //    operator stopping the fleet would cause kubelet to evict
        //    the pod.
        manager.servers.get_mut(&s1).unwrap().lifecycle = ServerLifecycle::Stopped;
        manager.servers.get_mut(&s2).unwrap().lifecycle = ServerLifecycle::Stopped;
        manager.recalculate_cluster_state();
        let status = manager.cluster_status();
        assert_eq!(
            status.state,
            ClusterState::SomeStarting,
            "all-Stopped must not trip the breaker"
        );
        assert_eq!(status.failed_servers, 0);
    }
}
