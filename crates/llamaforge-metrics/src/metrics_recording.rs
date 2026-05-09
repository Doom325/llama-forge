//! Observability port traits and test doubles.
//!
//! See design doc Section 2.7 for metrics definitions.

use llamaforge_config::ServerId;
use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

/// Port trait for recording request-level and server-level business metrics.
pub trait RequestMetricsRecorder: Send + Sync {
    fn record_request(&self, model: &str, task_type: &str, status: &str);
    fn observe_request_duration(&self, model: &str, task_type: &str, duration_sec: f64);
    fn observe_ttft(&self, model: &str, ttft_sec: f64);
    fn observe_tokens_per_second(&self, model: &str, phase: &str, tps: f64);
    fn set_queue_depth(&self, priority_tier: &str, depth: usize);
    fn observe_queue_wait(&self, priority_tier: &str, wait_sec: f64);
    fn set_server_slots(&self, server_id: &ServerId, active: usize, total: usize);
    fn record_server_restart(&self, server_id: &ServerId, reason: &str);
    fn record_config_reload(&self, result: &str);
}

/// Port trait for recording internal storage health metrics.
pub trait StoreHealthRecorder: Send + Sync {
    fn set_wal_size_bytes(&self, bytes: u64);
    fn set_channel_pending(&self, channel: &str, depth: usize);
    fn record_events_dropped(&self, channel: &str, count: u64);
    fn record_write_result(&self, result: &str);
    fn observe_write_duration(&self, duration_sec: f64);
    fn observe_checkpoint_duration(&self, mode: &str, duration_sec: f64);
    fn observe_read_duration(&self, query_type: &str, duration_sec: f64);
    fn set_read_pool_active(&self, active: usize);
    fn record_prune_rows(&self, deleted: u64);
    fn record_cmd_channel_dropped(&self, count: u64);
}

/// Combined trait for all application metrics.
pub trait AppMetrics: RequestMetricsRecorder + StoreHealthRecorder {}

/// Concrete implementation of the recording traits using the `metrics` facade.
#[derive(Clone, Default)]
#[allow(dead_code)]
pub struct MetricsFacadeRecorder;

#[allow(dead_code)]
impl MetricsFacadeRecorder {
    pub fn new() -> Self {
        describe_counter!("llamaforge_requests_total", "Total requests processed");
        describe_histogram!("llamaforge_request_duration_seconds", "Total latency of requests");
        describe_histogram!("llamaforge_ttft_seconds", "Time to first token");
        describe_histogram!("llamaforge_tokens_per_second", "Generation throughput");
        describe_gauge!("llamaforge_queue_depth", "Number of requests queued");
        describe_histogram!("llamaforge_queue_wait_seconds", "Time spent in queue");
        describe_gauge!("llamaforge_server_slots_active", "Number of active slots");
        describe_gauge!("llamaforge_server_slots_total", "Total available slots");
        describe_counter!("llamaforge_server_restarts_total", "Total server restarts");
        describe_counter!("llamaforge_config_reloads_total", "Total config reloads");

        describe_gauge!("llamaforge_sqlite_wal_size_bytes", "Size of the SQLite WAL file");
        describe_gauge!(
            "llamaforge_channel_pending",
            "Number of events pending in ingest channels"
        );
        describe_counter!(
            "llamaforge_events_dropped_total",
            "Total events dropped due to backpressure"
        );
        describe_counter!(
            "llamaforge_cmd_channel_events_dropped_total",
            "Total events dropped due to writer command channel backpressure"
        );
        describe_counter!("llamaforge_sqlite_writes_total", "Total SQLite writes");
        describe_histogram!(
            "llamaforge_sqlite_write_duration_seconds",
            "Duration of SQLite writes"
        );
        describe_histogram!(
            "llamaforge_sqlite_checkpoint_duration_seconds",
            "Duration of SQLite checkpoints"
        );
        describe_histogram!("llamaforge_sqlite_read_duration_seconds", "Duration of SQLite reads");
        describe_gauge!(
            "llamaforge_sqlite_read_pool_active",
            "Number of active read pool connections"
        );
        describe_counter!("llamaforge_prune_rows_deleted", "Total rows deleted during pruning");

        Self
    }
}

