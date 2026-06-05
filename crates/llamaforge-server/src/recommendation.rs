//! Config recommendation engine for the Setup Wizard.
//!
//! Given detected [`HardwareInfo`] and the models the user selected, computes a
//! per-model `llama-server` configuration — `gpu_layers`, `ctx_size`,
//! `parallel_slots`, KV cache types, `flash_attn`, an `estimated_vram_mb`, and
//! an auto-assigned `port` — each accompanied by a human-readable rationale.
//! Implements `08_SETUP_WIZARD.md` Section 11.5 (Config Recommendation) and the
//! Appendix implementation notes.
//!
//! VRAM is estimated from the model's **file size** (a quantized GGUF loads to
//! roughly its on-disk size) plus a KV-cache term and a fixed compute overhead.
//! Per Open Question #1 the wizard does not parse GGUF headers in v1, so the
//! richer [`crate::vram_estimate::compute_vram_estimate`] (which needs full
//! GGUF metadata) is not used here; this engine is intentionally a coarse,
//! transparent heuristic the user can override on the configure screen.
//!
//! Pure domain logic: no axum, no `tracing`. The PR5 handler adds the
//! `"version": 1` envelope and span instrumentation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hardware::HardwareInfo;

/// Bytes per MB.
const MB: u64 = 1024 * 1024;
/// Fixed compute/runtime overhead reserved on the GPU (CUDA context, buffers).
const OVERHEAD_MB: u64 = 1024;
/// Default context length when VRAM is comfortable.
const DEFAULT_CTX: u32 = 8192;
/// Reduced context length used when VRAM is tight.
const TIGHT_CTX: u32 = 4096;
/// Default concurrent slots.
const DEFAULT_SLOTS: u32 = 2;
/// Reduced slots used when VRAM is tight.
const TIGHT_SLOTS: u32 = 1;
/// Default KV cache quantization.
const DEFAULT_CACHE: &str = "q8_0";
/// First auto-assigned server port; subsequent servers increment from here.
const BASE_PORT: u16 = 8080;

// ---------------------------------------------------------------------------
// Request / response types.
// ---------------------------------------------------------------------------

/// One model the user selected, as sent to `POST /api/setup/recommend`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInput {
    pub path: String,
    pub size_bytes: u64,
}

/// A recommendation request (Section 11.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecommendationRequest {
    pub models: Vec<ModelInput>,
}

/// The recommended `llama-server` flags for one model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigRecommendation {
    /// `-1` = full offload, `0` = CPU-only, `N` = offload N layers.
    pub gpu_layers: i32,
    pub ctx_size: u32,
    pub parallel_slots: u32,
    pub cache_type_k: String,
    pub cache_type_v: String,
    pub flash_attn: String,
    pub estimated_vram_mb: u64,
    pub port: u16,
}

/// A per-model recommendation plus field-by-field rationale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerRecommendation {
    pub model_path: String,
    pub recommended: ConfigRecommendation,
    /// Human-readable explanation keyed by field name (stable ordering).
    pub rationale: BTreeMap<String, String>,
}

/// Aggregate result (Section 11.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecommendationResult {
    pub servers: Vec<ServerRecommendation>,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public API.
// ---------------------------------------------------------------------------

/// Produce a recommendation for each model given the detected hardware.
///
/// Ports auto-increment from [`BASE_PORT`]. A capacity summary (headroom or
/// over-budget) is emitted as a warning whenever a GPU is present.
pub fn recommend(hardware: &HardwareInfo, models: &[ModelInput]) -> RecommendationResult {
    // One model per GPU (Open Question #3): budget each model against the
    // largest single GPU; warn against the total across all GPUs.
    let per_gpu_budget_mb = hardware.gpus.iter().map(|g| g.vram_mb).max().unwrap_or(0);
    let total_vram_mb: u64 = hardware.gpus.iter().map(|g| g.vram_mb).sum();

    let servers: Vec<ServerRecommendation> = models
        .iter()
        .enumerate()
        .map(|(index, model)| {
            let port = BASE_PORT.saturating_add(index as u16);
            recommend_one(model, per_gpu_budget_mb, port)
        })
        .collect();

    let mut warnings = Vec::new();
    if per_gpu_budget_mb > 0 {
        let total_estimated_mb: u64 = servers.iter().map(|s| s.recommended.estimated_vram_mb).sum();
        warnings.push(capacity_summary(total_estimated_mb, total_vram_mb));
    } else if !models.is_empty() {
        warnings.push(
            "No GPU detected — all models will run on CPU, which is significantly slower."
                .to_string(),
        );
    }

    RecommendationResult { servers, warnings }
}

