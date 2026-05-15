//! LlamaForge orchestrator library surface.
//!
//! The binary at `src/main.rs` is the production entry point. This lib
//! target exists so workspace integration tests (`llamaforge-e2e`) can
//! construct the same composition root in-process without shelling out
//! to the binary.
//!
//! `init` and `validate` remain bin-private — they're CLI subcommand
//! handlers, not library API.

pub(crate) mod build;
pub(crate) mod serve;
pub(crate) mod tracing_init;

pub use build::{build, BackgroundTasks};
pub use serve::run;
