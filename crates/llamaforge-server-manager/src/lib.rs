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
pub use state::ServerStateSnapshot;
pub use types::{
    ClientRefreshEvent, ModelArchitecture, ServerCapabilities, ServerLifecycle, SlotState,
    SpeculativeMethod,
};