fn recommend_one(model: &ModelInput, gpu_budget_mb: u64, port: u16) -> ServerRecommendation {
    let model_mb = model.size_bytes / MB;
    let mut rationale = BTreeMap::new();

    // CPU-only path: no GPU detected.
    if gpu_budget_mb == 0 {
        let estimated = estimated_vram_mb(model_mb, DEFAULT_CTX, DEFAULT_SLOTS);
        rationale.insert(
            "gpu_layers".to_string(),
            "No GPU detected — running CPU-only (gpu_layers = 0). Weights stay in system RAM."
                .to_string(),
        );
        rationale.insert(
            "ctx_size".to_string(),
            format!("{DEFAULT_CTX} tokens — default context for CPU inference."),
        );
        rationale.insert(
            "parallel_slots".to_string(),
            format!("{DEFAULT_SLOTS} slots — default concurrency for CPU inference."),
        );
        return ServerRecommendation {
            model_path: model.path.clone(),
            recommended: ConfigRecommendation {
                gpu_layers: 0,
                ctx_size: DEFAULT_CTX,
                parallel_slots: DEFAULT_SLOTS,
                cache_type_k: DEFAULT_CACHE.to_string(),
                cache_type_v: DEFAULT_CACHE.to_string(),
                flash_attn: "auto".to_string(),
                estimated_vram_mb: estimated,
                port,
            },
            rationale,
        };
    }

    // Full offload fits at the default context/slots.
    let full_estimate = estimated_vram_mb(model_mb, DEFAULT_CTX, DEFAULT_SLOTS);
    if full_estimate <= gpu_budget_mb {
        rationale.insert(
            "gpu_layers".to_string(),
            format!(
                "Full offload (gpu_layers = -1) — model ({}) + KV cache + overhead ≈ {} fits in {} VRAM.",
                gb(model_mb),
                gb(full_estimate),
                gb(gpu_budget_mb)
            ),
        );
        rationale.insert(
            "ctx_size".to_string(),
            format!("{DEFAULT_CTX} tokens — comfortable for the available VRAM."),
        );
        rationale.insert(
            "parallel_slots".to_string(),
            format!("{DEFAULT_SLOTS} slots — balances concurrency against VRAM headroom."),
        );
        rationale.insert(
            "cache".to_string(),
            format!("{DEFAULT_CACHE} K/V cache with flash attention auto-detected."),
        );
        return ServerRecommendation {
            model_path: model.path.clone(),
            recommended: ConfigRecommendation {
                gpu_layers: -1,
                ctx_size: DEFAULT_CTX,
                parallel_slots: DEFAULT_SLOTS,
                cache_type_k: DEFAULT_CACHE.to_string(),
                cache_type_v: DEFAULT_CACHE.to_string(),
                flash_attn: "auto".to_string(),
                estimated_vram_mb: full_estimate,
                port,
            },
            rationale,
        };
    }

    // Tight: shrink context + slots and offload as many layers as fit.
    let ctx = TIGHT_CTX;
    let slots = TIGHT_SLOTS;
    let kv_mb = kv_cache_mb(model_mb, ctx, slots);
    let total_layers = estimated_layers(model_mb);
    let weight_budget_mb = gpu_budget_mb.saturating_sub(kv_mb + OVERHEAD_MB);
    let fraction = (weight_budget_mb as f64 / model_mb.max(1) as f64).clamp(0.0, 1.0);
    let gpu_layers = (fraction * total_layers as f64).floor() as i32;
    // VRAM actually used on the GPU for the offloaded portion.
    let offloaded_mb = (model_mb as f64 * fraction) as u64;
    let estimated = offloaded_mb + kv_mb + OVERHEAD_MB;

    rationale.insert(
        "gpu_layers".to_string(),
        format!(
            "Partial offload — model ({}) exceeds {} VRAM, so ~{gpu_layers}/{total_layers} layers fit on GPU; the rest run on CPU.",
            gb(model_mb),
            gb(gpu_budget_mb)
        ),
    );
    rationale.insert(
        "ctx_size".to_string(),
        format!("{ctx} tokens — reduced from {DEFAULT_CTX} to free VRAM under memory pressure."),
    );
    rationale.insert(
        "parallel_slots".to_string(),
        format!("{slots} slot — reduced concurrency to fit a large model in limited VRAM."),
    );
    rationale.insert(
        "cache".to_string(),
        format!("{DEFAULT_CACHE} K/V cache to minimize KV memory footprint."),
    );

    ServerRecommendation {
        model_path: model.path.clone(),
        recommended: ConfigRecommendation {
            gpu_layers,
            ctx_size: ctx,
            parallel_slots: slots,
            cache_type_k: DEFAULT_CACHE.to_string(),
            cache_type_v: DEFAULT_CACHE.to_string(),
            flash_attn: "auto".to_string(),
            estimated_vram_mb: estimated,
            port,
        },
        rationale,
    }
}