impl RequestMetricsRecorder for MetricsFacadeRecorder {
    fn record_request(&self, model: &str, task_type: &str, status: &str) {
        counter!(
            "llamaforge_requests_total",
            "model" => model.to_string(),
            "task_type" => task_type.to_string(),
            "status" => status.to_string()
        )
        .increment(1);
    }

    fn observe_request_duration(&self, model: &str, task_type: &str, duration_sec: f64) {
        histogram!(
            "llamaforge_request_duration_seconds",
            "model" => model.to_string(),
            "task_type" => task_type.to_string()
        )
        .record(duration_sec);
    }

    fn observe_ttft(&self, model: &str, ttft_sec: f64) {
        histogram!(
            "llamaforge_ttft_seconds",
            "model" => model.to_string()
        )
        .record(ttft_sec);
    }

    fn observe_tokens_per_second(&self, model: &str, phase: &str, tps: f64) {
        histogram!(
            "llamaforge_tokens_per_second",
            "model" => model.to_string(),
            "phase" => phase.to_string()
        )
        .record(tps);
    }

    fn set_queue_depth(&self, priority_tier: &str, depth: usize) {
        gauge!(
            "llamaforge_queue_depth",
            "priority_tier" => priority_tier.to_string()
        )
        .set(depth as f64);
    }

    fn observe_queue_wait(&self, priority_tier: &str, wait_sec: f64) {
        histogram!(
            "llamaforge_queue_wait_seconds",
            "priority_tier" => priority_tier.to_string()
        )
        .record(wait_sec);
    }

    fn set_server_slots(&self, server_id: &ServerId, active: usize, total: usize) {
        let sid = server_id.0.clone();
        gauge!(
            "llamaforge_server_slots_active",
            "server_id" => sid.clone()
        )
        .set(active as f64);
        gauge!(
            "llamaforge_server_slots_total",
            "server_id" => sid
        )
        .set(total as f64);
    }

    fn record_server_restart(&self, server_id: &ServerId, reason: &str) {
        counter!(
            "llamaforge_server_restarts_total",
            "server_id" => server_id.0.clone(),
            "reason" => reason.to_string()
        )
        .increment(1);
    }

    fn record_config_reload(&self, result: &str) {
        counter!(
            "llamaforge_config_reloads_total",
            "result" => result.to_string()
        )
        .increment(1);
    }
}

impl StoreHealthRecorder for MetricsFacadeRecorder {
    fn set_wal_size_bytes(&self, bytes: u64) {
        gauge!("llamaforge_sqlite_wal_size_bytes").set(bytes as f64);
    }

    fn set_channel_pending(&self, channel: &str, depth: usize) {
        gauge!(
            "llamaforge_channel_pending",
            "channel" => channel.to_string()
        )
        .set(depth as f64);
    }

    fn record_events_dropped(&self, channel: &str, count: u64) {
        counter!(
            "llamaforge_events_dropped_total",
            "channel" => channel.to_string()
        )
        .absolute(count);
    }

    fn record_write_result(&self, result: &str) {
        counter!(
            "llamaforge_sqlite_writes_total",
            "result" => result.to_string()
        )
        .increment(1);
    }

    fn observe_write_duration(&self, duration_sec: f64) {
        histogram!("llamaforge_sqlite_write_duration_seconds").record(duration_sec);
    }

    fn observe_checkpoint_duration(&self, mode: &str, duration_sec: f64) {
        histogram!(
            "llamaforge_sqlite_checkpoint_duration_seconds",
            "mode" => mode.to_string()
        )
        .record(duration_sec);
    }

    fn observe_read_duration(&self, query_type: &str, duration_sec: f64) {
        histogram!(
            "llamaforge_sqlite_read_duration_seconds",
            "query_type" => query_type.to_string()
        )
        .record(duration_sec);
    }

