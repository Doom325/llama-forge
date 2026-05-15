//! Request Queue & Scheduler — priority-aware FIFO with per-requestor
//! limits, admission control, and queue-only preemption.
//!
//! Implements design.md §5.2 (Request Queue & Scheduler) and §9
//! (Priority System & Eviction). Component spec:
//! `docs/component-designs/04_REQUEST_QUEUE_AND_SCHEDULER.md`.
//!
//!
//! **Contract Note for PR3 & PR4:**
//! `Arc<AtomicU32>` for queue depth is created by the composition root (`main.rs`)
//! and injected into both `AdmissionController::new()` and `DispatchQueue::new()`.
//! PR3 and PR4 implementations must accept `queue_depth: Arc<AtomicU32>` as a
//! constructor parameter and must NOT create their own distinct counters.

pub mod admission;
pub mod config;
pub mod error;
pub mod pipeline;
pub mod queue;
pub mod starvation;
pub mod types;
