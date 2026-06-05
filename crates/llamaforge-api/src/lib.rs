//! `llamaforge-api`: the merged API/transport/dispatch crate.
//!
//! This crate is the merged successor of three predecessors:
//! `llamaforge-transport`, `llamaforge-adapters`, and
//! `llamaforge-dispatch`. The three former crates now live as
//! submodules under this crate, keeping their original module
//! organization intact (see Phase 9 PR3 in `PLAN.md`).
//!
//! Implements design.md §4.2 (Hexagonal layering) and §7 (API
//! Specification / Transport layer). Component specs:
//! `docs/component-designs/06_DISPATCH_AND_PROXY.md` and
//! `docs/component-designs/07_TRANSPORT_LAYER.md`.

// Carry-over from PR1: the `clone_on_copy` lint is triggered by some
// borrow patterns under llamaforge-server that we deliberately
// kept rather than inline-fixing. The merged api crate has equivalent
// patterns in the dispatch/transport interaction layer; rather than
// scatter targeted fixes through behavior-preserving refactor code,
// allow at crate root with the same rationale.
#![allow(clippy::clone_on_copy)]

pub mod adapters;
pub mod dispatch;
pub mod setup;
pub mod transport;

// Re-exports of the load-bearing public surface (PLAN.md PR3 task 3.3).
// These are the symbols `llamaforge::build::*` and `llamaforge-e2e`
// import; surfacing them at the crate root is convenience, not policy.
pub use transport::streaming::StreamingInferenceService;
pub use transport::{
    start_servers, start_servers_with_setup, InferenceEngine, SetupMount, TransportDeps,
    TransportHandles,
};

pub use dispatch::client::ReqwestClientStore;
pub use dispatch::dispatcher::Dispatcher;

pub use adapters::metrics_exporter::MetricsExporterAdapter;
pub use adapters::queue::QueueSubmitterAdapter;
pub use adapters::server_control::ServerControlAdapter;

pub use setup::{setup_router, SetupState};

/// Router for the standalone setup wizard (08_SETUP_WIZARD.md Section 12): the
/// embedded web UI plus the `/api/setup/*` surface, with no inference, gRPC, or
/// config handle. Used by `llamaforge init --interactive`.
pub fn wizard_router(state: SetupState) -> axum::Router {
    transport::ui::ui_router().merge(setup_router(state))
}
