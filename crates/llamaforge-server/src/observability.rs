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
            restarted: bool,
            reason: Option<String>,
        }

        impl tracing::field::Visit for Visitor {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                if field.name() == "server.id" {
                    self.server_id = Some(value.to_string());
                } else if field.name() == "reason" {
                    self.reason = Some(value.to_string());
                }
            }
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "server.id" {
                    self.server_id = Some(format!("{:?}", value));
                } else if field.name() == "reason" {
                    self.reason = Some(format!("{:?}", value));
                }
            }
            fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
                if field.name() == "server.restarted" {
                    self.restarted = value;
                }
            }
        }

        let mut visitor = Visitor { server_id: None, restarted: false, reason: None };
        event.record(&mut visitor);

        if let Some(id) = visitor.server_id {
            // In the PR6 requirements: reason ∈ {crash, health_fail, degradation, deadlock, config}
            if visitor.restarted {
                let reason = visitor.reason.unwrap_or_else(|| "crash".to_string());
                metrics::counter!("llamaforge_server_restarts_total", "server_id" => id, "reason" => reason).increment(1);
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
            for state in mgr.servers.iter() {
                let s_id = state.id.to_string();
                let status_val = match state.lifecycle {
                    ServerLifecycle::Stopped => 0.0,
                    ServerLifecycle::Starting | ServerLifecycle::Restarting => 1.0,
                    ServerLifecycle::Running | ServerLifecycle::Draining => 2.0,
                    ServerLifecycle::Failed => 3.0,
                    _ => 0.0,
                };
                metrics::gauge!("llamaforge_server_status", "server_id" => s_id.clone())
                    .set(status_val);

                // Uptime
                if let Some(started) = state.first_health_ok_at {
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
