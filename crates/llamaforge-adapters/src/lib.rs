//! Adapters bridging concrete component types to the port traits consumed by `llamaforge-transport`.
//! See design doc §4.2 Hexagonal layering.

pub mod audit;
pub mod inference_engine;
pub mod metrics_exporter;
pub mod queue;
pub mod server_control;
