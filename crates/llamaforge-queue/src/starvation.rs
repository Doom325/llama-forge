use crate::admission::AdmissionController;
use crate::queue::DispatchQueue;
use crate::types::DispatchResult;
use llamaforge_config::{ConfigHandle, ConfigReader};
use llamaforge_types::traits::{ActiveRequestProvider, AdmissionAccounting, DequeueReason};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, info_span};

/// Start background tasks for the queue.
pub fn start_background_tasks(
    queue: Arc<DispatchQueue>,
    admission: Arc<AdmissionController>,
    config: Arc<ConfigHandle>,
    active_provider: Option<Arc<dyn ActiveRequestProvider>>,
    shutdown: CancellationToken,
) -> tokio::task::JoinSet<()> {
    let q1 = queue.clone();
    let q2 = queue.clone();
    let s1 = shutdown.clone();
    let s2 = shutdown.clone();
    let c1 = config.clone();
    let a2 = admission.clone();

    let mut tasks = tokio::task::JoinSet::new();

    tasks.spawn(async move {
        starvation_boost_loop(q1, c1, s1).await;
    });

    tasks.spawn(async move {
        maintenance_sweep_loop(q2, a2, s2).await;
    });

    if let Some(provider) = active_provider {
        let s3 = shutdown.clone();
        tasks.spawn(async move {
            reconciliation_sweep_loop(admission, provider, s3).await;
        });
    }
    tasks
}

async fn reconciliation_sweep_loop(
    admission: Arc<AdmissionController>,
    provider: Arc<dyn ActiveRequestProvider>,
    shutdown: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    // Skip immediate first tick
    interval.tick().await;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Reconciliation sweep loop exiting due to shutdown");
                break;
            }
            _ = interval.tick() => {
                let active_set = provider.active_requests();
                admission.reconcile_in_flight(&active_set);
            }
        }
    }
}

async fn starvation_boost_loop(
    queue: Arc<DispatchQueue>,
    config: Arc<ConfigHandle>,
    shutdown: CancellationToken,
) {
    let check_interval = { config.snapshot().priority_rules.starvation.check_interval_secs as u64 };
    let mut interval = tokio::time::interval(Duration::from_secs(check_interval.max(1)));

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Starvation boost loop exiting due to shutdown");
                break;
            }
            _ = interval.tick() => {
                let starv_cfg = config.snapshot().priority_rules.starvation.clone();
                let snapshot = queue.snapshot_for_starvation();

                // Phase 2: Compute
                let now = tokio::time::Instant::now();
                let mut candidates = Vec::new();
                for (id, enqueued_at, base_priority) in snapshot {
                    let elapsed_secs = now.duration_since(enqueued_at).as_secs();
                    let wait_secs = elapsed_secs.saturating_sub(starv_cfg.boost_start_after_secs as u64);

                    if wait_secs > 0 {
                        let intervals = wait_secs as f64 / starv_cfg.check_interval_secs as f64;
                        let boost = (starv_cfg.boost_scale * (1.0 + intervals).ln()).min(starv_cfg.max_boost as f64) as u32;
                        if boost > 0 {
                            candidates.push((id, base_priority + boost));
                        }
                    }
                }

                // Phase 3: Apply
                if !candidates.is_empty() {
                    let depth = queue.depth().total;
                    let boosted_count = candidates.len();
                    let span = info_span!("starvation_cycle", depth, boosted_count);

                    let q = queue.clone();
                    if depth > 200 {
                        let handle = tokio::task::spawn_blocking(move || {
                            span.in_scope(|| {
                                apply_boost(q, candidates);
                            });
                        });
                        if let Err(e) = handle.await {
                            tracing::error!(error = ?e, "queue.starvation_boost_panicked");
                        }
                    } else {
                        span.in_scope(|| {
                            apply_boost(q, candidates);
                        });
                    }

                    metrics::counter!("llamaforge_starvation_boost_applied_total").increment(boosted_count as u64);
                    metrics::histogram!("llamaforge_starvation_boost_duration_seconds").record(now.elapsed().as_secs_f64());
                }
            }
        }
    }
}

fn apply_boost(queue: Arc<DispatchQueue>, candidates: Vec<(llamaforge_types::RequestId, u32)>) {
    for (id, new_prio) in candidates {
        queue.boost(id, new_prio);
    }
}

async fn maintenance_sweep_loop(
    queue: Arc<DispatchQueue>,
    admission: Arc<AdmissionController>,
    shutdown: CancellationToken,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Maintenance sweep loop exiting due to shutdown");
                break;
            }
            _ = interval.tick() => {
                let removed = queue.sweep();
                let mut timeouts = 0;
                let mut cancellations = 0;

                for mut req in removed {
                    if req.cancel_token.is_cancelled() {
                        cancellations += 1;
                        drop(req.admission_token.take());
                        admission.record_dequeue(&req.requestor, DequeueReason::Cancelled);
                        let _ = req.response_tx.send(DispatchResult::Cancelled);
                    } else {
                        timeouts += 1;
                        drop(req.admission_token.take());
                        admission.record_dequeue(&req.requestor, DequeueReason::TimedOut);
                        let _ = req.response_tx.send(DispatchResult::TimedOut { waited: req.queue_timeout });
                    }
                }

                if timeouts > 0 {
                    metrics::counter!("llamaforge_queue_timeouts_total").increment(timeouts as u64);
                }
                if cancellations > 0 {
                    metrics::counter!("llamaforge_queue_cancellations_total", "reason" => "client_disconnect").increment(cancellations as u64);
                }
            }
        }
    }
}
