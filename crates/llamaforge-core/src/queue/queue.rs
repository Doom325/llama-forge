use crate::queue::types::{DispatchCandidate, QueueDepthInfo, QueuedRequest, TierLabel};
use crate::config::ConfigHandle;
use crate::types::{ModelAlias, ModelTarget, RequestId};
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// The internal multi-tier priority queue.
pub(crate) struct FlatPriorityQueue {
    queue: BTreeMap<Reverse<u32>, VecDeque<QueuedRequest>>,
    index: HashMap<RequestId, u32>,
    total_depth: u32,
}

impl FlatPriorityQueue {
    pub(crate) fn new() -> Self {
        Self { queue: BTreeMap::new(), index: HashMap::new(), total_depth: 0 }
    }

    pub(crate) fn insert(&mut self, req: QueuedRequest) {
        let prio = req.current_effective_priority;
        self.index.insert(req.id, prio);
        self.queue.entry(Reverse(prio)).or_default().push_back(req);
        self.total_depth += 1;
    }

    pub(crate) fn remove_by_id(&mut self, id: RequestId) -> Option<QueuedRequest> {
        let prio = self.index.remove(&id)?;
        let deque = self.queue.get_mut(&Reverse(prio))?;

        let idx = deque.iter().position(|r| r.id == id)?;
        let req = deque.remove(idx)?;

        if deque.is_empty() {
            self.queue.remove(&Reverse(prio));
        }

        self.total_depth -= 1;
        Some(req)
    }

    pub(crate) fn try_dispatch_scan<C: crate::routing::CapacityChecker + ?Sized>(
        &mut self,
        capacity: &C,
    ) -> Option<(DispatchCandidate, u32)> {
        let mut target_coord = None;
        let mut scan_depth = 0;

        for (rev_prio, deque) in self.queue.iter() {
            for (idx, req) in deque.iter().enumerate() {
                scan_depth += 1;
                if capacity.has_idle_slot(&req.model_target, &req.capabilities) {
                    target_coord = Some((rev_prio.0, idx));
                    break;
                }
            }
            if target_coord.is_some() {
                break;
            }
        }

        if let Some((prio, idx)) = target_coord {
            let deque = self.queue.get_mut(&Reverse(prio))?;
            let req = deque.remove(idx)?;

            if deque.is_empty() {
                self.queue.remove(&Reverse(prio));
            }

            self.index.remove(&req.id);
            self.total_depth -= 1;

            let wait = req.enqueued_at.elapsed();
            Some((
                DispatchCandidate { request: req, dispatch_priority: prio, queue_wait: wait },
                scan_depth,
            ))
        } else {
            None
        }
    }

    pub(crate) fn re_key(&mut self, id: RequestId, new_priority: u32) {
        let Some(&current_prio) = self.index.get(&id) else { return };
        if current_prio == new_priority {
            return;
        }

        if let Some(mut req) = self.remove_by_id(id) {
            req.current_effective_priority = new_priority;
            self.insert(req);
        }
    }

    pub(crate) fn position(&self, id: RequestId) -> Option<u32> {
        let target_prio = *self.index.get(&id)?;
        let mut pos = 1;

        for (&Reverse(prio), deque) in self.queue.iter() {
            if prio < target_prio {
                break;
            }
            if prio == target_prio {
                for req in deque.iter() {
                    if req.id == id {
                        return Some(pos);
                    }
                    pos += 1;
                }
                break;
            }
            pos += deque.len() as u32;
        }

        Some(pos)
    }

    pub(crate) fn snapshot_for_starvation(&self) -> Vec<(RequestId, tokio::time::Instant, u32)> {
        let mut snapshot = Vec::with_capacity(self.total_depth as usize);
        for deque in self.queue.values() {
            for req in deque.iter() {
                snapshot.push((req.id, req.enqueued_at, req.base_priority));
            }
        }
        snapshot
    }

    pub(crate) fn depth_info(&self) -> QueueDepthInfo {
        let mut by_tier = HashMap::new();
        let mut by_model = HashMap::new();

        for (&Reverse(prio), deque) in self.queue.iter() {
            let tier = TierLabel::from_priority(prio);
            *by_tier.entry(tier).or_default() += deque.len() as u32;

            for req in deque.iter() {
                if let ModelTarget::Exact(alias) = &req.model_target {
                    *by_model.entry(alias.clone()).or_default() += 1;
                }
            }
        }

        QueueDepthInfo { total: self.total_depth, by_tier, by_model }
    }
    pub(crate) fn maintenance_sweep(&mut self) -> Vec<QueuedRequest> {
        let mut to_remove = Vec::new();
        let now = tokio::time::Instant::now();

        for deque in self.queue.values() {
            for req in deque.iter() {
                if req.cancel_token.is_cancelled()
                    || now.duration_since(req.enqueued_at) > req.queue_timeout
                {
                    to_remove.push(req.id);
                }
            }
        }

        let mut removed = Vec::with_capacity(to_remove.len());
        for id in to_remove {
            if let Some(req) = self.remove_by_id(id) {
                removed.push(req);
            }
        }
        removed
    }
}

