//! Domain types and DTOs
//! See design doc Section 2.5.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::time::Instant;

use llamaforge_config::{RequestorId, ServerId, TaskTypeId};

/// Unique identifier for an inference request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub String);

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Unique identifier for a managed process (OS PID).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProcessId(pub u32);

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Status of a request through its lifecycle.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Queued,
    Processing,
    Completed,
    Failed,
    Evicted,
    Cancelled,
    Timeout,
}

/// Categorization of telemetry events.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryEventKind {
    ServerStarted,
    ServerStopped,
    ServerCrashed,
    ConfigReloaded,
}

/// A telemetry event emitted by the system.
#[derive(Debug, Clone)]
pub struct TelemetryEvent {
    pub kind: TelemetryEventKind,
    pub server_id: Option<ServerId>,
    pub timestamp: DateTime<Utc>,
    pub payload: serde_json::Value,
}

/// Billing and performance metrics for a single request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequestMetrics {
    pub request_id: RequestId,
    pub model_name: String,
    pub server_id: Option<ServerId>,
    pub slot_id: Option<u32>,
    pub task_type: TaskTypeId,
    pub requestor: RequestorId,
    pub effective_priority: u32,

    pub received_at: DateTime<Utc>,
    pub dispatched_at: Option<DateTime<Utc>>,
    pub first_token_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,

    pub queue_wait_ms: f64,
    pub ttft_ms: Option<f64>,
    pub generation_duration_ms: Option<f64>,
    pub total_latency_ms: f64,

    pub prompt_tokens: u32,
    pub completion_tokens: u32,

    pub prompt_tps: Option<f64>,
    pub generation_tps: Option<f64>,

    pub status: RequestStatus,
    pub finish_reason: Option<String>,
    pub was_evicted: bool,
    pub eviction_count: u32,
    pub error_message: Option<String>,
}

/// The final record of a request for audit and metrics purposes.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub request_metrics: RequestMetrics,
}

/// The type of an event stored in the event log.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Audit,
    Telemetry,
}

/// A generic event stored in the database.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub event_id: String,
    pub event_type: EventType,
    pub event_kind: String,
    pub server_id: Option<ServerId>,
    pub request_id: Option<RequestId>,
    pub created_at: DateTime<Utc>,
    pub payload: serde_json::Value,
}

/// Record of a running managed process (e.g. llama-server).
#[derive(Debug, Clone)]
pub struct ManagedProcess {
    pub pid: ProcessId,
    pub server_id: ServerId,
    pub started_at: DateTime<Utc>,
    pub cmdline: String,
}

/// What triggered a configuration snapshot to be saved.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSnapshotTrigger {
    Startup,
    Reload,
    Writeback,
}

/// A historical snapshot of the configuration.
#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub id: String,
    pub trigger: ConfigSnapshotTrigger,
    pub created_at: DateTime<Utc>,
    pub config_json: String,
}

/// WAL checkpoint mode.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointMode {
    Passive,
    Truncate,
}

/// Result of a WAL checkpoint operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointResult {
    pub pages_checkpointed: u32,
}

/// The current health state of the SQLite writer thread.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum WriterMode {
    Healthy,
    Degraded { consecutive_failures: u32, since: Instant },
}

/// Configuration for the metrics and storage subsystem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsConfig {
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_telemetry_channel_capacity")]
    pub telemetry_channel_capacity: usize,
    #[serde(default = "default_audit_channel_capacity")]
    pub audit_channel_capacity: usize,
    #[serde(default = "default_audit_semaphore_permits")]
    pub audit_semaphore_permits: usize,
    #[serde(default = "default_read_pool_size")]
    pub read_pool_size: usize,
    #[serde(default = "default_read_statement_timeout_secs")]
    pub read_statement_timeout_secs: u64,
    #[serde(default = "default_batch_flush_interval_ms")]
    pub batch_flush_interval_ms: u64,
    #[serde(default = "default_batch_flush_size")]
    pub batch_flush_size: usize,
    #[serde(default = "default_wal_checkpoint_interval_secs")]
    pub wal_checkpoint_interval_secs: u64,
    #[serde(default = "default_wal_oversize_threshold_mb")]
    pub wal_oversize_threshold_mb: u64,
    #[serde(default = "default_writer_failure_threshold")]
    pub writer_failure_threshold: u32,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            retention_days: default_retention_days(),
            telemetry_channel_capacity: default_telemetry_channel_capacity(),
            audit_channel_capacity: default_audit_channel_capacity(),
            audit_semaphore_permits: default_audit_semaphore_permits(),
            read_pool_size: default_read_pool_size(),
            read_statement_timeout_secs: default_read_statement_timeout_secs(),
            batch_flush_interval_ms: default_batch_flush_interval_ms(),
            batch_flush_size: default_batch_flush_size(),
            wal_checkpoint_interval_secs: default_wal_checkpoint_interval_secs(),
            wal_oversize_threshold_mb: default_wal_oversize_threshold_mb(),
            writer_failure_threshold: default_writer_failure_threshold(),
        }
    }
}

fn default_db_path() -> PathBuf {
    PathBuf::from("./llamaforge.db")
}
fn default_retention_days() -> u32 {
    30
}
fn default_telemetry_channel_capacity() -> usize {
    10000
}
fn default_audit_channel_capacity() -> usize {
    1000
}
fn default_audit_semaphore_permits() -> usize {
    5000 // A reasonable default since it wasn't strictly specified
}
fn default_read_pool_size() -> usize {
    4
}
fn default_read_statement_timeout_secs() -> u64 {
    30
}
fn default_batch_flush_interval_ms() -> u64 {
    100
}
fn default_batch_flush_size() -> usize {
    100
}
fn default_wal_checkpoint_interval_secs() -> u64 {
    60
}
fn default_wal_oversize_threshold_mb() -> u64 {
    100
}
fn default_writer_failure_threshold() -> u32 {
    10
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_types_debug() {
        let req_id = RequestId::from("req-123");
        assert!(format!("{:?}", req_id).contains("req-123"));

        let proc_id = ProcessId(1234);
        assert!(format!("{:?}", proc_id).contains("1234"));

        let mc = MetricsConfig::default();
        assert_eq!(mc.db_path, PathBuf::from("./llamaforge.db"));
        assert_eq!(mc.retention_days, 30);
        assert_eq!(mc.telemetry_channel_capacity, 10000);
        assert_eq!(mc.audit_channel_capacity, 1000);
    }

    #[test]
    fn test_request_status_serde() {
        let status = RequestStatus::Queued;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"queued\"");
        let parsed: RequestStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, status);
    }

    #[test]
    fn test_process_id_serde() {
        let pid = ProcessId(42);
        let json = serde_json::to_string(&pid).unwrap();
        assert_eq!(json, "42");
        let parsed: ProcessId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, pid);
    }
}
