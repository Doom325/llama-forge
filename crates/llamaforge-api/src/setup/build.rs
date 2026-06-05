//! Build orchestrator HTTP adapter (`/api/setup/build*`).
//!
//! Owns the in-memory build-job registry, drives [`llamaforge_server::run_build`]
//! on a background task, fans build events out to SSE subscribers, and exposes
//! the job pattern from `08_SETUP_WIZARD.md` Section 11.4: `POST` creates,
//! `GET` polls status, `GET /stream` streams (reconnectable, replays the
//! terminal event), `DELETE` cancels.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

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
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{SetupError, SetupState, SETUP_API_VERSION};

/// Trailing log lines retained for status polling.
const STATUS_TAIL: usize = 20;
/// Broadcast channel capacity (live SSE fan-out).
const BROADCAST_CAPACITY: usize = 1024;

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
    if let Some(available_gb) = available_disk_gb(&temp_base) {
        if available_gb < MIN_DISK_GB {
            return Ok((
                StatusCode::CONFLICT,
                Json(json!({
                    "version": SETUP_API_VERSION,
                    "error": "insufficient_disk",
                    "available_gb": available_gb,
                    "required_gb": MIN_DISK_GB,
                })),
            )
                .into_response());
        }
    }

    // Idempotent: reuse a running build for the same backend.
    {
        let builds = state.builds.read().await;
        for (id, job) in builds.iter() {
            if job.backend == request.backend && job.state.read().await.status == JobStatus::Running
            {
                return Ok(accepted(*id));
            }
        }
    }

    let id = Uuid::new_v4();
    let temp_dir = temp_base.join(format!("llamaforge-build-{id}"));
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|e| SetupError::Internal(format!("cannot create build dir: {e}")))?;

    let (events, _) = broadcast::channel(BROADCAST_CAPACITY);
    let job = Arc::new(BuildJob {
        backend: request.backend,
        state: RwLock::new(JobState::new()),
        events,
        cancel: CancellationToken::new(),
        temp_dir,
    });
    state.builds.write().await.insert(id, job.clone());

    spawn_build(job, request, state.clone());
    Ok(accepted(id))
}

/// Spawn the background task that runs the build and forwards its events into
/// the job's state + broadcast channel, cleaning up the temp dir at the end.
fn spawn_build(job: Arc<BuildJob>, request: BuildRequest, state: SetupState) {
    tokio::spawn(async move {
        let (tx, mut rx) = mpsc::channel::<BuildEvent>(256);

        let forward_job = job.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                // Hold the write lock across state-update + broadcast so a
                // reconnecting stream (which snapshots the backlog under the
                // read lock before subscribing) never loses or duplicates an
                // event in the gap between snapshot and subscribe.
                let mut s = forward_job.state.write().await;
                apply_event(&mut s, &event);
                let _ = forward_job.events.send(event);
            }
        });

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

        // Finalize status (the terminal event already set Complete/Failed; a
        // cancellation produces no terminal event, so record it here).
        if let BuildOutcome::Cancelled = outcome {
            job.state.write().await.status = JobStatus::Cancelled;
        }

        // Temp dir is cleaned up on every terminal path (success/failure/cancel).
        let _ = tokio::fs::remove_dir_all(&job.temp_dir).await;
    });
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
