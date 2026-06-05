//! Hardware detection for the Setup Wizard.
//!
//! Discovers GPUs, CPU, and RAM by shelling out to platform probes
//! (`nvidia-smi`, `rocm-smi`, `vulkaninfo`, `system_profiler`, `sysctl`)
//! and reading `/proc`. Implements `08_SETUP_WIZARD.md` Section 11.1
//! (Hardware Detection API contract) and the Appendix implementation notes.
//!
//! This module is **pure domain logic**: it owns no HTTP/axum types and emits
//! no `tracing` events. The setup API handler (PR5) wraps [`detect_hardware`]
//! and is responsible for the response envelope (`"version": 1`), the
//! `?diagnostics=true` projection, and span instrumentation.
//!
//! Design contract highlights:
//! - Each probe has an independent timeout (nvidia-smi 10 s, rocm-smi 10 s,
//!   vulkaninfo 10 s, system_profiler 15 s, `/proc` 5 s, sysctl 5 s).
//! - Probes run concurrently and the result is **partial on failure**: a
//!   timed-out probe contributes a [`HardwareWarning`] with the message
//!   `"detection timed out"` rather than a `null`/zeroed field, so the caller
//!   can distinguish "not present" from "present but unresponsive".
//! - The whole detection is bounded by a 30 s hard timeout backstop.

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::process::Command;

/// Per-probe timeouts and the overall detection budget (Section 11.1).
const NVIDIA_TIMEOUT: Duration = Duration::from_secs(10);
const ROCM_TIMEOUT: Duration = Duration::from_secs(10);
const VULKAN_TIMEOUT: Duration = Duration::from_secs(10);
const SYSTEM_PROFILER_TIMEOUT: Duration = Duration::from_secs(15);
const PROC_TIMEOUT: Duration = Duration::from_secs(5);
const SYSCTL_TIMEOUT: Duration = Duration::from_secs(5);
const OVERALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The reported message for any probe that exceeded its timeout. The frontend
/// keys off this exact string to render "present but unresponsive".
const TIMED_OUT_MESSAGE: &str = "detection timed out";

// ---------------------------------------------------------------------------
// Response types (serialized by the PR5 handler).
// ---------------------------------------------------------------------------

/// Inference backend recommended for this machine, in priority order
/// `cuda > metal > vulkan > cpu`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RecommendedBackend {
    Cuda,
    Metal,
    Vulkan,
    Cpu,
}

/// A single detected GPU.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GpuInfo {
    /// Sequential index within the merged GPU list.
    pub index: usize,
    pub name: String,
    pub vram_mb: u64,
    /// Driver version, when a probe exposes it. Surfaced only under
    /// `?diagnostics=true` by the handler; always collected here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver_version: Option<String>,
}

/// CPU summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CpuInfo {
    pub name: String,
    pub cores_physical: u32,
    pub cores_logical: u32,
}

impl CpuInfo {
    /// CPU placeholder used when no probe yielded usable data.
    fn unknown() -> Self {
        Self { name: "Unknown CPU".to_string(), cores_physical: 0, cores_logical: 0 }
    }
}

/// A non-fatal detection problem (a timed-out or unparseable probe). Collected
/// into [`HardwareInfo::warnings`] so partial results remain usable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HardwareWarning {
    /// Probe identifier, e.g. `"nvidia-smi"` or `"cpu"`.
    pub probe: String,
    pub message: String,
}

impl HardwareWarning {
    fn timed_out(probe: &str) -> Self {
        Self { probe: probe.to_string(), message: TIMED_OUT_MESSAGE.to_string() }
    }
}

/// Aggregate hardware detection result (Section 11.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HardwareInfo {
    pub gpus: Vec<GpuInfo>,
    pub cpu: CpuInfo,
    pub ram_mb: u64,
    pub recommended_backend: RecommendedBackend,
    /// One entry per probe that timed out. Omitted from JSON when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<HardwareWarning>,
}

// ---------------------------------------------------------------------------
// Probe execution port.
// ---------------------------------------------------------------------------

/// Error from a single probe invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// The probe exceeded its timeout and was killed (`SIGKILL` via
    /// `kill_on_drop`).
    Timeout,
    /// The command could not be spawned (not installed, not executable). This
    /// is treated as "not present", not a warning.
    Unavailable(String),
    /// The command ran but exited non-zero.
    Failed { code: Option<i32>, stderr: String },
}

