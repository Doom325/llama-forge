//! Tests for liveness monitor.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use llamaforge_metrics::{
    create_read_pool, open_and_migrate, start_liveness_monitor, writer_thread_main, MetricsConfig,
    MetricsFacadeRecorder, SqliteMetricsStore,
};

#[tokio::test]
async fn test_liveness_monitor_detects_writer_panic() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");

    let conn = open_and_migrate(&db_path).unwrap();
    let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel(100);

    let writer_handle = std::thread::spawn(move || {
        writer_thread_main(conn, cmd_rx, 10);
    });

    let config = MetricsConfig { db_path, ..Default::default() };

    let read_pool = create_read_pool(&config).unwrap();
    let raw_store = SqliteMetricsStore::new_for_test(cmd_tx, read_pool, Some(writer_handle), 30);

    let facade = MetricsFacadeRecorder::new();
    let store =
        Arc::new(llamaforge_metrics::InstrumentedMetricsStore::new(raw_store, facade.clone()));

    let (telemetry_tx, _) = mpsc::channel(10);
    let (audit_tx, _) = mpsc::channel(10);

    let _handle = start_liveness_monitor(&config, facade, store.clone(), telemetry_tx, audit_tx);

    // Fast-forward to tick
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(3)).await;
}
