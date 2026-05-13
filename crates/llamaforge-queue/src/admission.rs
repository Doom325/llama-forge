use crate::config::RequestorClassPolicy;
use crate::error::AdmissionError;
use crate::types::{AdmittedRequest, IncomingRequest, TaskType};
use dashmap::DashMap;
use llamaforge_config::{ConfigHandle, ConfigReader, TaskTypeId};
use llamaforge_routing::CapacityChecker;
use llamaforge_types::traits::{AdmissionAccounting, DequeueReason};
use llamaforge_types::{ModelAlias, ModelTarget, RequestorClass, RequestorId};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::info_span;

pub struct RequestorUsage {
    queued: AtomicU32,
    in_flight: AtomicU32,
}

impl Default for RequestorUsage {
    fn default() -> Self {
        Self { queued: AtomicU32::new(0), in_flight: AtomicU32::new(0) }
    }
}

pub struct AdmissionController {
    config: Arc<ConfigHandle>,
    requestor_usage: DashMap<RequestorId, RequestorUsage>,
    queue_depth: Arc<AtomicU32>,
    model_validator: Arc<dyn CapacityChecker>,
}

impl AdmissionController {
    pub fn new(
        config: Arc<ConfigHandle>,
        queue_depth: Arc<AtomicU32>,
        model_validator: Arc<dyn CapacityChecker>,
    ) -> Self {
        Self { config, requestor_usage: DashMap::new(), queue_depth, model_validator }
    }

    /// Gets or creates requestor usage tracking.
    pub fn requestor_usage(&self, requestor: &RequestorId) -> (u32, u32) {
        let usage = self.requestor_usage.entry(requestor.clone()).or_default();
        (usage.queued.load(Ordering::Relaxed), usage.in_flight.load(Ordering::Relaxed))
    }

    /// Get requestor policy from AppConfig.
    fn get_requestor_policy(&self, class: &RequestorClass) -> RequestorClassPolicy {
        let snapshot = self.config.snapshot();
        if let Some(policy) = snapshot.priority_rules.requestor_classes.get(class.as_str()) {
            policy.clone()
        } else {
            RequestorClassPolicy { max_concurrent: 100, ..Default::default() }
        }
    }

    pub fn get_max_queue_depth(&self) -> u32 {
        self.config.snapshot().global.queue_max_depth
    }

