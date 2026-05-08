//! Metrics pipeline assembly.
//!
//! See design doc Section 3.5.

use std::sync::mpsc;
use std::sync::Arc;

use tokio::sync::mpsc as tokio_mpsc;
use tokio::sync::Semaphore;

use crate::error::StoreError;
use crate::ingest::MetricsIngest;
use crate::sqlite::batch_collector::batch_collector_main;
use crate::sqlite::pool::create_read_pool;
use crate::sqlite::{open_and_migrate, store::SqliteMetricsStore, writer::writer_thread_main};
use crate::types::MetricsConfig;

/// The assembled metrics pipeline components.
///
/// # Shutdown Ordering
/// To cleanly shut down the pipeline:
/// 1. Ensure `ingest.audit_semaphore.close()` has been called and all existing
///    tasks holding permits have finished.
/// 2. Drop `MetricsPipeline` (which drops `SqliteMetricsStore`).
/// 3. The `SqliteMetricsStore` drop handler will send a Shutdown command to the
///    writer thread, which drains all remaining events and executes a WAL truncate
///    checkpoint before exiting.
pub struct MetricsPipeline {
    /// The frontend for producers to send telemetry and audit events.
    pub ingest: MetricsIngest,
    /// The SQLite store for querying metrics and maintenance.
    pub store: SqliteMetricsStore,
    /// The async task handling batching of incoming events.
    pub batch_collector_handle: tokio::task::JoinHandle<()>,
}

/// Assembles the complete metrics pipeline.
pub fn create_metrics_pipeline(config: &MetricsConfig) -> Result<MetricsPipeline, StoreError> {
    // 1. Open and migrate the database. Also acquires the advisory lock.
    let conn = open_and_migrate(&config.db_path)?;

    // 2. Create the communication channels
    let (telemetry_tx, telemetry_rx) = tokio_mpsc::channel(config.telemetry_channel_capacity);
    let (audit_tx, audit_rx) = tokio_mpsc::channel(config.audit_channel_capacity);
    let (writer_tx, writer_rx) = mpsc::sync_channel(64); // Hardcoded small bounded command channel

    // 3. Spawn the writer thread
    let failure_threshold = config.writer_failure_threshold;
    let writer_handle = std::thread::spawn(move || {
        writer_thread_main(conn, writer_rx, failure_threshold);
    });

    // 4. Spawn the async batch collector
    let batch_collector_handle = tokio::spawn(batch_collector_main(
        audit_rx,
        telemetry_rx,
        writer_tx.clone(),
        config.clone(), // Clone the config for the task
    ));

    // 5. Create the deadpool read pool
    let read_pool = create_read_pool(config)?;

    // 6. Create the ingest frontend
    let audit_semaphore = Arc::new(Semaphore::new(config.audit_semaphore_permits));
    let ingest = MetricsIngest::new(telemetry_tx, audit_tx, audit_semaphore);

    // 7. Assemble the pipeline
    let store = SqliteMetricsStore::new(
        writer_tx,
        read_pool,
        Some(writer_handle),
        config.read_statement_timeout_secs,
    );

    Ok(MetricsPipeline { ingest, store, batch_collector_handle })
}
