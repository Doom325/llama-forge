// PR1: ServerId became Copy after the unification. The .clone() calls on
// ServerId in this crate are now suboptimal but harmless; the borrow
// patterns they participate in (let bindings, dashmap inserts) would all
// need to flip to *deref/by-value, which is out of scope for the type
// unification. A follow-up PR can sweep them.
#![allow(clippy::clone_on_copy)]

//! Server Manager — supervises the lifecycle of `llama-server`
//! processes (spawn with the full flag surface, health-check,
//! restart-with-backoff, graceful drain) and exposes the live fleet
//! state to routing.
//!
//! Implements design.md §5.3 (Server Manager) and §10.1
//! (`llama-server` Control Surfaces). Component spec:
//! `docs/component-designs/03_SERVER_MANAGER.md`.

pub(crate) mod error;
pub mod hardware;
pub(crate) mod health;
pub(crate) mod instrumented;
pub(crate) mod manager;
pub mod model_scanner;
pub(crate) mod observability;
pub(crate) mod process;
pub mod recommendation;
pub(crate) mod state;
pub(crate) mod task;
pub(crate) mod telemetry;
#[cfg(feature = "telemetry-amdgpu")]
pub mod telemetry_amdgpu;
#[cfg(feature = "telemetry-nvml")]
pub mod telemetry_nvml;
pub(crate) mod types;
pub(crate) mod vram_estimate;

pub use error::ServerManagerError;
pub use hardware::{
    detect_hardware, detect_hardware_with, CommandRunner, CpuInfo, GpuInfo, HardwareInfo,
    HardwareProbeResult, HardwareWarning, ProbeError, RecommendedBackend, SystemCommandRunner,
};
pub use instrumented::InstrumentedServerManager;
pub use manager::ServerManager;
pub use model_scanner::{
    scan_models, scan_models_with_deadline, DiscoveredModel, ModelScanError, ModelScanRequest,
    ModelScanResult, ModelSource, TruncationReason,
};
pub use observability::{register_metrics, spawn_metrics_poller, DomainMetricsLayer};
pub use recommendation::{
    recommend, ConfigRecommendation, ModelInput, RecommendationRequest, RecommendationResult,
    ServerRecommendation,
};
pub use state::ServerStateSnapshot;
pub use telemetry::{create_telemetry, GpuDevice, GpuTelemetry, StubTelemetry, TelemetryError};
pub use types::{
    ClientRefreshEvent, ModelArchitecture, ServerCapabilities, ServerLifecycle, SlotState,
    SpeculativeMethod,
};

#[cfg(feature = "test-utils")]
#[doc(hidden)]
pub mod __test_exports {
    pub use crate::task::compute_backoff_delay;
}
