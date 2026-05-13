use crate::admission::AdmissionController;
use crate::queue::DispatchQueue;
use crate::types::{DispatchResult, IncomingRequest, QueueDepthInfo, QueuePosition};
use llamaforge_routing::{CapacityChecker, ServerSelector};
use llamaforge_types::traits::{AdmissionAccounting, DequeueReason, SlotDispatcher, SlotNotifier};
use llamaforge_types::{ModelTarget, RequestId, RequestorId};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, info_span, warn, Instrument};

pub struct HealthTracker {
    pub last_heartbeat: std::sync::RwLock<Option<tokio::time::Instant>>,
    pub degraded_since: std::sync::RwLock<Option<tokio::time::Instant>>,
}

impl Default for HealthTracker {
    fn default() -> Self {
        Self {
            last_heartbeat: std::sync::RwLock::new(None),
            degraded_since: std::sync::RwLock::new(None),
        }
    }
}

/// The primary entry point for submitting requests to the orchestrator.
///
/// See design doc Section 2.5.
pub struct RequestPipeline {
    admission: Arc<AdmissionController>,
    queue: Arc<DispatchQueue>,
    health: Arc<HealthTracker>,
}

impl RequestPipeline {
    pub fn new(
        admission: Arc<AdmissionController>,
        queue: Arc<DispatchQueue>,
        health: Arc<HealthTracker>,
    ) -> Self {
        Self { admission, queue, health }
    }

    pub fn submit(
        &self,
        req: IncomingRequest,
    ) -> Result<QueuePosition, crate::error::AdmissionError> {
        let admitted = self.admission.admit(req)?;
        // Note: enqueue is currently infallible, so rollback of admission counters is not needed.
        Ok(self.queue.enqueue(admitted))
    }

    pub fn position(&self, id: RequestId) -> Option<u32> {
        self.queue.position(id)
    }

    pub fn cancel(&self, id: RequestId) -> Option<RequestId> {
        let req = self.queue.cancel(id)?;
        self.admission.record_dequeue(&req.requestor, DequeueReason::Cancelled);
        Some(id)
    }

    pub fn subscribe_dispatch_epoch(&self) -> watch::Receiver<u64> {
        self.queue.subscribe_dispatch_epoch()
    }

    pub fn depth(&self) -> QueueDepthInfo {
        self.queue.depth()
    }

    pub fn requestor_usage(&self, requestor: &RequestorId) -> (u32, u32) {
        self.admission.requestor_usage(requestor)
    }

    pub fn ready(&self) -> bool {
        // Admission initialized + scheduler running (heartbeat > 0)
        self.health.last_heartbeat.read().unwrap().is_some()
    }

    pub fn live(&self) -> bool {
        let last = *self.health.last_heartbeat.read().unwrap();
        if let Some(last_time) = last {
            last_time.elapsed().as_secs() <= 10
        } else {
            false
        }
    }

    pub fn degraded(&self) -> bool {
        let since = *self.health.degraded_since.read().unwrap();
        if let Some(since_time) = since {
            since_time.elapsed().as_secs() > 30
        } else {
            false
        }
    }
}

/// A guard that records request completion and notifies the scheduler when dropped.
///
/// See design doc Section 2.2.
pub struct CompletionGuard {
    requestor: RequestorId,
    admission: Arc<dyn AdmissionAccounting>,
    queue_notify: Arc<dyn SlotNotifier>,
    _dispatch_token: Box<dyn std::any::Any + Send + Sync>, // Placeholder for Component 06 DispatchToken
}

impl CompletionGuard {
    /// Internal helper for testing only.
    pub fn new_for_test(
        requestor: RequestorId,
        admission: Arc<dyn AdmissionAccounting>,
        queue_notify: Arc<dyn SlotNotifier>,
    ) -> Self {
        Self { requestor, admission, queue_notify, _dispatch_token: Box::new(()) }
    }
}

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.admission.record_completion(&self.requestor);
        }));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.queue_notify.notify_slot_available();
        }));
    }
}

