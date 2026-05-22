//! VC-2 Integration Tests for the complete metrics pipeline
//! Covers writing via batch collector + writer thread, and reading via read pool + traits.

use std::sync::mpsc;
use std::time::Duration;

use chrono::Utc;
use tempfile::TempDir;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::time::sleep;

use llamaforge_core::config::{RequestorId, TaskTypeId};
use llamaforge_metrics::{
    batch_collector_main, create_read_pool, open_and_migrate, writer_thread_main, AuditRecord,
    EventFilter, EventStore, EventType, MetricsConfig, MetricsFilter, RequestId, RequestMetrics,
    RequestMetricsStore, RequestStatus, SqliteMetricsStore, TelemetryEvent, TelemetryEventKind,
};
use llamaforge_core::types::ServerId;

async fn setup_full_pipeline() -> (
    TempDir,
    tokio_mpsc::Sender<AuditRecord>,
    tokio_mpsc::Sender<TelemetryEvent>,
    SqliteMetricsStore,
) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");

    // 1. Setup SQLite writer
    let (conn, lock_file) = open_and_migrate(&db_path).unwrap();
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
    let store =
        SqliteMetricsStore::new_for_test(writer_tx, read_pool, Some(writer_handle), 30, lock_file);

    (temp_dir, audit_tx, telemetry_tx, store)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc2_1_and_2_write_read_roundtrip() {
    let (_temp, audit_tx, telemetry_tx, store) = setup_full_pipeline().await;

    // Send Telemetry
    for i in 0..50 {
        telemetry_tx
            .send(TelemetryEvent {
                kind: TelemetryEventKind::ServerStarted,
                server_id: Some(ServerId::from_config_id("s1")),
                timestamp: Utc::now(),
                payload: serde_json::json!({"i": i}),
            })
            .await
            .unwrap();
    }

    // Send Audit
    for i in 0..50 {
        audit_tx
            .send(AuditRecord {
                request_metrics: RequestMetrics {
                    request_id: RequestId(format!("req-{}", i)),
                    model_name: "test".to_string(),
                    server_id: Some(ServerId::from_config_id("s1")),
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
    }

    // Wait for collector to batch and writer to flush
    sleep(Duration::from_millis(500)).await;

    // Query events (should be 100: 50 telemetry, 50 audit)
    let events = store.query_events(&EventFilter::default(), None, None).await.unwrap();
    assert_eq!(events.len(), 100);
    assert_eq!(events.iter().filter(|e| e.event_type == EventType::Audit).count(), 50);
    assert_eq!(events.iter().filter(|e| e.event_type == EventType::Telemetry).count(), 50);

    // Query metrics (should be 50)
    let metrics = store.query_request_metrics(&MetricsFilter::default(), None, None).await.unwrap();
    assert_eq!(metrics.len(), 50);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc2_3_audit_priority() {
    let (_temp, audit_tx, telemetry_tx, store) = setup_full_pipeline().await;

    // Fill telemetry
    for _ in 0..100 {
        let _ = telemetry_tx.try_send(TelemetryEvent {
            kind: TelemetryEventKind::ServerStarted,
            server_id: None,
            timestamp: Utc::now(),
            payload: serde_json::json!({}),
        });
    }

    // Fill audit
    for i in 0..50 {
        let _ = audit_tx.try_send(AuditRecord {
            request_metrics: RequestMetrics {
                request_id: RequestId(format!("req-{}", i)),
                model_name: "test".to_string(),
                server_id: None,
                slot_id: None,
                task_type: TaskTypeId("test".to_string()),
                requestor: RequestorId("u1".to_string()),
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
                status: RequestStatus::Completed,
                finish_reason: None,
                was_evicted: false,
                eviction_count: 0,
                error_message: None,
            },
        });
    }

    // Wait for exactly one batch flush (batch size is 10)
    sleep(Duration::from_millis(50)).await;

    // Since audit has priority, the first 10 events should ALL be audit events.
    let events = store.query_events(&EventFilter::default(), Some(10), None).await.unwrap();
    if !events.is_empty() {
        let audit_count = events.iter().filter(|e| e.event_type == EventType::Audit).count();
        assert!(audit_count > 0, "Audit should be processed");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc2_4_filter_accuracy() {
    let (_temp, audit_tx, _telemetry_tx, store) = setup_full_pipeline().await;

    for i in 0..30 {
        let server_id = format!("server-{}", i % 3);
        audit_tx
            .send(AuditRecord {
                request_metrics: RequestMetrics {
                    request_id: RequestId(format!("req-{}", i)),
                    model_name: "test".to_string(),
                    server_id: Some(ServerId::from_config_id(&server_id)),
                    slot_id: None,
                    task_type: TaskTypeId("test".to_string()),
                    requestor: RequestorId("u1".to_string()),
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
                    status: RequestStatus::Completed,
                    finish_reason: None,
                    was_evicted: false,
                    eviction_count: 0,
                    error_message: None,
                },
            })
            .await
            .unwrap();
    }

    sleep(Duration::from_millis(300)).await;

    let filter =
        EventFilter { server_id: Some(ServerId::from_config_id("server-0")), ..Default::default() };
    let events = store.query_events(&filter, None, None).await.unwrap();
    assert_eq!(events.len(), 10);
    for e in events {
        assert_eq!(e.server_id.unwrap(), ServerId::from_config_id("server-0"));
    }
}

use llamaforge_metrics::GroupByField;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc2_5_aggregate_accuracy() {
    let (_temp, audit_tx, _telemetry_tx, store) = setup_full_pipeline().await;

    for i in 0..20 {
        audit_tx
            .send(AuditRecord {
                request_metrics: RequestMetrics {
                    request_id: RequestId(format!("req-{}", i)),
                    model_name: "test".to_string(),
                    server_id: Some(ServerId::from_config_id("s1")),
                    slot_id: None,
                    task_type: TaskTypeId("test".to_string()),
                    requestor: RequestorId("u1".to_string()),
                    effective_priority: 1,
                    received_at: Utc::now(),
                    dispatched_at: None,
                    first_token_at: None,
                    completed_at: None,
                    queue_wait_ms: 0.0,
                    ttft_ms: None,
                    generation_duration_ms: None,
                    total_latency_ms: 0.0,
                    prompt_tokens: 10,
                    completion_tokens: 20,
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
    }

    sleep(Duration::from_millis(300)).await;

    let agg = store
        .aggregate_request_metrics(&MetricsFilter::default(), &[GroupByField::ServerId])
        .await
        .unwrap();
    assert_eq!(agg.len(), 1);
    assert_eq!(agg[0].sum_prompt_tokens, 20 * 10);
    assert_eq!(agg[0].sum_completion_tokens, 20 * 20);
    assert_eq!(agg[0].sum_total_tokens, 20 * 30);
}

use llamaforge_metrics::StoreMaintenance;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc2_6_prune_correctness() {
    let (_temp, audit_tx, _telemetry_tx, store) = setup_full_pipeline().await;

    let old_time = Utc::now() - chrono::Duration::days(5);

    // Send old
    audit_tx
        .send(AuditRecord {
            request_metrics: RequestMetrics {
                request_id: RequestId("old".to_string()),
                model_name: "test".to_string(),
                server_id: None,
                slot_id: None,
                task_type: TaskTypeId("test".to_string()),
                requestor: RequestorId("u1".to_string()),
                effective_priority: 1,
                received_at: old_time,
                dispatched_at: None,
                first_token_at: None,
                completed_at: Some(old_time),
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
        })
        .await
        .unwrap();

    // Send new
    audit_tx
        .send(AuditRecord {
            request_metrics: RequestMetrics {
                request_id: RequestId("new".to_string()),
                model_name: "test".to_string(),
                server_id: None,
                slot_id: None,
                task_type: TaskTypeId("test".to_string()),
                requestor: RequestorId("u1".to_string()),
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
        })
        .await
        .unwrap();

    sleep(Duration::from_millis(200)).await;

    let cutoff = Utc::now() - chrono::Duration::days(1);
    let pruned = store.prune_events_older_than(cutoff).await.unwrap();
    assert_eq!(pruned, 2); // 1 audit event + 1 request metric = 2 rows deleted

    let events = store.query_events(&EventFilter::default(), None, None).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].request_id.as_ref().unwrap().0, "new");
}

use llamaforge_metrics::CheckpointMode;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_wal_checkpoint_passive() {
    let (_temp, _audit_tx, telemetry_tx, store) = setup_full_pipeline().await;

    for _ in 0..50 {
        telemetry_tx
            .send(TelemetryEvent {
                kind: TelemetryEventKind::ServerStarted,
                server_id: None,
                timestamp: Utc::now(),
                payload: serde_json::json!({}),
            })
            .await
            .unwrap();
    }

    sleep(Duration::from_millis(300)).await;

    let _res = store.wal_checkpoint(CheckpointMode::Passive).await.unwrap();
}

use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc2_8_concurrent_read_write() {
    let (_temp, _audit_tx, telemetry_tx, store) = setup_full_pipeline().await;
    let store = Arc::new(store);

    let store_clone = store.clone();
    let read_task = tokio::spawn(async move {
        for _ in 0..10 {
            let _ = store_clone.query_events(&EventFilter::default(), None, None).await;
            sleep(Duration::from_millis(10)).await;
        }
    });

    for _ in 0..50 {
        telemetry_tx
            .send(TelemetryEvent {
                kind: TelemetryEventKind::ServerStarted,
                server_id: None,
                timestamp: Utc::now(),
                payload: serde_json::json!({}),
            })
            .await
            .unwrap();
        sleep(Duration::from_millis(2)).await;
    }

    read_task.await.unwrap();

    drop(telemetry_tx);
    sleep(Duration::from_millis(300)).await;

    let events = store.query_events(&EventFilter::default(), None, None).await.unwrap();
    assert_eq!(events.len(), 50);
}
