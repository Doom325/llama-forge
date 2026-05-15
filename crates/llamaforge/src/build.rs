//! Composition root: wires the seven component crates plus the five
//! adapters into a single `TransportDeps`. See design doc §4.4 and
//! PLAN.md PR8.4 for the dependency-strict construction order.
//!
//! Three port-trait gaps remain on `main` and are bridged here with
//! local in-bin shims:
//!
//! * `ports::SlotNotifier` is satisfied by `QueueSlotNotifier`, which
//!   forwards into `DispatchQueue::notify_slot_available`. The queue's
//!   wake channel is queue-wide, so the per-server `server_id` argument
//!   is ignored — that matches the scheduler's actual semantics today.
//! * `ports::SlotErasure` has no production implementation on main;
//!   `NoopSlotErasure` is used as a deliberate placeholder. The dispatch
//!   reconciliation path that would call `erase_slot` becomes a no-op
//!   until a real impl lands (PR9 / PR10 will add it once mock-llama
//!   integration surfaces the failure mode).
//! * `routing::ServerStateView` is satisfied by `EmptyServerStateView`,
//!   another placeholder. A real adapter requires reconciling
//!   `server-manager::ServerStateSnapshot.id` (`config::ServerId`, a
//!   String wrapper) with the routing snapshot's `types::ServerId`
//!   (a Uuid wrapper). That work lives with the e2e harness in PR9 /
//!   PR10; routing currently returns "no servers" so the scheduler
//!   holds requests until the fleet is wired up.

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
use llamaforge_types::ports::{SlotErasure, SlotError, SlotNotifier};
use llamaforge_types::traits::{AdmissionError, IncomingRequest, RouteValidator};
use llamaforge_types::{ServerId, SlotId};
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
    //    `EmptyServerStateView` is a deliberate placeholder: bridging
    //    `server-manager::ServerStateSnapshot` to the routing layer's
    //    `ServerSnapshot` requires reconciling two different `ServerId`
    //    types (config::ServerId vs types::ServerId) — that reconciliation
    //    is dispatch-phase work, not composition. Until then routing
    //    answers "no servers"; the scheduler holds requests until the
    //    fleet is wired up, which lands with the e2e harness (PR9 / PR10).
    let server_state_view: Arc<dyn ServerStateView> = Arc::new(EmptyServerStateView);
    let _ = &server_manager;
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

    // 7) Backend HTTP clients to the llama-server fleet.
    let backend_client = Arc::new(ReqwestClientStore::new());

    // 8) Dispatcher.
    let slot_notifier: Arc<dyn SlotNotifier> = Arc::new(QueueSlotNotifier(dispatch_queue.clone()));
    let slot_erasure: Arc<dyn SlotErasure> = Arc::new(NoopSlotErasure);
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
        <Router as llamaforge_routing::RouteValidator>::validate_routable(
            &self.router,
            &req.model_target,
            &req.capabilities,
        )
        .map_err(|e| AdmissionError::InvalidRequest { reason: e.to_string() })
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

/// Placeholder slot-erasure impl. Real implementation (calling
/// llama-server's slot-free endpoint) lands with the e2e harness in
/// PR9 once mock-llama provides a verifiable contract.
struct NoopSlotErasure;

#[async_trait::async_trait]
impl SlotErasure for NoopSlotErasure {
    async fn erase_slot(&self, _: &ServerId, _: &SlotId) -> Result<(), SlotError> {
        Ok(())
    }
}

/// Placeholder server-state view. Returns no servers; routing therefore
/// rejects all requests as `NoServerForModel`. The proper adapter has
/// to reconcile `server-manager::ServerStateSnapshot.id`
/// (`config::ServerId`, a String wrapper) with the routing snapshot's
/// `types::ServerId` (a Uuid wrapper). That reconciliation is dispatch
/// substance, not composition; it lands with the e2e harness work in
/// PR9 / PR10.
struct EmptyServerStateView;

impl ServerStateView for EmptyServerStateView {
    fn all_servers(&self) -> Vec<ServerSnapshot> {
        Vec::new()
    }
}
