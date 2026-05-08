//! VC-2 Integration Tests for the complete metrics pipeline
//! Covers writing via batch collector + writer thread, and reading via read pool + traits.

use std::sync::mpsc;
use std::time::Duration;

use chrono::Utc;
use tempfile::TempDir;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::time::sleep;

use llamaforge_config::{RequestorId, ServerId, TaskTypeId};
use llamaforge_metrics::{
    batch_collector_main, create_read_pool, open_and_migrate, writer_thread_main, AuditRecord,
    EventFilter, EventStore, EventType, MetricsConfig, MetricsFilter, RequestId, RequestMetrics,
    RequestMetricsStore, RequestStatus, SqliteMetricsStore, TelemetryEvent, TelemetryEventKind,
};

async fn setup_full_pipeline() -> (
    TempDir,
    tokio_mpsc::Sender<AuditRecord>,
    tokio_mpsc::Sender<TelemetryEvent>,
    SqliteMetricsStore,
) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");

    // 1. Setup SQLite writer
    let conn = open_and_migrate(&db_path).unwrap();
    let (writer_tx, writer_rx) = mpsc::sync_channel(100);

    let writer_handle = std::thread::spawn(move || {
        writer_thread_main(conn, writer_rx, 10);
    });

    // 2. Setup batch collector
    let config = MetricsConfig {
        db_path,
        batch_flush_interval_ms: 10,
        batch_flush_size: 10,
        ..Default::default()
    };

    let (audit_tx, audit_rx) = tokio_mpsc::channel(100);
    let (telemetry_tx, telemetry_rx) = tokio_mpsc::channel(100);

    let _collector_handle = tokio::spawn(batch_collector_main(
        audit_rx,
        telemetry_rx,
        writer_tx.clone(),
        config.clone(),
    ));

    // 3. Setup read pool and store
    let read_pool = create_read_pool(&config).unwrap();
    let store = SqliteMetricsStore::new_for_test(writer_tx, read_pool, Some(writer_handle), 30);

    (temp_dir, audit_tx, telemetry_tx, store)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc2_pipeline() {
    let (_temp, audit_tx, telemetry_tx, store) = setup_full_pipeline().await;

    // Send Telemetry
    telemetry_tx
        .send(TelemetryEvent {
            kind: TelemetryEventKind::ServerStarted,
            server_id: Some(ServerId("s1".to_string())),
            timestamp: Utc::now(),
            payload: serde_json::json!({}),
        })
        .await
        .unwrap();

    // Send Audit
    audit_tx
        .send(AuditRecord {
            request_metrics: RequestMetrics {
                request_id: RequestId("req-1".to_string()),
                model_name: "test".to_string(),
                server_id: Some(ServerId("s1".to_string())),
                slot_id: None,
                task_type: TaskTypeId("test".to_string()),
                requestor: RequestorId("u1".to_string()),
                effective_priority: 1,
                received_at: Utc::now(),
                dispatched_at: None,
                first_token_at: None,
                completed_at: Some(Utc::now()),
                queue_wait_ms: 10.0,
                ttft_ms: Some(20.0),
                generation_duration_ms: Some(30.0),
                total_latency_ms: 60.0,
                prompt_tokens: 5,
                completion_tokens: 15,
                prompt_tps: None,
                generation_tps: None,
                status: RequestStatus::Completed,
                finish_reason: None,
                was_evicted: false,
                eviction_count: 0,
                error_message: None,
            },
        })
        .await
        .unwrap();

    // Wait for collector to batch and writer to flush
    sleep(Duration::from_millis(100)).await;

    // Query events (should be 2: one telemetry, one audit)
    let events = store.query_events(&EventFilter::default(), None, None).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events.iter().filter(|e| e.event_type == EventType::Audit).count(), 1);
    assert_eq!(events.iter().filter(|e| e.event_type == EventType::Telemetry).count(), 1);

    // Query metrics (should be 1)
    let metrics = store.query_request_metrics(&MetricsFilter::default(), None, None).await.unwrap();
    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].request_id.0, "req-1");
}