    pub fn admit(&self, req: IncomingRequest) -> Result<AdmittedRequest, AdmissionError> {
        let start_time = tokio::time::Instant::now();
        let max_depth = self.get_max_queue_depth();

        // 1. CAS depth reservation
        let mut current_depth = self.queue_depth.load(Ordering::Relaxed);
        loop {
            if current_depth >= max_depth {
                tracing::info!(
                    request_id = %req.id,
                    requestor = %req.requestor,
                    depth = current_depth,
                    max = max_depth,
                    "queue.rejected: Queue full"
                );
                metrics::counter!("llamaforge_queue_admissions_total", "result" => "rejected_full")
                    .increment(1);
                return Err(AdmissionError::QueueFull {
                    depth: current_depth,
                    max: max_depth,
                    retry_after_secs: 5,
                });
            }
            match self.queue_depth.compare_exchange_weak(
                current_depth,
                current_depth + 1,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_depth = actual,
            }
        }

        // We reserved a spot. We must decrement queue_depth if we fail below.
        let queue_depth_rollback = &self.queue_depth;
        let guard = scopeguard::guard((), |_| {
            queue_depth_rollback.fetch_sub(1, Ordering::SeqCst);
        });

        // 2. Requestor lookup & concurrent limits
        let policy = self.get_requestor_policy(&req.requestor_class);

        {
            let usage = self.requestor_usage.entry(req.requestor.clone()).or_default();
            let queued = usage.queued.load(Ordering::Relaxed);
            let in_flight = usage.in_flight.load(Ordering::Relaxed);

            if queued + in_flight >= policy.max_concurrent {
                tracing::info!(
                    request_id = %req.id,
                    requestor = %req.requestor,
                    current = queued + in_flight,
                    max = policy.max_concurrent,
                    "queue.rejected: Requestor concurrent limit reached"
                );
                metrics::counter!("llamaforge_queue_admissions_total", "result" => "rejected_limit").increment(1);
                return Err(AdmissionError::RequestorLimit {
                    requestor: req.requestor.clone(),
                    current: queued + in_flight,
                    max: policy.max_concurrent,
                });
            }

            // Increment queued atomically
            usage.queued.fetch_add(1, Ordering::SeqCst);
        }

        let requestor_for_rollback = req.requestor.clone();
        let requestor_usage_map = &self.requestor_usage;
        let guard_queued = scopeguard::guard((), move |_| {
            if let Some(usage) = requestor_usage_map.get(&requestor_for_rollback) {
                usage.queued.fetch_sub(1, Ordering::SeqCst);
            }
        });

        // 3. Task type authorization
        let task_type_id = TaskTypeId::from(req.task_type.to_string());
        if !policy.allowed_task_types.is_empty()
            && !policy.allowed_task_types.contains_key(&task_type_id)
        {
            tracing::warn!(
                request_id = %req.id,
                requestor = %req.requestor,
                task_type = %req.task_type,
                "queue.rejected: Unauthorized task type"
            );
            metrics::counter!("llamaforge_queue_admissions_total", "result" => "rejected_unauthorized").increment(1);
            return Err(AdmissionError::UnauthorizedTaskType { task_type: req.task_type });
        }

        // 4. Model validation
        if !self.model_validator.has_capacity(&req.model_target, &req.capabilities) {
            let model_alias = match &req.model_target {
                ModelTarget::Exact(m) => m.clone(),
                ModelTarget::AnyOf(models) => {
                    models.first().cloned().unwrap_or_else(|| ModelAlias::new("unknown".into()))
                }
                ModelTarget::AnyWithCapability(_) => ModelAlias::new("capability".into()),
            };
            tracing::info!(
                request_id = %req.id,
                requestor = %req.requestor,
                model = %model_alias,
                "queue.rejected: No server for model"
            );
            metrics::counter!("llamaforge_queue_admissions_total", "result" => "rejected_no_server").increment(1);
            return Err(AdmissionError::NoServerForModel { model: model_alias });
        }

        // 5. Priority computation
        let base_priority: u32 = match req.task_type {
            TaskType::Emergency => 1000,
            TaskType::RealtimeChat => 400,
            TaskType::CodeReview => 200,
            TaskType::CodeGen => 200,
            TaskType::BatchAnalysis => 100,
            TaskType::Summarization => 100,
            TaskType::Background => 0,
            TaskType::Backfill => 0,
        };

        let mut effective_priority = base_priority.saturating_add_signed(policy.priority_bonus);

        if let Some(override_val) = req.explicit_override {
            if let Some(range) = policy.allowed_task_types.get(&task_type_id) {
                if override_val < range.min || override_val > range.max {
                    tracing::warn!(
                        request_id = %req.id,
                        requestor = %req.requestor,
                        priority = override_val,
                        min = range.min,
                        max = range.max,
                        "queue.rejected: Priority out of range"
                    );
                    metrics::counter!("llamaforge_queue_admissions_total", "result" => "rejected_priority").increment(1);
                    return Err(AdmissionError::PriorityOutOfRange { priority: override_val });
                }
            }
            effective_priority = override_val;
        }

        effective_priority = effective_priority.min(crate::types::PRIORITY_CEILING);

        let queue_timeout = Duration::from_secs(policy.queue_timeout_secs.unwrap_or(300) as u64);

        // 6. Produce AdmittedRequest
        let request_span = info_span!(
            "request",
            request_id = %req.id,
            requestor = %req.requestor,
            model = ?req.model_target
        );

        if let Some(ctx) = &req.trace_context {
            use tracing_opentelemetry::OpenTelemetrySpanExt;
            request_span.set_parent(ctx.clone());
        }

        let admitted = AdmittedRequest::from_incoming(
            req,
            effective_priority,
            base_priority,
            policy.priority_bonus,
            queue_timeout,
            Some(request_span.clone()),
        );

        // Metrics
        metrics::counter!("llamaforge_queue_admissions_total", "result" => "accepted").increment(1);
        metrics::gauge!("llamaforge_admission_queued").increment(1.0);
        metrics::histogram!("llamaforge_admission_duration_seconds")
            .record(start_time.elapsed().as_secs_f64());

        request_span.in_scope(|| {
            tracing::debug!(
                request_id = %admitted.id,
                model = ?admitted.model_target,
                tier = %crate::types::TierLabel::from_priority(admitted.effective_priority),
                position = current_depth + 1, // rough estimate
                "queue.admitted"
            );
        });

        // Success, disable rollbacks
        scopeguard::ScopeGuard::into_inner(guard);
        scopeguard::ScopeGuard::into_inner(guard_queued);

        Ok(admitted)
    }

