//! Observability port traits and test doubles.
//!
//! See design doc Section 2.7 for metrics definitions.

use llamaforge_config::ServerId;

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

/// Test double that discards all recorded metrics.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct NoopMetrics;

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