// ---------------------------------------------------------------------------
// VRAM heuristic (file-size based; pure, unit-tested).
// ---------------------------------------------------------------------------

/// Estimated KV-cache size in MB. Scales with context length, concurrent
/// slots, and model size (a proxy for layer count / hidden size). Calibrated so
/// an 8 B model at ctx 8192 / 2 slots lands near ~1.2 GB, per the Section 11.5
/// example.
pub(crate) fn kv_cache_mb(model_mb: u64, ctx_size: u32, parallel_slots: u32) -> u64 {
    let ctx_factor = ctx_size as f64 / DEFAULT_CTX as f64;
    (ctx_factor * parallel_slots as f64 * model_mb as f64 * 0.15) as u64
}

/// Total estimated VRAM (MB) to fully serve a model at the given settings.
pub(crate) fn estimated_vram_mb(model_mb: u64, ctx_size: u32, parallel_slots: u32) -> u64 {
    model_mb + kv_cache_mb(model_mb, ctx_size, parallel_slots) + OVERHEAD_MB
}

/// Approximate transformer layer count, bucketed by model file size. Used only
/// to turn a "fraction of weights that fit" into a `gpu_layers` count.
pub(crate) fn estimated_layers(model_mb: u64) -> u32 {
    match model_mb {
        0..=6_000 => 32,       // ~7–8 B
        6_001..=16_000 => 40,  // ~13–20 B
        16_001..=45_000 => 60, // ~30–34 B
        _ => 80,               // 65–70 B+
    }
}

/// Format an MB amount as a `"X.Y GB"` string for rationale text.
fn gb(mb: u64) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

/// Build the fleet-wide capacity summary warning.
fn capacity_summary(total_estimated_mb: u64, total_vram_mb: u64) -> String {
    if total_estimated_mb > total_vram_mb {
        format!(
            "Total estimated VRAM for all models: {} / {} available — over budget. \
             Reduce the number of models, lower context sizes, or use partial offload.",
            gb(total_estimated_mb),
            gb(total_vram_mb)
        )
    } else {
        format!(
            "Total estimated VRAM for all models: {} / {} available. \
             You have headroom for additional models.",
            gb(total_estimated_mb),
            gb(total_vram_mb)
        )
    }
}
