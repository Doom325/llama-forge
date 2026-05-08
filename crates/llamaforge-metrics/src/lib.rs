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
pub use sqlite::{
    migrations::get_migrations,
    pool::create_read_pool,
    rows::{
        AuditEventRow, ConfigOverlayRow, ConfigSnapshotRow, ManagedProcessRow, RequestMetricsRow,
    },
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
