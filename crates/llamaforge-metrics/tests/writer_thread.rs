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
    let (temp_dir, writer) = spawn_writer(10);
    let db_path = temp_dir.path().join("llamaforge.db");
    let read_conn = rusqlite::Connection::open(&db_path).unwrap();

    // a) InsertEvents (10 events)
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut events = Vec::new();
    for i in 0..10 {
        events.push(AuditEvent {
            event_id: format!("evt-{}", i),
            event_type: EventType::Audit,
            event_kind: "test".to_string(),
            server_id: None,
            request_id: None,
            created_at: Utc::now(),
            payload: serde_json::json!({}),
        });
    }
    writer
        .tx
        .send(WriterCommand::InsertEvents { events, reply: reply_tx, span: Span::none() })
        .unwrap();
    reply_rx.await.unwrap().expect("insert failed");

    let count: i64 =
        read_conn.query_row("SELECT count(*) FROM audit_events", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 10);

    // b) InsertMetrics (5 records)
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut metrics = Vec::new();
    for i in 0..5 {
        metrics.push(RequestMetrics {
            request_id: RequestId(format!("req-{}", i)),
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
        });
    }
    writer
        .tx
        .send(WriterCommand::InsertMetrics {
            records: metrics,
            reply: reply_tx,
            span: Span::none(),
        })
        .unwrap();
    reply_rx.await.unwrap().expect("insert failed");

    let count: i64 =
        read_conn.query_row("SELECT count(*) FROM request_metrics", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 5);

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

    let count: i64 =
        read_conn.query_row("SELECT count(*) FROM managed_processes", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);

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

    let count: i64 =
        read_conn.query_row("SELECT count(*) FROM managed_processes", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);

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

    let count: i64 =
        read_conn.query_row("SELECT count(*) FROM config_snapshots", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);

    // f) Prune
    let (reply_tx, reply_rx) = oneshot::channel();
    // Prune cutoff is in the future, so it should delete everything
    let cutoff = Utc::now() + chrono::Duration::hours(1);
    writer.tx.send(WriterCommand::Prune { cutoff, reply: reply_tx, span: Span::none() }).unwrap();
    let deleted = reply_rx.await.unwrap().expect("prune failed");
    assert!(deleted > 0);

    let ev_count: i64 =
        read_conn.query_row("SELECT count(*) FROM audit_events", [], |r| r.get(0)).unwrap();
    assert_eq!(ev_count, 0);

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
