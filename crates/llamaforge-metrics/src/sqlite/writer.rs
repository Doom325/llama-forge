//! Synchronous SQLite writer thread and state machine.
//!
//! See design doc Section 3.3.

use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use tracing::{debug, error, info, warn};

use crate::error::StoreError;
use crate::sqlite::commands::WriterCommand;
use crate::sqlite::rows::{AuditEventRow, ConfigSnapshotRow, ManagedProcessRow, RequestMetricsRow};
use crate::types::{CheckpointMode, CheckpointResult, WriterMode};

/// The internal state of the writer thread.
struct WriterState {
    mode: WriterMode,
    consecutive_failures: u32,
    failure_threshold: u32,
    last_integrity_check: Option<Instant>,
}

impl WriterState {
    fn new(failure_threshold: u32) -> Self {
        Self {
            mode: WriterMode::Healthy,
            consecutive_failures: 0,
            failure_threshold,
            last_integrity_check: None,
        }
    }

    fn record_success(&mut self) {
        if matches!(self.mode, WriterMode::Healthy) {
            self.consecutive_failures = 0;
        } else {
            // Keep degraded state, but reset consecutive failure counter
            self.consecutive_failures = 0;
        }
    }

    fn record_failure(&mut self) {
        if matches!(self.mode, WriterMode::Degraded { .. }) {
            return; // Already degraded
        }

        self.consecutive_failures += 1;
        if self.consecutive_failures >= self.failure_threshold {
            error!(
                "Writer thread encountered {} consecutive failures. Transitioning to DEGRADED mode.",
                self.consecutive_failures
            );
            self.mode = WriterMode::Degraded {
                consecutive_failures: self.consecutive_failures,
                since: Instant::now(),
            };
        }
    }

    fn is_degraded(&self) -> bool {
        matches!(self.mode, WriterMode::Degraded { .. })
    }

    fn maybe_run_integrity_check(&mut self, conn: &mut Connection) {
        if !self.is_degraded() {
            return;
        }

        let now = Instant::now();
        if let Some(last) = self.last_integrity_check {
            if now.duration_since(last) < Duration::from_secs(60) {
                return;
            }
        }

        self.last_integrity_check = Some(now);
        match conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)) {
            Ok(res) if res == "ok" => {
                info!("Integrity check passed (still in degraded mode).");
            }
            Ok(res) => {
                error!("Integrity check failed: {}", res);
            }
            Err(e) => {
                error!("Failed to run integrity check: {}", e);
            }
        }
    }
}

/// Entry point for the dedicated SQLite writer thread.
pub fn writer_thread_main(
    mut conn: Connection,
    cmd_rx: Receiver<WriterCommand>,
    failure_threshold: u32,
) {
    info!("Metrics writer thread started");

    let mut state = WriterState::new(failure_threshold);
    let mut last_checkpoint = Instant::now();
    let checkpoint_interval = Duration::from_secs(60);

    loop {
        state.maybe_run_integrity_check(&mut conn);

        // Wait for a command, or timeout to run periodic maintenance
        let cmd_res = cmd_rx.recv_timeout(Duration::from_secs(1));
        let cmd = match cmd_res {
            Ok(cmd) => Some(cmd),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                warn!("Writer command channel disconnected unexpectedly");
                break;
            }
        };

        if let Some(cmd) = cmd {
            if matches!(cmd, WriterCommand::Shutdown) {
                info!("Writer thread received Shutdown command");
                handle_shutdown(&mut conn, &cmd_rx);
                break;
            }

            handle_command(&mut conn, &mut state, cmd);
        }

        // Periodic checkpoint
        if last_checkpoint.elapsed() >= checkpoint_interval && !state.is_degraded() {
            match conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| row.get::<_, i64>(2)) {
                Ok(pages) => debug!("Periodic WAL checkpoint: {} pages", pages),
                Err(e) => {
                    error!("Periodic WAL checkpoint failed: {}", e);
                    state.record_failure();
                }
            }
            last_checkpoint = Instant::now();
        }
    }

    info!("Metrics writer thread exiting");
}

