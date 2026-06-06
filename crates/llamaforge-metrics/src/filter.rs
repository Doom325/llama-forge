//! Query and filter types

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use llamaforge_core::config::RequestorId;
use llamaforge_core::types::ServerId;

use crate::types::{EventType, RequestId, RequestMetrics, RequestStatus};

/// Filter for querying the event log.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub server_id: Option<ServerId>,
    pub event_type: Option<EventType>,
    pub request_id: Option<RequestId>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

/// Filter for querying request metrics.
///
/// The `Deserialize` impl enables direct extraction from a URL query string
/// via `axum::extract::Query<MetricsFilter>` — see
/// `llamaforge-transport::management`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsFilter {
    #[serde(default)]
    pub server_id: Option<ServerId>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub requestor: Option<RequestorId>,
    #[serde(default)]
    pub status: Option<RequestStatus>,
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
}

/// Fields by which request metrics can be aggregated.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupByField {
    ServerId,
    Hour,
    ModelName,
    Requestor,
}

/// Aggregated metrics result.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AggregatedMetrics {
    pub group_key: String,
    pub request_count: u64,
    pub sum_prompt_tokens: u64,
    pub sum_completion_tokens: u64,
    pub sum_total_tokens: u64,
    pub avg_ttft_ms: Option<f64>,
    pub p50_ttft_ms: Option<f64>,
    pub p99_ttft_ms: Option<f64>,
    pub avg_generation_tps: Option<f64>,
    pub p50_generation_tps: Option<f64>,
    pub p99_generation_tps: Option<f64>,
    pub avg_total_latency_ms: Option<f64>,
}

/// Fleet-wide dashboard summary (`GET /api/metrics/summary`): one throughput
/// figure plus request-latency percentiles, matching the frontend
/// `AggregatedMetrics` shape. Distinct from [`AggregatedMetrics`], which is the
/// explorer's grouped, per-dimension view.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequestMetricsSummary {
    /// Aggregate completion tokens/sec over the sampled window.
    pub throughput_total: f64,
    /// Total request-latency percentiles in ms; `None` when there are no samples.
    pub latency_p50_ms: Option<f64>,
    pub latency_p90_ms: Option<f64>,
    pub latency_p99_ms: Option<f64>,
}

/// Summarise a window of request records into a [`RequestMetricsSummary`].
///
/// `throughput_total` is the sum of completion tokens divided by the wall-clock
/// span of the window (`max(received_at) − min(received_at)`, floored at 1s so a
/// single-sample window can't divide by zero). Latency percentiles are
/// nearest-rank over `total_latency_ms` (matching `aggregate_request_metrics`).
pub fn summarize_request_metrics(records: &[RequestMetrics]) -> RequestMetricsSummary {
    if records.is_empty() {
        return RequestMetricsSummary::default();
    }

    let mut latencies: Vec<f64> = records.iter().map(|r| r.total_latency_ms).collect();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = |q: f64| -> Option<f64> {
        let idx = ((latencies.len() as f64 * q) as usize).min(latencies.len() - 1);
        Some(latencies[idx])
    };

    let total_completion: u64 = records.iter().map(|r| r.completion_tokens as u64).sum();
    let lo = records.iter().map(|r| r.received_at).min();
    let hi = records.iter().map(|r| r.received_at).max();
    let span_secs = match (lo, hi) {
        (Some(lo), Some(hi)) => ((hi - lo).num_milliseconds() as f64 / 1000.0).max(1.0),
        _ => 1.0,
    };

    RequestMetricsSummary {
        throughput_total: total_completion as f64 / span_secs,
        latency_p50_ms: pct(0.50),
        latency_p90_ms: pct(0.90),
        latency_p99_ms: pct(0.99),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filters_default() {
        let event_filter = EventFilter::default();
        assert!(event_filter.server_id.is_none());
        assert!(event_filter.event_type.is_none());
        assert!(event_filter.request_id.is_none());
        assert!(event_filter.since.is_none());
        assert!(event_filter.until.is_none());

        let metrics_filter = MetricsFilter::default();
        assert!(metrics_filter.server_id.is_none());
        assert!(metrics_filter.model_name.is_none());
        assert!(metrics_filter.requestor.is_none());
        assert!(metrics_filter.status.is_none());
        assert!(metrics_filter.since.is_none());
        assert!(metrics_filter.until.is_none());
    }

    fn mk(completion_tokens: u32, latency_ms: f64, recv_offset_s: i64) -> RequestMetrics {
        use llamaforge_core::config::{RequestorId, TaskTypeId};
        let base =
            DateTime::parse_from_rfc3339("2026-06-06T00:00:00Z").unwrap().with_timezone(&Utc);
        RequestMetrics {
            request_id: RequestId(format!("r{recv_offset_s}")),
            model_name: "m".to_string(),
            server_id: None,
            slot_id: None,
            task_type: TaskTypeId("chat".to_string()),
            requestor: RequestorId("anon".to_string()),
            effective_priority: 0,
            received_at: base + chrono::Duration::seconds(recv_offset_s),
            dispatched_at: None,
            first_token_at: None,
            completed_at: None,
            queue_wait_ms: 0.0,
            ttft_ms: None,
            generation_duration_ms: None,
            total_latency_ms: latency_ms,
            prompt_tokens: 0,
            completion_tokens,
            prompt_tps: None,
            generation_tps: None,
            status: RequestStatus::Completed,
            finish_reason: None,
            was_evicted: false,
            eviction_count: 0,
            error_message: None,
        }
    }

    #[test]
    fn summarize_empty_is_zero() {
        let s = summarize_request_metrics(&[]);
        assert_eq!(s, RequestMetricsSummary::default());
        assert_eq!(s.throughput_total, 0.0);
        assert!(s.latency_p50_ms.is_none());
    }

    #[test]
    fn summarize_computes_throughput_and_percentiles() {
        // 10 requests: latencies 10..=100ms, 100 completion tokens each, received
        // one second apart (a 9s window).
        let recs: Vec<_> = (0..10).map(|i| mk(100, (i + 1) as f64 * 10.0, i as i64)).collect();
        let s = summarize_request_metrics(&recs);

        // 1000 completion tokens / 9s span.
        assert!((s.throughput_total - (1000.0 / 9.0)).abs() < 1e-6);
        // Nearest-rank over sorted [10,20,…,100]: p50→idx5→60, p90→idx9→100, p99→idx9→100.
        assert_eq!(s.latency_p50_ms, Some(60.0));
        assert_eq!(s.latency_p90_ms, Some(100.0));
        assert_eq!(s.latency_p99_ms, Some(100.0));
    }
}
