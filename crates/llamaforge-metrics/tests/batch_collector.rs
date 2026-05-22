//! Tests for the async batch collector.

use std::sync::mpsc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::mpsc as tokio_mpsc;

use llamaforge_core::config::{RequestorId, TaskTypeId};
use llamaforge_metrics::{
    batch_collector_main, cmd_channel_drops, AuditRecord, MetricsConfig, RequestId, RequestMetrics,
    RequestStatus, TelemetryEvent, TelemetryEventKind, WriterCommand,
};
use llamaforge_core::types::ServerId;

fn dummy_telemetry_event() -> TelemetryEvent {
    TelemetryEvent {
        kind: TelemetryEventKind::ServerStarted,
        server_id: Some(ServerId::from_config_id("s1")),
        timestamp: Utc::now(),
        payload: serde_json::json!({}),
    }
}

fn dummy_audit_record() -> AuditRecord {
    AuditRecord {
        request_metrics: RequestMetrics {
            request_id: RequestId("req-1".to_string()),
            model_name: "test".to_string(),
            server_id: None,
            slot_id: None,
            task_type: TaskTypeId("test".to_string()),
            requestor: RequestorId("user-1".to_string()),
            effective_priority: 1,
            received_at: Utc::now(),
            dispatched_at: None,
            first_token_at: None,
            completed_at: Some(Utc::now()),
            queue_wait_ms: 0.0,
            ttft_ms: None,
            generation_duration_ms: None,
            total_latency_ms: 0.0,
            prompt_tokens: 0,
            completion_tokens: 0,
            prompt_tps: None,
            generation_tps: None,
            status: RequestStatus::Completed,
            finish_reason: None,
            was_evicted: false,
            eviction_count: 0,
            error_message: None,
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_batch_collector_draining_priority() {
    let (audit_tx, audit_rx) = tokio_mpsc::channel(100);
    let (telemetry_tx, telemetry_rx) = tokio_mpsc::channel(100);
    let (cmd_tx, cmd_rx) = mpsc::sync_channel(64);

    let config = MetricsConfig {
        batch_flush_interval_ms: 10, // fast tick
        batch_flush_size: 10,        // small batch
        ..Default::default()
    };

    // Pre-fill channels
    // 12 audit records + 5 telemetry events
    for _ in 0..12 {
        audit_tx.send(dummy_audit_record()).await.unwrap();
    }
    for _ in 0..5 {
        telemetry_tx.send(dummy_telemetry_event()).await.unwrap();
    }

    let _collector_handle =
        tokio::spawn(batch_collector_main(audit_rx, telemetry_rx, cmd_tx, config));

    // First tick should drain exactly 10 audit records, no telemetry.
    // Produces 2 commands: InsertEvents (10) and InsertMetrics (10).
    let mut cmd1 = cmd_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let mut cmd2 = cmd_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    // Sort out which is which since order of phase 3 and 4 in code is InsertEvents then InsertMetrics
    if matches!(cmd1, WriterCommand::InsertMetrics { .. }) {
        std::mem::swap(&mut cmd1, &mut cmd2);
    }

    match cmd1 {
        WriterCommand::InsertEvents { events, .. } => assert_eq!(events.len(), 10),
        _ => panic!("Expected InsertEvents"),
    }
    match cmd2 {
        WriterCommand::InsertMetrics { records, .. } => assert_eq!(records.len(), 10),
        _ => panic!("Expected InsertMetrics"),
    }

    // Second tick should drain the remaining 2 audit records and all 5 telemetry events.
    // That's 7 events total, well under the 10 batch limit.
    // Produces 2 commands: InsertEvents (7) and InsertMetrics (2).
    let mut cmd3 = cmd_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let mut cmd4 = cmd_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    if matches!(cmd3, WriterCommand::InsertMetrics { .. }) {
        std::mem::swap(&mut cmd3, &mut cmd4);
    }

    match cmd3 {
        WriterCommand::InsertEvents { events, .. } => assert_eq!(events.len(), 7),
        _ => panic!("Expected InsertEvents"),
    }
    match cmd4 {
        WriterCommand::InsertMetrics { records, .. } => assert_eq!(records.len(), 2),
        _ => panic!("Expected InsertMetrics"),
    }

    // Drop senders to trigger graceful exit
    drop(audit_tx);
    drop(telemetry_tx);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_command_channel_backpressure() {
    let (audit_tx, audit_rx) = tokio_mpsc::channel(100);
    let (telemetry_tx, telemetry_rx) = tokio_mpsc::channel(100);
    let (cmd_tx, cmd_rx) = mpsc::sync_channel(1); // very small bounded channel

    let config =
        MetricsConfig { batch_flush_interval_ms: 10, batch_flush_size: 5, ..Default::default() };

    // Fill the command channel
    let _ = cmd_tx.send(WriterCommand::Shutdown); // occupies the single slot

    let collector_handle =
        tokio::spawn(batch_collector_main(audit_rx, telemetry_rx, cmd_tx, config));

    let initial_drops = cmd_channel_drops();

    // Send events. They will be batched, but sending the command will fail because cmd_tx is full.
    for _ in 0..5 {
        audit_tx.send(dummy_audit_record()).await.unwrap();
    }

    // Give the collector time to process and drop
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Both InsertEvents and InsertMetrics will be dropped (5 each = 10 drops total)
    let new_drops = cmd_channel_drops();
    assert_eq!(new_drops - initial_drops, 10);

    drop(cmd_rx); // Disconnect
    drop(audit_tx);
    drop(telemetry_tx);
    let _ = collector_handle.await;
}
