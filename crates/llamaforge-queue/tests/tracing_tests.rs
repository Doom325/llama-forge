use llamaforge_config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
use llamaforge_queue::admission::AdmissionController;
use llamaforge_queue::types::{IncomingRequest, TaskType};
use llamaforge_routing::CapacityChecker;
use llamaforge_types::{ModelAlias, ModelTarget, RequestId, RequestorClass, RequestorId};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

struct MockCapacityChecker;
impl CapacityChecker for MockCapacityChecker {
    fn has_capacity(&self, _: &ModelTarget) -> bool {
        true
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

#[test]
fn test_trace_context_propagation() {
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(0));
    let admission = Arc::new(AdmissionController::new(
        config,
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let (tx, _rx) = oneshot::channel();

    // Create an empty context
    let ctx = opentelemetry::Context::new();

    let req = IncomingRequest {
        id: RequestId::new(uuid::Uuid::now_v7()),
        payload: Arc::new(()),
        task_type: TaskType::Background,
        requestor: RequestorId::new(Arc::from("user1")),
        requestor_class: RequestorClass::new(Arc::from("default")),
        model_target: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        explicit_override: None,
        response_tx: tx,
        cancel_token: CancellationToken::new(),
        trace_context: Some(ctx.clone()),
    };

    let admitted = admission.admit(req).unwrap();
    assert!(admitted.trace_context.is_some());

    let queued = llamaforge_queue::types::QueuedRequest::from_admitted(admitted);
    assert!(queued.trace_context.is_some());
}

#[test]
fn test_rejection_log_levels() {
    // We cannot easily test tracing macro outputs without tracing-test or similar
    // We will just call the methods to ensure they don't panic and exercise the code paths.
    let config = test_config();
    let queue_depth = Arc::new(AtomicU32::new(1000)); // Queue full
    let admission = Arc::new(AdmissionController::new(
        config,
        queue_depth.clone(),
        Arc::new(MockCapacityChecker),
    ));
    let (tx, _rx) = oneshot::channel();

    let req = IncomingRequest {
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
    };

    let res = admission.admit(req);
    assert!(matches!(res, Err(llamaforge_queue::error::AdmissionError::QueueFull { .. })));
}
