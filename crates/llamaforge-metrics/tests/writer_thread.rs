//! Tests for the SQLite writer thread and its degradation state machine.

use std::sync::mpsc;

use chrono::Utc;
use tempfile::TempDir;
use tokio::sync::oneshot;
use tracing::Span;

use llamaforge_config::{RequestorId, ServerId, TaskTypeId};
use llamaforge_metrics::{
    open_and_migrate, writer_thread_main, AuditEvent, CheckpointMode, ConfigSnapshot,
    ConfigSnapshotTrigger, EventType, ManagedProcess, ProcessId, RequestId, RequestMetrics,
    RequestStatus, StoreError, WriterCommand, WriterTestHandle,
};

fn spawn_writer(failure_threshold: u32) -> (TempDir, WriterTestHandle) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");
    let conn = open_and_migrate(&db_path).unwrap();

    let (tx, rx) = mpsc::sync_channel(64);
    let handle = std::thread::spawn(move || {
        writer_thread_main(conn, rx, failure_threshold);
    });

    (temp_dir, WriterTestHandle { tx, handle: Some(handle) })
}

#[tokio::test]
async fn test_writer_command_dispatch() {
    let (_temp, writer) = spawn_writer(10);

    // a) InsertEvents
    let (reply_tx, reply_rx) = oneshot::channel();
    let event = AuditEvent {
        event_id: "evt-1".to_string(),
        event_type: EventType::Audit,
        event_kind: "test".to_string(),
        server_id: None,
        request_id: None,
        created_at: Utc::now(),
        payload: serde_json::json!({}),
    };
    writer
        .tx
        .send(WriterCommand::InsertEvents {
            events: vec![event],
            reply: reply_tx,
            span: Span::none(),
        })
        .unwrap();
    reply_rx.await.unwrap().expect("insert failed");

    // b) InsertMetrics
    let (reply_tx, reply_rx) = oneshot::channel();
    let metrics = RequestMetrics {
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
        completed_at: None,
        queue_wait_ms: 0.0,
        ttft_ms: None,
        generation_duration_ms: None,
        total_latency_ms: 0.0,
        prompt_tokens: 0,
        completion_tokens: 0,
        prompt_tps: None,
        generation_tps: None,
        status: RequestStatus::Queued,
        finish_reason: None,
        was_evicted: false,
        eviction_count: 0,
        error_message: None,
    };
    writer
        .tx
        .send(WriterCommand::InsertMetrics {
            records: vec![metrics],
            reply: reply_tx,
            span: Span::none(),
        })
        .unwrap();
    reply_rx.await.unwrap().expect("insert failed");

    // c) UpsertProcess
    let (reply_tx, reply_rx) = oneshot::channel();
    let proc = ManagedProcess {
        pid: ProcessId(123),
        server_id: ServerId("s1".to_string()),
        started_at: Utc::now(),
        cmdline: "cmd".to_string(),
    };
    writer
        .tx
        .send(WriterCommand::UpsertProcess { proc, reply: reply_tx, span: Span::none() })
        .unwrap();
    reply_rx.await.unwrap().expect("upsert failed");

    // d) RemoveProcess
    let (reply_tx, reply_rx) = oneshot::channel();
    writer
        .tx
        .send(WriterCommand::RemoveProcess {
            pid: ProcessId(123),
            reply: reply_tx,
            span: Span::none(),
        })
        .unwrap();
    reply_rx.await.unwrap().expect("remove failed");

    // e) InsertConfigSnapshot
    let (reply_tx, reply_rx) = oneshot::channel();
    let snap = ConfigSnapshot {
        id: "snap-1".to_string(),
        trigger: ConfigSnapshotTrigger::Startup,
        created_at: Utc::now(),
        config_json: "{}".to_string(),
    };
    writer
        .tx
        .send(WriterCommand::InsertConfigSnapshot {
            snapshot: snap,
            reply: reply_tx,
            span: Span::none(),
        })
        .unwrap();
    reply_rx.await.unwrap().expect("insert failed");

    // f) Prune
    let (reply_tx, reply_rx) = oneshot::channel();
    writer
        .tx
        .send(WriterCommand::Prune { cutoff: Utc::now(), reply: reply_tx, span: Span::none() })
        .unwrap();
    let deleted = reply_rx.await.unwrap().expect("prune failed");
    assert!(deleted > 0);

    // g) Checkpoint(Passive)
    let (reply_tx, reply_rx) = oneshot::channel();
    writer
        .tx
        .send(WriterCommand::Checkpoint {
            mode: CheckpointMode::Passive,
            reply: reply_tx,
            span: Span::none(),
        })
        .unwrap();
    let _res = reply_rx.await.unwrap().expect("checkpoint failed");

    // writer drop automatically sends Shutdown and joins
    drop(writer);
}

#[tokio::test]
async fn test_writer_degradation() {
    let (_temp, writer) = spawn_writer(2); // low threshold

    // Send intentionally failing commands (e.g. malformed data or duplicate PKs without REPLACE)
    // Actually, duplicating an AuditEvent PK will cause a failure.
    let event = AuditEvent {
        event_id: "evt-dup".to_string(),
        event_type: EventType::Audit,
        event_kind: "test".to_string(),
        server_id: None,
        request_id: None,
        created_at: Utc::now(),
        payload: serde_json::json!({}),
    };

    let (reply_tx, reply_rx) = oneshot::channel();
    writer
        .tx
        .send(WriterCommand::InsertEvents {
            events: vec![event.clone()],
            reply: reply_tx,
            span: Span::none(),
        })
        .unwrap();
    reply_rx.await.unwrap().expect("first insert should succeed");

    // Failure 1
    let (reply_tx, reply_rx) = oneshot::channel();
    writer
        .tx
        .send(WriterCommand::InsertEvents {
            events: vec![event.clone()],
            reply: reply_tx,
            span: Span::none(),
        })
        .unwrap();
    let res1 = reply_rx.await.unwrap();
    assert!(matches!(res1, Err(StoreError::Internal { .. })));

    // Failure 2 -> triggers degradation (threshold is 2)
    let (reply_tx, reply_rx) = oneshot::channel();
    writer
        .tx
        .send(WriterCommand::InsertEvents {
            events: vec![event.clone()],
            reply: reply_tx,
            span: Span::none(),
        })
        .unwrap();
    let res2 = reply_rx.await.unwrap();
    assert!(matches!(res2, Err(StoreError::Internal { .. })));

    // Command 3 -> should fail with Unavailable because degraded
    let (reply_tx, reply_rx) = oneshot::channel();
    writer
        .tx
        .send(WriterCommand::Prune { cutoff: Utc::now(), reply: reply_tx, span: Span::none() })
        .unwrap();
    let res3 = reply_rx.await.unwrap();
    assert!(matches!(res3, Err(StoreError::Unavailable { .. })));
}