/// Outcome of a probe that yields parsed data `T`.
pub type HardwareProbeResult<T> = Result<T, ProbeError>;

/// Port abstracting subprocess execution and file reads, so the detection
/// orchestrator can be driven by a mock in tests (timeouts, failures, fixture
/// output) without touching the host. The real adapter is
/// [`SystemCommandRunner`].
pub trait CommandRunner: Send + Sync {
    /// Run `program args...`, returning captured stdout on success. Must
    /// enforce `timeout` and kill the child if it fires.
    fn run(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> impl std::future::Future<Output = HardwareProbeResult<String>> + Send;

    /// Read a file (e.g. `/proc/cpuinfo`) to a string, bounded by `timeout`.
    fn read_file(
        &self,
        path: &str,
        timeout: Duration,
    ) -> impl std::future::Future<Output = HardwareProbeResult<String>> + Send;
}

/// Real adapter: spawns subprocesses via Tokio with `kill_on_drop` so a
/// timed-out probe is `SIGKILL`ed when its future is dropped.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    async fn run(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> HardwareProbeResult<String> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let child = command.spawn().map_err(|e| ProbeError::Unavailable(e.to_string()))?;

        // On elapse the output future is dropped, dropping the child, which
        // triggers kill_on_drop → SIGKILL.
        let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => return Err(ProbeError::Unavailable(e.to_string())),
            Err(_elapsed) => return Err(ProbeError::Timeout),
        };

        if !output.status.success() {
            return Err(ProbeError::Failed {
                code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    async fn read_file(&self, path: &str, timeout: Duration) -> HardwareProbeResult<String> {
        match tokio::time::timeout(timeout, tokio::fs::read_to_string(path)).await {
            Ok(Ok(contents)) => Ok(contents),
            Ok(Err(e)) => Err(ProbeError::Unavailable(e.to_string())),
            Err(_elapsed) => Err(ProbeError::Timeout),
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry points.
// ---------------------------------------------------------------------------

/// Detect hardware on the host using real subprocess probes.
///
/// Returns partial results on probe failure. The whole detection is bounded by
/// a 30 s hard ceiling ([`OVERALL_TIMEOUT`]): each probe's effective timeout is
/// the smaller of its own budget and the time remaining until the ceiling, so a
/// pathological hang cannot push detection past 30 s — and every probe that
/// *did* complete before the ceiling is preserved (Section 11.1).
pub async fn detect_hardware() -> HardwareInfo {
    detect_hardware_with(&SystemCommandRunner).await
}

/// Detection orchestrator, generic over the probe [`CommandRunner`] so tests
/// can inject deterministic outputs. Probes run concurrently, each bounded by
/// [`capped`] against a shared 30 s deadline; the join resolves by the deadline
/// with whatever probes finished, and any that hit the ceiling become timeout
/// warnings rather than discarding the collected results.
pub async fn detect_hardware_with<R: CommandRunner>(runner: &R) -> HardwareInfo {
    let deadline = Instant::now() + OVERALL_TIMEOUT;
    collect_hardware(runner, deadline).await
}

/// Effective per-probe timeout: the smaller of the probe's own budget and the
/// time left until the overall deadline. Saturates to zero once the deadline
/// has passed, turning a still-running probe into a [`ProbeError::Timeout`].
fn capped(deadline: Instant, probe_timeout: Duration) -> Duration {
    probe_timeout.min(deadline.saturating_duration_since(Instant::now()))
}

async fn collect_hardware<R: CommandRunner>(runner: &R, deadline: Instant) -> HardwareInfo {
    // All probes attempted on every host; commands absent on this platform
    // simply fail to spawn and contribute nothing (not a warning).
    let (nvidia, amd, vulkan, metal, cpu, ram) = tokio::join!(
        detect_nvidia(runner, deadline),
        detect_amd(runner, deadline),
        detect_vulkan(runner, deadline),
        detect_metal(runner, deadline),
        detect_cpu(runner, deadline),
        detect_ram(runner, deadline),
    );

    let mut warnings = Vec::new();
    let mut gpus = Vec::new();
    let has_nvidia = collect_category(nvidia, "nvidia-smi", &mut gpus, &mut warnings);
    let has_metal = collect_category(metal, "system_profiler", &mut gpus, &mut warnings);
    let has_amd = collect_category(amd, "rocm-smi", &mut gpus, &mut warnings);
    let has_vulkan = collect_category(vulkan, "vulkaninfo", &mut gpus, &mut warnings);

    // Re-index the merged list so `index` is contiguous 0..n.
    for (position, gpu) in gpus.iter_mut().enumerate() {
        gpu.index = position;
    }

    let (cpu, cpu_warning) = cpu;
    if let Some(w) = cpu_warning {
        warnings.push(w);
    }
    let (ram_mb, ram_warning) = ram;
    if let Some(w) = ram_warning {
        warnings.push(w);
    }

    let recommended_backend = if has_nvidia {
        RecommendedBackend::Cuda
    } else if has_metal {
        RecommendedBackend::Metal
    } else if has_amd || has_vulkan {
        RecommendedBackend::Vulkan
    } else {
        RecommendedBackend::Cpu
    };

    HardwareInfo { gpus, cpu, ram_mb, recommended_backend, warnings }
}

/// Fold one GPU probe outcome into the merged list, returning whether any GPU
/// of this category was found and recording a timeout warning if applicable.
fn collect_category(
    outcome: HardwareProbeResult<Vec<GpuInfo>>,
    probe: &str,
    gpus: &mut Vec<GpuInfo>,
    warnings: &mut Vec<HardwareWarning>,
) -> bool {
    match outcome {
        Ok(found) if !found.is_empty() => {
            gpus.extend(found);
            true
        }
        Ok(_) => false,
        Err(ProbeError::Timeout) => {
            warnings.push(HardwareWarning::timed_out(probe));
            false
        }
        // Unavailable (not installed) or non-zero exit → "not present".
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// GPU probes.
// ---------------------------------------------------------------------------

async fn detect_nvidia<R: CommandRunner>(
    runner: &R,
    deadline: Instant,
) -> HardwareProbeResult<Vec<GpuInfo>> {
    let out = runner
        .run(
            "nvidia-smi",
            &["--query-gpu=name,memory.total,driver_version", "--format=csv,noheader"],
            capped(deadline, NVIDIA_TIMEOUT),
        )
        .await?;
    Ok(parse_nvidia_smi(&out))
}

async fn detect_amd<R: CommandRunner>(
    runner: &R,
    deadline: Instant,
) -> HardwareProbeResult<Vec<GpuInfo>> {
    let out = runner
        .run(
            "rocm-smi",
            &["--showproductname", "--showmeminfo", "vram", "--csv"],
            capped(deadline, ROCM_TIMEOUT),
        )
        .await?;
    Ok(parse_rocm_smi(&out))
}

async fn detect_vulkan<R: CommandRunner>(
    runner: &R,
    deadline: Instant,
) -> HardwareProbeResult<Vec<GpuInfo>> {
    let out = runner.run("vulkaninfo", &["--summary"], capped(deadline, VULKAN_TIMEOUT)).await?;
    Ok(parse_vulkaninfo(&out))
}

async fn detect_metal<R: CommandRunner>(
    runner: &R,
    deadline: Instant,
) -> HardwareProbeResult<Vec<GpuInfo>> {
    let out = runner
        .run(
            "system_profiler",
            &["SPDisplaysDataType", "-json"],
            capped(deadline, SYSTEM_PROFILER_TIMEOUT),
        )
        .await?;
    Ok(parse_system_profiler(&out))
}

// ---------------------------------------------------------------------------
// CPU / RAM probes (try Linux `/proc` + sysctl, prefer whichever yields data).
// ---------------------------------------------------------------------------

async fn detect_cpu<R: CommandRunner>(
    runner: &R,
    deadline: Instant,
) -> (CpuInfo, Option<HardwareWarning>) {
    // Linux first.
    match runner.read_file("/proc/cpuinfo", capped(deadline, PROC_TIMEOUT)).await {
        Ok(text) => {
            if let Some(cpu) = parse_proc_cpuinfo(&text) {
                return (cpu, None);
            }
        }
        Err(ProbeError::Timeout) => {
            return (CpuInfo::unknown(), Some(HardwareWarning::timed_out("cpu")));
        }
        Err(_) => {}
    }

    // macOS sysctl fallback.
    let brand = runner
        .run("sysctl", &["-n", "machdep.cpu.brand_string"], capped(deadline, SYSCTL_TIMEOUT))
        .await;
    if let Err(ProbeError::Timeout) = brand {
        return (CpuInfo::unknown(), Some(HardwareWarning::timed_out("cpu")));
    }
    let physical =
        runner.run("sysctl", &["-n", "hw.physicalcpu"], capped(deadline, SYSCTL_TIMEOUT)).await;
    let logical =
        runner.run("sysctl", &["-n", "hw.logicalcpu"], capped(deadline, SYSCTL_TIMEOUT)).await;
    if let Ok(name) = brand {
        return (parse_sysctl_cpu(&name, physical.ok().as_deref(), logical.ok().as_deref()), None);
    }

    (CpuInfo::unknown(), None)
}

async fn detect_ram<R: CommandRunner>(
    runner: &R,
    deadline: Instant,
) -> (u64, Option<HardwareWarning>) {
    match runner.read_file("/proc/meminfo", capped(deadline, PROC_TIMEOUT)).await {
        Ok(text) => {
            if let Some(mb) = parse_proc_meminfo(&text) {
                return (mb, None);
            }
        }
        Err(ProbeError::Timeout) => {
            return (0, Some(HardwareWarning::timed_out("ram")));
        }
        Err(_) => {}
    }

    match runner.run("sysctl", &["-n", "hw.memsize"], capped(deadline, SYSCTL_TIMEOUT)).await {
        Ok(text) => (parse_sysctl_memsize(&text).unwrap_or(0), None),
        Err(ProbeError::Timeout) => (0, Some(HardwareWarning::timed_out("ram"))),
        Err(_) => (0, None),
    }
}

// ---------------------------------------------------------------------------
// Pure parsers (unit-tested directly against fixture output).
// ---------------------------------------------------------------------------

/// Parse `nvidia-smi --query-gpu=name,memory.total,driver_version
/// --format=csv,noheader`. Lines whose VRAM does not parse are skipped, so
/// gibberish yields no phantom 0-VRAM GPU.
pub(crate) fn parse_nvidia_smi(output: &str) -> Vec<GpuInfo> {
    output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .filter_map(|(index, line)| {
            let mut fields = line.split(',').map(str::trim);
            let name = fields.next().filter(|s| !s.is_empty())?.to_string();
            let vram_mb = parse_mib(fields.next()?)?;
            let driver_version = fields.next().map(str::to_string).filter(|s| !s.is_empty());
            Some(GpuInfo { index, name, vram_mb, driver_version })
        })
        .collect()
}

/// Parse a `"<number> MiB"` (or bare number) VRAM field to MB.
fn parse_mib(field: &str) -> Option<u64> {
    let digits: String = field.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Parse `rocm-smi --showproductname --showmeminfo vram --csv`. ROCm CSV has a
/// header row; columns vary by version, so match by header name.
pub(crate) fn parse_rocm_smi(output: &str) -> Vec<GpuInfo> {
    let mut lines = output.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else {
        return Vec::new();
    };
    let columns: Vec<&str> = header.split(',').map(str::trim).collect();
    let name_col = columns
        .iter()
        .position(|c| c.eq_ignore_ascii_case("Card series") || c.contains("Product Name"));
    let vram_col = columns.iter().position(|c| {
        let lower = c.to_ascii_lowercase();
        lower.contains("vram") && lower.contains("total")
    });

    lines
        .enumerate()
        .filter_map(|(index, line)| {
            let cells: Vec<&str> = line.split(',').map(str::trim).collect();
            let name = name_col
                .and_then(|i| cells.get(i))
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "AMD GPU".to_string());
            // ROCm reports VRAM total in bytes.
            let vram_bytes: u64 = vram_col.and_then(|i| cells.get(i))?.parse().ok()?;
            Some(GpuInfo { index, name, vram_mb: vram_bytes / (1024 * 1024), driver_version: None })
        })
        .collect()
}

/// Parse `vulkaninfo --summary`. Extracts `deviceName` lines; VRAM is not
/// reliably present in the summary, so it defaults to 0 and the recommendation
/// engine treats Vulkan VRAM conservatively.
pub(crate) fn parse_vulkaninfo(output: &str) -> Vec<GpuInfo> {
    output
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            if key.trim().eq_ignore_ascii_case("devicename") {
                let name = value.trim();
                if name.is_empty() {
                    return None;
                }
                Some(name.to_string())
            } else {
                None
            }
        })
        .enumerate()
        .map(|(index, name)| GpuInfo { index, name, vram_mb: 0, driver_version: None })
        .collect()
}

/// Parse `system_profiler SPDisplaysDataType -json`. Handles Apple Silicon
/// unified memory (no dedicated VRAM field → 0, treated as unified).
pub(crate) fn parse_system_profiler(output: &str) -> Vec<GpuInfo> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(output) else {
        return Vec::new();
    };
    let Some(displays) = root.get("SPDisplaysDataType").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    displays
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let name = entry
                .get("sppci_model")
                .or_else(|| entry.get("_name"))
                .and_then(|v| v.as_str())?
                .to_string();
            let vram_mb = entry
                .get("spdisplays_vram")
                .or_else(|| entry.get("spdisplays_vram_shared"))
                .and_then(|v| v.as_str())
                .and_then(parse_apple_vram)
                .unwrap_or(0);
            Some(GpuInfo { index, name, vram_mb, driver_version: None })
        })
        .collect()
}

/// Parse Apple VRAM strings like `"8 GB"` or `"1536 MB"` to MB.
fn parse_apple_vram(value: &str) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let amount: u64 = parts.next()?.parse().ok()?;
    match parts.next().map(|u| u.to_ascii_uppercase()) {
        Some(ref u) if u == "GB" => Some(amount * 1024),
        Some(ref u) if u == "MB" => Some(amount),
        _ => Some(amount),
    }
}

/// Parse `/proc/cpuinfo` for model name and physical/logical core counts.
pub(crate) fn parse_proc_cpuinfo(output: &str) -> Option<CpuInfo> {
    let mut name = None;
    let mut logical = 0u32;
    let mut cores_per_socket = 0u32;
    let mut sockets = std::collections::HashSet::new();

    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "model name" => {
                logical += 1;
                if name.is_none() {
                    name = Some(value.to_string());
                }
            }
            "cpu cores" => {
                if let Ok(n) = value.parse::<u32>() {
                    cores_per_socket = cores_per_socket.max(n);
                }
            }
            "physical id" => {
                sockets.insert(value.to_string());
            }
            _ => {}
        }
    }

    let name = name?;
    let socket_count = sockets.len().max(1) as u32;
    let physical = if cores_per_socket > 0 { cores_per_socket * socket_count } else { logical };
    Some(CpuInfo { name, cores_physical: physical.max(1), cores_logical: logical.max(1) })
}

