//! Query and filter types

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use llamaforge_config::RequestorId;
use llamaforge_types::ServerId;

use crate::types::{EventType, RequestId, RequestStatus};

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
}
