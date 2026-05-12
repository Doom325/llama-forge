//! LlamaForge Request Queue & Scheduler crate.
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
pub mod types;