/// Build a [`CpuInfo`] from macOS `sysctl -n` outputs.
pub(crate) fn parse_sysctl_cpu(
    brand: &str,
    physical: Option<&str>,
    logical: Option<&str>,
) -> CpuInfo {
    let logical = logical.and_then(|s| s.trim().parse().ok()).unwrap_or(1);
    let physical = physical.and_then(|s| s.trim().parse().ok()).unwrap_or(logical);
    CpuInfo { name: brand.trim().to_string(), cores_physical: physical, cores_logical: logical }
}

/// Parse `MemTotal` (kB) from `/proc/meminfo`, returning MB.
pub(crate) fn parse_proc_meminfo(output: &str) -> Option<u64> {
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.split_whitespace().next().and_then(|s| s.parse().ok())?;
            return Some(kb / 1024);
        }
    }
    None
}

/// Parse macOS `sysctl -n hw.memsize` (bytes), returning MB.
pub(crate) fn parse_sysctl_memsize(output: &str) -> Option<u64> {
    let bytes: u64 = output.trim().parse().ok()?;
    Some(bytes / (1024 * 1024))
}

#[cfg(test)]
mod deadline_tests {
    //! In-module tests for the overall-deadline mechanism, which exercises the
    //! private `collect_hardware`/`capped` (the public mock-driven cases live in
    //! `tests/hardware_tests.rs`).
    use super::*;