    pub fn reconcile_in_flight(&self, active_set: &std::collections::HashMap<RequestorId, u32>) {
        let mut corrections = 0;
        for entry in self.requestor_usage.iter() {
            let requestor = entry.key();
            let usage = entry.value();
            let tracked = usage.in_flight.load(Ordering::SeqCst);
            let actual = active_set.get(requestor).copied().unwrap_or(0);

            if tracked != actual {
                tracing::warn!(
                    requestor = %requestor,
                    tracked = tracked,
                    actual = actual,
                    "queue.reconciliation_correction: Correcting in_flight count"
                );

                // For simplicity, directly set the actual value. In reality, handling concurrent
                // requests properly would require a more sophisticated compare-and-swap loop,
                // but since we're setting it to the known active set, an overwrite is acceptable.
                usage.in_flight.store(actual, Ordering::SeqCst);
                let diff = (actual as i64) - (tracked as i64);
                if diff > 0 {
                    metrics::gauge!("llamaforge_admission_inflight").increment(diff as f64);
                } else {
                    metrics::gauge!("llamaforge_admission_inflight").decrement((-diff) as f64);
                }
                corrections += 1;
            }
        }

        if corrections > 0 {
            metrics::counter!("llamaforge_requestor_reconciliation_corrections")
                .increment(corrections);
        }
    }
}

impl AdmissionAccounting for AdmissionController {
    fn record_completion(&self, requestor: &RequestorId) {
        self.queue_depth.fetch_sub(1, Ordering::SeqCst);

        if let Some(usage) = self.requestor_usage.get(requestor) {
            usage.in_flight.fetch_sub(1, Ordering::SeqCst);
            metrics::gauge!("llamaforge_admission_inflight").decrement(1.0);
        } else {
            tracing::warn!("record_completion called for unknown requestor: {}", requestor);
        }
    }

