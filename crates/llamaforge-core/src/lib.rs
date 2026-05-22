//! LlamaForge core — domain types, configuration, routing policy, and the
//! request queue.
//!
//! This crate is the "brain" of the orchestrator: it owns the domain model
//! and the in-process decision-making (admission, queueing, routing). It has
//! no infrastructure I/O dependencies — every other internal crate depends
//! on it.
//!
//! Implements design.md §4.2 (Workspace decomposition). Component specs:
//! `docs/component-designs/{01_CONFIG_MANAGER,04_REQUEST_QUEUE_AND_SCHEDULER,05_SMART_ROUTING}.md`.
//!
//! ## Submodules
//!
//! - [`types`]   — domain types, IDs, port traits (formerly `llamaforge-types`).
//! - [`config`]  — config manager and `ArcSwap`-based hot reload (formerly `llamaforge-config`).
//! - [`routing`] — smart routing policy: slot/affinity selection (formerly `llamaforge-routing`).
//! - [`queue`]   — priority-aware request queue and scheduler (formerly `llamaforge-queue`).
//!
//! Inside the crate the submodules retain their old top-level structure so
//! blame and call-site paths stay stable; everything else continues to use
//! the fully-qualified `llamaforge_core::types::Foo` path.

pub mod config;
pub mod queue;
pub mod routing;
pub mod types;
