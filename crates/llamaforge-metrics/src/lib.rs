pub(crate) mod error;
pub(crate) mod filter;
pub(crate) mod ingest;
pub(crate) mod instrumented;
pub(crate) mod metrics_recording;
pub(crate) mod sqlite;
pub(crate) mod traits;
pub(crate) mod types;

// Public API
pub use error::{AuditSendError, StoreError};
pub use filter::{AggregatedMetrics, EventFilter, GroupByField, MetricsFilter};
pub use ingest::{audit_drops, cmd_channel_drops, telemetry_drops, MetricsIngest};
pub use instrumented::InstrumentedMetricsStore;
pub use metrics_recording::{
    install_metrics_recorder, AppMetrics, MetricsFacadeRecorder, RequestMetricsRecorder,
    StoreHealthRecorder,
};
pub use sqlite::pipeline::{create_metrics_pipeline, MetricsPipeline};
pub use sqlite::store::SqliteMetricsStore;
pub use traits::{
    ConfigOverlay, ConfigOverlayStore, EventStore, ProcessStore, RequestMetricsStore,
    StoreMaintenance,
};
pub use types::{
    AuditEvent, AuditRecord, CheckpointMode, CheckpointResult, ConfigSnapshot,
    ConfigSnapshotTrigger, EventType, ManagedProcess, MetricsConfig, ProcessId, RequestId,
    RequestMetrics, RequestStatus, TelemetryEvent, TelemetryEventKind, WriterMode,
};

// Internal types exposed for integration tests
#[cfg(any(test, feature = "test-utils"))]
pub use metrics_recording::NoopMetrics;

#[cfg(any(test, feature = "test-utils"))]
pub use sqlite::{
    batch_collector::batch_collector_main,
    commands::WriterCommand,
    liveness::start_liveness_monitor,
    migrations::get_migrations,
    open_and_migrate,
    pool::create_read_pool,
    prune::start_prune_task,
    query_builder::{build_event_where, build_group_by, build_metrics_where},
    rows::{
        AuditEventRow, ConfigOverlayRow, ConfigSnapshotRow, ManagedProcessRow, RequestMetricsRow,
    },
    writer::{writer_thread_main, WriterTestHandle},
};
