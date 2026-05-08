//! SQLite-backed implementations of the storage traits.
//!
//! See design doc Section 3.1 and 3.5.

use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{DateTime, Utc};
use deadpool_sqlite::Pool;
use tokio::sync::oneshot;
use tracing::Span;

use crate::error::StoreError;
use crate::filter::{AggregatedMetrics, EventFilter, GroupByField, MetricsFilter};
use crate::sqlite::commands::WriterCommand;
use crate::sqlite::query_builder::{build_event_where, build_group_by, build_metrics_where};
use crate::sqlite::rows::{AuditEventRow, ManagedProcessRow, RequestMetricsRow};
use crate::traits::{private, EventStore, ProcessStore, RequestMetricsStore, StoreMaintenance};
use crate::types::{
    AuditEvent, CheckpointMode, CheckpointResult, ConfigSnapshot, ManagedProcess, ProcessId,
    RequestMetrics,
};

/// The main entry point for the SQLite metrics and storage subsystem.
///
/// Implements all storage traits using a dedicated synchronous writer thread
/// for mutations and an async read pool for queries.
pub struct SqliteMetricsStore {
    cmd_tx: SyncSender<WriterCommand>,
    read_pool: Pool,
    writer_join_handle: Option<JoinHandle<()>>,
    read_timeout: Duration,
}

impl SqliteMetricsStore {
    /// Creates a new `SqliteMetricsStore`.
    #[allow(dead_code)]
    pub fn new(
        cmd_tx: SyncSender<WriterCommand>,
        read_pool: Pool,
        writer_join_handle: Option<JoinHandle<()>>,
        read_timeout_secs: u64,
    ) -> Self {
        Self {
            cmd_tx,
            read_pool,
            writer_join_handle,
            read_timeout: Duration::from_secs(read_timeout_secs),
        }
    }

    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn new_for_test(
        cmd_tx: SyncSender<WriterCommand>,
        read_pool: Pool,
        writer_join_handle: Option<JoinHandle<()>>,
        read_timeout_secs: u64,
    ) -> Self {
        Self::new(cmd_tx, read_pool, writer_join_handle, read_timeout_secs)
    }

    /// Perform a WAL checkpoint. This is an inherent method because it is SQLite-specific.
    pub async fn wal_checkpoint(
        &self,
        mode: CheckpointMode,
    ) -> Result<CheckpointResult, StoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = WriterCommand::Checkpoint { mode, reply: reply_tx, span: Span::current() };

        self.cmd_tx
            .send(cmd)
            .map_err(|_| StoreError::Unavailable { message: "Writer thread is dead".into() })?;

        reply_rx.await.map_err(|_| StoreError::Unavailable {
            message: "Writer thread dropped the request".into(),
        })?
    }

    async fn interact_with_timeout<F, R>(&self, f: F) -> Result<R, StoreError>
    where
        F: FnOnce(&mut rusqlite::Connection) -> Result<R, StoreError> + Send + 'static,
        R: Send + 'static,
    {
        let conn = self.read_pool.get().await.map_err(|e| StoreError::Internal {
            source: Box::new(std::io::Error::other(e.to_string())),
        })?;

        let future = conn.interact(|c| f(c));

        match tokio::time::timeout(self.read_timeout, future).await {
            Ok(Ok(result)) => result,
            Ok(Err(e)) => {
                Err(StoreError::Internal { source: Box::new(std::io::Error::other(e.to_string())) })
            }
            Err(_) => Err(StoreError::Timeout { elapsed: self.read_timeout }),
        }
    }
}

impl private::Sealed for SqliteMetricsStore {}

impl EventStore for SqliteMetricsStore {
    async fn insert_events(&self, events: Vec<AuditEvent>) -> Result<(), StoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = WriterCommand::InsertEvents { events, reply: reply_tx, span: Span::current() };

