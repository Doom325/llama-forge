//! Telemetry and Prometheus metrics registration.
//! See design doc Section 7.1.

use metrics::{describe_counter, describe_gauge, describe_histogram, Unit};
use std::sync::Arc;
use tokio::time::{interval, Duration, MissedTickBehavior};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::manager::ServerManager;
use crate::types::ServerLifecycle;

pub fn register_metrics() {
    describe_gauge!(
        "llamaforge_server_status",
        Unit::Count,
        "Current lifecycle state of the server (0=Stopped, 1=Starting, 2=Running, 3=Failed)"
    );
    describe_counter!(
        "llamaforge_server_restarts_total",
        Unit::Count,
        "Total number of times a server has been restarted"
    );
    describe_gauge!(
        "llamaforge_server_uptime_seconds",
        Unit::Seconds,
        "Seconds since the server process started"
    );
    describe_gauge!(
        "llamaforge_vram_reserved_bytes",
        Unit::Bytes,
        "Bytes of VRAM reserved in the ledger for a specific GPU"
    );
    describe_gauge!(
        "llamaforge_vram_free_bytes",
        Unit::Bytes,
        "Bytes of free VRAM currently reported by the GPU telemetry"
    );
    describe_gauge!(
        "llamaforge_gpu_utilization_pct",
        Unit::Percent,
        "Current GPU utilization percentage (0-100)"
    );
    describe_counter!(
        "llamaforge_server_cumulative_requests",
        Unit::Count,
        "Total number of dispatch requests routed to a server since startup"
    );
    describe_gauge!(
        "llamaforge_pending_dispatches",
        Unit::Count,
        "Number of requests currently waiting in the dispatch queue for a server"
    );
    describe_histogram!(
        "llamaforge_health_check_duration_seconds",
        Unit::Seconds,
        "Duration of the health check HTTP request to the server process"
    );
}

/// A tracing layer that converts specific domain log events into Prometheus metrics.
pub struct DomainMetricsLayer;

impl<S> Layer<S> for DomainMetricsLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        struct Visitor {
            server_id: Option<String>,
            health_fail: bool,
            degraded: bool,
            deadlock: bool,
            stopped: bool,
        }

        impl tracing::field::Visit for Visitor {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "server.id" {
                    self.server_id = Some(value.to_string());
                }
            }
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "server.id" {
                    self.server_id = Some(format!("{:?}", value));
                }
            }
            fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
                match field.name() {
                    "server.health_fail" => self.health_fail = value,
                    "server.performance_degraded" => self.degraded = value,
                    "server.deadlock_detected" => self.deadlock = value,
                    "server.stopped" => self.stopped = value,
                    _ => {}
                }
            }
        }

        let mut visitor = Visitor {
            server_id: None,
            health_fail: false,
            degraded: false,
            deadlock: false,
            stopped: false,
        };
        event.record(&mut visitor);

        if let Some(id) = visitor.server_id {
            // In the PR6 requirements: reason ∈ {crash, health_fail, degradation, deadlock, config}
            if visitor.deadlock {
                metrics::counter!("llamaforge_server_restarts_total", "server_id" => id, "reason" => "deadlock").increment(1);
            } else if visitor.degraded {
                metrics::counter!("llamaforge_server_restarts_total", "server_id" => id, "reason" => "degradation").increment(1);
            } else if visitor.health_fail {
                metrics::counter!("llamaforge_server_restarts_total", "server_id" => id, "reason" => "health_fail").increment(1);
            } else if visitor.stopped {
                // If it stopped and we restart it, we can call it a crash
                metrics::counter!("llamaforge_server_restarts_total", "server_id" => id, "reason" => "crash").increment(1);
            }
        }
    }
}

/// Spawns a background task that periodically updates Prometheus gauges based on the ServerManager's state.
pub fn spawn_metrics_poller(mgr: Arc<ServerManager>) {
    tokio::spawn(async move {
        let mut interval = interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            interval.tick().await;

            // Gather metrics from DashMaps
            for state in mgr.running_servers() {
                let s_id = state.0.to_string();
                let status_val = match state.1.lifecycle {
                    ServerLifecycle::Stopped => 0.0,
                    ServerLifecycle::Starting => 1.0,
                    ServerLifecycle::Running => 2.0,
                    ServerLifecycle::Draining | ServerLifecycle::Restarting => 1.5,
                    ServerLifecycle::Failed => 3.0,
                };
                metrics::gauge!("llamaforge_server_status", "server_id" => s_id.clone())
                    .set(status_val);

                // Uptime
                if let Some(started) = state.1.last_health_check {
                    let uptime = started.elapsed().as_secs_f64();
                    metrics::gauge!("llamaforge_server_uptime_seconds", "server_id" => s_id)
                        .set(uptime);
                }
            }

            // VRAM ledger
            let ledger = mgr.vram_ledger();
            for (gpu, bytes) in ledger {
                metrics::gauge!("llamaforge_vram_reserved_bytes", "gpu_id" => gpu.0.to_string())
                    .set(bytes as f64);
            }
        }
    });
}
