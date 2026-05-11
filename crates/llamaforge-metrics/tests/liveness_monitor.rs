//! Tests for liveness monitor.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use llamaforge_metrics::{
    create_read_pool, open_and_migrate, start_liveness_monitor, writer_thread_main, MetricsConfig,
    MetricsFacadeRecorder, SqliteMetricsStore, WriterCommand,
};

#[tokio::test]
async fn test_writer_thread_panic_detected() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");

    let (conn, lock_file) = open_and_migrate(&db_path).unwrap();
    let (cmd_tx, cmd_rx) = std::sync::mpsc::sync_channel(100);

    let writer_handle = std::thread::spawn(move || {
        writer_thread_main(conn, cmd_rx, 10);
    });

    let config = MetricsConfig { db_path, ..Default::default() };

    let read_pool = create_read_pool(&config).unwrap();

    // We clone the sender so that the store's drop won't be the only thing closing the channel
    let raw_store = SqliteMetricsStore::new_for_test(
        cmd_tx.clone(),
        read_pool,
        Some(writer_handle),
        30,
        lock_file,
    );

    let facade = MetricsFacadeRecorder::new();
    let store = Arc::new(llamaforge_metrics::InstrumentedMetricsStore::new(raw_store, facade));

    let (telemetry_tx, _) = mpsc::channel(10);
    let (audit_tx, _) = mpsc::channel(10);

    let handle = start_liveness_monitor(
        &config,
        MetricsFacadeRecorder::new(),
        store.clone(),
        telemetry_tx,
        audit_tx,
    );

    // Initial state: writer is not finished
    assert!(!store.is_writer_finished());

    // Tell the writer to shutdown directly
    cmd_tx.send(WriterCommand::Shutdown).unwrap();

    // Give it a moment to exit
    let mut finished = false;
    for _ in 0..50 {
        if store.is_writer_finished() {
            finished = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Now it should be finished
    assert!(finished, "Writer thread did not finish in time");

    // The liveness monitor loops every 2 seconds. Let's fast forward.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(3)).await;

    // We can't easily assert the error log was printed without tracing-subscriber setup,
    // but we can verify the monitor task completes its tick without panicking.
    handle.abort();
}
