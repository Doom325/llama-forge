pub(crate) mod error;
pub(crate) mod filter;
pub(crate) mod ingest;
pub(crate) mod instrumented;
pub(crate) mod metrics_recording;
pub(crate) mod sqlite;
pub(crate) mod traits;
pub(crate) mod types;

pub use error::{AuditSendError, StoreError};
pub use filter::{AggregatedMetrics, EventFilter, GroupByField, MetricsFilter};
pub use metrics_recording::{AppMetrics, RequestMetricsRecorder, StoreHealthRecorder};
pub use sqlite::open_and_migrate;
pub use sqlite::writer::writer_thread_main;
pub use sqlite::{
    batch_collector::{batch_collector_main, cmd_channel_drops},
    commands::WriterCommand,
    migrations::get_migrations,
    pool::create_read_pool,
    rows::{
        AuditEventRow, ConfigOverlayRow, ConfigSnapshotRow, ManagedProcessRow, RequestMetricsRow,
    },
    store::SqliteMetricsStore,
};
#[cfg(any(test, feature = "test-utils"))]
pub use sqlite::{
    query_builder::{build_event_where, build_group_by, build_metrics_where},
    writer::WriterTestHandle,
};
pub use traits::{
    ConfigOverlay, ConfigOverlayStore, EventStore, ProcessStore, RequestMetricsStore,
    StoreMaintenance,
};
pub use types::{
    AuditEvent, AuditRecord, CheckpointMode, CheckpointResult, ConfigSnapshot,
    ConfigSnapshotTrigger, EventType, ManagedProcess, MetricsConfig, ProcessId, RequestId,
    RequestMetrics, RequestStatus, TelemetryEvent, TelemetryEventKind, WriterMode,
};
