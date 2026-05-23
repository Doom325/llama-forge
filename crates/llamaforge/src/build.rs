//! Composition root: wires the seven component crates plus the five
//! adapters into a single `TransportDeps`. See design doc §4.4 and
//! PLAN.md PR8.4 for the dependency-strict construction order.
//!
//! Port-trait wiring summary:
//!
//! * `traits::SlotNotifier` is satisfied directly by `DispatchQueue`,
//!   whose `notify_slot_available` wakes the scheduler via a queue-wide
//!   notify. No adapter is needed.
//! * `ports::SlotErasure` is satisfied by `ReqwestClientStore`, which
//!   forwards to `POST {base_url}/slots/{id}?action=erase` per design
//!   doc §6.629. Each configured server is registered in the store via
//!   `add_server` during build (step 7), populating the
//!   `server_id → base_url` map both the inference dispatch and the
//!   reconciliation path consume.
//! * `routing::ServerStateView` is satisfied by `ServerStateAdapter`,
//!   which projects `server-manager::ServerStateSnapshot` into the
//!   routing-layer `ServerSnapshot`. After the ServerId unification
//!   (PR1), `config::ServerId` and `types::ServerId` are the same
//!   `Uuid`-backed newtype, so the projection no longer requires a
//!   bridge function.

use std::sync::Arc;

use llamaforge_api::adapters::inference_engine::InferenceEngineAdapter;
use llamaforge_api::adapters::metrics_exporter::MetricsExporterAdapter;
use llamaforge_api::adapters::queue::QueueSubmitterAdapter;
use llamaforge_api::adapters::server_control::ServerControlAdapter;
use llamaforge_api::dispatch::client::ReqwestClientStore;
use llamaforge_api::dispatch::dispatcher::Dispatcher;
use llamaforge_api::dispatch::slot_map::SlotDispatchMap;
use llamaforge_api::dispatch::tracker::{DispatchCountAdapter, DispatchTrackerRegistry};
use llamaforge_api::transport::TransportDeps;
use llamaforge_core::config::{ConfigHandle, ConfigReader};
use llamaforge_core::queue::admission::AdmissionController;
use llamaforge_core::queue::pipeline::{run_scheduler, HealthTracker, RequestPipeline};
use llamaforge_core::queue::queue::DispatchQueue;
use llamaforge_core::routing::types::{ServerSnapshot, ServerStateView};
use llamaforge_core::routing::Router;
use llamaforge_core::types::ports::SlotErasure;
use llamaforge_core::types::traits::SlotNotifier;
use llamaforge_metrics::{create_metrics_pipeline, InstrumentedMetricsStore};
use llamaforge_server::ServerManager;
use std::sync::atomic::AtomicU32;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Background tasks spawned during composition. Held by the caller so
/// they can be awaited / aborted on shutdown.
pub struct BackgroundTasks {
    pub scheduler: JoinHandle<()>,
    pub server_manager: Arc<ServerManager>,
}

