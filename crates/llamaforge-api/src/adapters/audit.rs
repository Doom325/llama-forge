//! Translation layer between the dispatch crate's per-request
//! `AuditRecord` (proxy.rs:41) and the metrics crate's
//! `AuditRecord` / `RequestMetrics` (types.rs:82).
//!
//! Dispatch emits a small post-completion summary; metrics expects a
//! fully populated `RequestMetrics`. The translator fills the
//! caller-known fields (`model_name`, `requestor`, `task_type`,
//! `received_at`, `dispatched_at`, `slot_id`) from the surrounding
//! request context, derives durations from `duration`/`ttft`, and
//! computes a real `queue_wait_ms` (dispatch − receipt). `prompt_tokens`
//! is carried through from the dispatch audit record (#134);
//! `dispatched_at`/`slot_id`/`queue_wait_ms` were wired in #230.
//! `effective_priority` is still a neutral default pending a
//! priority-carrying audit record.

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
    /// Wall-clock moment the request was received (before queueing).
    pub received_at: DateTime<Utc>,
    /// Wall-clock moment the request transitioned queue → slot. With
    /// `received_at` this yields a real `queue_wait_ms` (#230).
    pub dispatched_at: DateTime<Utc>,
    /// The llama-server slot the request was dispatched to (#230).
    pub slot_id: u32,
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
        slot_id: Some(ctx.slot_id),
        task_type: ctx.task_type.clone(),
        requestor: ctx.requestor.clone(),
        effective_priority: 0,
        received_at: ctx.received_at,
        dispatched_at: Some(ctx.dispatched_at),
        // TTFT is measured by the proxy from dispatch start (proxy.rs), so the
        // first token lands at `dispatched_at + ttft`, not `received_at + ttft`.
        first_token_at: ttft
            .map(|t| ctx.dispatched_at + chrono::Duration::from_std(t).unwrap_or_default()),
        completed_at: Some(now),
        // Real queue wait: receipt → dispatch. Clamped at 0 to absorb any
        // clock skew between the two captures.
        queue_wait_ms: ((ctx.dispatched_at - ctx.received_at).num_microseconds().unwrap_or(0)
            as f64
            / 1000.0)
            .max(0.0),
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
        let received_at = Utc::now();
        AuditContext {
            model_name: "test-model".to_string(),
            requestor: ConfigRequestorId("anonymous".to_string()),
            task_type: TaskTypeId("default".to_string()),
            received_at,
            // 250ms in the queue before dispatch to slot 3.
            dispatched_at: received_at + chrono::Duration::milliseconds(250),
            slot_id: 3,
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

        // #230: dispatch metadata is populated, not null.
        assert_eq!(out.request_metrics.slot_id, Some(3));
        let dispatched_at = out.request_metrics.dispatched_at.expect("dispatched_at set");
        // queue_wait_ms reflects the 250ms receipt→dispatch gap (not 0).
        assert!((out.request_metrics.queue_wait_ms - 250.0).abs() < 1.0);
        // first_token_at is dispatched_at + ttft (30ms), not received_at + ttft.
        let first_token_at = out.request_metrics.first_token_at.expect("first_token_at set");
        assert_eq!(first_token_at, dispatched_at + chrono::Duration::milliseconds(30));
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
