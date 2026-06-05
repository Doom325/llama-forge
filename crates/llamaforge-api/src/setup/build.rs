//! Build orchestrator HTTP adapter (`/api/setup/build*`).
//!
//! Owns the in-memory build-job registry, drives [`llamaforge_server::run_build`]
//! on a background task, fans build events out to SSE subscribers, and exposes
//! the job pattern from `08_SETUP_WIZARD.md` Section 11.4: `POST` creates,
//! `GET` polls status, `GET /stream` streams (reconnectable, replays the
//! terminal event), `DELETE` cancels.

use std::convert::Infallible;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_core::Stream;
use llamaforge_server::{
    available_disk_gb, run_build, BuildBackend, BuildEvent, BuildOutcome, BuildPhase, BuildRequest,
    MIN_DISK_GB,
};
use serde::Serialize;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{SetupError, SetupState, SETUP_API_VERSION};

/// Trailing log lines retained for status polling.
const STATUS_TAIL: usize = 20;
/// Broadcast channel capacity (live SSE fan-out).
const BROADCAST_CAPACITY: usize = 1024;
/// Default SSE-disconnect grace before an abandoned build is killed (§11.4).
const DEFAULT_DISCONNECT_GRACE: Duration = Duration::from_secs(120);
/// Persisted build logs retained (newest-first); older ones are pruned.
const MAX_BUILD_LOGS: usize = 5;

/// Lifecycle status of a build job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum JobStatus {
    Running,
    Complete,
    Failed,
    Cancelled,
}

/// Mutable snapshot of a build job's progress.
struct JobState {
    status: JobStatus,
    phase: Option<BuildPhase>,
    percent: u8,
    last_lines: Vec<String>,
    /// All events emitted so far, replayed to late SSE subscribers.
    backlog: Vec<BuildEvent>,
}

impl JobState {
    fn new() -> Self {
        Self {
            status: JobStatus::Running,
            phase: None,
            percent: 0,
            last_lines: Vec::new(),
            backlog: Vec::new(),
        }
    }
}

