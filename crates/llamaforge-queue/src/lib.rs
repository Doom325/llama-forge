//! Request Queue & Scheduler — priority-aware FIFO with per-requestor
//! limits, admission control, and queue-only preemption.
//!
//! Implements design.md §5.2 (Request Queue & Scheduler) and §9
//! (Priority System & Eviction). Component spec:
//! `docs/component-designs/04_REQUEST_QUEUE_AND_SCHEDULER.md`.
//!
//! **Queue-depth gauge invariant.** The `Arc<AtomicU32>` counter is
//! constructed by the composition root and injected into both
//! `AdmissionController::new()` and `DispatchQueue::new()`. Both must
//! accept the shared counter rather than creating their own.

pub mod admission;
pub mod config;
pub mod error;
pub mod pipeline;
pub mod queue;
pub mod starvation;
pub mod types;
