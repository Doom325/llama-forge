//! Tracing and metrics decorator for the SQLite store.
//!
//! See design doc Section 2.7.

use std::time::Instant;

use chrono::{DateTime, Utc};
use tracing::{info_span, Instrument};

use crate::error::StoreError;
use crate::filter::{AggregatedMetrics, EventFilter, GroupByField, MetricsFilter};
use crate::metrics_recording::{MetricsFacadeRecorder, StoreHealthRecorder};
use crate::sqlite::store::SqliteMetricsStore;
use crate::traits::{private, EventStore, ProcessStore, RequestMetricsStore, StoreMaintenance};
use crate::types::{AuditEvent, ConfigSnapshot, ManagedProcess, ProcessId, RequestMetrics};

/// Decorator for `SqliteMetricsStore` that adds tracing spans and metrics.
pub struct InstrumentedMetricsStore {
    inner: SqliteMetricsStore,
    metrics: MetricsFacadeRecorder,
}

impl InstrumentedMetricsStore {
    /// Create a new instrumented store.
    pub fn new(inner: SqliteMetricsStore, metrics: MetricsFacadeRecorder) -> Self {
        Self { inner, metrics }
    }

    /// Exposes inherent wal_checkpoint from inner.
    pub async fn wal_checkpoint(
        &self,
        mode: crate::types::CheckpointMode,
    ) -> Result<crate::types::CheckpointResult, StoreError> {
        let mode_str = match mode {
            crate::types::CheckpointMode::Passive => "passive",
            crate::types::CheckpointMode::Truncate => "truncate",
        };
        let start = Instant::now();
        let result = self.inner.wal_checkpoint(mode).await;
        self.metrics.observe_checkpoint_duration(mode_str, start.elapsed().as_secs_f64());
        result
    }

    /// Check if the writer thread has panicked or finished.
    pub fn is_writer_finished(&self) -> bool {
        self.inner.is_writer_finished()
    }
}

impl private::Sealed for InstrumentedMetricsStore {}

impl EventStore for InstrumentedMetricsStore {
    async fn insert_events(&self, events: Vec<AuditEvent>) -> Result<(), StoreError> {
        let start = Instant::now();
        let count = events.len();
        let span = info_span!("metrics.insert_events", count);
        let res = self.inner.insert_events(events).instrument(span).await;

        self.metrics.observe_write_duration(start.elapsed().as_secs_f64());
        self.metrics.record_write_result(if res.is_ok() { "ok" } else { "error" });
        res
    }

    async fn query_events(
        &self,
        filter: &EventFilter,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<AuditEvent>, StoreError> {
        let start = Instant::now();
        let span = info_span!("metrics.query_events");
        let res = self.inner.query_events(filter, limit, offset).instrument(span).await;

        self.metrics.observe_read_duration("events", start.elapsed().as_secs_f64());
        res
    }
}

impl RequestMetricsStore for InstrumentedMetricsStore {
    async fn insert_request_metrics(&self, metrics: Vec<RequestMetrics>) -> Result<(), StoreError> {
        let start = Instant::now();
        let count = metrics.len();
        let span = info_span!("metrics.insert_request_metrics", count);
        let res = self.inner.insert_request_metrics(metrics).instrument(span).await;

        self.metrics.observe_write_duration(start.elapsed().as_secs_f64());
        self.metrics.record_write_result(if res.is_ok() { "ok" } else { "error" });
        res
    }