/// A single build job and its fan-out machinery.
pub(super) struct BuildJob {
    backend: BuildBackend,
    state: RwLock<JobState>,
    events: broadcast::Sender<BuildEvent>,
    cancel: CancellationToken,
    temp_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// POST /api/setup/build
// ---------------------------------------------------------------------------

pub(super) async fn build_create_handler(
    State(state): State<SetupState>,
    Json(request): Json<BuildRequest>,
) -> Result<Response, SetupError> {
    // Disk-space gate (Section 11.4): refuse before opening a stream.
    let temp_base = std::env::temp_dir();
    let min_disk_gb = state.build_min_disk_gb.unwrap_or(MIN_DISK_GB);
    if let Some(available_gb) = available_disk_gb(&temp_base) {
        if available_gb < min_disk_gb {
            return Ok((
                StatusCode::CONFLICT,
                Json(json!({
                    "version": SETUP_API_VERSION,
                    "error": "insufficient_disk",
                    "available_gb": available_gb,
                    "required_gb": min_disk_gb,
                })),
            )
                .into_response());
        }
    }

    let id = Uuid::new_v4();
    let temp_dir = temp_base.join(format!("llamaforge-build-{id}"));
    let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
    let job = Arc::new(BuildJob {
        backend: request.backend,
        state: RwLock::new(JobState::new()),
        events,
        cancel: CancellationToken::new(),
        temp_dir: temp_dir.clone(),
    });

    // Idempotency: scan for a running same-backend build and insert atomically
    // under one write lock so two concurrent POSTs cannot both create a build.
    let existing = {
        let mut builds = state.builds.write().await;
        let mut found = None;
        for (eid, ejob) in builds.iter() {
            if ejob.backend == request.backend
                && ejob.state.read().await.status == JobStatus::Running
            {
                found = Some(*eid);
                break;
            }
        }
        if found.is_none() {
            builds.insert(id, job.clone());
        }
        found
    };
    if let Some(existing_id) = existing {
        return Ok(accepted(existing_id));
    }

    if let Err(e) = tokio::fs::create_dir_all(&temp_dir).await {
        state.builds.write().await.remove(&id);
        return Err(SetupError::Internal(format!("cannot create build dir: {e}")));
    }

    spawn_build(job, request, state.clone());
    Ok(accepted(id))
}

/// Spawn the background task that runs the build, persists its logs, forwards
/// events into the job state + broadcast channel, watches for SSE-disconnect
/// abandonment, and cleans up the temp dir at the end.
fn spawn_build(job: Arc<BuildJob>, request: BuildRequest, state: SetupState) {
    let log_dir = state.build_log_dir.clone().unwrap_or_else(default_build_log_dir);
    let grace = state.build_disconnect_grace.unwrap_or(DEFAULT_DISCONNECT_GRACE);

    tokio::spawn(async move {
        // Build logs are always persisted (§11.7), even at RUST_LOG=warn.
        let log_path = prepare_log_file(&log_dir).await;
        let mut writer = match &log_path {
            Some(path) => tokio::fs::File::create(path).await.ok(),
            None => None,
        };

        let (tx, mut rx) = mpsc::channel::<BuildEvent>(256);
        let forward_job = job.clone();
        let log_path_str = log_path.as_ref().map(|p| p.to_string_lossy().into_owned());
        let forwarder = tokio::spawn(async move {
            while let Some(mut event) = rx.recv().await {
                // Persist log lines and stamp the terminal events with log_path.
                match &mut event {
                    BuildEvent::Log { lines } => {
                        if let Some(file) = writer.as_mut() {
                            for line in lines.iter() {
                                let _ = file.write_all(line.as_bytes()).await;
                                let _ = file.write_all(b"\n").await;
                            }
                        }
                    }
                    BuildEvent::Complete(complete) => complete.log_path = log_path_str.clone(),
                    BuildEvent::Failed(failed) => failed.log_path = log_path_str.clone(),
                    _ => {}
                }
                // Hold the write lock across state-update + broadcast so a
                // reconnecting stream (which snapshots the backlog under the
                // read lock before subscribing) never loses or duplicates an
                // event in the gap between snapshot and subscribe.
                let mut s = forward_job.state.write().await;
                apply_event(&mut s, &event);
                let _ = forward_job.events.send(event);
            }
            if let Some(file) = writer.as_mut() {
                let _ = file.flush().await;
            }
        });

        let monitor = spawn_disconnect_monitor(job.clone(), grace);

        let outcome = run_build(
            &request,
            &job.temp_dir,
            &state.build_commands,
            &state.build_timeouts,
            tx,
            &job.cancel,
        )
        .await;
        let _ = forwarder.await;
        monitor.abort();

        // Finalize status (the terminal event already set Complete/Failed; a
        // cancellation produces no terminal event, so record it here).
        if let BuildOutcome::Cancelled = outcome {
            job.state.write().await.status = JobStatus::Cancelled;
        }

        // Temp dir is cleaned up on every terminal path (success/failure/cancel).
        let _ = tokio::fs::remove_dir_all(&job.temp_dir).await;
    });
}

/// Watch SSE subscriber count; once a client has connected and then all
/// disconnect, cancel the build after `grace` unless someone reconnects
/// (§11.4 — abandoned builds must not run unwatched).
fn spawn_disconnect_monitor(job: Arc<BuildJob>, grace: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let poll = (grace / 4).clamp(Duration::from_millis(50), Duration::from_secs(5));
        let mut ever_connected = false;
        let mut deadline: Option<Instant> = None;
        loop {
            tokio::select! {
                _ = job.cancel.cancelled() => break,
                _ = tokio::time::sleep(poll) => {}
            }
            if job.state.read().await.status != JobStatus::Running {
                break;
            }
            if job.events.receiver_count() > 0 {
                ever_connected = true;
                deadline = None;
            } else if ever_connected {
                match deadline {
                    None => deadline = Some(Instant::now() + grace),
                    Some(at) if Instant::now() >= at => {
                        job.cancel.cancel();
                        break;
                    }
                    Some(_) => {}
                }
            }
        }
    })
}

/// Fold one event into the job state.
fn apply_event(state: &mut JobState, event: &BuildEvent) {
    match event {
        BuildEvent::Phase { phase, .. } => state.phase = Some(*phase),
        BuildEvent::Progress { percent, .. } => state.percent = (*percent).max(state.percent),
        BuildEvent::Log { lines } => {
            for line in lines {
                state.last_lines.push(line.clone());
                if state.last_lines.len() > STATUS_TAIL {
                    state.last_lines.remove(0);
                }
            }
        }
        BuildEvent::Complete(_) => state.status = JobStatus::Complete,
        BuildEvent::Failed(_) => state.status = JobStatus::Failed,
    }
    state.backlog.push(event.clone());
}