        self.cmd_tx
            .send(cmd)
            .map_err(|_| StoreError::Unavailable { message: "Writer thread is dead".into() })?;

        reply_rx.await.map_err(|_| StoreError::Unavailable {
            message: "Writer thread dropped the request".into(),
        })?
    }

    async fn query_events(
        &self,
        filter: &EventFilter,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<AuditEvent>, StoreError> {
        let (where_clause, params) = build_event_where(filter);
        let limit_val = limit.unwrap_or(1000);
        let offset_val = offset.unwrap_or(0);

        let sql = format!(
            "SELECT event_id, event_type, event_kind, server_id, request_id, created_at, payload
             FROM audit_events
             {}
             ORDER BY created_at DESC
             LIMIT ? OFFSET ?",
            where_clause
        );

        // We can't move `params` directly into `interact` because `Box<dyn ToSql + Send + Sync>`
        // doesn't quite work seamlessly with rusqlite's variadic params if we mix types.
        // Instead, we convert to a Vec of simple rusqlite types, but since the query_builder
        // gives us dyn ToSql, we need to extract the string values as that's what we pushed.
        // Actually, rusqlite's params_from_iter accepts &dyn ToSql.

        // However, `params` is captured by value.
        self.interact_with_timeout(move |conn| {
            let mut stmt = conn
                .prepare_cached(&sql)
                .map_err(|e| StoreError::Internal { source: Box::new(e) })?;

            let mut sql_params: Vec<&dyn rusqlite::ToSql> =
                params.iter().map(|b| b.as_ref() as &dyn rusqlite::ToSql).collect();

            let limit_i64 = limit_val as i64;
            let offset_i64 = offset_val as i64;
            sql_params.push(&limit_i64);
            sql_params.push(&offset_i64);

            let rows = stmt
                .query_map(rusqlite::params_from_iter(sql_params), |row| {
                    Ok(AuditEventRow {
                        event_id: row.get(0)?,
                        event_type: row.get(1)?,
                        event_kind: row.get(2)?,
                        server_id: row.get(3)?,
                        request_id: row.get(4)?,
                        created_at: row.get(5)?,
                        payload: row.get(6)?,
                    })
                })
                .map_err(|e| StoreError::Internal { source: Box::new(e) })?;

            let mut events = Vec::new();
            for r in rows {
                let row = r.map_err(|e| StoreError::Internal { source: Box::new(e) })?;
                events.push(
                    AuditEvent::try_from(row)
                        .map_err(|e| StoreError::Internal { source: Box::new(e) })?,
                );
            }

            Ok(events)
        })
        .await
    }
}

impl RequestMetricsStore for SqliteMetricsStore {
    async fn insert_request_metrics(&self, metrics: Vec<RequestMetrics>) -> Result<(), StoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = WriterCommand::InsertMetrics {
            records: metrics,
            reply: reply_tx,
            span: Span::current(),
        };

        self.cmd_tx
            .send(cmd)
            .map_err(|_| StoreError::Unavailable { message: "Writer thread is dead".into() })?;