    fn set_read_pool_active(&self, active: usize) {
        gauge!("llamaforge_sqlite_read_pool_active").set(active as f64);
    }

    fn record_prune_rows(&self, deleted: u64) {
        counter!("llamaforge_prune_rows_deleted").increment(deleted);
    }

    fn record_cmd_channel_dropped(&self, count: u64) {
        counter!("llamaforge_cmd_channel_events_dropped_total").absolute(count);
    }
}

impl AppMetrics for MetricsFacadeRecorder {}

/// Helper to generate exponential buckets.
fn exp_buckets(start: f64, end: f64, factor: f64) -> Vec<f64> {
    let mut buckets = Vec::new();
    let mut current = start;
    while current <= end {
        buckets.push(current);
        current *= factor;
    }
    buckets
}

/// Installs the global metrics recorder with LLM-tuned histogram buckets.
pub fn install_metrics_recorder(
) -> Result<PrometheusHandle, metrics_exporter_prometheus::BuildError> {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full(
                "llamaforge_request_duration_seconds".to_string(),
            ),
            &exp_buckets(0.1, 300.0, 2.0),
        )?
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full("llamaforge_ttft_seconds".to_string()),
            &exp_buckets(0.01, 60.0, 2.0),
        )?
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full("llamaforge_tokens_per_second".to_string()),
            &[1.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0],
        )?
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full(
                "llamaforge_sqlite_write_duration_seconds".to_string(),
            ),
            &exp_buckets(0.001, 5.0, 2.0),
        )?
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full(
                "llamaforge_sqlite_checkpoint_duration_seconds".to_string(),
            ),
            &exp_buckets(0.01, 30.0, 2.0),
        )?
        .set_buckets_for_metric(
            metrics_exporter_prometheus::Matcher::Full(
                "llamaforge_sqlite_read_duration_seconds".to_string(),
            ),
            &exp_buckets(0.001, 30.0, 2.0),
        )?
        .install_recorder()
}

/// Test double that discards all recorded metrics.
#[cfg_attr(not(test), allow(dead_code))]
pub struct NoopMetrics;

impl RequestMetricsRecorder for NoopMetrics {
    fn record_request(&self, _model: &str, _task_type: &str, _status: &str) {}
    fn observe_request_duration(&self, _model: &str, _task_type: &str, _duration_sec: f64) {}
    fn observe_ttft(&self, _model: &str, _ttft_sec: f64) {}
    fn observe_tokens_per_second(&self, _model: &str, _phase: &str, _tps: f64) {}
    fn set_queue_depth(&self, _priority_tier: &str, _depth: usize) {}
    fn observe_queue_wait(&self, _priority_tier: &str, _wait_sec: f64) {}
    fn set_server_slots(&self, _server_id: &ServerId, _active: usize, _total: usize) {}
    fn record_server_restart(&self, _server_id: &ServerId, _reason: &str) {}
    fn record_config_reload(&self, _result: &str) {}
}

impl StoreHealthRecorder for NoopMetrics {
    fn set_wal_size_bytes(&self, _bytes: u64) {}
    fn set_channel_pending(&self, _channel: &str, _depth: usize) {}
    fn record_events_dropped(&self, _channel: &str, _count: u64) {}
    fn record_write_result(&self, _result: &str) {}
    fn observe_write_duration(&self, _duration_sec: f64) {}
    fn observe_checkpoint_duration(&self, _mode: &str, _duration_sec: f64) {}
    fn observe_read_duration(&self, _query_type: &str, _duration_sec: f64) {}
    fn set_read_pool_active(&self, _active: usize) {}
    fn record_prune_rows(&self, _deleted: u64) {}
    fn record_cmd_channel_dropped(&self, _count: u64) {}
}

impl AppMetrics for NoopMetrics {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_metrics() {
        let _ = NoopMetrics;
    }
}
