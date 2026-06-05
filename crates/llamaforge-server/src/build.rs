//! llama.cpp build orchestrator (domain logic).
//!
//! Clones llama.cpp and runs `cmake` configure + build as child processes,
//! streaming output as [`BuildEvent`]s. Implements `08_SETUP_WIZARD.md`
//! Section 11.4 (Build API) and the Appendix implementation notes. The API
//! adapter (PR6 adapter layer) owns the job registry, SSE fan-out, and
//! reconnection; this module owns the actual build pipeline.
//!
//! Each phase runs in its **own process group** (`Command::process_group(0)`)
//! with a per-phase timeout; on timeout or cancellation the whole group is
//! signalled `SIGTERM`, then `SIGKILL` after a grace period, so cmake/make
//! children are reaped too. Pure of axum/HTTP; no `tracing` in the hot path.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// llama.cpp upstream repository.
const LLAMA_CPP_REPO: &str = "https://github.com/ggerganov/llama.cpp.git";
/// The built executable's name.
pub const BINARY_NAME: &str = "llama-server";
/// Minimum free disk required before a build starts.
pub const MIN_DISK_GB: u64 = 5;

/// Per-phase timeouts (Section 11.4), injectable so the timeout path is
/// testable without a multi-minute wait.
#[derive(Debug, Clone, Copy)]
pub struct PhaseTimeouts {
    pub clone: Duration,
    pub configure: Duration,
    pub build: Duration,
}

impl Default for PhaseTimeouts {
    fn default() -> Self {
        Self {
            clone: Duration::from_secs(5 * 60),
            configure: Duration::from_secs(5 * 60),
            build: Duration::from_secs(20 * 60),
        }
    }
}

impl PhaseTimeouts {
    fn for_phase(&self, phase: BuildPhase) -> Duration {
        match phase {
            BuildPhase::Cloning => self.clone,
            BuildPhase::Configuring => self.configure,
            BuildPhase::Building => self.build,
        }
    }
}
/// Grace between `SIGTERM` and `SIGKILL` for a process group.
const KILL_GRACE: Duration = Duration::from_secs(5);
/// Number of trailing log lines retained for `failed`/status reporting.
const TAIL_LINES: usize = 20;

// ---------------------------------------------------------------------------
// Types.
// ---------------------------------------------------------------------------

/// Build backend selection (Section 11.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildBackend {
    Cuda,
    Metal,
    Vulkan,
    Cpu,
}

impl BuildBackend {
    /// The `cmake` feature flag that selects this backend.
    fn cmake_flag(self) -> &'static str {
        match self {
            BuildBackend::Cuda => "-DGGML_CUDA=ON",
            BuildBackend::Metal => "-DGGML_METAL=ON",
            BuildBackend::Vulkan => "-DGGML_VULKAN=ON",
            BuildBackend::Cpu => "-DGGML_CPU_ONLY=ON",
        }
    }
}

/// Build phase (Section 11.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildPhase {
    Cloning,
    Configuring,
    Building,
}

impl BuildPhase {
    fn label(self) -> &'static str {
        match self {
            BuildPhase::Cloning => "cloning",
            BuildPhase::Configuring => "configuring",
            BuildPhase::Building => "building",
        }
    }
}

/// A build request (Section 11.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildRequest {
    pub backend: BuildBackend,
    pub install_dir: String,
    #[serde(default)]
    pub jobs: usize,
}

/// Terminal success payload (the SSE `complete` event data).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildComplete {
    pub binary_path: String,
    pub duration_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    pub replaced_existing: bool,
    pub backup_path: Option<String>,
}

/// Terminal failure payload (the SSE `failed` event data).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildFailed {
    pub phase: BuildPhase,
    pub error: String,
    pub last_lines: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
}

/// A streamed build event. The API adapter maps each variant to an SSE event
/// name (`phase`/`progress`/`log`/`complete`/`failed`) and serializes the data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "lowercase")]
pub enum BuildEvent {
    Phase { phase: BuildPhase, message: String },
    Progress { phase: BuildPhase, percent: u8 },
    Log { lines: Vec<String> },
    Complete(BuildComplete),
    Failed(BuildFailed),
}

/// Programs used by the build, injectable so tests can supply mock git/cmake
/// shell scripts without a real toolchain. `nice_level` lowers the build's
/// scheduling priority when set (Section 11.4 resource controls).
#[derive(Debug, Clone)]
pub struct BuildCommands {
    pub git: PathBuf,
    pub cmake: PathBuf,
    pub nice_level: Option<i32>,
}

