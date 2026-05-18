//! Composition root: wires the seven component crates plus the five
//! adapters into a single `TransportDeps`. See design doc §4.4 and
//! PLAN.md PR8.4 for the dependency-strict construction order.
//!
//! Port-trait wiring summary:
//!
//! * `ports::SlotNotifier` is satisfied by `QueueSlotNotifier`, which
//!   forwards into `DispatchQueue::notify_slot_available`. The queue's
//!   wake channel is queue-wide, so the per-server `server_id` argument
//!   is ignored — that matches the scheduler's actual semantics today.
//! * `ports::SlotErasure` is satisfied by `ReqwestClientStore`, which
//!   forwards to `POST {base_url}/slots/{id}?action=erase` per design
//!   doc §6.629. Each configured server is registered in the store via
//!   `add_server` during build (step 7), populating the
//!   `server_id → base_url` map both the inference dispatch and the
//!   reconciliation path consume.
//! * `routing::ServerStateView` is satisfied by `ServerStateAdapter`,
//!   which projects `server-manager::ServerStateSnapshot` into the
//!   routing-layer `ServerSnapshot` via
//!   [`config_server_id_to_types_id`].

use std::sync::Arc;

use llamaforge_adapters::inference_engine::InferenceEngineAdapter;
use llamaforge_adapters::metrics_exporter::MetricsExporterAdapter;
use llamaforge_adapters::queue::QueueSubmitterAdapter;
use llamaforge_adapters::server_control::ServerControlAdapter;
use llamaforge_config::{ConfigHandle, ConfigReader};
use llamaforge_dispatch::client::ReqwestClientStore;
use llamaforge_dispatch::dispatcher::Dispatcher;
use llamaforge_dispatch::slot_map::SlotDispatchMap;
use llamaforge_dispatch::tracker::{DispatchCountAdapter, DispatchTrackerRegistry};
use llamaforge_metrics::{create_metrics_pipeline, InstrumentedMetricsStore};
use llamaforge_queue::admission::AdmissionController;
use llamaforge_queue::pipeline::{run_scheduler, HealthTracker, RequestPipeline};
use llamaforge_queue::queue::DispatchQueue;
use llamaforge_routing::types::{ServerSnapshot, ServerStateView};
use llamaforge_routing::Router;
use llamaforge_server_manager::{ServerManager, StubTelemetry};
use llamaforge_transport::TransportDeps;
use llamaforge_types::ports::{SlotErasure, SlotNotifier};
use llamaforge_types::traits::{AdmissionError, IncomingRequest, RouteValidator};
use llamaforge_types::ServerId;
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
    let metrics_cfg = llamaforge_metrics::MetricsConfig::default();
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
    let gpu_telemetry = Arc::new(StubTelemetry);
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
            let types_id = config_server_id_to_types_id(cfg_id);
            backend_client
                .add_server(types_id, server_cfg)
                .map_err(|e| anyhow::anyhow!("register backend client for {}: {}", cfg_id, e))?;
        }
    }

    // 8) Dispatcher.
    let slot_notifier: Arc<dyn SlotNotifier> = Arc::new(QueueSlotNotifier(dispatch_queue.clone()));
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
        route_validator: Arc::new(RouterValidatorAdapter { router: router.clone() }),
        inference_engine,
    };

    Ok((deps, BackgroundTasks { scheduler: scheduler_handle, server_manager }))
}

/// Bridges `routing::Router`'s `(model, capabilities) -> RoutingError`
/// validation into the transport's `&IncomingRequest -> AdmissionError`
/// shape. Maps any routing error to `AdmissionError::InvalidRequest`.
struct RouterValidatorAdapter {
    router: Arc<Router>,
}

impl RouteValidator for RouterValidatorAdapter {
    fn validate_routable(&self, req: &IncomingRequest) -> Result<(), AdmissionError> {
        use llamaforge_routing::RoutingError;
        use llamaforge_types::ModelAlias;
        <Router as llamaforge_routing::RouteValidator>::validate_routable(
            &self.router,
            &req.model_target,
            &req.capabilities,
        )
        .map_err(|e| match e {
            RoutingError::ModelNotFound { model, .. }
            | RoutingError::ModelUnavailable { model, .. } => AdmissionError::NoServerForModel {
                model: ModelAlias::new(std::sync::Arc::from(model.as_str())),
            },
            RoutingError::NoCapableServer { requested, .. } => AdmissionError::NoServerForModel {
                model: ModelAlias::new(std::sync::Arc::from(requested.as_str())),
            },
        })
    }
}

/// Wraps `DispatchQueue`'s queue-wide wake notify into the per-server
/// `ports::SlotNotifier` shape the dispatcher expects. The scheduler
/// wake signal is queue-wide today, so `server_id` is ignored.
struct QueueSlotNotifier(Arc<DispatchQueue>);

impl SlotNotifier for QueueSlotNotifier {
    fn notify_slot_available(&self, _server_id: &ServerId) {
        self.0.notify_slot_available();
    }
}

/// Bridges `server-manager::ServerStateSnapshot` to the routing layer's
/// `ServerSnapshot`. The two `ServerId` types are reconciled via
/// [`config_server_id_to_types_id`]: load-time validation in
/// `ConfigHandle::load` (`handle.rs:27-35`) guarantees every
/// `config::ServerId` is UUID-shaped, so the parse is infallible by
/// construction — the lone `.expect` cites that invariant.
///
/// `all_servers()` is called per routing decision (hot path). The
/// projection is `O(n)` over the fleet plus one `Uuid::parse_str` per
/// server. With v1's expected fleet size (low single digits) the cost
/// is unmeasurable; a caching layer ("parse once on snapshot diff,
/// store the `types::ServerId` alongside") is the obvious next step
/// once PR9b's load tests show numbers.
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
        use llamaforge_types::{Capability, LoraAdapterId, ModelAlias};
        self.manager
            .all_servers()
            .into_iter()
            .map(|(config_id, snap)| {
                let id = config_server_id_to_types_id(&config_id);
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

/// Convert a load-validated `config::ServerId` (slug or UUID) to the
/// routing-layer `types::ServerId`. UUID-shaped slugs are preserved
/// verbatim; arbitrary slugs are hashed to a stable UUIDv5 — see
/// `llamaforge_types::ServerId::from_config_id`.
fn config_server_id_to_types_id(s: &llamaforge_config::ServerId) -> llamaforge_types::ServerId {
    llamaforge_types::ServerId::from_config_id(s.0.as_str())
}