        reply_rx.await.map_err(|_| StoreError::Unavailable {
            message: "Writer thread dropped the request".into(),
        })?
    }

    async fn query_request_metrics(
        &self,
        filter: &MetricsFilter,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<RequestMetrics>, StoreError> {
        let (where_clause, params) = build_metrics_where(filter);
        let limit_val = limit.unwrap_or(1000);
        let offset_val = offset.unwrap_or(0);

        let sql = format!(
            r#"SELECT
                request_id, model_name, server_id, slot_id, task_type, requestor, effective_priority,
                received_at, dispatched_at, first_token_at, completed_at,
                queue_wait_ms, ttft_ms, generation_duration_ms, total_latency_ms,
                prompt_tokens, completion_tokens, prompt_tps, generation_tps,
                status, finish_reason, was_evicted, eviction_count, error_message
             FROM request_metrics
             {}
             ORDER BY received_at DESC
             LIMIT ? OFFSET ?"#,
            where_clause
        );

        self.interact_with_timeout(move |conn| {
            let mut stmt = conn
                .prepare_cached(&sql)
                .map_err(|e| StoreError::Internal { source: Box::new(e) })?;

            let mut sql_params: Vec<&dyn rusqlite::ToSql> =
                params.iter().map(|b| b.as_ref() as &dyn rusqlite::ToSql).collect();

            let limit_i64 = limit_val as i64;
            let offset_i64 = offset_val as i64;
            sql_params.push(&limit_i64);
            sql_params.push(&offset_i64);

            let rows = stmt
                .query_map(rusqlite::params_from_iter(sql_params), |row| {
                    Ok(RequestMetricsRow {
                        request_id: row.get(0)?,
                        model_name: row.get(1)?,
                        server_id: row.get(2)?,
                        slot_id: row.get(3)?,
                        task_type: row.get(4)?,
                        requestor: row.get(5)?,
                        effective_priority: row.get(6)?,
                        received_at: row.get(7)?,
                        dispatched_at: row.get(8)?,
                        first_token_at: row.get(9)?,
                        completed_at: row.get(10)?,
                        queue_wait_ms: row.get(11)?,
                        ttft_ms: row.get(12)?,
                        generation_duration_ms: row.get(13)?,
                        total_latency_ms: row.get(14)?,
                        prompt_tokens: row.get(15)?,
                        completion_tokens: row.get(16)?,
                        prompt_tps: row.get(17)?,
                        generation_tps: row.get(18)?,
                        status: row.get(19)?,
                        finish_reason: row.get(20)?,
                        was_evicted: row.get(21)?,
                        eviction_count: row.get(22)?,
                        error_message: row.get(23)?,
                    })
                })
                .map_err(|e| StoreError::Internal { source: Box::new(e) })?;

            let mut metrics = Vec::new();
            for r in rows {
                let row = r.map_err(|e| StoreError::Internal { source: Box::new(e) })?;
                metrics.push(
                    RequestMetrics::try_from(row)
                        .map_err(|e| StoreError::Internal { source: Box::new(e) })?,
                );
            }

            Ok(metrics)
        })
        .await
    }

    async fn aggregate_request_metrics(
        &self,
        filter: &MetricsFilter,
        group_by: &[GroupByField],
    ) -> Result<Vec<AggregatedMetrics>, StoreError> {
        let (where_clause, params) = build_metrics_where(filter);
        let group_by_clause = build_group_by(group_by);

        let mut selects = Vec::new();
        for field in group_by {
            match field {
                GroupByField::ServerId => selects.push("server_id as server_id"),
                GroupByField::ModelName => selects.push("model_name as model_name"),
                GroupByField::Requestor => selects.push("requestor as requestor"),
                GroupByField::Hour => {
                    selects.push("strftime('%Y-%m-%d %H:00:00', received_at) as hour")
                }
            }
        }

        let select_clause =
            if selects.is_empty() { "".to_string() } else { format!("{},", selects.join(", ")) };

        // We fetch the individual latencies using group_concat to compute percentiles in Rust
        // Since sqlite lacks a native percentile function and group_concat has a size limit,
        // we limit this aggregate behavior to a bounded number or just accept the SQLite limitations.
        // Actually, a better approach for percentiles in SQLite without extensions is to fetch the raw
        // values and compute. To avoid enormous payloads, we can fetch them sorted if there is no group by,
        // but with group by, we can use group_concat. Or we can just omit p50/p99 if it's too complex here
        // and add a separate query. For now, we will collect total_latency_ms into a string via group_concat.

        let sql = format!(
            r#"SELECT 
                {}
                COUNT(*) as request_count,
                SUM(prompt_tokens) as total_prompt_tokens,
                SUM(completion_tokens) as total_completion_tokens,
                AVG(total_latency_ms) as avg_latency_ms,
                AVG(ttft_ms) as avg_ttft_ms,
                AVG(generation_tps) as avg_generation_tps,
                GROUP_CONCAT(ttft_ms) as ttfts,
                GROUP_CONCAT(generation_tps) as tps
             FROM (SELECT * FROM request_metrics {} ORDER BY received_at DESC LIMIT 10000)
             {}"#,
            select_clause, where_clause, group_by_clause
        );

        let group_by_len = group_by.len();
        let group_by_cloned = group_by.to_vec();

        self.interact_with_timeout(move |conn| {
            let mut stmt = conn
                .prepare_cached(&sql)
                .map_err(|e| StoreError::Internal { source: Box::new(e) })?;

            let sql_params: Vec<&dyn rusqlite::ToSql> =
                params.iter().map(|b| b.as_ref() as &dyn rusqlite::ToSql).collect();

            let rows = stmt
                .query_map(rusqlite::params_from_iter(sql_params), |row| {
                    let sum_prompt: i64 = row.get(group_by_len + 1).unwrap_or(0);
                    let sum_comp: i64 = row.get(group_by_len + 2).unwrap_or(0);
                    let mut aggregated = AggregatedMetrics {
                        group_key: String::new(),
                        request_count: row.get(group_by_len)?,
                        sum_prompt_tokens: sum_prompt as u64,
                        sum_completion_tokens: sum_comp as u64,
                        sum_total_tokens: (sum_prompt + sum_comp) as u64,
                        avg_ttft_ms: row.get(group_by_len + 4)?,
                        p50_ttft_ms: None,
                        p99_ttft_ms: None,
                        avg_generation_tps: row.get(group_by_len + 5)?,
                        p50_generation_tps: None,
                        p99_generation_tps: None,
                        avg_total_latency_ms: row.get(group_by_len + 3)?,
                    };

                    let mut keys = Vec::new();
                    for (i, field) in group_by_cloned.iter().enumerate() {
                        let val: Option<String> = row.get(i)?;
                        let key = match field {
                            GroupByField::ServerId => "server_id",
                            GroupByField::ModelName => "model_name",
                            GroupByField::Requestor => "requestor",
                            GroupByField::Hour => "hour",
                        };
                        keys.push(format!("{}={}", key, val.unwrap_or_default()));
                    }
                    aggregated.group_key = keys.join(",");

                    let compute_percentiles = |s: Option<String>| -> (Option<f64>, Option<f64>) {
                        if let Some(s) = s {
                            let mut vals: Vec<f64> =
                                s.split(',').filter_map(|s| s.parse::<f64>().ok()).collect();
                            if !vals.is_empty() {
                                vals.sort_by(|a, b| {
                                    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                                });
                                let p50_idx = (vals.len() as f64 * 0.50) as usize;
                                let p99_idx = (vals.len() as f64 * 0.99) as usize;
                                return (
                                    Some(vals[p50_idx.min(vals.len() - 1)]),
                                    Some(vals[p99_idx.min(vals.len() - 1)]),
                                );
                            }
                        }
                        (None, None)
                    };

                    let (p50_ttft, p99_ttft) = compute_percentiles(row.get(group_by_len + 6)?);
                    aggregated.p50_ttft_ms = p50_ttft;
                    aggregated.p99_ttft_ms = p99_ttft;

                    let (p50_tps, p99_tps) = compute_percentiles(row.get(group_by_len + 7)?);
                    aggregated.p50_generation_tps = p50_tps;
                    aggregated.p99_generation_tps = p99_tps;

                    Ok(aggregated)
                })
                .map_err(|e| StoreError::Internal { source: Box::new(e) })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| StoreError::Internal { source: Box::new(e) })?);
            }

            Ok(results)
        })
        .await
    }
}

