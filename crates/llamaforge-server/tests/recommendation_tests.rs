//! Config recommendation tests (08_SETUP_WIZARD.md Section 11.5).
//!
//! Deterministic inputs (fixed model file sizes + synthetic hardware) exercise
//! the VRAM-aware `gpu_layers`/`ctx_size`/`parallel_slots` logic, rationale
//! generation, port auto-assignment, and multi-model VRAM warnings.

use llamaforge_server::{
    recommend, CpuInfo, GpuInfo, HardwareInfo, ModelInput, RecommendedBackend,
};

/// Synthetic hardware: one GPU of `gpu_vram_mb` MB, or CPU-only when `None`.
fn hardware(gpu_vram_mb: Option<u64>) -> HardwareInfo {
    let (gpus, backend) = match gpu_vram_mb {
        Some(vram_mb) => (
            vec![GpuInfo { index: 0, name: "Test GPU".to_string(), vram_mb, driver_version: None }],
            RecommendedBackend::Cuda,
        ),
        None => (Vec::new(), RecommendedBackend::Cpu),
    };
    HardwareInfo {
        gpus,
        cpu: CpuInfo { name: "Test CPU".to_string(), cores_physical: 8, cores_logical: 16 },
        ram_mb: 65_536,
        recommended_backend: backend,
        warnings: Vec::new(),
    }
}

fn model(path: &str, gb: f64) -> ModelInput {
    ModelInput { path: path.to_string(), size_bytes: (gb * 1024.0 * 1024.0 * 1024.0) as u64 }
}

// --- 4.12: single model, ample VRAM ---------------------------------------

#[test]
fn single_model_ample_vram_full_offload() {
    let hw = hardware(Some(24_576)); // 24 GB
    let result = recommend(&hw, &[model("/models/llama-3-8b-Q4_K_M.gguf", 4.1)]);

    assert_eq!(result.servers.len(), 1);
    let rec = &result.servers[0].recommended;
    assert_eq!(rec.gpu_layers, -1, "8B on 24GB should fully offload");
    assert_eq!(rec.ctx_size, 8192);
    assert_eq!(rec.parallel_slots, 2);
    assert_eq!(rec.cache_type_k, "q8_0");
    assert_eq!(rec.flash_attn, "auto");
    assert_eq!(rec.port, 8080);
    // Rationale strings are present and non-empty.
    assert!(result.servers[0].rationale.contains_key("gpu_layers"));
    assert!(result.servers[0].rationale.values().all(|r| !r.trim().is_empty()));
}

// --- 4.13: single model, tight VRAM ---------------------------------------

#[test]
fn single_model_tight_vram_partial_offload() {
    let hw = hardware(Some(24_576)); // 24 GB
    let result = recommend(&hw, &[model("/models/llama-70b-Q4_K_M.gguf", 40.0)]);

    let rec = &result.servers[0].recommended;
    // Partial offload: some but not all layers, and not full (-1) or CPU (0).
    assert!(rec.gpu_layers > 0, "expected partial offload, got {}", rec.gpu_layers);
    assert!(rec.ctx_size < 8192, "context should shrink under VRAM pressure");
    assert_eq!(rec.ctx_size, 4096);
    assert_eq!(rec.parallel_slots, 1);
    assert!(result.servers[0].rationale.get("gpu_layers").unwrap().contains("Partial offload"));
}

// --- 4.14: no GPU (CPU-only) ----------------------------------------------

#[test]
fn no_gpu_recommends_cpu_only() {
    let hw = hardware(None);
    let result = recommend(&hw, &[model("/models/mistral-7b-Q4_K_M.gguf", 4.0)]);

    let rec = &result.servers[0].recommended;
    assert_eq!(rec.gpu_layers, 0);
    let gpu_rationale = result.servers[0].rationale.get("gpu_layers").unwrap();
    assert!(
        gpu_rationale.contains("CPU-only"),
        "rationale should mention CPU-only: {gpu_rationale}"
    );
}

// --- 4.15: multi-model port auto-assignment -------------------------------

#[test]
fn multi_model_ports_increment_from_8080() {
    let hw = hardware(Some(24_576));
    let models =
        [model("/models/a.gguf", 4.0), model("/models/b.gguf", 4.0), model("/models/c.gguf", 4.0)];
    let result = recommend(&hw, &models);

    let ports: Vec<u16> = result.servers.iter().map(|s| s.recommended.port).collect();
    assert_eq!(ports, vec![8080, 8081, 8082]);
}

// --- 4.16: multi-model VRAM overflow warning ------------------------------

#[test]
fn multi_model_vram_overflow_warns() {
    let hw = hardware(Some(24_576)); // 24 GB total
    let models = [model("/models/big-1.gguf", 40.0), model("/models/big-2.gguf", 40.0)];
    let result = recommend(&hw, &models);

    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("over budget") || w.contains("Total estimated VRAM")),
        "expected an over-budget warning, got: {:?}",
        result.warnings
    );
}
