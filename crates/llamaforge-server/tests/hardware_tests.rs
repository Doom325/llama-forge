//! Hardware detection tests (08_SETUP_WIZARD.md Section 11.1).
//!
//! The detection orchestrator is driven through its public [`CommandRunner`]
//! port by an in-test [`MockRunner`], so probe parsing, partial-failure
//! handling, and `recommended_backend` priority are all exercised against the
//! public surface without touching host hardware.

use std::time::Duration;

use llamaforge_server::{
    detect_hardware_with, CommandRunner, HardwareProbeResult, ProbeError, RecommendedBackend,
};

/// A deterministic [`CommandRunner`] with one slot per probe. Unset slots
/// behave as "command not installed" (`ProbeError::Unavailable`).
#[derive(Default)]
struct MockRunner {
    nvidia: Option<HardwareProbeResult<String>>,
    rocm: Option<HardwareProbeResult<String>>,
    vulkan: Option<HardwareProbeResult<String>>,
    system_profiler: Option<HardwareProbeResult<String>>,
    cpuinfo: Option<HardwareProbeResult<String>>,
    meminfo: Option<HardwareProbeResult<String>>,
    sysctl_brand: Option<HardwareProbeResult<String>>,
    sysctl_physical: Option<HardwareProbeResult<String>>,
    sysctl_logical: Option<HardwareProbeResult<String>>,
    sysctl_memsize: Option<HardwareProbeResult<String>>,
}

fn unavailable() -> HardwareProbeResult<String> {
    Err(ProbeError::Unavailable("not mocked".to_string()))
}

impl CommandRunner for MockRunner {
    async fn run(
        &self,
        program: &str,
        args: &[&str],
        _timeout: Duration,
    ) -> HardwareProbeResult<String> {
        let slot = match program {
            "nvidia-smi" => &self.nvidia,
            "rocm-smi" => &self.rocm,
            "vulkaninfo" => &self.vulkan,
            "system_profiler" => &self.system_profiler,
            "sysctl" => {
                if args.iter().any(|a| a.contains("brand_string")) {
                    &self.sysctl_brand
                } else if args.iter().any(|a| a.contains("hw.physicalcpu")) {
                    &self.sysctl_physical
                } else if args.iter().any(|a| a.contains("hw.logicalcpu")) {
                    &self.sysctl_logical
                } else if args.iter().any(|a| a.contains("hw.memsize")) {
                    &self.sysctl_memsize
                } else {
                    &None
                }
            }
            _ => &None,
        };
        slot.clone().unwrap_or_else(unavailable)
    }

    async fn read_file(&self, path: &str, _timeout: Duration) -> HardwareProbeResult<String> {
        let slot = match path {
            "/proc/cpuinfo" => &self.cpuinfo,
            "/proc/meminfo" => &self.meminfo,
            _ => &None,
        };
        slot.clone().unwrap_or_else(unavailable)
    }
}

const LINUX_CPUINFO: &str = "\
processor\t: 0
model name\t: AMD Ryzen 9 7950X 16-Core Processor
physical id\t: 0
cpu cores\t: 16
processor\t: 1
model name\t: AMD Ryzen 9 7950X 16-Core Processor
physical id\t: 0
cpu cores\t: 16
";

const LINUX_MEMINFO: &str = "\
MemTotal:       65808392 kB
MemFree:         1234567 kB
";

fn linux_cpu_and_ram() -> MockRunner {
    MockRunner {
        cpuinfo: Some(Ok(LINUX_CPUINFO.to_string())),
        meminfo: Some(Ok(LINUX_MEMINFO.to_string())),
        ..Default::default()
    }
}

// --- 1.12: nvidia-smi parsing ---------------------------------------------

#[tokio::test]
async fn nvidia_single_gpu_parses_and_recommends_cuda() {
    let runner = MockRunner {
        nvidia: Some(Ok("NVIDIA GeForce RTX 4090, 24564 MiB, 535.154.05\n".to_string())),
        ..linux_cpu_and_ram()
    };
    let info = detect_hardware_with(&runner).await;

    assert_eq!(info.gpus.len(), 1);
    assert_eq!(info.gpus[0].index, 0);
    assert_eq!(info.gpus[0].name, "NVIDIA GeForce RTX 4090");
    assert_eq!(info.gpus[0].vram_mb, 24564);
    assert_eq!(info.gpus[0].driver_version.as_deref(), Some("535.154.05"));
    assert_eq!(info.recommended_backend, RecommendedBackend::Cuda);
    assert!(info.warnings.is_empty());
}

#[tokio::test]
async fn nvidia_multi_gpu_parses_all_with_contiguous_indices() {
    let runner = MockRunner {
        nvidia: Some(Ok("\
NVIDIA GeForce RTX 4090, 24564 MiB, 535.154.05
NVIDIA GeForce RTX 4090, 24564 MiB, 535.154.05
NVIDIA A100-SXM4-80GB, 81920 MiB, 535.154.05
"
        .to_string())),
        ..linux_cpu_and_ram()
    };
    let info = detect_hardware_with(&runner).await;

    assert_eq!(info.gpus.len(), 3);
    assert_eq!(info.gpus[2].index, 2);
    assert_eq!(info.gpus[2].name, "NVIDIA A100-SXM4-80GB");
    assert_eq!(info.gpus[2].vram_mb, 81920);
    assert_eq!(info.recommended_backend, RecommendedBackend::Cuda);
}

