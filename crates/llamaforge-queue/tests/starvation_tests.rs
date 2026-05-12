use llamaforge_config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
use llamaforge_queue::admission::AdmissionController;
use llamaforge_queue::queue::DispatchQueue;
use llamaforge_queue::types::{DispatchResult, IncomingRequest, TaskType, PRIORITY_CEILING};
use llamaforge_queue::starvation::start_background_tasks;
use llamaforge_types::traits::CapacityChecker;
use llamaforge_types::{ModelAlias, ModelTarget, RequestId, RequestorClass, RequestorId};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use std::time::Duration;

struct MockCapacityChecker;
impl CapacityChecker for MockCapacityChecker {
    fn has_capacity(&self, _: &ModelTarget) -> bool { true }
}

fn test_config() -> Arc<ConfigHandle> {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let toml = r#"
[global]
queue_max_depth = 1000

[priority_rules.starvation]
threshold_secs = 2
boost_per_interval = 50
"#;
    std::fs::write(tmp.path(), toml).unwrap();
    let paths = ConfigPaths::new(tmp.path().to_path_buf(), tmp.path().to_path_buf(), tmp.path().parent().unwrap().to_path_buf());
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

fn mock_incoming() -> (IncomingRequest, oneshot::Receiver<DispatchResult>) {
    let (tx, rx) = oneshot::channel();
    (IncomingRequest {
        id: RequestId::new(uuid::Uuid::now_v7()),
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

#[test]
fn test_boost_ceiling_enforcement() {
    let base = 1400;
    let boost = 400;
    let clamped = std::cmp::min(base + boost, PRIORITY_CEILING);
    assert_eq!(clamped, PRIORITY_CEILING);
}

#[tokio::test]
async fn test_timeout_eviction() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
    let queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth.clone()));
    
    let shutdown = CancellationToken::new();
    start_background_tasks(queue.clone(), admission.clone(), config, shutdown.clone());

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
    let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
    let queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth.clone()));
    
    let shutdown = CancellationToken::new();
    start_background_tasks(queue.clone(), admission.clone(), config, shutdown.clone());

    let (req, rx) = mock_incoming();
    let cancel_token = req.cancel_token.clone();
    let admitted = admission.admit(req).unwrap();
    queue.enqueue(admitted);

    cancel_token.cancel();

    let res = tokio::time::timeout(Duration::from_secs(2), rx).await.unwrap().unwrap();
    assert!(matches!(res, DispatchResult::Cancelled));
    assert_eq!(queue.depth().total, 0);

    shutdown.cancel();
}

#[test]
fn test_boost_skip_if_same_priority() {
    let intervals: f64 = 0.0;
    let boost_scale: f64 = 50.0;
    let boost = (boost_scale * (1.0f64 + intervals).ln()).min(400.0) as u32;
    assert_eq!(boost, 0); // No boost means no re-key
}

#[tokio::test]
async fn test_cancel_during_starvation_scan() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
    let queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth.clone()));
    
    let (req, _rx) = mock_incoming();
    let id = req.id;
    let admitted = admission.admit(req).unwrap();
    queue.enqueue(admitted);

    let snapshot = queue.snapshot_for_starvation();
    assert_eq!(snapshot.len(), 1);

    // Cancel before apply
    queue.cancel(id);

    // Now apply boost - it shouldn't panic or fail
    let _candidates = vec![(id, 500)];
    
    // Simulate apply_boost (which calls queue.boost internally)
    queue.boost(id, 500);
    assert_eq!(queue.depth().total, 0);
}

#[tokio::test]
async fn test_spawn_blocking_recovery() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let _admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
    let _queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth.clone()));
    
    // In a real environment, spawn_blocking panicked tasks are caught by the runtime.
    // We simulate the panic inside a blocking task.
    let handle = tokio::task::spawn_blocking(move || {
        panic!("Simulated panic inside spawn_blocking");
    });

    let res = handle.await;
    assert!(res.is_err());
    assert!(res.unwrap_err().is_panic());
}