    /// Returns canned probe output regardless of the (deadline-derived) timeout.
    struct CannedRunner;

    impl CommandRunner for CannedRunner {
        async fn run(
            &self,
            program: &str,
            _args: &[&str],
            _timeout: Duration,
        ) -> HardwareProbeResult<String> {
            match program {
                "nvidia-smi" => Ok("NVIDIA GeForce RTX 4090, 24564 MiB, 535.0\n".to_string()),
                _ => Err(ProbeError::Unavailable("not mocked".to_string())),
            }
        }

        async fn read_file(&self, path: &str, _timeout: Duration) -> HardwareProbeResult<String> {
            match path {
                "/proc/cpuinfo" => Ok("model name\t: Test CPU\ncpu cores\t: 8\n".to_string()),
                "/proc/meminfo" => Ok("MemTotal:       16384000 kB\n".to_string()),
                _ => Err(ProbeError::Unavailable("not mocked".to_string())),
            }
        }
    }

    #[test]
    fn capped_saturates_to_zero_after_the_deadline() {
        let past = Instant::now() - Duration::from_secs(1);
        assert_eq!(capped(past, Duration::from_secs(10)), Duration::ZERO);

        let future = Instant::now() + Duration::from_secs(60);
        assert_eq!(capped(future, Duration::from_secs(10)), Duration::from_secs(10));
    }