// ---------------------------------------------------------------------------
// GET /api/setup/build/{id}
// ---------------------------------------------------------------------------

pub(super) async fn build_status_handler(
    State(state): State<SetupState>,
    Path(id): Path<Uuid>,
) -> Result<Response, SetupError> {
    let job = lookup(&state, id).await?;
    let s = job.state.read().await;
    Ok(Json(json!({
        "version": SETUP_API_VERSION,
        "status": s.status,
        "phase": s.phase,
        "percent": s.percent,
        "last_lines": s.last_lines,
    }))
    .into_response())
}

// ---------------------------------------------------------------------------
// GET /api/setup/build/{id}/stream
// ---------------------------------------------------------------------------

pub(super) async fn build_stream_handler(
    State(state): State<SetupState>,
    Path(id): Path<Uuid>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, SetupError> {
    let job = lookup(&state, id).await?;

    // Snapshot the backlog and subscribe under the read lock so no event slips
    // between the two (see the write-lock comment in `spawn_build`).
    let (backlog, mut receiver) = {
        let s = job.state.read().await;
        (s.backlog.clone(), job.events.subscribe())
    };

    let stream = async_stream::stream! {
        for event in backlog {
            let terminal = is_terminal(&event);
            yield Ok(sse_event(&event));
            if terminal {
                return;
            }
        }
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let terminal = is_terminal(&event);
                    yield Ok(sse_event(&event));
                    if terminal {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn is_terminal(event: &BuildEvent) -> bool {
    matches!(event, BuildEvent::Complete(_) | BuildEvent::Failed(_))
}

/// Map a [`BuildEvent`] to an SSE event named `phase`/`progress`/`log`/
/// `complete`/`failed` carrying the JSON data payload.
fn sse_event(event: &BuildEvent) -> Event {
    let value = serde_json::to_value(event).unwrap_or_else(|_| json!({}));
    let name = value.get("event").and_then(|n| n.as_str()).unwrap_or("message").to_string();
    let data = value.get("data").cloned().unwrap_or_else(|| json!({}));
    Event::default().event(name).data(data.to_string())
}

// ---------------------------------------------------------------------------
// DELETE /api/setup/build/{id}
// ---------------------------------------------------------------------------

pub(super) async fn build_cancel_handler(
    State(state): State<SetupState>,
    Path(id): Path<Uuid>,
) -> Result<Response, SetupError> {
    let job = lookup(&state, id).await?;
    job.cancel.cancel();
    Ok(Json(json!({ "version": SETUP_API_VERSION, "cancelled": true })).into_response())
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

async fn lookup(state: &SetupState, id: Uuid) -> Result<Arc<BuildJob>, SetupError> {
    state
        .builds
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(|| SetupError::BadRequest(format!("no build with id {id}")))
}

/// `202 Accepted` with the build id.
fn accepted(id: Uuid) -> Response {
    (StatusCode::ACCEPTED, Json(json!({ "version": SETUP_API_VERSION, "build_id": id })))
        .into_response()
}

/// Default persisted-log directory: `$XDG_DATA_HOME/llamaforge/build-logs`
/// (falling back to `~/.local/share`, then the temp dir).
fn default_build_log_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("llamaforge").join("build-logs")
}

/// Ensure the log directory exists, prune old logs, and return the path for
/// this build's log file. `None` if the directory cannot be created (logging
/// is best-effort and never fails the build).
async fn prepare_log_file(log_dir: &FsPath) -> Option<PathBuf> {
    if tokio::fs::create_dir_all(log_dir).await.is_err() {
        return None;
    }
    prune_build_logs(log_dir).await;
    let stamp = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
    Some(log_dir.join(format!("build-{stamp}.log")))
}

/// Keep only the newest `MAX_BUILD_LOGS - 1` `build-*.log` files so adding this
/// build's log stays within the retention cap.
async fn prune_build_logs(log_dir: &FsPath) {
    let Ok(mut entries) = tokio::fs::read_dir(log_dir).await else {
        return;
    };
    let mut logs: Vec<PathBuf> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("build-") && name.ends_with(".log") {
            logs.push(entry.path());
        }
    }
    logs.sort(); // timestamped names sort oldest-first
    let keep = MAX_BUILD_LOGS.saturating_sub(1);
    if logs.len() > keep {
        for old in &logs[..logs.len() - keep] {
            let _ = tokio::fs::remove_file(old).await;
        }
    }
}
