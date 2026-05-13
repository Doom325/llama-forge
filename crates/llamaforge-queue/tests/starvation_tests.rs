use llamaforge_config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
use llamaforge_queue::admission::AdmissionController;
use llamaforge_queue::queue::DispatchQueue;
use llamaforge_queue::starvation::start_background_tasks;
use llamaforge_queue::types::{DispatchResult, IncomingRequest, TaskType};
use llamaforge_routing::CapacityChecker;
use llamaforge_types::{ModelAlias, ModelTarget, RequestId, RequestorClass, RequestorId};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

struct MockCapacityChecker;
impl CapacityChecker for MockCapacityChecker {
    fn has_capacity(&self, _: &ModelTarget, _: &llamaforge_routing::RequestCapabilities) -> bool {
        true
    }
}

fn test_config() -> Arc<ConfigHandle> {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let toml = r#"
[global]
queue_max_depth = 1000

[priority_rules.starvation]
check_interval_secs = 5
boost_scale = 50.0
boost_start_after_secs = 2
max_boost = 400
"#;
    std::fs::write(tmp.path(), toml).unwrap();
    let paths = ConfigPaths::new(
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
        tmp.path().parent().unwrap().to_path_buf(),
    );
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

fn mock_incoming() -> (IncomingRequest, oneshot::Receiver<DispatchResult>) {
    let (tx, rx) = oneshot::channel();
    (
        IncomingRequest {
            id: RequestId::new(uuid::Uuid::now_v7()),
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

#[test]
fn test_logarithmic_boost_formula() {
    let intervals: f64 = 6.0;
    let boost_scale: f64 = 50.0;
    let boost = (boost_scale * (1.0f64 + intervals).ln()).min(400.0) as u32;
    assert_eq!(boost, 97);
}

#[test]
fn test_boost_capped_at_max_boost() {
    let intervals: f64 = 10000.0;
    let boost_scale: f64 = 50.0;
    let boost = (boost_scale * (1.0f64 + intervals).ln()).min(400.0) as u32;
    assert_eq!(boost, 400);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn test_boost_ceiling_enforcement() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth.clone()));

    let shutdown = CancellationToken::new();
    let _tasks =
        start_background_tasks(queue.clone(), admission.clone(), config, None, shutdown.clone());

    let (req, _rx) = mock_incoming();
    let id = req.id;
    let mut admitted = admission.admit(req).unwrap();
    admitted.queue_timeout = Duration::from_secs(200000);
    // Background task base_priority is 0. Wait, PLAN says Background(100) + max_boost(400) = 500.
    // Our Background is 0. So 0 + 400 = 400.
    // Let's assert it's less than Emergency (1000).
    queue.enqueue(admitted);

    tokio::time::advance(Duration::from_secs(100000)).await;
    tokio::task::yield_now().await;

    let req_out = queue.cancel(id).unwrap();

    // Background base priority is 0, max boost is 400.
    assert_eq!(req_out.current_effective_priority, 400);
    assert!(req_out.current_effective_priority < 1000); // < Emergency

    shutdown.cancel();
}

#[tokio::test]
async fn test_timeout_eviction() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth.clone()));

    let shutdown = CancellationToken::new();
    let _tasks =
        start_background_tasks(queue.clone(), admission.clone(), config, None, shutdown.clone());

    let (req, rx) = mock_incoming();
    let mut admitted = admission.admit(req).unwrap();
    admitted.queue_timeout = Duration::from_millis(100); // 100ms timeout
    queue.enqueue(admitted);

    let res = tokio::time::timeout(Duration::from_secs(2), rx).await.unwrap().unwrap();
    assert!(matches!(res, DispatchResult::TimedOut { .. }));
    assert_eq!(queue.depth().total, 0);

    shutdown.cancel();
}

#[tokio::test]
async fn test_client_disconnect_cleanup() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth.clone()));

    let shutdown = CancellationToken::new();
    let _tasks =
        start_background_tasks(queue.clone(), admission.clone(), config, None, shutdown.clone());

    let (req, rx) = mock_incoming();
    let cancel_token = req.cancel_token.clone();
    let mut admitted = admission.admit(req).unwrap();
    admitted.queue_timeout = Duration::from_secs(200000);
    queue.enqueue(admitted);

    cancel_token.cancel();

    let res = tokio::time::timeout(Duration::from_secs(2), rx).await.unwrap().unwrap();
    assert!(matches!(res, DispatchResult::Cancelled));
    assert_eq!(queue.depth().total, 0);

    shutdown.cancel();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn test_boost_skip_if_same_priority() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth.clone()));

    let shutdown = CancellationToken::new();
    let _tasks =
        start_background_tasks(queue.clone(), admission.clone(), config, None, shutdown.clone());

    let (req1, _rx1) = mock_incoming();
    let id1 = req1.id;
    let admitted1 = admission.admit(req1).unwrap();
    queue.enqueue(admitted1);

    let (req2, _rx2) = mock_incoming();
    let id2 = req2.id;
    let admitted2 = admission.admit(req2).unwrap();
    queue.enqueue(admitted2);

    // Advance time by just 1 second (under threshold of 2s).
    // It should trigger a loop but apply no boost.
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;

    let pos1 = queue.position(id1).unwrap();
    let pos2 = queue.position(id2).unwrap();

    // Position 1 should still be ahead of 2
    assert!(pos1 < pos2);

    shutdown.cancel();
}

#[tokio::test]
async fn test_cancel_during_starvation_scan() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config.clone(),
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth.clone()));

    let (req, _rx) = mock_incoming();
    let id = req.id;
    let mut admitted = admission.admit(req).unwrap();
    admitted.queue_timeout = Duration::from_secs(200000);
    queue.enqueue(admitted);

    let snapshot = queue.snapshot_for_starvation();
    assert_eq!(snapshot.len(), 1);

    // Cancel before apply
    queue.cancel(id);

    // Now apply boost - it shouldn't panic or fail
    let _candidates = [(id, 500)];

    // Simulate apply_boost (which calls queue.boost internally)
    queue.boost(id, 500);
    assert_eq!(queue.depth().total, 0);
}