fn handle_shutdown(conn: &mut Connection, cmd_rx: &Receiver<WriterCommand>) {
    // 1. Drain all pending commands
    let mut drained = 0;
    while let Ok(cmd) = cmd_rx.try_recv() {
        if matches!(cmd, WriterCommand::Shutdown) {
            continue;
        }

        // We bypass the state machine during shutdown to ensure we drain
        // as much as possible, though if we are degraded, writes will fail anyway.
        let mut dummy_state = WriterState::new(u32::MAX);
        handle_command(conn, &mut dummy_state, cmd);
        drained += 1;
    }

    if drained > 0 {
        info!("Drained {} pending commands during shutdown", drained);
    }

    // 2. Truncate checkpoint
    match conn.execute("PRAGMA wal_checkpoint(TRUNCATE)", []) {
        Ok(_) => info!("Shutdown WAL checkpoint (TRUNCATE) complete"),
        Err(e) => error!("Shutdown WAL checkpoint failed: {}", e),
    }

    // 3. Connection is closed implicitly when dropped
}

fn handle_command(conn: &mut Connection, state: &mut WriterState, cmd: WriterCommand) {
    match cmd {
        WriterCommand::InsertEvents { events, reply, span } => {
            let _enter = span.enter();
            if state.is_degraded() {
                let _ =
                    reply.send(Err(StoreError::Unavailable { message: "Writer degraded".into() }));
                return;
            }

            match execute_insert_events(conn, events) {
                Ok(_) => {
                    state.record_success();
                    let _ = reply.send(Ok(()));
                }
                Err(e) => {
                    state.record_failure();
                    let _ = reply.send(Err(StoreError::Internal { source: Box::new(e) }));
                }
            }
        }
        WriterCommand::InsertMetrics { records, reply, span } => {
            let _enter = span.enter();
            if state.is_degraded() {
                let _ =
                    reply.send(Err(StoreError::Unavailable { message: "Writer degraded".into() }));
                return;
            }

            match execute_insert_metrics(conn, records) {
                Ok(_) => {
                    state.record_success();
                    let _ = reply.send(Ok(()));
                }
                Err(e) => {
                    state.record_failure();
                    let _ = reply.send(Err(StoreError::Internal { source: Box::new(e) }));
                }
            }
        }
        WriterCommand::UpsertProcess { proc, reply, span } => {
            let _enter = span.enter();
            if state.is_degraded() {
                let _ =
                    reply.send(Err(StoreError::Unavailable { message: "Writer degraded".into() }));
                return;
            }

            let row = ManagedProcessRow::from(proc);
            let res = conn.execute(
                "INSERT OR REPLACE INTO managed_processes (pid, server_id, started_at, cmdline) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![row.pid, row.server_id, row.started_at, row.cmdline],
            );

            match res {
                Ok(_) => {
                    state.record_success();
                    let _ = reply.send(Ok(()));
                }
                Err(e) => {
                    state.record_failure();
                    let _ = reply.send(Err(StoreError::Internal { source: Box::new(e) }));
                }
            }
        }
        WriterCommand::RemoveProcess { pid, reply, span } => {
            let _enter = span.enter();
            if state.is_degraded() {
                let _ =
                    reply.send(Err(StoreError::Unavailable { message: "Writer degraded".into() }));
                return;
            }

            let res = conn.execute("DELETE FROM managed_processes WHERE pid = ?1", [pid.0]);

            match res {
                Ok(_) => {
                    state.record_success();
                    let _ = reply.send(Ok(()));
                }
                Err(e) => {
                    state.record_failure();
                    let _ = reply.send(Err(StoreError::Internal { source: Box::new(e) }));
                }
            }
        }
        WriterCommand::InsertConfigSnapshot { snapshot, reply, span } => {
            let _enter = span.enter();
            if state.is_degraded() {
                let _ =
                    reply.send(Err(StoreError::Unavailable { message: "Writer degraded".into() }));
                return;
            }

            let row = ConfigSnapshotRow::from(snapshot);
            let res = conn.execute(
                "INSERT INTO config_snapshots (id, trigger, created_at, config_json) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![row.id, row.trigger, row.created_at, row.config_json],
            );

            match res {
                Ok(_) => {
                    state.record_success();
                    let _ = reply.send(Ok(()));
                }
                Err(e) => {
                    state.record_failure();
                    let _ = reply.send(Err(StoreError::Internal { source: Box::new(e) }));
                }
            }
        }
        WriterCommand::Prune { cutoff, reply, span } => {
            let _enter = span.enter();
            if state.is_degraded() {
                let _ =
                    reply.send(Err(StoreError::Unavailable { message: "Writer degraded".into() }));
                return;
            }

            let cutoff_str = cutoff.to_rfc3339();

            match conn.transaction() {
                Ok(tx) => {
                    let res1 =
                        tx.execute("DELETE FROM audit_events WHERE created_at < ?1", [&cutoff_str]);
                    let res2 = tx.execute(
                        "DELETE FROM request_metrics WHERE received_at < ?1",
                        [&cutoff_str],
                    );

                    match (res1, res2) {
                        (Ok(r1), Ok(r2)) => {
                            if let Err(e) = tx.commit() {
                                state.record_failure();
                                let _ =
                                    reply.send(Err(StoreError::Internal { source: Box::new(e) }));
                            } else {
                                state.record_success();
                                let _ = reply.send(Ok((r1 + r2) as u64));
                            }
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            let _ = tx.rollback();
                            state.record_failure();
                            let _ = reply.send(Err(StoreError::Internal { source: Box::new(e) }));
                        }
                    }
                }
                Err(e) => {
                    state.record_failure();
                    let _ = reply.send(Err(StoreError::Internal { source: Box::new(e) }));
                }
            }
        }
        WriterCommand::Checkpoint { mode, reply, span } => {
            let _enter = span.enter();
            let mode_str = match mode {
                CheckpointMode::Passive => "PASSIVE",
                CheckpointMode::Truncate => "TRUNCATE",
            };

            match conn.query_row(&format!("PRAGMA wal_checkpoint({})", mode_str), [], |row| {
                row.get::<_, u32>(2)
            }) {
                Ok(pages) => {
                    state.record_success();
                    let _ = reply.send(Ok(CheckpointResult { pages_checkpointed: pages }));
                }
                Err(e) => {
                    state.record_failure();
                    let _ = reply.send(Err(StoreError::Internal { source: Box::new(e) }));
                }
            }
        }
        WriterCommand::Shutdown => unreachable!("Handled in loop"),
    }
}