impl ProcessStore for SqliteMetricsStore {
    async fn upsert_managed_process(&self, process: &ManagedProcess) -> Result<(), StoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = WriterCommand::UpsertProcess {
            proc: process.clone(),
            reply: reply_tx,
            span: Span::current(),
        };

        self.cmd_tx
            .send(cmd)
            .map_err(|_| StoreError::Unavailable { message: "Writer thread is dead".into() })?;

        reply_rx.await.map_err(|_| StoreError::Unavailable {
            message: "Writer thread dropped the request".into(),
        })?
    }

    async fn remove_managed_process(&self, pid: ProcessId) -> Result<(), StoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = WriterCommand::RemoveProcess { pid, reply: reply_tx, span: Span::current() };

        self.cmd_tx
            .send(cmd)
            .map_err(|_| StoreError::Unavailable { message: "Writer thread is dead".into() })?;

        reply_rx.await.map_err(|_| StoreError::Unavailable {
            message: "Writer thread dropped the request".into(),
        })?
    }

    async fn list_managed_processes(&self) -> Result<Vec<ManagedProcess>, StoreError> {
        self.interact_with_timeout(move |conn| {
            let mut stmt = conn
                .prepare_cached("SELECT pid, server_id, started_at, cmdline FROM managed_processes")
                .map_err(|e| StoreError::Internal { source: Box::new(e) })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(ManagedProcessRow {
                        pid: row.get(0)?,
                        server_id: row.get(1)?,
                        started_at: row.get(2)?,
                        cmdline: row.get(3)?,
                    })
                })
                .map_err(|e| StoreError::Internal { source: Box::new(e) })?;

            let mut procs = Vec::new();
            for r in rows {
                let row = r.map_err(|e| StoreError::Internal { source: Box::new(e) })?;
                procs.push(
                    ManagedProcess::try_from(row)
                        .map_err(|e| StoreError::Internal { source: Box::new(e) })?,
                );
            }

            Ok(procs)
        })
        .await
    }
}

