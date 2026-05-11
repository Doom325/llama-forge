use chrono::Utc;
use std::time::Duration;
use tempfile::TempDir;

use llamaforge_config::{RequestorId, ServerId, TaskTypeId};
use llamaforge_metrics::{
    create_metrics_pipeline, telemetry_drops, AuditRecord, EventFilter, EventStore, EventType,
    ManagedProcess, MetricsConfig, ProcessId, ProcessStore, RequestId, RequestMetrics,
    RequestStatus, TelemetryEvent, TelemetryEventKind,
};

fn dummy_telemetry(i: u32) -> TelemetryEvent {
    TelemetryEvent {
        kind: TelemetryEventKind::ServerStarted,
        server_id: Some(ServerId(format!("s{}", i))),
        timestamp: Utc::now(),
        payload: serde_json::json!({"i": i}),
    }
}

fn dummy_audit(i: u32) -> AuditRecord {
    AuditRecord {
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
    }
}

async fn create_test_pipeline(config: &MetricsConfig) -> llamaforge_metrics::MetricsPipeline {
    use std::io::Write;
    let temp_dir = tempfile::TempDir::new().unwrap();
    let config_path = temp_dir.path().join("llamaforge.toml");
    let priority_rules = temp_dir.path().join("priority_rules.toml");
    let state_dir = temp_dir.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

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
    std::fs::write(&priority_rules, "").unwrap();

    let paths = llamaforge_config::ConfigPaths::new(config_path, priority_rules, state_dir);
    let source = llamaforge_config::TomlFileConfigSource::new(paths);
    let handle = llamaforge_config::ConfigHandle::load(std::sync::Arc::new(source)).unwrap();
    let reader: std::sync::Arc<dyn llamaforge_config::ConfigReader> = std::sync::Arc::new(handle);
    create_metrics_pipeline(config, reader).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc3_1_producer_storage_query() {
    let temp_dir = TempDir::new().unwrap();
    let config = MetricsConfig {
        db_path: temp_dir.path().join("llamaforge.db"),
        batch_flush_interval_ms: 10,
        ..Default::default()
    };

    let pipeline = create_test_pipeline(&config).await;

    pipeline.ingest.send_telemetry(dummy_telemetry(1));
    tokio::time::sleep(Duration::from_millis(200)).await;

    let filter = EventFilter { server_id: Some(ServerId("s1".to_string())), ..Default::default() };
    let events = pipeline.store.query_events(&filter, None, None).await.unwrap();
    assert_eq!(events.len(), 1);

    pipeline.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc3_2_audit_both_tables() {
    let temp_dir = TempDir::new().unwrap();
    let config = MetricsConfig {
        db_path: temp_dir.path().join("llamaforge.db"),
        batch_flush_interval_ms: 10,
        ..Default::default()
    };

    let pipeline = create_test_pipeline(&config).await;

    pipeline.ingest.send_audit(dummy_audit(1)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let events = pipeline.store.query_events(&EventFilter::default(), None, None).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, EventType::Audit);

    use llamaforge_metrics::RequestMetricsStore;
    let metrics = pipeline
        .store
        .query_request_metrics(&llamaforge_metrics::MetricsFilter::default(), None, None)
        .await
        .unwrap();
    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].request_id.0, "req-1");

    pipeline.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc3_3_backpressure_under_load() {
    let temp_dir = TempDir::new().unwrap();
    let config = MetricsConfig {
        db_path: temp_dir.path().join("llamaforge.db"),
        telemetry_channel_capacity: 100,
        batch_flush_interval_ms: 100,
        batch_flush_size: 100,
        ..Default::default()
    };

    let pipeline = create_test_pipeline(&config).await;
    let start_drops = telemetry_drops();

    for i in 0..20_000 {
        pipeline.ingest.send_telemetry(dummy_telemetry(i));
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    let events = pipeline.store.query_events(&EventFilter::default(), None, None).await.unwrap();
    assert!(!events.is_empty());
    assert!(events.len() < 20_000);
    assert!(telemetry_drops() > start_drops);

    pipeline.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc3_4_shutdown_data_safety() {
    let temp_dir = TempDir::new().unwrap();
    let config = MetricsConfig {
        db_path: temp_dir.path().join("llamaforge.db"),
        batch_flush_interval_ms: 1000,
        ..Default::default()
    };

    let pipeline = create_test_pipeline(&config).await;

    for i in 0..100 {
        pipeline.ingest.send_telemetry(dummy_telemetry(i));
    }

    pipeline.shutdown().await;
    tokio::time::sleep(Duration::from_millis(50)).await; // wait for OS to drop advisory lock

    let pipeline2 = create_test_pipeline(&config).await;
    let events = pipeline2.store.query_events(&EventFilter::default(), None, None).await.unwrap();
    assert_eq!(events.len(), 100);

    pipeline2.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc3_5_dual_instance_prevention() {
    let temp_dir = TempDir::new().unwrap();
    let config =
        MetricsConfig { db_path: temp_dir.path().join("llamaforge.db"), ..Default::default() };

    let pipeline = create_test_pipeline(&config).await;

    // Try to open a second pipeline on the same DB
    let temp_dir2 = tempfile::TempDir::new().unwrap();
    let config_path = temp_dir2.path().join("llamaforge.toml");
    let priority_rules = temp_dir2.path().join("priority_rules.toml");
    let state_dir = temp_dir2.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    use std::io::Write;
    let mut file = std::fs::File::create(&config_path).unwrap();
    writeln!(file, "[global]").unwrap();
    writeln!(file, "metrics_db_path = \"{}\"", config.db_path.to_str().unwrap()).unwrap();
    writeln!(file, "metrics_retention_days = 30").unwrap();
    std::fs::write(&priority_rules, "").unwrap();

    let paths = llamaforge_config::ConfigPaths::new(config_path, priority_rules, state_dir);
    let source = llamaforge_config::TomlFileConfigSource::new(paths);
    let handle = llamaforge_config::ConfigHandle::load(std::sync::Arc::new(source)).unwrap();
    let reader: std::sync::Arc<dyn llamaforge_config::ConfigReader> = std::sync::Arc::new(handle);

    let result = llamaforge_metrics::create_metrics_pipeline(&config, reader);
    assert!(matches!(result, Err(llamaforge_metrics::StoreError::Unavailable { .. })));

    pipeline.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_vc3_6_process_tracking_roundtrip() {
    let temp_dir = TempDir::new().unwrap();
    let config =
        MetricsConfig { db_path: temp_dir.path().join("llamaforge.db"), ..Default::default() };

    let pipeline = create_test_pipeline(&config).await;

    pipeline
        .store
        .upsert_managed_process(&ManagedProcess {
            pid: ProcessId(123),
            server_id: ServerId("s1".to_string()),
            started_at: Utc::now(),
            cmdline: "cmd".to_string(),
            port: 8080,
            model_alias: "test-model".to_string(),
        })
        .await
        .unwrap();

    let procs = pipeline.store.list_managed_processes().await.unwrap();
    assert_eq!(procs.len(), 1);

    pipeline.store.remove_managed_process(ProcessId(123)).await.unwrap();

    let procs = pipeline.store.list_managed_processes().await.unwrap();
    assert!(procs.is_empty());

    pipeline.shutdown().await;
}