pub(crate) struct SlidingWindowRate {
    timestamps: VecDeque<std::time::Instant>,
    window_size: Duration,
}

impl SlidingWindowRate {
    pub(crate) fn new(window_size: Duration) -> Self {
        Self { timestamps: VecDeque::new(), window_size }
    }

    pub(crate) fn record(&mut self) {
        let now = std::time::Instant::now();
        self.timestamps.push_back(now);
        while self.timestamps.len() > crate::queue::types::SLIDING_WINDOW_MAX_ENTRIES {
            self.timestamps.pop_front();
        }
        self.prune(now);
    }

    pub(crate) fn rate(&mut self) -> Option<f64> {
        let now = std::time::Instant::now();
        self.prune(now);

        if self.timestamps.len() < 5 {
            return None;
        }

        let count = self.timestamps.len();
        Some(count as f64 / self.window_size.as_secs_f64())
    }

    fn prune(&mut self, now: std::time::Instant) {
        while let Some(&t) = self.timestamps.front() {
            if now.duration_since(t) > self.window_size {
                self.timestamps.pop_front();
            } else {
                break;
            }
        }
    }
}

pub struct DispatchQueue {
    inner: std::sync::Mutex<FlatPriorityQueue>,
    #[allow(dead_code)]
    config: Arc<ConfigHandle>,
    dispatch_notify: tokio::sync::Notify,
    #[allow(dead_code)]
    queue_depth: Arc<AtomicU32>,
    dispatch_epoch: watch::Sender<u64>,
    model_dispatch_rate: std::sync::Mutex<HashMap<ModelAlias, SlidingWindowRate>>,
}

impl DispatchQueue {
    pub fn new(config: Arc<ConfigHandle>, queue_depth: Arc<AtomicU32>) -> Self {
        let (tx, _) = watch::channel(0);
        Self {
            inner: std::sync::Mutex::new(FlatPriorityQueue::new()),
            config,
            dispatch_notify: tokio::sync::Notify::new(),
            queue_depth,
            dispatch_epoch: tx,
            model_dispatch_rate: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn enqueue(&self, req: crate::queue::types::AdmittedRequest) -> crate::queue::types::QueuePosition {
        let id = req.id;
        let prio = req.effective_priority;
        let model_target = req.model_target.clone();

        let pos = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.insert(QueuedRequest::from_admitted(req));
            inner.position(id).expect("request must be in index after insert")
        };

        self.dispatch_notify.notify_one();
        self.dispatch_epoch.send_modify(|v| *v += 1);

        let estimated_wait_secs = if let ModelTarget::Exact(alias) = &model_target {
            let mut rates = self.model_dispatch_rate.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(rate_tracker) = rates.get_mut(alias) {
                rate_tracker.rate().map(|rate| if rate > 0.0 { (pos as f64) / rate } else { 0.0 })
            } else {
                None
            }
        } else {
            None
        };

        crate::queue::types::QueuePosition {
            position: pos,
            estimated_wait_secs,
            tier_label: TierLabel::from_priority(prio),
        }
    }

    pub fn try_dispatch<C: crate::routing::CapacityChecker + ?Sized>(
        &self,
        capacity: &C,
    ) -> Option<DispatchCandidate> {
        let start_time = std::time::Instant::now();
        let depth_at_scan;

        let candidate = {
            let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let result = inner.try_dispatch_scan(capacity);
            if let Some((_, scan_depth)) = result {
                depth_at_scan = scan_depth;
            } else {
                depth_at_scan = inner.total_depth;
            }
            result.map(|(c, _)| c)
        }; // Lock released here

        metrics::histogram!("llamaforge_dispatch_scan_depth").record(depth_at_scan as f64);

        if let Some(cand) = candidate {
            self.dispatch_epoch.send_modify(|v| *v += 1);
            if let ModelTarget::Exact(alias) = &cand.request.model_target {
                let mut rates = self.model_dispatch_rate.lock().unwrap_or_else(|p| p.into_inner());
                rates
                    .entry(alias.clone())
                    .or_insert_with(|| SlidingWindowRate::new(Duration::from_secs(60)))
                    .record();
            }

            metrics::histogram!("llamaforge_queue_wait_seconds")
                .record(cand.queue_wait.as_secs_f64());
            metrics::histogram!("llamaforge_dispatch_duration_seconds")
                .record(start_time.elapsed().as_secs_f64());

            Some(cand)
        } else {
            None
        }
    }

    pub fn cancel(&self, id: RequestId) -> Option<QueuedRequest> {
        // Note: queue_depth is decremented in PR5 orchestration layer via AdmissionAccounting
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut req = inner.remove_by_id(id)?;
        self.dispatch_epoch.send_modify(|v| *v += 1);
        let (dummy_tx, _) = tokio::sync::oneshot::channel();
        let real_tx = std::mem::replace(&mut req.response_tx, dummy_tx);
        let _ = real_tx.send(crate::queue::types::DispatchResult::Cancelled);
        Some(req)
    }

    pub fn boost(&self, id: RequestId, new_priority: u32) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let clamped_priority = new_priority.min(crate::queue::types::PRIORITY_CEILING);
        inner.re_key(id, clamped_priority);
    }