/// The scheduler coordinator loop.
///
/// See design doc Section 2.6.
pub async fn run_scheduler(
    queue: Arc<DispatchQueue>,
    admission: Arc<AdmissionController>,
    capacity: Arc<dyn CapacityChecker>,
    server_selector: Arc<dyn ServerSelector>,
    dispatcher: Arc<dyn SlotDispatcher>,
    health: Arc<HealthTracker>,
    shutdown: CancellationToken,
) {
    info!("Scheduler loop starting");
    let mut dispatch_tasks = tokio::task::JoinSet::new();

    // Create a background heartbeat and degraded-check loop
    let h1 = health.clone();
    let q1 = queue.clone();
    let s1 = shutdown.clone();
    let a1 = admission.clone();
    dispatch_tasks.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = s1.cancelled() => break,
                _ = interval.tick() => {
                    let now = tokio::time::Instant::now();
                    *h1.last_heartbeat.write().unwrap() = Some(now);

                    let depth = q1.depth().total;
                    let max_depth = a1.get_max_queue_depth();
                    if depth > (max_depth as f64 * 0.8) as u32 {
                        let mut since = h1.degraded_since.write().unwrap();
                        if since.is_none() {
                            *since = Some(now);
                        }
                    } else {
                        *h1.degraded_since.write().unwrap() = None;
                    }
                }
            }
        }
    });

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Shutdown signal received, draining queue");
                let drain_future = drain_queue(&queue, &admission);
                let _ = tokio::time::timeout(Duration::from_secs(10), drain_future).await;
                break;
            }
            Some(res) = dispatch_tasks.join_next(), if !dispatch_tasks.is_empty() => {
                if let Err(e) = res {
                    error!("Dispatch task panicked: {:?}", e);
                }
            }
            _ = queue.wait_for_dispatch_signal() => {
                // Wake up and try to dispatch as many as possible
                while let Some(candidate) = queue.try_dispatch(capacity.as_ref()) {
                    let req = candidate.request;
                    let request_id = req.id;
                    let requestor = req.requestor.clone();

                    // Record dequeue BEFORE potentially slow dispatch/routing
                    admission.record_dequeue(&requestor, DequeueReason::Dispatched);

                    let decision = server_selector.select_server(&req);
                    let Ok(routing) = decision else {
                        warn!(%request_id, "No server selected for candidate model target {:?}", req.model_target);
                        // If we can't route it now, we re-enqueue or fail it.
                        // Design doc says if has_capacity passed but select_server fails, it's a race.
                        // We fail the request to avoid HOL blocking if we can't find a server.
                        admission.record_completion(&requestor);
                        queue.notify_slot_available();
                        let _ = req.response_tx.send(DispatchResult::Cancelled);
                        continue;
                    };

                    let dispatcher = dispatcher.clone();
                    let admission_acc: Arc<dyn AdmissionAccounting> = admission.clone();
                    let queue_notify: Arc<dyn SlotNotifier> = queue.clone();
                    let payload = req.payload.clone();

                    let span = req.request_span.unwrap_or_else(|| info_span!("request", %request_id));

                    dispatch_tasks.spawn(async move {
                        match dispatcher.dispatch(routing.server_id(), payload).await {
                            Ok((stream, token)) => {
                                let guard = CompletionGuard {
                                    requestor,
                                    admission: admission_acc,
                                    queue_notify,
                                    _dispatch_token: Box::new(token),
                                };
                                let _ = req.response_tx.send(DispatchResult::Dispatched(stream, Box::new(guard)));
                            }
                            Err(e) => {
                                error!(%request_id, "Dispatch failed: {:?}", e);
                                // Fallback: if dispatch failed, we still need to record completion
                                admission_acc.record_completion(&requestor);
                                queue_notify.notify_slot_available();
                                let _ = req.response_tx.send(DispatchResult::ShuttingDown); // Or appropriate error
                            }
                        }
                    }.instrument(span));
                }
            }
        }
    }

    while let Some(res) = dispatch_tasks.join_next().await {
        if let Err(e) = res {
            error!("Dispatch task panicked during shutdown: {:?}", e);
        }
    }

    info!("Scheduler loop exited");
}

async fn drain_queue(queue: &DispatchQueue, admission: &AdmissionController) {
    // Priority-order drain
    // We don't have a specific "drain all" but we can try_dispatch with a checker that says yes to everything
    // and then send ShuttingDown.
    struct DrainChecker;
    impl CapacityChecker for DrainChecker {
        fn has_capacity(
            &self,
            _: &ModelTarget,
            _: &llamaforge_routing::RequestCapabilities,
        ) -> bool {
            true
        }
    }

    while let Some(candidate) = queue.try_dispatch(&DrainChecker) {
        admission.record_dequeue(&candidate.request.requestor, DequeueReason::ShuttingDown);
        let _ = candidate.request.response_tx.send(DispatchResult::ShuttingDown);
    }
}

// Support DispatchResult as a token by wrapping CompletionGuard in a trait
impl llamaforge_types::traits::DispatchToken for CompletionGuard {}