impl Default for BuildCommands {
    fn default() -> Self {
        Self { git: PathBuf::from("git"), cmake: PathBuf::from("cmake"), nice_level: Some(10) }
    }
}

/// Final result of a build run (distinct from the streamed terminal event so
/// the caller can act on the outcome after the stream closes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildOutcome {
    Succeeded(BuildComplete),
    Failed(BuildFailed),
    Cancelled,
}

// ---------------------------------------------------------------------------
// Resource controls.
// ---------------------------------------------------------------------------

/// Effective parallelism: `min(requested, num_cpus / 2)`, floored at 1, so a
/// build never saturates every core (Section 11.4).
pub fn effective_jobs(requested: usize) -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let cap = (cores / 2).max(1);
    requested.clamp(1, cap)
}

/// Free space (whole GB, truncated) at `path`'s filesystem, or `None` if the
/// query fails.
pub fn available_disk_gb(path: &Path) -> Option<u64> {
    let stat = nix::sys::statvfs::statvfs(path).ok()?;
    let free_bytes = (stat.blocks_available() as u64).saturating_mul(stat.fragment_size() as u64);
    Some(free_bytes / 1_000_000_000)
}

// ---------------------------------------------------------------------------
// Orchestration.
// ---------------------------------------------------------------------------

/// Run the full build pipeline (clone → configure → build → install) in
/// `work_dir`, emitting events to `events` and honouring `cancel`. Returns the
/// terminal [`BuildOutcome`]; the matching terminal `BuildEvent` is also sent.
pub async fn run_build(
    request: &BuildRequest,
    work_dir: &Path,
    commands: &BuildCommands,
    timeouts: &PhaseTimeouts,
    events: mpsc::Sender<BuildEvent>,
    cancel: &CancellationToken,
) -> BuildOutcome {
    let started = Instant::now();
    let jobs = effective_jobs(request.jobs);
    let source_dir = work_dir.join("llama.cpp");

    // --- Clone ---
    let clone_msg = format!("Cloning llama.cpp (effective jobs: {jobs})...");
    let outcome = run_phase(
        BuildPhase::Cloning,
        &clone_msg,
        prepare(
            commands,
            &commands.git,
            &["clone", "--depth=1", LLAMA_CPP_REPO, "llama.cpp"],
            work_dir,
        ),
        timeouts.for_phase(BuildPhase::Cloning),
        &events,
        cancel,
    )
    .await;
    if let Some(terminal) = outcome.into_failure_or_cancel() {
        return finish(terminal, &events).await;
    }

    // --- Configure ---
    let outcome = run_phase(
        BuildPhase::Configuring,
        "Running cmake configure...",
        prepare(
            commands,
            &commands.cmake,
            &["-B", "build", request.backend.cmake_flag()],
            &source_dir,
        ),
        timeouts.for_phase(BuildPhase::Configuring),
        &events,
        cancel,
    )
    .await;
    if let Some(terminal) = outcome.into_failure_or_cancel() {
        return finish(terminal, &events).await;
    }

    // --- Build ---
    let jobs_str = jobs.to_string();
    let outcome = run_phase(
        BuildPhase::Building,
        "Compiling llama-server...",
        prepare(
            commands,
            &commands.cmake,
            &["--build", "build", "-j", &jobs_str, "--target", BINARY_NAME],
            &source_dir,
        ),
        timeouts.for_phase(BuildPhase::Building),
        &events,
        cancel,
    )
    .await;
    if let Some(terminal) = outcome.into_failure_or_cancel() {
        return finish(terminal, &events).await;
    }

    // --- Install (copy binary, backing up any existing one) ---
    match install_binary(&source_dir, &request.install_dir).await {
        Ok((replaced_existing, backup_path)) => {
            let complete = BuildComplete {
                binary_path: PathBuf::from(&request.install_dir)
                    .join(BINARY_NAME)
                    .to_string_lossy()
                    .into_owned(),
                duration_secs: started.elapsed().as_secs(),
                log_path: None,
                replaced_existing,
                backup_path,
            };
            let _ = events.send(BuildEvent::Complete(complete.clone())).await;
            BuildOutcome::Succeeded(complete)
        }
        Err(error) => {
            finish(
                Terminal::Failed(BuildFailed {
                    phase: BuildPhase::Building,
                    error,
                    last_lines: Vec::new(),
                    log_path: None,
                }),
                &events,
            )
            .await
        }
    }
}

/// Outcome of a single phase before it is folded into the pipeline.
enum PhaseOutcome {
    Ok,
    Terminal(Terminal),
}

