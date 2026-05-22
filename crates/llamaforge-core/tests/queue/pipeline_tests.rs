use llamaforge_core::config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
use llamaforge_core::queue::admission::AdmissionController;
use llamaforge_core::queue::pipeline::{run_scheduler, RequestPipeline};
use llamaforge_core::queue::queue::DispatchQueue;
use llamaforge_core::queue::types::{DispatchResult, IncomingRequest, TaskType};
use llamaforge_core::routing::{CapacityChecker, RoutingDecision, ServerSelector};
use llamaforge_core::types::traits::{ActiveRequestProvider, AdmissionAccounting, DequeueReason};
use llamaforge_core::types::{
    ModelAlias, ModelTarget, RequestId, RequestorClass, RequestorId, ServerId,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

struct MockCapacityChecker;
impl CapacityChecker for MockCapacityChecker {
    fn serves_model(
        &self,
        t: &ModelTarget,
        c: &llamaforge_core::routing::RequestCapabilities,
    ) -> bool {
        self.has_idle_slot(t, c)
    }

    fn has_idle_slot(
        &self,
        _: &ModelTarget,
        _: &llamaforge_core::routing::RequestCapabilities,
    ) -> bool {
        true
    }
}

struct MockServerSelector;
impl ServerSelector for MockServerSelector {
    fn select_server(
        &self,
        _: &dyn llamaforge_core::routing::RequestView,
    ) -> Result<RoutingDecision, llamaforge_core::routing::RoutingError> {
        Ok(RoutingDecision::new_for_test(ServerId::new(uuid::Uuid::now_v7())))
    }
}

fn test_config() -> Arc<ConfigHandle> {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "[global]\nqueue_max_depth = 1000\n").unwrap();
    let paths = ConfigPaths::new(
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
        tmp.path().parent().unwrap().to_path_buf(),
    );
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

fn mock_incoming(id: uuid::Uuid) -> (IncomingRequest, oneshot::Receiver<DispatchResult>) {
    let (tx, rx) = oneshot::channel();
    (
        IncomingRequest {
            id: RequestId::new(id),
            payload: Arc::new(()),
            task_type: TaskType::Background,
            requestor: RequestorId::new(Arc::from("user1")),
            requestor_class: RequestorClass::new(Arc::from("default")),
            model_target: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
            capabilities: Default::default(),
            session_id: None,
            explicit_override: None,
            response_tx: tx,
            cancel_token: CancellationToken::new(),
            trace_context: None,
        },
        rx,
    )
}

#[tokio::test]
async fn test_capability_constrained_request_stays_queued() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));

    struct CapAwareChecker {
        // True during admission, false during dispatch to simulate a server becoming busy
        has_free_slots: std::sync::atomic::AtomicBool,
    }
    impl CapacityChecker for CapAwareChecker {
        fn serves_model(
            &self,
            t: &ModelTarget,
            c: &llamaforge_core::routing::RequestCapabilities,
        ) -> bool {
            self.has_idle_slot(t, c)
        }

        fn has_idle_slot(
            &self,

            _: &ModelTarget,

            caps: &llamaforge_core::routing::RequestCapabilities,
        ) -> bool {
            // Must have free slots AND not be multimodal
            self.has_free_slots.load(Ordering::SeqCst) && !caps.multimodal
        }
    }

    let checker =
        Arc::new(CapAwareChecker { has_free_slots: std::sync::atomic::AtomicBool::new(true) });

    let admission =
        Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), checker.clone()));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_core::queue::pipeline::HealthTracker::default());
    let pipeline = RequestPipeline::new(admission, queue.clone(), health);

    // 1. Submit request WITHOUT multimodal (has capacity)
    let (mut req, mut rx) = mock_incoming(uuid::Uuid::now_v7());
    req.capabilities.multimodal = false;

    let res = pipeline.submit(req);
    assert!(res.is_ok());
    assert_eq!(queue_depth.load(Ordering::SeqCst), 1);

    // 2. Server becomes busy
    checker.has_free_slots.store(false, Ordering::SeqCst);

    // 3. Try to dispatch. The queue should check capacity, checker returns false,
    // so it stays in the queue and we don't get a candidate out.
    let candidate = queue.try_dispatch(&*checker);
    assert!(candidate.is_none());

    // The request should still be in the queue (depth = 1) and not cancelled.
    assert_eq!(queue_depth.load(Ordering::SeqCst), 1);

    // Quick check that rx hasn't received a Cancellation
    assert!(rx.try_recv().is_err());
}
#[tokio::test]
async fn test_pipeline_submit_happy_path() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_core::queue::pipeline::HealthTracker::default());
    let pipeline = RequestPipeline::new(admission, queue, health);

    let (req, _rx) = mock_incoming(uuid::Uuid::now_v7());
    let res = pipeline.submit(req);
    assert!(res.is_ok());
    assert_eq!(queue_depth.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_pipeline_submit_rejection() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(1000)); // Queue full
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_core::queue::pipeline::HealthTracker::default());
    let pipeline = RequestPipeline::new(admission, queue, health);

    let (req, _rx) = mock_incoming(uuid::Uuid::now_v7());
    let res = pipeline.submit(req);
    assert!(matches!(res, Err(llamaforge_core::queue::error::AdmissionError::QueueFull { .. })));
}

#[tokio::test]
async fn test_pipeline_cancel_notifies_admission() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_core::queue::pipeline::HealthTracker::default());
    let pipeline = RequestPipeline::new(admission.clone(), queue, health);

    let (req, _rx) = mock_incoming(uuid::Uuid::now_v7());
    let id = req.id;
    let requestor = req.requestor.clone();
    pipeline.submit(req).unwrap();

    let usage_before = admission.requestor_usage(&requestor);
    assert_eq!(usage_before.0, 1);

    pipeline.cancel(id);

    let usage_after = admission.requestor_usage(&requestor);
    assert_eq!(usage_after.0, 0);
}