/// Build the whole orchestrator. Returns the `TransportDeps` ready to
/// hand to `start_servers`, plus the background tasks the caller must
/// hold for the lifetime of the process.
pub async fn build(
    config: Arc<ConfigHandle>,
    shutdown_token: CancellationToken,
) -> anyhow::Result<(
    TransportDeps<ConfigHandle, InstrumentedMetricsStore, InstrumentedMetricsStore>,
    BackgroundTasks,
)> {
    // 1) Metrics pipeline (SQLite-backed). Must come before any component
    //    that emits metrics via `metrics::counter!` macros so the recorder
    //    is installed; the global recorder is installed by the
    //    MetricsExporterAdapter constructor below.
    let snap = config.snapshot();
    let metrics_cfg = llamaforge_metrics::MetricsConfig {
        db_path: snap.global.metrics_db_path.clone(),
        // Note: metrics_retention_days is read live in prune.rs
        ..llamaforge_metrics::MetricsConfig::default()
    };
    let metrics_pipeline = create_metrics_pipeline(&metrics_cfg, config.clone())
        .map_err(|e| anyhow::anyhow!("metrics pipeline: {}", e))?;

    let metrics_ingest = metrics_pipeline.ingest.clone();
    let metrics_store: Arc<InstrumentedMetricsStore> = metrics_pipeline.store.clone();

    // 2) Prometheus recorder install (process-global).
    let metrics_exporter = Arc::new(
        MetricsExporterAdapter::install_global()
            .map_err(|e| anyhow::anyhow!("install prometheus recorder: {}", e))?,
    );

    // 3) Server manager. Holds the lifecycle of `llama-server` instances.
    let backend_choice = config.snapshot().global.telemetry_backend.clone();
    let gpu_telemetry = llamaforge_server::create_telemetry(backend_choice.as_deref());
    let server_manager = Arc::new(ServerManager::new(
        gpu_telemetry,
        config.clone(),
        metrics_ingest.clone(),
        metrics_store.clone(),
        shutdown_token.clone(),
    ));

    // 4) Dispatch infrastructure.
    let trackers = Arc::new(DispatchTrackerRegistry::new());
    let dispatch_counts = Arc::new(DispatchCountAdapter::new(trackers.clone()));
    let slot_map = Arc::new(SlotDispatchMap::new());

    // 5) Router. Consumes server-state + dispatch-count views.
    let server_state_view: Arc<dyn ServerStateView> =
        Arc::new(ServerStateAdapter::new(server_manager.clone()));
    let router = Arc::new(Router::new(config.clone(), server_state_view.clone(), dispatch_counts));

    // 6) Queue + scheduler. Routing is performed by the scheduler loop,
    //    not the dispatch adapter (see InferenceEngineAdapter docs).
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission =
        Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), router.clone()));
    let dispatch_queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth));
    let health = Arc::new(HealthTracker::default());
    let pipeline =
        Arc::new(RequestPipeline::new(admission.clone(), dispatch_queue.clone(), health.clone()));

    // Spawn the scheduler. It pulls from the queue, runs routing, and
    // emits `DispatchResult::Dispatched` back to the queue adapter.
    let scheduler_handle = {
        let q = dispatch_queue.clone();
        let a = admission.clone();
        let r = router.clone();
        let h = health.clone();
        let st = shutdown_token.clone();
        tokio::spawn(run_scheduler(q, a, r.clone(), r, h, st))
    };

    // 7) Backend HTTP clients to the llama-server fleet. Registered up
    //    front from `config.servers` so the dispatcher can resolve
    //    `server_id → base_url` for completion POSTs *and* for the
    //    slot-erase reconciliation path.
    let backend_client = Arc::new(ReqwestClientStore::new());
    {
        let snap = config.snapshot();
        for (cfg_id, server_cfg) in snap.servers.iter() {
            backend_client
                .add_server(*cfg_id, server_cfg)
                .map_err(|e| anyhow::anyhow!("register backend client for {}: {}", cfg_id, e))?;
        }
    }

    // 8) Dispatcher.
    let slot_notifier: Arc<dyn SlotNotifier> = dispatch_queue.clone();
    // Real slot-erasure: forwards to `POST /slots/{id}?action=erase` via
    // ReqwestClientStore. The store knows each server's base_url because
    // we registered it above. See dispatch::client::ReqwestClientStore
    // for the impl.
    let slot_erasure: Arc<dyn SlotErasure> = backend_client.clone();
    let dispatcher = Arc::new(Dispatcher::new(
        server_state_view.clone(),
        metrics_ingest.clone(),
        trackers.clone(),
        slot_map.clone(),
        config.clone(),
        backend_client.clone(),
        backend_client.clone(),
        slot_notifier,
        slot_erasure,
    ));

    // 9) Adapters that satisfy transport's port traits.
    let queue_submitter = Arc::new(QueueSubmitterAdapter::new(pipeline.clone()));
    let server_control = Arc::new(ServerControlAdapter::new(server_manager.clone()));

    // 10) The keystone — wires queue/router/dispatcher/metrics.
    let inference_engine = Arc::new(InferenceEngineAdapter::new(
        queue_submitter.clone(),
        router.clone(),
        dispatcher.clone(),
        Arc::new(metrics_ingest.clone()),
    ));

    let deps = TransportDeps {
        config_reader: config.clone() as Arc<dyn ConfigReader>,
        config_writer: config.clone(),
        server_control,
        queue_submitter,
        metrics_exporter,
        request_metrics_store: metrics_store.clone(),
        store_maintenance: metrics_store,
        route_validator: router.clone(),
        inference_engine,
    };

    Ok((deps, BackgroundTasks { scheduler: scheduler_handle, server_manager }))
}

/// Bridges `server-manager::ServerStateSnapshot` to the routing layer's
/// `ServerSnapshot`. After the ServerId unification (PR1) the two
/// `ServerId` types are the same `Uuid`-backed newtype, so the
/// projection is now a straight field-by-field copy with no parse.
struct ServerStateAdapter {
    manager: Arc<ServerManager>,
}

impl ServerStateAdapter {
    fn new(manager: Arc<ServerManager>) -> Self {
        Self { manager }
    }
}

impl ServerStateView for ServerStateAdapter {
    fn all_servers(&self) -> Vec<ServerSnapshot> {
        use llamaforge_core::types::{Capability, LoraAdapterId, ModelAlias};
        self.manager
            .all_servers()
            .into_iter()
            .map(|(id, snap)| {
                let models: std::collections::HashSet<ModelAlias> =
                    std::iter::once(snap.model_alias.clone())
                        .chain(snap.capabilities.models.iter().cloned())
                        .filter(|s| !s.is_empty())
                        .map(|s| ModelAlias::new(Arc::from(s.as_str())))
                        .collect();
                let lora_adapters: std::collections::HashSet<LoraAdapterId> = snap
                    .capabilities
                    .lora_adapters
                    .iter()
                    .filter_map(|s| LoraAdapterId::new(s).ok())
                    .collect();
                let idle_slots = snap.slots.iter().filter(|s| !s.is_processing).count() as u32;
                ServerSnapshot {
                    id,
                    lifecycle: snap.lifecycle,
                    models,
                    capabilities: std::collections::HashSet::<Capability>::new(),
                    lora_adapters,
                    multimodal: snap.capabilities.multimodal,
                    embedding: false,
                    reranking: false,
                    architecture: snap.capabilities.architecture,
                    total_slots: snap.parallel_slots,
                    idle_slots,
                    kv_usage: 0.0,
                    deferred_ratio: 0.0,
                }
            })
            .collect()
    }
}