impl PhaseOutcome {
    fn into_failure_or_cancel(self) -> Option<Terminal> {
        match self {
            PhaseOutcome::Ok => None,
            PhaseOutcome::Terminal(t) => Some(t),
        }
    }
}

/// A terminal condition (failure or cancellation) carrying its event.
enum Terminal {
    Failed(BuildFailed),
    Cancelled,
}

async fn finish(terminal: Terminal, events: &mpsc::Sender<BuildEvent>) -> BuildOutcome {
    match terminal {
        Terminal::Failed(failed) => {
            let _ = events.send(BuildEvent::Failed(failed.clone())).await;
            BuildOutcome::Failed(failed)
        }
        Terminal::Cancelled => BuildOutcome::Cancelled,
    }
}

/// Build a [`Command`] for one phase: optionally `nice`-wrapped, in its own
/// process group, with piped stdio and `kill_on_drop`.
fn prepare(commands: &BuildCommands, program: &Path, args: &[&str], cwd: &Path) -> Command {
    let mut command = match commands.nice_level {
        Some(level) => {
            let mut c = Command::new("nice");
            c.arg("-n").arg(level.to_string()).arg(program).args(args);
            c
        }
        None => {
            let mut c = Command::new(program);
            c.args(args);
            c
        }
    };
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    command
}

/// Spawn and drive one phase: emit a `phase` event, stream stdout+stderr as
/// `log` events, parse progress, and enforce the phase timeout / cancellation
/// (killing the whole process group). Returns whether to continue.
async fn run_phase(
    phase: BuildPhase,
    message: &str,
    mut command: Command,
    timeout: Duration,
    events: &mpsc::Sender<BuildEvent>,
    cancel: &CancellationToken,
) -> PhaseOutcome {
    let _ = events.send(BuildEvent::Phase { phase, message: message.to_string() }).await;

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return PhaseOutcome::Terminal(Terminal::Failed(BuildFailed {
                phase,
                error: format!("failed to start {} step: {e}", phase.label()),
                last_lines: Vec::new(),
                log_path: None,
            }));
        }
    };

    let pgid = child.id().map(|pid| pid as i32);
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let mut tail: Vec<String> = Vec::new();

    let stream = stream_output(phase, stdout, stderr, events, &mut tail);
    let deadline = tokio::time::sleep(timeout);

    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            kill_group(&mut child, pgid).await;
            PhaseOutcome::Terminal(Terminal::Cancelled)
        }
        _ = deadline => {
            kill_group(&mut child, pgid).await;
            PhaseOutcome::Terminal(Terminal::Failed(BuildFailed {
                phase,
                error: format!(
                    "Build timed out during {} (exceeded {}m limit)",
                    phase.label(),
                    timeout.as_secs() / 60
                ),
                last_lines: tail,
                log_path: None,
            }))
        }
        status = run_to_completion(&mut child, stream) => {
            match status {
                Ok(true) => PhaseOutcome::Ok,
                Ok(false) => PhaseOutcome::Terminal(Terminal::Failed(BuildFailed {
                    phase,
                    error: format!("{} step failed", phase.label()),
                    last_lines: tail,
                    log_path: None,
                })),
                Err(e) => PhaseOutcome::Terminal(Terminal::Failed(BuildFailed {
                    phase,
                    error: format!("{} step error: {e}", phase.label()),
                    last_lines: tail,
                    log_path: None,
                })),
            }
        }
    }
}

/// Drive the streaming future and the child's exit together; returns whether
/// the child exited successfully.
async fn run_to_completion(
    child: &mut Child,
    stream: impl std::future::Future<Output = ()>,
) -> std::io::Result<bool> {
    stream.await;
    let status = child.wait().await?;
    Ok(status.success())
}

/// Merge stdout+stderr line-by-line, emitting batched `log` events and parsing
/// `[N/M]` build progress. Retains the trailing [`TAIL_LINES`] in `tail`.
async fn stream_output(
    phase: BuildPhase,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    events: &mpsc::Sender<BuildEvent>,
    tail: &mut Vec<String>,
) {
    let mut out_lines = stdout.map(|s| BufReader::new(s).lines());
    let mut err_lines = stderr.map(|s| BufReader::new(s).lines());
    let mut batch: Vec<String> = Vec::new();
    let mut flush = tokio::time::interval(Duration::from_millis(100));
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_percent: u8 = 0;

    loop {
        let line = tokio::select! {
            l = next_line(&mut out_lines) => l,
            l = next_line(&mut err_lines) => l,
            _ = flush.tick() => {
                if !batch.is_empty() {
                    let _ = events.send(BuildEvent::Log { lines: std::mem::take(&mut batch) }).await;
                }
                continue;
            }
        };

        match line {
            Some(line) => {
                if let Some(percent) = parse_progress(&line) {
                    if percent >= last_percent {
                        last_percent = percent;
                        let _ = events.send(BuildEvent::Progress { phase, percent }).await;
                    }
                }
                push_tail(tail, line.clone());
                batch.push(line);
                if batch.len() >= 50 {
                    let _ =
                        events.send(BuildEvent::Log { lines: std::mem::take(&mut batch) }).await;
                }
            }
            None if out_lines.is_none() && err_lines.is_none() => break,
            None => continue,
        }
    }

    if !batch.is_empty() {
        let _ = events.send(BuildEvent::Log { lines: batch }).await;
    }
}