#[tokio::test]
async fn test_scheduler_loop_dispatches() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_core::queue::pipeline::HealthTracker::default());
    let pipeline = Arc::new(RequestPipeline::new(admission.clone(), queue.clone(), health.clone()));

    let shutdown = CancellationToken::new();
    let scheduler_handle = tokio::spawn(run_scheduler(
        queue.clone(),
        admission.clone(),
        Arc::new(MockCapacityChecker),
        Arc::new(MockServerSelector),
        health.clone(),
        shutdown.clone(),
    ));

    let mut rxs = vec![];
    for _ in 0..5 {
        let (req, rx) = mock_incoming(uuid::Uuid::now_v7());
        pipeline.submit(req).unwrap();
        rxs.push(rx);
    }

    for rx in rxs {
        let res = tokio::time::timeout(Duration::from_secs(1), rx).await.unwrap().unwrap();
        assert!(matches!(res, DispatchResult::Dispatched(..)));
    }

    shutdown.cancel();
    scheduler_handle.await.unwrap();
}

#[tokio::test]
async fn test_scheduler_respects_shutdown() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_core::queue::pipeline::HealthTracker::default());
    let pipeline = Arc::new(RequestPipeline::new(admission.clone(), queue.clone(), health.clone()));

    let shutdown = CancellationToken::new();
    let scheduler_handle = tokio::spawn(run_scheduler(
        queue.clone(),
        admission.clone(),
        Arc::new(MockCapacityChecker),
        Arc::new(MockServerSelector),
        health.clone(),
        shutdown.clone(),
    ));

    let (req1, rx1) = mock_incoming(uuid::Uuid::now_v7());
    let (req2, rx2) = mock_incoming(uuid::Uuid::now_v7());

    pipeline.submit(req1).unwrap();
    pipeline.submit(req2).unwrap();

    shutdown.cancel();
    scheduler_handle.await.unwrap();

    let res1 = rx1.await.unwrap();
    let res2 = rx2.await.unwrap();

    // Check they got ShuttingDown
    assert!(matches!(res1, DispatchResult::ShuttingDown | DispatchResult::Dispatched(..)));
    assert!(matches!(res2, DispatchResult::ShuttingDown | DispatchResult::Dispatched(..)));
}

struct MockActiveRequestProvider(std::collections::HashMap<RequestorId, u32>);
impl llamaforge_core::types::traits::ActiveRequestProvider for MockActiveRequestProvider {
    fn active_requests(&self) -> std::collections::HashMap<RequestorId, u32> {
        self.0.clone()
    }
}

#[tokio::test]
async fn test_reconciliation_corrects_leak() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let requestor = RequestorId::new(Arc::from("user1"));

    // Create usage entry
    let (req, _) = mock_incoming(uuid::Uuid::now_v7());
    let _ = admission.admit(req).unwrap();
    // This adds 1 queued.

    // Inject a leak
    for _ in 0..5 {
        admission.record_dequeue(&requestor, DequeueReason::Dispatched);
    }
    assert_eq!(admission.requestor_usage(&requestor).1, 5); // 5 in flight

    // Reconcile with 3 active
    let mut active = std::collections::HashMap::new();
    active.insert(requestor.clone(), 3);
    let provider = Arc::new(MockActiveRequestProvider(active));

    admission.reconcile_in_flight(&provider.active_requests());

    // Verify correction
    assert_eq!(admission.requestor_usage(&requestor).1, 3);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn test_health_signals() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));

    struct NoCapacityChecker;
    impl CapacityChecker for NoCapacityChecker {
        fn serves_model(
            &self,
            t: &ModelTarget,
            c: &llamaforge_core::routing::RequestCapabilities,
        ) -> bool {
            self.has_idle_slot(t, c)
        }

        fn has_idle_slot(
            &self,

            _: &ModelTarget,

            _: &llamaforge_core::routing::RequestCapabilities,
        ) -> bool {
            false
        }
    }

    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_core::queue::pipeline::HealthTracker::default());
    let pipeline = Arc::new(RequestPipeline::new(admission.clone(), queue.clone(), health.clone()));

    // Initially not ready/live because no heartbeat
    assert!(!pipeline.ready());
    assert!(!pipeline.live());

    let shutdown = CancellationToken::new();
    let scheduler_handle = tokio::spawn(run_scheduler(
        queue.clone(),
        admission.clone(),
        Arc::new(NoCapacityChecker),
        Arc::new(MockServerSelector),
        health.clone(),
        shutdown.clone(),
    ));

    // Wait for heartbeat
    tokio::time::sleep(Duration::from_millis(1500)).await;

    assert!(pipeline.ready());
    assert!(pipeline.live());
    assert!(!pipeline.degraded());

    // Trigger degraded
    // Simulate high depth by enqueuing 801 items
    for i in 0..801 {
        let (mut req, _rx) = mock_incoming(uuid::Uuid::now_v7());
        req.requestor = RequestorId::new(Arc::from(format!("user{}", i)));
        pipeline.submit(req).unwrap();
    }

    // Let the heartbeat trigger and set degraded_since
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!pipeline.degraded()); // Not degraded yet, only since 2 seconds

    // Advance 31 seconds so it crosses the 30s degraded threshold
    tokio::time::sleep(Duration::from_secs(31)).await;
    assert!(pipeline.degraded());

    shutdown.cancel();
    scheduler_handle.await.unwrap();
}
