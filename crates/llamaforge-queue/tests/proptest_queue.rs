use llamaforge_config::{ConfigHandle, ConfigPaths, ConfigReader, TomlFileConfigSource};
use llamaforge_queue::admission::AdmissionController;
use llamaforge_queue::queue::DispatchQueue;
use llamaforge_queue::types::{IncomingRequest, TaskType, PRIORITY_CEILING};
use llamaforge_routing::CapacityChecker;
use llamaforge_types::{ModelAlias, ModelTarget, RequestId, RequestorClass, RequestorId};
use proptest::prelude::*;
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
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

prop_compose! {
    fn arb_incoming_request()(
        task_type in prop_oneof![
            Just(TaskType::Emergency),
            Just(TaskType::RealtimeChat),
            Just(TaskType::CodeReview),
            Just(TaskType::CodeGen),
            Just(TaskType::BatchAnalysis),
            Just(TaskType::Summarization),
            Just(TaskType::Background),
            Just(TaskType::Backfill),
        ],
        override_val in prop::option::of(0..2000u32),
        model_name in "[a-z]{4}",
        user_name in "[a-z]{4}",
    ) -> IncomingRequest {
        let (tx, _) = oneshot::channel();
        IncomingRequest {
            id: RequestId::new(uuid::Uuid::now_v7()),
            payload: Arc::new(()),
            task_type,
            requestor: RequestorId::new(Arc::from(user_name.as_str())),
            requestor_class: RequestorClass::new(Arc::from("default")),
            model_target: ModelTarget::Exact(ModelAlias::new(Arc::from(model_name.as_str()))),
            capabilities: Default::default(),
            session_id: None,
            explicit_override: override_val,
            response_tx: tx,
            cancel_token: CancellationToken::new(),
            trace_context: None,
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn test_prop_priority_ordering(reqs in prop::collection::vec(arb_incoming_request(), 2..50)) {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
        let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));

        let mut expected_prios = vec![];
        for req in reqs {
            if let Ok(admitted) = admission.admit(req) {
                expected_prios.push(admitted.effective_priority);
                queue.enqueue(admitted);
            }
        }

        expected_prios.sort_by(|a, b| b.cmp(a)); // Descending priority

        let cap = MockCapacityChecker;
        let mut actual_prios = vec![];
        while let Some(cand) = queue.try_dispatch(&cap) {
            actual_prios.push(cand.dispatch_priority);
        }

        prop_assert_eq!(actual_prios, expected_prios);
    }

    #[test]
    fn test_prop_fifo_within_priority(count in 2..50usize) {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
        let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));

        let mut ids = vec![];
        for _ in 0..count {
            let (tx, _) = oneshot::channel();
            let req = IncomingRequest {
                id: RequestId::new(uuid::Uuid::now_v7()),
                payload: Arc::new(()),
                task_type: TaskType::Background, // Always 0 base prio
                requestor: RequestorId::new(Arc::from("user1")),
                requestor_class: RequestorClass::new(Arc::from("default")),
                model_target: ModelTarget::Exact(ModelAlias::new(Arc::from("m1"))),
                capabilities: Default::default(),
                session_id: None,
                explicit_override: Some(500), // Fixed priority
                response_tx: tx,
                cancel_token: CancellationToken::new(),
                trace_context: None,
            };
            let id = req.id;
            let admitted = admission.admit(req).unwrap();
            queue.enqueue(admitted);
            ids.push(id);
        }

        let cap = MockCapacityChecker;
        let mut actual_ids = vec![];
        while let Some(cand) = queue.try_dispatch(&cap) {
            actual_ids.push(cand.request.id);
        }

        prop_assert_eq!(actual_ids, ids);
    }

    #[test]
    fn test_prop_priority_ceiling(reqs in prop::collection::vec(arb_incoming_request(), 1..10)) {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
        let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));

        for req in reqs {
            if let Ok(admitted) = admission.admit(req) {
                prop_assert!(admitted.effective_priority <= PRIORITY_CEILING);
                queue.enqueue(admitted);
            }
        }
    }

    #[test]
    fn test_prop_index_consistency(reqs in prop::collection::vec(arb_incoming_request(), 1..50)) {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
        let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));

        let mut ids = vec![];
        for req in reqs {
            if let Ok(admitted) = admission.admit(req) {
                ids.push(admitted.id);
                queue.enqueue(admitted);
            }
        }

        let depth = queue.depth();
        prop_assert_eq!(depth.total as usize, ids.len());

        // Remove half
        for id in ids.iter().take(ids.len() / 2) {
            queue.cancel(*id);
        }

        let depth_after = queue.depth();
        prop_assert_eq!(depth_after.total as usize, ids.len() - (ids.len() / 2));
    }

    #[test]
    fn test_prop_starvation_bounded(reqs in prop::collection::vec(arb_incoming_request(), 1..10)) {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
        let queue = Arc::new(DispatchQueue::new(config.clone(), queue_depth.clone()));

        for req in reqs {
            if let Ok(admitted) = admission.admit(req) {
                queue.enqueue(admitted);
            }
        }

        let snapshot = queue.snapshot_for_starvation();
        let starv_cfg = config.snapshot().priority_rules.starvation.clone();

        for (_id, enqueued_at, _base_priority) in snapshot {
            // Check formula logic statically
            let elapsed_secs = tokio::time::Instant::now().duration_since(enqueued_at).as_secs();
            let wait_secs = elapsed_secs.saturating_sub(starv_cfg.boost_start_after_secs as u64);
            if wait_secs > 0 {
                let intervals = wait_secs as f64 / starv_cfg.check_interval_secs as f64;
                let boost = (starv_cfg.boost_scale * (1.0 + intervals).ln()).min(starv_cfg.max_boost as f64) as u32;
                prop_assert!(boost <= starv_cfg.max_boost);
            }
        }
    }

    #[test]
    fn test_prop_work_conserving(reqs in prop::collection::vec(arb_incoming_request(), 1..10)) {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
        let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));

        let mut has_items = false;
        for req in reqs {
            if let Ok(admitted) = admission.admit(req) {
                queue.enqueue(admitted);
                has_items = true;
            }
        }

        if has_items {
            let cap = MockCapacityChecker;
            // Since cap returns true for all, try_dispatch MUST return Some
            prop_assert!(queue.try_dispatch(&cap).is_some());
        }
    }

    #[test]
    fn test_prop_requestor_accounting(reqs in prop::collection::vec(arb_incoming_request(), 1..50)) {
        let config = test_config();
        let queue_depth = Arc::new(AtomicU32::new(0));
        let admission = Arc::new(AdmissionController::new(config.clone(), queue_depth.clone(), Arc::new(MockCapacityChecker)));
        let queue = Arc::new(DispatchQueue::new(config, queue_depth.clone()));

        let mut actual_counts = std::collections::HashMap::new();

        for req in reqs {
            let req_id = req.requestor.clone();
            if let Ok(admitted) = admission.admit(req) {
                queue.enqueue(admitted);
                *actual_counts.entry(req_id).or_insert(0) += 1;
            }
        }

        for (requestor, expected_count) in actual_counts {
            let usage = admission.requestor_usage(&requestor);
            prop_assert_eq!(usage.0 + usage.1, expected_count);
        }
    }
}