    async fn query_request_metrics(
        &self,
        filter: &MetricsFilter,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<RequestMetrics>, StoreError> {
        let start = Instant::now();
        let span = info_span!("metrics.query_request_metrics");
        let res = self.inner.query_request_metrics(filter, limit, offset).instrument(span).await;

        self.metrics.observe_read_duration("request_metrics", start.elapsed().as_secs_f64());
        res
    }

    async fn aggregate_request_metrics(
        &self,
        filter: &MetricsFilter,
        group_by: &[GroupByField],
    ) -> Result<Vec<AggregatedMetrics>, StoreError> {
        let start = Instant::now();
        let span = info_span!("metrics.aggregate_request_metrics");
        let res = self.inner.aggregate_request_metrics(filter, group_by).instrument(span).await;

        self.metrics.observe_read_duration("aggregate_metrics", start.elapsed().as_secs_f64());
        res
    }
}

impl ProcessStore for InstrumentedMetricsStore {
    async fn upsert_managed_process(&self, process: &ManagedProcess) -> Result<(), StoreError> {
        let start = Instant::now();
        let span = info_span!("metrics.upsert_managed_process", pid = process.pid.0);
        let res = self.inner.upsert_managed_process(process).instrument(span).await;

        self.metrics.observe_write_duration(start.elapsed().as_secs_f64());
        self.metrics.record_write_result(if res.is_ok() { "ok" } else { "error" });
        res
    }

    async fn remove_managed_process(&self, pid: ProcessId) -> Result<(), StoreError> {
        let start = Instant::now();
        let span = info_span!("metrics.remove_managed_process", pid = pid.0);
        let res = self.inner.remove_managed_process(pid).instrument(span).await;

        self.metrics.observe_write_duration(start.elapsed().as_secs_f64());
        self.metrics.record_write_result(if res.is_ok() { "ok" } else { "error" });
        res
    }

    async fn list_managed_processes(&self) -> Result<Vec<ManagedProcess>, StoreError> {
        let start = Instant::now();
        let span = info_span!("metrics.list_managed_processes");
        let res = self.inner.list_managed_processes().instrument(span).await;

        self.metrics.observe_read_duration("list_processes", start.elapsed().as_secs_f64());
        res
    }
}

impl StoreMaintenance for InstrumentedMetricsStore {
    async fn insert_config_snapshot(&self, snapshot: &ConfigSnapshot) -> Result<(), StoreError> {
        let start = Instant::now();
        let span = info_span!("metrics.insert_config_snapshot", id = %snapshot.id);
        let res = self.inner.insert_config_snapshot(snapshot).instrument(span).await;

        self.metrics.observe_write_duration(start.elapsed().as_secs_f64());
        self.metrics.record_write_result(if res.is_ok() { "ok" } else { "error" });
        res
    }

    fn prune_events_older_than(
        &self,
        cutoff: DateTime<Utc>,
    ) -> impl std::future::Future<Output = Result<u64, StoreError>> + Send {
        let span = info_span!("metrics.prune_events", cutoff = %cutoff);
        let future = self.inner.prune_events_older_than(cutoff).instrument(span);
        let metrics = self.metrics.clone();

        async move {
            let start = Instant::now();
            let res = future.await;
            metrics.observe_write_duration(start.elapsed().as_secs_f64());
            metrics.record_write_result(if res.is_ok() { "ok" } else { "error" });
            if let Ok(deleted) = &res {
                metrics.record_prune_rows(*deleted);
            }
            res
        }
    }

    fn estimated_size_bytes(
        &self,
    ) -> impl std::future::Future<Output = Result<u64, StoreError>> + Send {
        let span = info_span!("metrics.estimated_size_bytes");
        let future = self.inner.estimated_size_bytes().instrument(span);

        async move {
            let res = future.await;
            res
        }
    }

    #[allow(clippy::manual_async_fn)]
    fn wal_checkpoint(
        &self,
        mode: crate::types::CheckpointMode,
    ) -> impl std::future::Future<Output = Result<crate::types::CheckpointResult, StoreError>> + Send
    {
        let span = info_span!("metrics.wal_checkpoint");
        let future = self.inner.wal_checkpoint(mode).instrument(span);
        let metrics = self.metrics.clone();

        async move {
            let start = std::time::Instant::now();
            let res = future.await;
            metrics.observe_write_duration(start.elapsed().as_secs_f64());
            metrics.record_write_result(if res.is_ok() { "ok" } else { "error" });
            res
        }
    }
}
