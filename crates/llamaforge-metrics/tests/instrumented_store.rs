//! Tracing and metrics decorator for `SqliteMetricsStore` tests.

use std::sync::mpsc;

use llamaforge_metrics::InstrumentedMetricsStore;
use llamaforge_metrics::{create_read_pool, open_and_migrate, writer_thread_main, MetricsConfig};
use llamaforge_metrics::{AuditEvent, EventStore, EventType};

#[tokio::test]
async fn test_instrumented_store_events() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");

    let (conn, lock_file) = open_and_migrate(&db_path).unwrap();
    let (cmd_tx, cmd_rx) = mpsc::sync_channel(100);

    let writer_handle = std::thread::spawn(move || {
        writer_thread_main(conn, cmd_rx, 10);
    });

    let config = MetricsConfig { db_path, ..Default::default() };

    let read_pool = create_read_pool(&config).unwrap();
    let store = llamaforge_metrics::SqliteMetricsStore::new_for_test(
        cmd_tx,
        read_pool,
        Some(writer_handle),
        30,
        lock_file,
    );

    let metrics = llamaforge_metrics::MetricsFacadeRecorder::new();
    let instrumented = InstrumentedMetricsStore::new(store, metrics);

    // We can't easily assert on spans without a custom layer, but we can verify the delegation works.

    let event = AuditEvent {
        event_id: "test1".to_string(),
        event_type: EventType::Audit,
        event_kind: "test".to_string(),
        server_id: None,
        request_id: None,
        created_at: chrono::Utc::now(),
        payload: serde_json::json!({}),
    };

    // Insert
    instrumented.insert_events(vec![event]).await.unwrap();

    // Query
    let res = instrumented.query_events(&Default::default(), None, None).await.unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].event_id, "test1");
}