    fn record_dequeue(&self, requestor: &RequestorId, reason: DequeueReason) {
        if reason != DequeueReason::Dispatched {
            // Cancelled, TimedOut, or ShuttingDown means it leaves the queue without dispatching.
            self.queue_depth.fetch_sub(1, Ordering::SeqCst);
        }

        if let Some(usage) = self.requestor_usage.get(requestor) {
            usage.queued.fetch_sub(1, Ordering::SeqCst);
            metrics::gauge!("llamaforge_admission_queued").decrement(1.0);

            if reason == DequeueReason::Dispatched {
                usage.in_flight.fetch_add(1, Ordering::SeqCst);
                metrics::gauge!("llamaforge_admission_inflight").increment(1.0);
            }
        } else {
            tracing::warn!("record_dequeue called for unknown requestor: {}", requestor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llamaforge_config::{ConfigPaths, TomlFileConfigSource};
    use llamaforge_routing::CapacityChecker;
    use llamaforge_types::RequestId;
    use std::sync::atomic::AtomicU32;
    use tokio::sync::oneshot;
    use tokio_util::sync::CancellationToken;

    struct MockCapacityChecker {
        capacity: bool,
    }

    impl CapacityChecker for MockCapacityChecker {
        fn has_capacity(
            &self,
            _target: &ModelTarget,
            _capabilities: &llamaforge_routing::RequestCapabilities,
        ) -> bool {
            self.capacity
        }
    }

    fn mock_incoming() -> IncomingRequest {
        let (tx, _rx) = oneshot::channel();
        IncomingRequest {
            id: RequestId::new(uuid::Uuid::now_v7()),
            payload: Arc::new(()),
            task_type: TaskType::RealtimeChat,
            requestor: RequestorId::new(Arc::from("user1")),
            requestor_class: RequestorClass::new(Arc::from("default")),
            model_target: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
            capabilities: Default::default(),
            session_id: None,
            explicit_override: None,
            response_tx: tx,
            cancel_token: CancellationToken::new(),
            trace_context: None,
        }
    }

    fn test_config() -> Arc<ConfigHandle> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let tmp_rules = tempfile::NamedTempFile::new().unwrap();
        let toml_str = r#"
[global]
queue_max_depth = 1000

[priority_rules.requestor_classes.default]
max_concurrent = 100
priority_bonus = 50

[priority_rules.requestor_classes.default.allowed_task_types.realtime_chat]
min = 400
max = 600

[priority_rules.requestor_classes.restricted]
max_concurrent = 10
"#;
        std::fs::write(tmp.path(), toml_str).unwrap();
        std::fs::write(tmp_rules.path(), "").unwrap();
        let paths = ConfigPaths::new(
            tmp.path().to_path_buf(),
            tmp_rules.path().to_path_buf(),
            tmp.path().parent().unwrap().to_path_buf(),
        );
        let source = Arc::new(TomlFileConfigSource::new(paths));
        Arc::new(ConfigHandle::load(source).unwrap())
    }

    #[test]
    fn test_admission_happy_path() {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let model_validator = Arc::new(MockCapacityChecker { capacity: true });
        let controller = AdmissionController::new(config, queue_depth.clone(), model_validator);

        let req = mock_incoming();
        let res = controller.admit(req);
        assert!(res.is_ok());

        let admitted = res.unwrap();
        assert_eq!(admitted.effective_priority, 450); // RealtimeChat base (400) + bonus (50)

        assert_eq!(queue_depth.load(Ordering::SeqCst), 1);
        let usage = controller.requestor_usage(&admitted.requestor);
        assert_eq!(usage.0, 1); // queued
        assert_eq!(usage.1, 0); // in_flight
    }

    #[test]
    fn test_queue_full_rejection() {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(1000)); // max_depth
        let model_validator = Arc::new(MockCapacityChecker { capacity: true });
        let controller = AdmissionController::new(config, queue_depth.clone(), model_validator);

        let req = mock_incoming();
        let res = controller.admit(req);
        assert!(matches!(res, Err(AdmissionError::QueueFull { .. })));
        assert_eq!(queue_depth.load(Ordering::SeqCst), 1000);
    }

    #[test]
    fn test_model_validation_failure() {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let model_validator = Arc::new(MockCapacityChecker { capacity: false });
        let controller = AdmissionController::new(config, queue_depth.clone(), model_validator);

        let req = mock_incoming();
        let res = controller.admit(req);
        assert!(matches!(res, Err(AdmissionError::NoServerForModel { .. })));

        // Ensure rollbacks happened
        assert_eq!(queue_depth.load(Ordering::SeqCst), 0);
        let usage = controller.requestor_usage(&RequestorId::new(Arc::from("user1")));
        assert_eq!(usage.0, 0);
    }

    #[test]
    fn test_dequeue_reason_transitions() {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let model_validator = Arc::new(MockCapacityChecker { capacity: true });
        let controller = AdmissionController::new(config, queue_depth.clone(), model_validator);

        let req = mock_incoming();
        let requestor = req.requestor.clone();
        let _admitted = controller.admit(req).unwrap();

        // 1. Dispatch
        controller.record_dequeue(&requestor, DequeueReason::Dispatched);
        let usage = controller.requestor_usage(&requestor);
        assert_eq!(usage.0, 0); // queued
        assert_eq!(usage.1, 1); // in_flight

        // 2. Complete
        controller.record_completion(&requestor);
        let usage = controller.requestor_usage(&requestor);
        assert_eq!(usage.0, 0);
        assert_eq!(usage.1, 0);

        // 3. Cancel
        let req2 = mock_incoming();
        let _admitted2 = controller.admit(req2).unwrap();
        controller.record_dequeue(&requestor, DequeueReason::Cancelled);
        let usage = controller.requestor_usage(&requestor);
        assert_eq!(usage.0, 0);
        assert_eq!(usage.1, 0);
    }

    #[test]
    fn test_requestor_limit() {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let model_validator = Arc::new(MockCapacityChecker { capacity: true });
        let controller = AdmissionController::new(config, queue_depth.clone(), model_validator);

        for _ in 0..100 {
            let req = mock_incoming();
            assert!(controller.admit(req).is_ok());
        }

        // 101st request should fail
        let req = mock_incoming();
        let res = controller.admit(req);
        assert!(matches!(res, Err(AdmissionError::RequestorLimit { .. })));
    }

    #[test]
    fn test_task_type_authorization() {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let model_validator = Arc::new(MockCapacityChecker { capacity: true });
        let controller = AdmissionController::new(config, queue_depth.clone(), model_validator);

        // "default" class only allows "realtime_chat" in the test config
        let mut req = mock_incoming();
        req.task_type = TaskType::Emergency; // Not in allowed_task_types
        let res = controller.admit(req);
        assert!(matches!(res, Err(AdmissionError::UnauthorizedTaskType { .. })));
    }

    #[test]
    fn test_priority_range_validation() {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let model_validator = Arc::new(MockCapacityChecker { capacity: true });
        let controller = AdmissionController::new(config, queue_depth.clone(), model_validator);

        let mut req = mock_incoming();
        // Range is 400..=600 for realtime_chat
        req.explicit_override = Some(9000);
        let res = controller.admit(req);
        assert!(matches!(res, Err(AdmissionError::PriorityOutOfRange { .. })));

        let mut req2 = mock_incoming();
        req2.explicit_override = Some(500);
        let admitted = controller.admit(req2).unwrap();
        assert_eq!(admitted.effective_priority, 500);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cas_contention() {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(950)); // max is 1000, so exactly 50 can succeed
        let model_validator = Arc::new(MockCapacityChecker { capacity: true });
        let controller =
            Arc::new(AdmissionController::new(config, queue_depth.clone(), model_validator));

        let mut handles = vec![];
        for i in 0..100 {
            let ctrl = controller.clone();
            handles.push(tokio::spawn(async move {
                let mut req = mock_incoming();
                req.requestor = RequestorId::new(Arc::from(format!("user{}", i))); // distinct users
                ctrl.admit(req)
            }));
        }

        let mut success = 0;
        let mut reject = 0;
        for h in handles {
            match h.await.unwrap() {
                Ok(_) => success += 1,
                Err(AdmissionError::QueueFull { .. }) => reject += 1,
                Err(e) => panic!("Unexpected error: {:?}", e),
            }
        }

        assert_eq!(success, 50);
        assert_eq!(reject, 50);
        assert_eq!(queue_depth.load(Ordering::SeqCst), 1000);
    }
}
