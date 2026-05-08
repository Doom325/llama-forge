//! Tests for converting domain objects to/from SQLite rows.

use chrono::Utc;
use llamaforge_config::{RequestorId, ServerId, TaskTypeId};
use llamaforge_metrics::{
    AuditEvent, AuditEventRow, ConfigOverlay, ConfigOverlayRow, ConfigSnapshot, ConfigSnapshotRow,
    ConfigSnapshotTrigger, EventType, ManagedProcess, ManagedProcessRow, ProcessId, RequestId,
    RequestMetrics, RequestMetricsRow, RequestStatus,
};

#[test]
fn test_audit_event_roundtrip() {
    let event = AuditEvent {
        event_id: "evt-123".to_string(),
        event_type: EventType::Audit,
        event_kind: "request_completed".to_string(),
        server_id: Some(ServerId("s1".to_string())),
        request_id: Some(RequestId("req-1".to_string())),
        created_at: Utc::now(),
        payload: serde_json::json!({"tokens": 100}),
    };

    let row = AuditEventRow::from(event.clone());
    let restored = AuditEvent::try_from(row).expect("failed to restore");

    assert_eq!(restored.event_id, event.event_id);
    assert_eq!(restored.event_type, event.event_type);
    assert_eq!(restored.server_id, event.server_id);
    assert_eq!(restored.payload, event.payload);
}

#[test]
fn test_request_metrics_roundtrip() {
    let metrics = RequestMetrics {
        request_id: RequestId("req-42".to_string()),
        model_name: "llama-3-8b".to_string(),
        server_id: Some(ServerId("gpu-1".to_string())),
        slot_id: Some(4),
        task_type: TaskTypeId("chat".to_string()),
        requestor: RequestorId("user-A".to_string()),
        effective_priority: 5,

        received_at: Utc::now(),
        dispatched_at: Some(Utc::now()),
        first_token_at: Some(Utc::now()),
        completed_at: Some(Utc::now()),

        queue_wait_ms: 12.5,
        ttft_ms: Some(45.2),
        generation_duration_ms: Some(120.0),
        total_latency_ms: 177.7,

        prompt_tokens: 15,
        completion_tokens: 30,

        prompt_tps: Some(300.0),
        generation_tps: Some(250.0),

        status: RequestStatus::Completed,
        finish_reason: Some("stop".to_string()),
        was_evicted: false,
        eviction_count: 0,
        error_message: None,
    };

    let row = RequestMetricsRow::from(metrics.clone());
    let restored = RequestMetrics::try_from(row).expect("failed to restore");

    assert_eq!(restored.request_id, metrics.request_id);
    assert_eq!(restored.status, metrics.status);
    assert_eq!(restored.queue_wait_ms, metrics.queue_wait_ms);
    assert_eq!(restored.prompt_tokens, metrics.prompt_tokens);
}

#[test]
fn test_managed_process_roundtrip() {
    let proc = ManagedProcess {
        pid: ProcessId(12345),
        server_id: ServerId("s2".to_string()),
        started_at: Utc::now(),
        cmdline: "./llama-server --model x".to_string(),
    };

    let row = ManagedProcessRow::from(proc.clone());
    let restored = ManagedProcess::try_from(row).expect("failed to restore");

    assert_eq!(restored.pid, proc.pid);
    assert_eq!(restored.cmdline, proc.cmdline);
}

#[test]
fn test_config_snapshot_roundtrip() {
    let snap = ConfigSnapshot {
        id: "snap-1".to_string(),
        trigger: ConfigSnapshotTrigger::Reload,
        created_at: Utc::now(),
        config_json: "{}".to_string(),
    };

    let row = ConfigSnapshotRow::from(snap.clone());
    let restored = ConfigSnapshot::try_from(row).expect("failed to restore");

    assert_eq!(restored.id, snap.id);
    assert_eq!(restored.trigger, snap.trigger);
}

#[test]
fn test_config_overlay_roundtrip() {
    let overlay = ConfigOverlay {
        server_id: ServerId("s3".to_string()),
        overlay_key: "api_key".to_string(),
        value: "secret".to_string(),
        applied_at: Utc::now(),
    };

    let row = ConfigOverlayRow::from(overlay.clone());
    let restored = ConfigOverlay::try_from(row).expect("failed to restore");

    assert_eq!(restored.server_id, overlay.server_id);
    assert_eq!(restored.overlay_key, overlay.overlay_key);
}