impl StoreMaintenance for SqliteMetricsStore {
    async fn insert_config_snapshot(&self, snapshot: &ConfigSnapshot) -> Result<(), StoreError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = WriterCommand::InsertConfigSnapshot {
            snapshot: snapshot.clone(),
            reply: reply_tx,
            span: Span::current(),
        };

        self.cmd_tx
            .send(cmd)
            .map_err(|_| StoreError::Unavailable { message: "Writer thread is dead".into() })?;

        reply_rx.await.map_err(|_| StoreError::Unavailable {
            message: "Writer thread dropped the request".into(),
        })?
    }

    fn prune_events_older_than(
        &self,
        cutoff: DateTime<Utc>,
    ) -> impl std::future::Future<Output = Result<u64, StoreError>> + Send {
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = WriterCommand::Prune { cutoff, reply: reply_tx, span: Span::current() };

        let cmd_tx_res = self
            .cmd_tx
            .send(cmd)
            .map_err(|_| StoreError::Unavailable { message: "Writer thread is dead".into() });

        async move {
            cmd_tx_res?;
            reply_rx.await.map_err(|_| StoreError::Unavailable {
                message: "Writer thread dropped the request".into(),
            })?
        }
    }
    fn estimated_size_bytes(
        &self,
    ) -> impl std::future::Future<Output = Result<u64, StoreError>> + Send {
        self.interact_with_timeout(move |conn| {
            let page_count: i64 = conn
                .query_row("PRAGMA page_count", [], |r| r.get(0))
                .map_err(|e| StoreError::Internal { source: Box::new(e) })?;
            let page_size: i64 = conn
                .query_row("PRAGMA page_size", [], |r| r.get(0))
                .map_err(|e| StoreError::Internal { source: Box::new(e) })?;

            Ok((page_count * page_size) as u64)
        })
    }
}

impl Drop for SqliteMetricsStore {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(WriterCommand::Shutdown);
        if let Some(handle) = self.writer_join_handle.take() {
            let _ = handle.join();
        }
    }
}