    pub fn sweep(&self) -> Vec<QueuedRequest> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let removed = inner.maintenance_sweep();
        if !removed.is_empty() {
            self.dispatch_epoch.send_modify(|v| *v += 1);
        }
        removed
    }

    pub fn snapshot_for_starvation(&self) -> Vec<(RequestId, tokio::time::Instant, u32)> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.snapshot_for_starvation()
    }

    pub fn position(&self, id: RequestId) -> Option<u32> {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        inner.position(id)
    }

    pub fn depth(&self) -> QueueDepthInfo {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let info = inner.depth_info();

        metrics::gauge!("llamaforge_queue_depth").set(info.total as f64);
        for (model, depth) in &info.by_model {
            metrics::gauge!("llamaforge_queue_depth_by_model", "model" => model.to_string())
                .set(*depth as f64);
        }

        // Update dispatch rate metrics
        let mut rates = self.model_dispatch_rate.lock().unwrap_or_else(|p| p.into_inner());
        for (model, rate) in rates.iter_mut() {
            if let Some(r) = rate.rate() {
                metrics::gauge!("llamaforge_dispatch_rate_per_second", "model" => model.to_string()).set(r);
            }
        }

        info
    }

    pub fn subscribe_dispatch_epoch(&self) -> watch::Receiver<u64> {
        self.dispatch_epoch.subscribe()
    }

    pub async fn wait_for_dispatch_signal(&self) {
        self.dispatch_notify.notified().await;
    }

    pub fn notify_slot_available(&self) {
        self.dispatch_notify.notify_one();
    }
}

impl crate::types::traits::SlotNotifier for DispatchQueue {
    fn notify_slot_available(&self) {
        self.notify_slot_available();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigPaths, TomlFileConfigSource};
    use crate::routing::CapacityChecker;
    use crate::types::{ModelAlias, RequestId, RequestorClass, RequestorId};
    use std::sync::Arc;
    use tokio::sync::oneshot;
    use tokio_util::sync::CancellationToken;

    struct MockCapacityChecker {
        allowed_models: Vec<ModelAlias>,
    }

    impl CapacityChecker for MockCapacityChecker {
        fn serves_model(
            &self,
            target: &ModelTarget,
            capabilities: &crate::routing::RequestCapabilities,
        ) -> bool {
            // For the mock, "serves model" and "has idle slot" both
            // collapse to the allow-listed-models check.
            self.has_idle_slot(target, capabilities)
        }
        fn has_idle_slot(
            &self,
            target: &ModelTarget,
            _capabilities: &crate::routing::RequestCapabilities,
        ) -> bool {
            match target {
                ModelTarget::Exact(alias) => self.allowed_models.contains(alias),
                ModelTarget::AnyOf(aliases) => {
                    aliases.iter().any(|a| self.allowed_models.contains(a))
                }
                ModelTarget::AnyWithCapability(_) => !self.allowed_models.is_empty(), // mock logic
            }
        }
    }

