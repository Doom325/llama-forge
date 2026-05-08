//! Tests for the full metrics pipeline assembly and graceful shutdown.

use std::time::Duration;

use chrono::Utc;
use tempfile::TempDir;

use llamaforge_config::{RequestorId, ServerId, TaskTypeId};
use llamaforge_metrics::{
    audit_drops, create_metrics_pipeline, telemetry_drops, AuditRecord, AuditSendError,
    EventFilter, EventStore, EventType, MetricsConfig, RequestId, RequestMetrics, RequestStatus,
    TelemetryEvent, TelemetryEventKind,
};

fn dummy_telemetry() -> TelemetryEvent {
    TelemetryEvent {
        kind: TelemetryEventKind::ServerStarted,
        server_id: Some(ServerId("s1".to_string())),
        timestamp: Utc::now(),
        payload: serde_json::json!({}),
    }
}

fn dummy_audit() -> AuditRecord {
    AuditRecord {
        request_metrics: RequestMetrics {
            request_id: RequestId("req-1".to_string()),
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
    }
}

async fn create_test_pipeline(config: &MetricsConfig) -> llamaforge_metrics::MetricsPipeline {
    use std::io::Write;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let config_path = temp_dir.path().join("llamaforge.toml");
    let priority_rules = temp_dir.path().join("priority_rules.toml");
    let state_dir = temp_dir.path().join("state");
    std::fs::create_dir(&state_dir).unwrap();

    let mut file = std::fs::File::create(&config_path).unwrap();
    writeln!(file, "[global]").unwrap();
    writeln!(file, "metrics_db_path = \"{}\"", config.db_path.to_str().unwrap()).unwrap();
    writeln!(file, "metrics_retention_days = 30").unwrap();
    writeln!(file, "[metrics]").unwrap();
    writeln!(file, "telemetry_channel_capacity = {}", config.telemetry_channel_capacity).unwrap();
    writeln!(file, "audit_channel_capacity = {}", config.audit_channel_capacity).unwrap();
    writeln!(file, "batch_flush_interval_ms = {}", config.batch_flush_interval_ms).unwrap();
    writeln!(file, "batch_flush_size = {}", config.batch_flush_size).unwrap();
    writeln!(file, "audit_semaphore_permits = {}", config.audit_semaphore_permits).unwrap();

    let paths = llamaforge_config::ConfigPaths::new(config_path, priority_rules, state_dir);
    let source = llamaforge_config::TomlFileConfigSource::new(paths);
    let handle = llamaforge_config::ConfigHandle::load(std::sync::Arc::new(source)).unwrap();
    let reader: std::sync::Arc<dyn llamaforge_config::ConfigReader> = std::sync::Arc::new(handle);
    create_metrics_pipeline(config, reader).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pipeline_integration_happy_path() {
    println!("test_pipeline_integration_happy_path: start");
    let temp_dir = TempDir::new().unwrap();
    let config = MetricsConfig {
        db_path: temp_dir.path().join("llamaforge.db"),
        batch_flush_interval_ms: 10,
        ..Default::default()
    };

    println!("test_pipeline_integration_happy_path: create_test_pipeline");
    let pipeline = create_test_pipeline(&config).await;

    println!("test_pipeline_integration_happy_path: send_telemetry");
    // Send telemetry
    assert!(pipeline.ingest.send_telemetry(dummy_telemetry()));

    println!("test_pipeline_integration_happy_path: send_audit");
    // Send audit
    pipeline.ingest.send_audit(dummy_audit()).await.unwrap();

    println!("test_pipeline_integration_happy_path: sleep");
    // Wait for batch collector to process
    tokio::time::sleep(Duration::from_millis(100)).await;

    println!("test_pipeline_integration_happy_path: query_events");
    // Verify
    let events = pipeline.store.query_events(&EventFilter::default(), None, None).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events.iter().filter(|e| e.event_type == EventType::Telemetry).count(), 1);
    assert_eq!(events.iter().filter(|e| e.event_type == EventType::Audit).count(), 1);

    println!("test_pipeline_integration_happy_path: shutdown");
    // Test shutdown
    pipeline.shutdown().await;
    println!("test_pipeline_integration_happy_path: done");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_telemetry_drop_backpressure() {
    let temp_dir = TempDir::new().unwrap();
    let config = MetricsConfig {
        db_path: temp_dir.path().join("llamaforge.db"),
        telemetry_channel_capacity: 1, // Small channel
        batch_flush_size: 1,           // Small batch
        batch_flush_interval_ms: 1000, // Slow flush so it fills up
        ..Default::default()
    };

    let pipeline = create_test_pipeline(&config).await;
    let initial_drops = telemetry_drops();

    // Fill batch collector buffer
    pipeline.ingest.send_telemetry(dummy_telemetry());
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Fill channel
    pipeline.ingest.send_telemetry(dummy_telemetry());

    // Next one should drop
    assert!(!pipeline.ingest.send_telemetry(dummy_telemetry()));

    assert_eq!(telemetry_drops(), initial_drops + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_audit_semaphore_exhaustion() {
    let temp_dir = TempDir::new().unwrap();
    let config = MetricsConfig {
        db_path: temp_dir.path().join("llamaforge.db"),
        audit_semaphore_permits: 2,
        ..Default::default()
    };

    let _pipeline = create_test_pipeline(&config).await;
    let initial_drops = audit_drops();

    // Let's just create a dummy ingest with a 0-permit semaphore for this specific test
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let ingest = llamaforge_metrics::MetricsIngest::new_for_test(
        tokio::sync::mpsc::channel(1).0,
        tx,
        std::sync::Arc::new(tokio::sync::Semaphore::new(0)),
    );

    let res = ingest.send_audit(dummy_audit()).await;
    assert!(matches!(res, Err(AuditSendError::SemaphoreExhausted)));
    assert_eq!(audit_drops(), initial_drops + 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_graceful_shutdown_data_safety() {
    let temp_dir = TempDir::new().unwrap();
    let config = MetricsConfig {
        db_path: temp_dir.path().join("llamaforge.db"),
        batch_flush_interval_ms: 1000, // Very slow so nothing flushes before we drop
        ..Default::default()
    };

    let pipeline = create_test_pipeline(&config).await;

    // Send 50 telemetry events
    for _ in 0..50 {
        pipeline.ingest.send_telemetry(dummy_telemetry());
    }

    // The shutdown method awaits the batch collector finishing, which guarantees all events are flushed.
    pipeline.shutdown().await;

    // Small sleep to ensure OS thread finishes
    tokio::time::sleep(Duration::from_millis(50)).await;

    // We can just open a raw rusqlite connection instead of a full pipeline which grabs the lock again
    let conn = rusqlite::Connection::open(&config.db_path).unwrap();
    let count: i64 =
        conn.query_row("SELECT count(*) FROM audit_events", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 50);
}

#[tokio::test]
async fn test_audit_timeout() {
    let temp_dir = TempDir::new().unwrap();
    let config = MetricsConfig {
        db_path: temp_dir.path().join("llamaforge.db"),
        batch_flush_interval_ms: 1000,
        ..Default::default()
    };

    let _pipeline = create_test_pipeline(&config).await;
    let initial_drops = audit_drops();

    let (audit_tx, _audit_rx) = tokio::sync::mpsc::channel(1);
    let (telemetry_tx, _telemetry_rx) = tokio::sync::mpsc::channel(1);
    let ingest = llamaforge_metrics::MetricsIngest::new_for_test(
        telemetry_tx,
        audit_tx,
        std::sync::Arc::new(tokio::sync::Semaphore::new(10)),
    );

    // Fill the channel of size 1
    ingest.send_audit(dummy_audit()).await.unwrap();

    // Fast forward time to trigger timeout
    tokio::time::pause();

    let res = ingest.send_audit(dummy_audit()).await;
    assert!(matches!(res, Err(AuditSendError::Timeout)));
    let current_drops = audit_drops();
    assert!(current_drops > initial_drops);
}
