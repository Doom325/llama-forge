//! Translation layer between the dispatch crate's per-request
//! `AuditRecord` (proxy.rs:41) and the metrics crate's
//! `AuditRecord` / `RequestMetrics` (types.rs:82).
//!
//! Dispatch emits a small post-completion summary; metrics expects a
//! fully populated `RequestMetrics`. The translator fills the
//! caller-known fields (`model_name`, `requestor`, `task_type`,
//! `received_at`) from the surrounding request context, derives
//! durations from `duration`/`ttft`, and leaves request-pipeline-only
//! fields (`effective_priority`, `dispatched_at`/`first_token_at`) as
//! their sensible neutral defaults — those are populated upstream
//! once the dispatch path grows the corresponding observations.
//! `prompt_tokens` is now carried through from the dispatch audit
//! record (#134).

use crate::dispatch::proxy::AuditRecord as DispatchAuditRecord;
use chrono::{DateTime, Utc};
use llamaforge_core::config::{RequestorId as ConfigRequestorId, TaskTypeId};
use llamaforge_metrics::{
    AuditRecord as MetricsAuditRecord, RequestId as MetricsRequestId, RequestMetrics, RequestStatus,
};

/// Context the dispatch `AuditRecord` doesn't carry but the metrics
/// `RequestMetrics` needs. Populated by the caller (typically the
/// `StreamingInferenceService`) at the moment the request is submitted.
#[derive(Debug, Clone)]
pub struct AuditContext {
    pub model_name: String,
    pub requestor: ConfigRequestorId,
    pub task_type: TaskTypeId,
    pub received_at: DateTime<Utc>,
}

/// Convert a dispatch-layer `AuditRecord` into a metrics-layer
/// `AuditRecord`. The metrics crate's batch collector converts this
/// into the on-disk `AuditEvent` and the latency aggregates.
pub fn translate_audit(record: DispatchAuditRecord, ctx: &AuditContext) -> MetricsAuditRecord {
    let DispatchAuditRecord {
        request_id,
        server_id,
        duration,
        ttft,
        tokens_predicted,
        prompt_tokens,
        success,
    } = record;

    let now = Utc::now();
    let total_latency_ms = duration.as_secs_f64() * 1000.0;
    let ttft_ms = ttft.map(|t| t.as_secs_f64() * 1000.0);
    // Generation duration is total - TTFT when both are known.
    let generation_duration_ms = ttft_ms.map(|t| (total_latency_ms - t).max(0.0));
    let generation_tps =
        generation_duration_ms.filter(|d| *d > 0.0).map(|d| (tokens_predicted as f64) * 1000.0 / d);
    // Prompt TPS = prompt_tokens / TTFT. Only meaningful when both are
    // observed and non-zero — TTFT is the time the upstream spent
    // evaluating the prompt before emitting the first decoded token.
    let prompt_tps = ttft_ms
        .filter(|t| *t > 0.0 && prompt_tokens > 0)
        .map(|t| (prompt_tokens as f64) * 1000.0 / t);

    let status = if success { RequestStatus::Completed } else { RequestStatus::Failed };
    let finish_reason = if success { Some("stop".to_string()) } else { None };

    let request_metrics = RequestMetrics {
        request_id: MetricsRequestId::from(request_id.as_uuid().to_string()),
        model_name: ctx.model_name.clone(),
        server_id: Some(server_id),
        slot_id: None,
        task_type: ctx.task_type.clone(),
        requestor: ctx.requestor.clone(),
        effective_priority: 0,
        received_at: ctx.received_at,
        dispatched_at: None,
        first_token_at: ttft
            .map(|t| ctx.received_at + chrono::Duration::from_std(t).unwrap_or_default()),
        completed_at: Some(now),
        queue_wait_ms: 0.0,
        ttft_ms,
        generation_duration_ms,
        total_latency_ms,
        prompt_tokens,
        completion_tokens: tokens_predicted,
        prompt_tps,
        generation_tps,
        status,
        finish_reason,
        was_evicted: false,
        eviction_count: 0,
        error_message: None,
    };

    MetricsAuditRecord { request_metrics }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ctx() -> AuditContext {
        AuditContext {
            model_name: "test-model".to_string(),
            requestor: ConfigRequestorId("anonymous".to_string()),
            task_type: TaskTypeId("default".to_string()),
            received_at: Utc::now(),
        }
    }

    #[test]
    fn translate_success_record() {
        let record = DispatchAuditRecord {
            request_id: llamaforge_core::types::RequestId::new(uuid::Uuid::now_v7()),
            server_id: llamaforge_core::types::ServerId::new(uuid::Uuid::now_v7()),
            duration: Duration::from_millis(120),
            ttft: Some(Duration::from_millis(30)),
            tokens_predicted: 42,
            prompt_tokens: 9,
            success: true,
        };
        let out = translate_audit(record, &ctx());

        assert_eq!(out.request_metrics.model_name, "test-model");
        assert_eq!(out.request_metrics.completion_tokens, 42);
        assert_eq!(out.request_metrics.prompt_tokens, 9);
        // prompt_tps = 9 / 0.030 = 300
        let p_tps = out.request_metrics.prompt_tps.unwrap();
        assert!((p_tps - 300.0).abs() < 1e-6);
        assert_eq!(out.request_metrics.status, RequestStatus::Completed);
        assert_eq!(out.request_metrics.finish_reason.as_deref(), Some("stop"));
        assert!((out.request_metrics.total_latency_ms - 120.0).abs() < 1e-6);
        assert_eq!(out.request_metrics.ttft_ms, Some(30.0));
        // generation = 120 - 30 = 90ms; tps = 42/0.09 ≈ 466.67
        let gen_tps = out.request_metrics.generation_tps.unwrap();
        assert!((gen_tps - 466.666_666_666_666_7).abs() < 0.1);
    }

    #[test]
    fn translate_failed_record_has_no_finish_reason() {
        let record = DispatchAuditRecord {
            request_id: llamaforge_core::types::RequestId::new(uuid::Uuid::now_v7()),
            server_id: llamaforge_core::types::ServerId::new(uuid::Uuid::now_v7()),
            duration: Duration::from_millis(10),
            ttft: None,
            tokens_predicted: 0,
            prompt_tokens: 0,
            success: false,
        };
        let out = translate_audit(record, &ctx());

        assert_eq!(out.request_metrics.status, RequestStatus::Failed);
        assert!(out.request_metrics.finish_reason.is_none());
        assert!(out.request_metrics.ttft_ms.is_none());
        assert!(out.request_metrics.generation_duration_ms.is_none());
        assert!(out.request_metrics.generation_tps.is_none());
    }
}
