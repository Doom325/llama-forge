//! LlamaForge Server Manager.
//! Supervises the lifecycle of llama-server processes.
//! See design doc Section 03_SERVER_MANAGER.md.

pub(crate) mod error;
pub(crate) mod health;
pub(crate) mod instrumented;
pub(crate) mod manager;
pub(crate) mod observability;
pub(crate) mod process;
pub(crate) mod state;
pub(crate) mod task;
pub(crate) mod telemetry;
pub(crate) mod types;
pub(crate) mod vram_estimate;

pub use error::ServerManagerError;
pub use instrumented::InstrumentedServerManager;
pub use manager::ServerManager;
pub use observability::{register_metrics, spawn_metrics_poller, DomainMetricsLayer};
pub use state::ServerStateSnapshot;
pub use telemetry::{GpuDevice, GpuTelemetry, StubTelemetry, TelemetryError};
pub use types::{
    ClientRefreshEvent, ModelArchitecture, ServerCapabilities, ServerLifecycle, SlotState,
    SpeculativeMethod,
};

#[cfg(feature = "test-utils")]
#[doc(hidden)]
pub mod __test_exports {
    pub use crate::task::compute_backoff_delay;
}