    #[tokio::test]
    async fn completed_probes_survive_an_already_expired_deadline() {
        // Regression for the old 30s backstop, which discarded all collected
        // results and returned a degenerate CPU-only summary. With the deadline
        // already in the past, completed probes must still be reported.
        let expired = Instant::now() - Duration::from_secs(1);
        let info = collect_hardware(&CannedRunner, expired).await;

        assert_eq!(info.gpus.len(), 1);
        assert_eq!(info.recommended_backend, RecommendedBackend::Cuda);
        assert!(info.cpu.name.contains("Test CPU"));
    }
}

#[cfg(test)]
mod parser_tests {
    //! Direct unit tests for the `pub(crate)` parsers, which the integration
    //! test crate cannot reach. Covers the otherwise indirectly-tested paths
    //! (notably `parse_vulkaninfo`, whose VRAM is intentionally 0 — `--summary`
    //! does not reliably expose it, and Section 11.1 does not require it).
    use super::*;

    #[test]
    fn nvidia_skips_unparseable_lines() {
        assert!(parse_nvidia_smi("garbage with no vram column").is_empty());
        let gpus = parse_nvidia_smi("NVIDIA RTX 4090, 24564 MiB, 535.0");
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].vram_mb, 24564);
    }

    #[test]
    fn rocm_reads_vram_bytes_from_header() {
        let csv = "device,Card series,VRAM Total Memory (B)\ncard0,Instinct MI210,68719476736\n";
        let gpus = parse_rocm_smi(csv);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "Instinct MI210");
        assert_eq!(gpus[0].vram_mb, 65536);
    }

    #[test]
    fn vulkaninfo_extracts_device_names_with_zero_vram() {
        let summary = "GPU0:\n\tapiVersion = 1.3\n\tdeviceName = NVIDIA GeForce RTX 4090\n";
        let gpus = parse_vulkaninfo(summary);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 4090");
        assert_eq!(gpus[0].vram_mb, 0);
    }

    #[test]
    fn system_profiler_parses_apple_unified_memory() {
        let json = r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M2 Max"}]}"#;
        let gpus = parse_system_profiler(json);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "Apple M2 Max");
        assert_eq!(gpus[0].vram_mb, 0);
    }

    #[test]
    fn proc_cpuinfo_counts_cores() {
        let text = "model name\t: Test CPU\nphysical id\t: 0\ncpu cores\t: 8\n\
            model name\t: Test CPU\nphysical id\t: 0\ncpu cores\t: 8\n";
        let cpu = parse_proc_cpuinfo(text).expect("cpu parsed");
        assert_eq!(cpu.name, "Test CPU");
        assert_eq!(cpu.cores_physical, 8);
        assert_eq!(cpu.cores_logical, 2);
    }

    #[test]
    fn meminfo_and_memsize_convert_to_mb() {
        assert_eq!(parse_proc_meminfo("MemTotal:       16384000 kB\n"), Some(16000));
        assert_eq!(parse_sysctl_memsize("17179869184\n"), Some(16384));
    }
}