    fn test_config() -> Arc<ConfigHandle> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "[global]\n").unwrap();
        let paths = ConfigPaths::new(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            tmp.path().parent().unwrap().to_path_buf(),
        );
        let source = Arc::new(TomlFileConfigSource::new(paths));
        Arc::new(ConfigHandle::load(source).unwrap())
    }

    fn mock_admitted_with_rx(
        prio: u32,
        model: &str,
    ) -> (crate::queue::types::AdmittedRequest, oneshot::Receiver<crate::queue::types::DispatchResult>) {
        let (tx, rx) = oneshot::channel();
        (
            crate::queue::types::AdmittedRequest {
                id: RequestId::new(uuid::Uuid::now_v7()),
                payload: Arc::new(()),
                task_type: crate::queue::types::TaskType::Background,
                requestor: RequestorId::new(Arc::from("user1")),
                requestor_class: RequestorClass::new(Arc::from("default")),
                model_target: ModelTarget::Exact(ModelAlias::new(Arc::from(model))),
                capabilities: Default::default(),
                session_id: None,
                effective_priority: prio,
                base_priority: prio,
                requestor_bonus: 0,
                explicit_override: None,
                enqueued_at: tokio::time::Instant::now(),
                enqueued_at_utc: chrono::Utc::now(),
                queue_timeout: Duration::from_secs(300),
                response_tx: tx,
                cancel_token: CancellationToken::new(),
                trace_context: None,
                request_span: None,
                admission_token: None,
            },
            rx,
        )
    }

    fn mock_admitted(prio: u32, model: &str) -> crate::queue::types::AdmittedRequest {
        mock_admitted_with_rx(prio, model).0
    }

    #[test]
    fn test_fifo_within_same_priority() {
        let q = DispatchQueue::new(test_config(), Arc::new(AtomicU32::new(0)));
        let mut ids = vec![];
        for _ in 0..5 {
            let req = mock_admitted(600, "model1");
            ids.push(req.id);
            q.enqueue(req);
        }

        let cap =
            MockCapacityChecker { allowed_models: vec![ModelAlias::new(Arc::from("model1"))] };
        for id in ids {
            let cand = q.try_dispatch(&cap).unwrap();
            assert_eq!(cand.request.id, id);
        }
        assert!(q.try_dispatch(&cap).is_none());
    }

    #[test]
    fn test_priority_ordering() {
        let q = DispatchQueue::new(test_config(), Arc::new(AtomicU32::new(0)));
        q.enqueue(mock_admitted(100, "model1"));
        q.enqueue(mock_admitted(600, "model1"));
        q.enqueue(mock_admitted(300, "model1"));

        let cap =
            MockCapacityChecker { allowed_models: vec![ModelAlias::new(Arc::from("model1"))] };
        assert_eq!(q.try_dispatch(&cap).unwrap().request.current_effective_priority, 600);
        assert_eq!(q.try_dispatch(&cap).unwrap().request.current_effective_priority, 300);
        assert_eq!(q.try_dispatch(&cap).unwrap().request.current_effective_priority, 100);
        assert!(q.try_dispatch(&cap).is_none());
    }

    #[test]
    fn test_model_aware_dispatch() {
        let q = DispatchQueue::new(test_config(), Arc::new(AtomicU32::new(0)));
        let req_a = mock_admitted(600, "model_a");
        let id_a = req_a.id;
        q.enqueue(req_a);

        let req_b = mock_admitted(100, "model_b");
        let id_b = req_b.id;
        q.enqueue(req_b);

        // Only model_b has capacity. Even though model_a is higher priority,
        // model_b should dispatch.
        let cap =
            MockCapacityChecker { allowed_models: vec![ModelAlias::new(Arc::from("model_b"))] };
        let cand = q.try_dispatch(&cap).unwrap();
        assert_eq!(cand.request.id, id_b);

        // model_a still waiting
        assert!(q.try_dispatch(&cap).is_none());
        assert_eq!(q.depth().total, 1);

        // Now model_a has capacity
        let cap_a =
            MockCapacityChecker { allowed_models: vec![ModelAlias::new(Arc::from("model_a"))] };
        let cand_a = q.try_dispatch(&cap_a).unwrap();
        assert_eq!(cand_a.request.id, id_a);
        assert_eq!(q.depth().total, 0);
    }

    #[tokio::test]
    async fn test_cancel_removes_and_sends_result() {
        let q = DispatchQueue::new(test_config(), Arc::new(AtomicU32::new(0)));
        let (req, rx) = mock_admitted_with_rx(100, "model1");
        let id = req.id;

        q.enqueue(req);
        assert_eq!(q.depth().total, 1);

        let removed = q.cancel(id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, id);
        assert_eq!(q.depth().total, 0);

        let result = rx.await.unwrap();
        assert!(matches!(result, crate::queue::types::DispatchResult::Cancelled));
    }

    #[test]
    fn test_boost_clamps_to_ceiling() {
        let q = DispatchQueue::new(test_config(), Arc::new(AtomicU32::new(0)));
        let req = mock_admitted(100, "model1");
        let id = req.id;
        q.enqueue(req);

        q.boost(id, 5000);

        let cap =
            MockCapacityChecker { allowed_models: vec![ModelAlias::new(Arc::from("model1"))] };
        let cand = q.try_dispatch(&cap).unwrap();
        assert_eq!(cand.request.current_effective_priority, crate::queue::types::PRIORITY_CEILING);
    }

    #[test]
    fn test_boost_skip_if_same_priority() {
        let q = DispatchQueue::new(test_config(), Arc::new(AtomicU32::new(0)));
        let req1 = mock_admitted(500, "model1");
        let id1 = req1.id;
        q.enqueue(req1);

        let req2 = mock_admitted(500, "model1");
        let id2 = req2.id;
        q.enqueue(req2);

        // Boost req1 to 500 (same prio). It should not move to the back of the queue.
        q.boost(id1, 500);

        let cap =
            MockCapacityChecker { allowed_models: vec![ModelAlias::new(Arc::from("model1"))] };
        assert_eq!(q.try_dispatch(&cap).unwrap().request.id, id1);
        assert_eq!(q.try_dispatch(&cap).unwrap().request.id, id2);
    }

    #[test]
    fn test_sliding_window_rate() {
        let mut rate = SlidingWindowRate::new(Duration::from_secs(10));
        assert_eq!(rate.rate(), None); // Cold start

        for _ in 0..5 {
            rate.record();
        }
        assert!(rate.rate().is_some());

        // Fill up to max entries
        for _ in 0..3000 {
            rate.record();
        }
        assert_eq!(rate.timestamps.len(), crate::queue::types::SLIDING_WINDOW_MAX_ENTRIES);
    }

    #[tokio::test]
    async fn test_dispatch_epoch_increments() {
        let q = DispatchQueue::new(test_config(), Arc::new(AtomicU32::new(0)));
        let mut rx = q.subscribe_dispatch_epoch();
        let initial = *rx.borrow();

        q.enqueue(mock_admitted(100, "model1"));
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), initial + 1);

        let cap =
            MockCapacityChecker { allowed_models: vec![ModelAlias::new(Arc::from("model1"))] };
        q.try_dispatch(&cap);
        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), initial + 2);
    }

    #[tokio::test]
    async fn test_dispatch_epoch_coalescing() {
        let q = DispatchQueue::new(test_config(), Arc::new(AtomicU32::new(0)));
        let mut rx = q.subscribe_dispatch_epoch();
        let initial = *rx.borrow();

        for _ in 0..10 {
            q.enqueue(mock_admitted(100, "model1"));
        }

        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), initial + 10);
    }

    #[test]
    fn test_estimated_wait_time() {
        let q = DispatchQueue::new(test_config(), Arc::new(AtomicU32::new(0)));

        // Cold model (no dispatches)
        let pos1 = q.enqueue(mock_admitted(100, "model1"));
        assert_eq!(pos1.estimated_wait_secs, None);

        // Record 10 dispatches in 60s window (rate = 1/6 per sec)
        let cap =
            MockCapacityChecker { allowed_models: vec![ModelAlias::new(Arc::from("model1"))] };
        for _ in 0..10 {
            q.enqueue(mock_admitted(100, "model1"));
            q.try_dispatch(&cap);
        }

        let pos2 = q.enqueue(mock_admitted(100, "model1"));
        assert!(pos2.estimated_wait_secs.is_some());

        // AnyOf model target
        let mut req_any = mock_admitted(100, "model2");
        req_any.model_target = ModelTarget::AnyOf(vec![
            ModelAlias::new(Arc::from("model1")),
            ModelAlias::new(Arc::from("model2")),
        ]);
        let pos3 = q.enqueue(req_any);
        assert_eq!(pos3.estimated_wait_secs, None);
    }

    #[test]
    fn test_index_consistency() {
        let q = DispatchQueue::new(test_config(), Arc::new(AtomicU32::new(0)));
        let req = mock_admitted(100, "model1");
        let id = req.id;

        q.enqueue(req);

        let inner = q.inner.lock().unwrap();
        assert_eq!(inner.index.len(), 1);
        assert_eq!(inner.queue.len(), 1);
        drop(inner);

        q.boost(id, 200);
        let inner = q.inner.lock().unwrap();
        assert_eq!(inner.index.len(), 1);
        assert_eq!(inner.queue.len(), 1);
        drop(inner);

        q.cancel(id);
        let inner = q.inner.lock().unwrap();
        assert_eq!(inner.index.len(), 0);
        assert_eq!(inner.queue.len(), 0);
    }
}
