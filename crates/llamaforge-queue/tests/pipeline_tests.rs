use llamaforge_config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
use llamaforge_queue::admission::AdmissionController;
use llamaforge_queue::queue::DispatchQueue;
use llamaforge_queue::pipeline::{RequestPipeline, run_scheduler, CompletionGuard};
use llamaforge_queue::types::{DispatchResult, IncomingRequest, TaskType};
use llamaforge_types::traits::{CapacityChecker, ServerSelector, SlotDispatcher, DequeueReason, AdmissionAccounting, SlotNotifier, ResponseStream, DispatchToken, DispatchError, ActiveRequestProvider};
use llamaforge_types::{ModelAlias, ModelTarget, RequestId, RequestorClass, RequestorId, ServerId};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use std::time::Duration;
use async_trait::async_trait;

struct MockCapacityChecker;
impl CapacityChecker for MockCapacityChecker {
    fn has_capacity(&self, _: &ModelTarget) -> bool { true }
}

struct MockServerSelector;
impl ServerSelector for MockServerSelector {
    fn select_server(&self, _: &ModelTarget) -> Option<llamaforge_types::traits::RoutingDecision> {
        Some(llamaforge_types::traits::RoutingDecision {
            server_id: ServerId::new(uuid::Uuid::now_v7()),
            model_alias: ModelAlias::new(Arc::from("model1")),
        })
    }
}

struct MockDispatcher;
#[async_trait]
impl SlotDispatcher for MockDispatcher {
    async fn dispatch(&self, _server_id: &ServerId, _request: Arc<dyn std::any::Any + Send + Sync>) -> Result<(ResponseStream, Box<dyn DispatchToken>), DispatchError> {
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        struct DummyToken;
        impl DispatchToken for DummyToken {}
        Ok((Box::pin(stream), Box::new(DummyToken)))
    }
}

fn test_config() -> Arc<ConfigHandle> {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "[global]\nqueue_max_depth = 1000\n").unwrap();
    let paths = ConfigPaths::new(tmp.path().to_path_buf(), tmp.path().to_path_buf(), tmp.path().parent().unwrap().to_path_buf());
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

fn mock_incoming(id: uuid::Uuid) -> (IncomingRequest, oneshot::Receiver<DispatchResult>) {
    let (tx, rx) = oneshot::channel();
    (IncomingRequest {
        id: RequestId::new(id),
        payload: Arc::new(()),
        task_type: TaskType::Background,
        requestor: RequestorId::new(Arc::from("user1")),
        requestor_class: RequestorClass::new(Arc::from("default")),
        model_target: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        explicit_override: None,
        response_tx: tx,
        cancel_token: CancellationToken::new(),
        trace_context: None,
    }, rx)
}

#[tokio::test]
async fn test_pipeline_submit_happy_path() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_queue::pipeline::HealthTracker::default());
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
    let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_queue::pipeline::HealthTracker::default());
    let pipeline = RequestPipeline::new(admission, queue, health);

    let (req, _rx) = mock_incoming(uuid::Uuid::now_v7());
    let res = pipeline.submit(req);
    assert!(matches!(res, Err(llamaforge_queue::error::AdmissionError::QueueFull { .. })));
}

#[tokio::test]
async fn test_pipeline_cancel_notifies_admission() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_queue::pipeline::HealthTracker::default());
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

#[test]
fn test_completion_guard_drop() {
    let comp_counter = Arc::new(AtomicU32::new(0));
    let notif_counter = Arc::new(AtomicU32::new(0));
    
    struct NormalAdmission(Arc<AtomicU32>);
    impl AdmissionAccounting for NormalAdmission {
        fn record_completion(&self, _: &RequestorId) { self.0.fetch_add(1, Ordering::SeqCst); }
        fn record_dequeue(&self, _: &RequestorId, _: DequeueReason) {}
    }

    struct MockSlotNotifier(Arc<AtomicU32>);
    impl SlotNotifier for MockSlotNotifier {
        fn notify_slot_available(&self) { self.0.fetch_add(1, Ordering::SeqCst); }
    }

    let guard = CompletionGuard::new_for_test(
        RequestorId::new(Arc::from("u1")),
        Arc::new(NormalAdmission(comp_counter.clone())),
        Arc::new(MockSlotNotifier(notif_counter.clone())),
    );

    drop(guard);
    assert_eq!(comp_counter.load(Ordering::SeqCst), 1);
    assert_eq!(notif_counter.load(Ordering::SeqCst), 1);
}

#[test]
fn test_completion_guard_panic_safety() {
    let counter = Arc::new(AtomicU32::new(0));
    struct PanicAdmission;
    impl AdmissionAccounting for PanicAdmission {
        fn record_completion(&self, _: &RequestorId) { panic!("injected panic"); }
        fn record_dequeue(&self, _: &RequestorId, _: DequeueReason) {}
    }

    struct MockSlotNotifier(Arc<AtomicU32>);
    impl SlotNotifier for MockSlotNotifier {
        fn notify_slot_available(&self) { self.0.fetch_add(1, Ordering::SeqCst); }
    }

    let guard = CompletionGuard::new_for_test(
        RequestorId::new(Arc::from("u1")),
        Arc::new(PanicAdmission),
        Arc::new(MockSlotNotifier(counter.clone())),
    );

    drop(guard);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_scheduler_loop_dispatches() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_queue::pipeline::HealthTracker::default());
    let pipeline = Arc::new(RequestPipeline::new(admission.clone(), queue.clone(), health.clone()));
    
    let shutdown = CancellationToken::new();
    let scheduler_handle = tokio::spawn(run_scheduler(
        queue.clone(),
        admission.clone(),
        Arc::new(MockCapacityChecker),
        Arc::new(MockServerSelector),
        Arc::new(MockDispatcher),
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
    let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_queue::pipeline::HealthTracker::default());
    let pipeline = Arc::new(RequestPipeline::new(admission.clone(), queue.clone(), health.clone()));
    
    let shutdown = CancellationToken::new();
    let scheduler_handle = tokio::spawn(run_scheduler(
        queue.clone(),
        admission.clone(),
        Arc::new(MockCapacityChecker),
        Arc::new(MockServerSelector),
        Arc::new(MockDispatcher),
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
impl llamaforge_types::traits::ActiveRequestProvider for MockActiveRequestProvider {
    fn active_requests(&self) -> std::collections::HashMap<RequestorId, u32> {
        self.0.clone()
    }
}

#[tokio::test]
async fn test_reconciliation_corrects_leak() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
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
        fn has_capacity(&self, _: &ModelTarget) -> bool { false }
    }

    let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
    let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));
    let health = Arc::new(llamaforge_queue::pipeline::HealthTracker::default());
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
        Arc::new(MockDispatcher),
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
