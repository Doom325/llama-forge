//! Commands sent from the async world to the synchronous writer thread.
//!
//! See design doc Section 3.3 and 3.4.

use chrono::{DateTime, Utc};
use tokio::sync::oneshot;
use tracing::Span;

use crate::error::StoreError;
use crate::types::{
    AuditEvent, CheckpointMode, CheckpointResult, ConfigSnapshot, ManagedProcess, ProcessId,
    RequestMetrics,
};

/// Command protocol for the SQLite writer thread.
#[allow(dead_code)]
#[non_exhaustive]
#[derive(Debug)]
pub enum WriterCommand {
    /// Batch insert audit events.
    InsertEvents {
        events: Vec<AuditEvent>,
        reply: oneshot::Sender<Result<(), StoreError>>,
        span: Span,
    },
    /// Batch insert request metrics.
    InsertMetrics {
        records: Vec<RequestMetrics>,
        reply: oneshot::Sender<Result<(), StoreError>>,
        span: Span,
    },
    /// Upsert managed process record.
    UpsertProcess {
        proc: ManagedProcess,
        reply: oneshot::Sender<Result<(), StoreError>>,
        span: Span,
    },
    /// Remove managed process record.
    RemoveProcess { pid: ProcessId, reply: oneshot::Sender<Result<(), StoreError>>, span: Span },
    /// Insert config snapshot.
    InsertConfigSnapshot {
        snapshot: ConfigSnapshot,
        reply: oneshot::Sender<Result<(), StoreError>>,
        span: Span,
    },
    /// Prune old events.
    Prune { cutoff: DateTime<Utc>, reply: oneshot::Sender<Result<u64, StoreError>>, span: Span },
    /// WAL checkpoint.
    Checkpoint {
        mode: CheckpointMode,
        reply: oneshot::Sender<Result<CheckpointResult, StoreError>>,
        span: Span,
    },
    /// Graceful shutdown — flush remaining, close connection.
    Shutdown,
}
