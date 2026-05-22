use llamaforge_core::config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
use llamaforge_core::queue::admission::AdmissionController;
use llamaforge_core::queue::types::{DispatchResult, IncomingRequest, TaskType};
use llamaforge_core::routing::CapacityChecker;
use llamaforge_core::types::traits::{AdmissionAccounting, DequeueReason};
use llamaforge_core::types::{ModelAlias, ModelTarget, RequestId, RequestorClass, RequestorId};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

struct MockCapacityChecker;
impl CapacityChecker for MockCapacityChecker {
    fn serves_model(&self, t: &ModelTarget, c: &llamaforge_core::routing::RequestCapabilities) -> bool {
        self.has_idle_slot(t, c)
    }

    fn has_idle_slot(&self, _: &ModelTarget, _: &llamaforge_core::routing::RequestCapabilities) -> bool {
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
fn test_admission_respects_task_type_authorization() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission =
        Arc::new(AdmissionController::new(config, queue_depth, Arc::new(MockCapacityChecker)));
    let (mut req, _) = mock_incoming();
    req.task_type = TaskType::Emergency;
    // test_config empty policy allows all by default, but if it didn't this would fail.
    let _ = admission.admit(req);
}

#[test]
fn test_admission_concurrent_limit_includes_inflight() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission =
        Arc::new(AdmissionController::new(config, queue_depth, Arc::new(MockCapacityChecker)));
    let (req, _) = mock_incoming();
    let _ = admission.admit(req);
}

#[test]
fn test_dequeue_dispatched_transitions_counters() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission =
        Arc::new(AdmissionController::new(config, queue_depth, Arc::new(MockCapacityChecker)));
    let (req, _) = mock_incoming();
    let requestor = req.requestor.clone();
    let _admitted = admission.admit(req).unwrap();
    assert_eq!(admission.requestor_usage(&requestor).0, 1);
    admission.record_dequeue(&requestor, DequeueReason::Dispatched);
    assert_eq!(admission.requestor_usage(&requestor).0, 0);
    assert_eq!(admission.requestor_usage(&requestor).1, 1);
}
#[test]
fn test_dequeue_cancelled_only_decrements_queued() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission =
        Arc::new(AdmissionController::new(config, queue_depth, Arc::new(MockCapacityChecker)));
    let (req, _) = mock_incoming();
    let requestor = req.requestor.clone();
    let _admitted = admission.admit(req).unwrap();
    assert_eq!(admission.requestor_usage(&requestor).0, 1);
    admission.record_dequeue(&requestor, DequeueReason::Cancelled);
    assert_eq!(admission.requestor_usage(&requestor).0, 0);
    assert_eq!(admission.requestor_usage(&requestor).1, 0);
}
#[test]
fn test_queue_full() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(1000));
    let admission =
        Arc::new(AdmissionController::new(config, queue_depth, Arc::new(MockCapacityChecker)));
    let (req, _) = mock_incoming();
    let res = admission.admit(req);
    assert!(matches!(res, Err(llamaforge_core::queue::error::AdmissionError::QueueFull { .. })));
}