fn execute_insert_events(
    conn: &mut Connection,
    events: Vec<crate::types::AuditEvent>,
) -> Result<(), rusqlite::Error> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO audit_events (event_id, event_type, event_kind, server_id, request_id, created_at, payload) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        for event in events {
            let row = AuditEventRow::from(event);
            stmt.execute(rusqlite::params![
                row.event_id,
                row.event_type,
                row.event_kind,
                row.server_id,
                row.request_id,
                row.created_at,
                row.payload
            ])?;
        }
    }
    tx.commit()
}

fn execute_insert_metrics(
    conn: &mut Connection,
    metrics: Vec<crate::types::RequestMetrics>,
) -> Result<(), rusqlite::Error> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            r#"INSERT INTO request_metrics (
                request_id, model_name, server_id, slot_id, task_type, requestor, effective_priority,
                received_at, dispatched_at, first_token_at, completed_at,
                queue_wait_ms, ttft_ms, generation_duration_ms, total_latency_ms,
                prompt_tokens, completion_tokens, prompt_tps, generation_tps,
                status, finish_reason, was_evicted, eviction_count, error_message
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19,
                ?20, ?21, ?22, ?23, ?24
            )"#,
        )?;

        for m in metrics {
            let row = RequestMetricsRow::from(m);
            stmt.execute(rusqlite::params![
                row.request_id,
                row.model_name,
                row.server_id,
                row.slot_id,
                row.task_type,
                row.requestor,
                row.effective_priority,
                row.received_at,
                row.dispatched_at,
                row.first_token_at,
                row.completed_at,
                row.queue_wait_ms,
                row.ttft_ms,
                row.generation_duration_ms,
                row.total_latency_ms,
                row.prompt_tokens,
                row.completion_tokens,
                row.prompt_tps,
                row.generation_tps,
                row.status,
                row.finish_reason,
                row.was_evicted,
                row.eviction_count,
                row.error_message
            ])?;
        }
    }
    tx.commit()
}

/// A test-only struct that owns `SyncSender<WriterCommand>` + `JoinHandle<()>`.
/// `impl Drop`: sends `WriterCommand::Shutdown` and `join()`s the thread.
pub struct WriterTestHandle {
    pub tx: SyncSender<WriterCommand>,
    pub handle: Option<JoinHandle<()>>,
}

impl Drop for WriterTestHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(WriterCommand::Shutdown);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
