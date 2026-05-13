//! Dispatcher and related orchestration types.
//!
//! See design doc Section 2.1

use llamaforge_types::SlotId;

/// A completion request payload sent to the backend.
///
/// See design doc Section 2.5.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prompt: String,
    pub n_predict: Option<u32>,
    pub temperature: f32,
    pub stream: bool,
    pub stop: Vec<String>,
    pub id_slot: Option<SlotId>,
}