// --- 1.13: system_profiler parsing ----------------------------------------

#[tokio::test]
async fn system_profiler_apple_silicon_unified_memory() {
    let json = r#"{
        "SPDisplaysDataType": [
            { "_name": "Apple M2 Max", "sppci_model": "Apple M2 Max" }
        ]
    }"#;
    let runner = MockRunner {
        system_profiler: Some(Ok(json.to_string())),
        sysctl_brand: Some(Ok("Apple M2 Max\n".to_string())),
        sysctl_physical: Some(Ok("12\n".to_string())),
        sysctl_logical: Some(Ok("12\n".to_string())),
        sysctl_memsize: Some(Ok("68719476736\n".to_string())),
        ..Default::default()
    };
    let info = detect_hardware_with(&runner).await;

    assert_eq!(info.gpus.len(), 1);
    assert_eq!(info.gpus[0].name, "Apple M2 Max");
    assert_eq!(info.cpu.name, "Apple M2 Max");
    assert_eq!(info.cpu.cores_logical, 12);
    assert_eq!(info.ram_mb, 65536);
    assert_eq!(info.recommended_backend, RecommendedBackend::Metal);
}

#[tokio::test]
async fn system_profiler_intel_discrete_vram() {
    let json = r#"{
        "SPDisplaysDataType": [
            { "sppci_model": "AMD Radeon Pro 5500M", "spdisplays_vram": "8 GB" }
        ]
    }"#;
    let runner = MockRunner {
        system_profiler: Some(Ok(json.to_string())),
        sysctl_brand: Some(Ok("Intel(R) Core(TM) i9\n".to_string())),
        sysctl_logical: Some(Ok("16\n".to_string())),
        sysctl_physical: Some(Ok("8\n".to_string())),
        ..Default::default()
    };
    let info = detect_hardware_with(&runner).await;

    assert_eq!(info.gpus[0].vram_mb, 8192);
    assert_eq!(info.recommended_backend, RecommendedBackend::Metal);
}

// --- 1.14: rocm-smi parsing -----------------------------------------------

#[tokio::test]
async fn rocm_smi_parses_and_recommends_vulkan() {
    // ROCm CSV: header row then per-card rows. VRAM total reported in bytes.
    let csv = "\
device,Card series,VRAM Total Memory (B)
card0,Instinct MI210,68719476736
";
    let runner = MockRunner { rocm: Some(Ok(csv.to_string())), ..linux_cpu_and_ram() };
    let info = detect_hardware_with(&runner).await;

    assert_eq!(info.gpus.len(), 1);
    assert_eq!(info.gpus[0].name, "Instinct MI210");
    assert_eq!(info.gpus[0].vram_mb, 65536);
    assert_eq!(info.recommended_backend, RecommendedBackend::Vulkan);
}

// --- 1.15: partial failure (one probe times out) --------------------------

#[tokio::test]
async fn nvidia_timeout_yields_warning_and_partial_results() {
    let runner = MockRunner { nvidia: Some(Err(ProbeError::Timeout)), ..linux_cpu_and_ram() };
    let info = detect_hardware_with(&runner).await;

    // CPU/RAM still detected despite the GPU probe timing out.
    assert!(info.cpu.name.contains("Ryzen"));
    assert_eq!(info.ram_mb, 64266);
    assert!(info.gpus.is_empty());
    assert_eq!(info.recommended_backend, RecommendedBackend::Cpu);
    let warning = info
        .warnings
        .iter()
        .find(|w| w.probe == "nvidia-smi")
        .expect("expected a timeout warning for nvidia-smi");
    assert_eq!(warning.message, "detection timed out");
}

// --- 1.16: all probes fail (CPU-only fallback) ----------------------------

#[tokio::test]
async fn all_probes_unavailable_falls_back_to_cpu() {
    let runner = MockRunner::default();
    let info = detect_hardware_with(&runner).await;

    assert!(info.gpus.is_empty());
    assert_eq!(info.cpu.name, "Unknown CPU");
    assert_eq!(info.ram_mb, 0);
    assert_eq!(info.recommended_backend, RecommendedBackend::Cpu);
    // Unavailable (not installed) is "not present", not a warning.
    assert!(info.warnings.is_empty());
}

// --- 1.17: gibberish output (no phantom 0-VRAM GPU) -----------------------

#[tokio::test]
async fn gibberish_nvidia_output_yields_no_gpu() {
    let runner = MockRunner {
        nvidia: Some(Ok("this is not valid csv output at all\n".to_string())),
        ..linux_cpu_and_ram()
    };
    let info = detect_hardware_with(&runner).await;

    // The unparseable line must not become a GpuInfo { vram_mb: 0 }.
    assert!(info.gpus.is_empty());
    assert_eq!(info.recommended_backend, RecommendedBackend::Cpu);
}