/// Read the next line from an optional line stream, retiring it on EOF.
async fn next_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut Option<tokio::io::Lines<R>>,
) -> Option<String> {
    match reader {
        Some(lines) => match lines.next_line().await {
            Ok(Some(line)) => Some(line),
            _ => {
                *reader = None;
                None
            }
        },
        None => std::future::pending().await,
    }
}

fn push_tail(tail: &mut Vec<String>, line: String) {
    tail.push(line);
    if tail.len() > TAIL_LINES {
        tail.remove(0);
    }
}

/// Parse a cmake `[N/M]` progress marker or a `[ NN%]` marker into a percent.
pub(crate) fn parse_progress(line: &str) -> Option<u8> {
    let line = line.trim_start();
    // `[ 42%] Building ...`
    if let Some(rest) = line.strip_prefix('[') {
        if let Some((inside, _)) = rest.split_once(']') {
            let inside = inside.trim();
            if let Some(pct) = inside.strip_suffix('%') {
                return pct.trim().parse::<u8>().ok().map(|p| p.min(100));
            }
            // `[N/M]`
            if let Some((n, m)) = inside.split_once('/') {
                let n: f64 = n.trim().parse().ok()?;
                let m: f64 = m.trim().parse().ok()?;
                if m > 0.0 {
                    return Some(((n / m) * 100.0).min(100.0) as u8);
                }
            }
        }
    }
    None
}

/// Terminate a process group: `SIGTERM`, wait [`KILL_GRACE`], then `SIGKILL`.
async fn kill_group(child: &mut Child, pgid: Option<i32>) {
    if let Some(pgid) = pgid {
        signal_group(pgid, nix::sys::signal::Signal::SIGTERM);
        if tokio::time::timeout(KILL_GRACE, child.wait()).await.is_err() {
            signal_group(pgid, nix::sys::signal::Signal::SIGKILL);
        }
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Send a signal to the whole process group led by `pgid`.
fn signal_group(pgid: i32, signal: nix::sys::signal::Signal) {
    // The child is its own group leader (process_group(0)), so its pid == pgid.
    let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pgid), signal);
}

/// Copy the built binary into `install_dir`, backing up any existing one to
/// `llama-server.backup.<ISO-timestamp>`. Returns `(replaced_existing, backup)`.
async fn install_binary(
    source_dir: &Path,
    install_dir: &str,
) -> Result<(bool, Option<String>), String> {
    let built = source_dir.join("build").join("bin").join(BINARY_NAME);
    let built = if tokio::fs::try_exists(&built).await.unwrap_or(false) {
        built
    } else {
        // Some cmake layouts emit directly under build/.
        source_dir.join("build").join(BINARY_NAME)
    };
    if !tokio::fs::try_exists(&built).await.unwrap_or(false) {
        return Err(format!("built binary not found at {}", built.display()));
    }

    let install_dir = PathBuf::from(install_dir);
    tokio::fs::create_dir_all(&install_dir)
        .await
        .map_err(|e| format!("cannot create install dir: {e}"))?;
    let target = install_dir.join(BINARY_NAME);

    let mut replaced_existing = false;
    let mut backup_path = None;
    if tokio::fs::try_exists(&target).await.unwrap_or(false) {
        let stamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
        let backup = install_dir.join(format!("{BINARY_NAME}.backup.{stamp}"));
        tokio::fs::rename(&target, &backup)
            .await
            .map_err(|e| format!("cannot back up existing binary: {e}"))?;
        replaced_existing = true;
        backup_path = Some(backup.to_string_lossy().into_owned());
    }

    tokio::fs::copy(&built, &target).await.map_err(|e| format!("cannot install binary: {e}"))?;
    Ok((replaced_existing, backup_path))
}
