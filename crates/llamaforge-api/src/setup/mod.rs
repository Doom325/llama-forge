//! Setup Wizard HTTP API (`/api/setup/*`).
//!
//! Adapter layer that wires the `llamaforge-server` domain modules (hardware
//! detection, model/binary scanning, config recommendation) plus config
//! assembly to axum handlers. Implements `08_SETUP_WIZARD.md` Section 11 (API
//! contracts), Section 11.7 (observability), and Section 12.1 (the shared
//! [`setup_router`] reused by both standalone-wizard and first-run modes).
//!
//! Every response carries a top-level `"version": 1` envelope field. Mutating
//! endpoints require the `X-LlamaForge-Request: 1` CSRF header (enforced by the
//! shared [`crate::transport::middleware::csrf_middleware`]). The
//! `X-Setup-Session-Id` header is attached to each request's tracing span for
//! correlation.

mod build;
mod config_assembly;
mod handlers;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{extract::Request, Json, Router};
use llamaforge_server::{BuildCommands, HardwareInfo, PhaseTimeouts};
use serde::Serialize;
use serde_json::json;
use tokio::sync::{Mutex, RwLock};
use tracing::Instrument;
use uuid::Uuid;

pub use config_assembly::{GlobalOverrides, SaveConfigRequest, ServerConfigDraft};

/// Envelope version for all `/api/setup/*` responses (Section 11).
const SETUP_API_VERSION: u8 = 1;

/// Header carrying the wizard session id for log correlation (Section 11.7).
const SESSION_HEADER: &str = "x-setup-session-id";

/// Shared state for the setup router. Cheap to clone (all `Arc`).
#[derive(Clone, Default)]
pub struct SetupState {
    /// Last hardware detection, cached so `recommend` can reuse it without a
    /// re-probe (Section 11.5). Populated by the hardware handler.
    hardware_cache: Arc<RwLock<Option<HardwareInfo>>>,
    /// Serializes `save-config` writes so concurrent saves can't interleave.
    save_lock: Arc<Mutex<()>>,
    /// In-flight and finished build jobs, keyed by build id (Section 11.4).
    builds: Arc<RwLock<HashMap<Uuid, Arc<build::BuildJob>>>>,
    /// git/cmake programs used by builds (overridable for tests).
    build_commands: BuildCommands,
    /// Per-phase build timeouts (overridable for tests).
    build_timeouts: PhaseTimeouts,
    /// Directory for persisted build logs; `None` → the default XDG location.
    build_log_dir: Option<PathBuf>,
    /// SSE-disconnect grace before an abandoned build is killed; `None` → 2 min.
    build_disconnect_grace: Option<Duration>,
    /// Minimum free disk (GB) required to start a build; `None` → `MIN_DISK_GB`.
    build_min_disk_gb: Option<u64>,
    /// Notified by `validate-launch` on success so a host (standalone wizard
    /// server, or first-run serve loop) can react to a completed setup
    /// (Sections 12.2 / 13.4). Re-armable so reload-retry works. `None` outside.
    completion: Option<Arc<tokio::sync::Notify>>,
}

impl SetupState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the build toolchain + timeouts (test seam for mock git/cmake).
    #[doc(hidden)]
    pub fn with_build_config(mut self, commands: BuildCommands, timeouts: PhaseTimeouts) -> Self {
        self.build_commands = commands;
        self.build_timeouts = timeouts;
        self
    }

    /// Override build log directory, disconnect grace, and min-disk threshold
    /// (test seam — keeps logs out of the real XDG dir and lets tests exercise
    /// the grace-kill and insufficient-disk paths).
    #[doc(hidden)]
    pub fn with_build_overrides(
        mut self,
        log_dir: PathBuf,
        disconnect_grace: Duration,
        min_disk_gb: u64,
    ) -> Self {
        self.build_log_dir = Some(log_dir);
        self.build_disconnect_grace = Some(disconnect_grace);
        self.build_min_disk_gb = Some(min_disk_gb);
        self
    }

    /// Set the notifier signaled when a setup completes (`validate-launch`
    /// succeeds), so a host can react (shut down, or hot-reload into running
    /// mode). Sections 12.2 / 13.4.
    pub fn with_completion(mut self, notify: Arc<tokio::sync::Notify>) -> Self {
        self.completion = Some(notify);
        self
    }

    /// Signal that setup completed successfully.
    pub(crate) fn signal_complete(&self) {
        if let Some(notify) = &self.completion {
            notify.notify_one();
        }
    }
}

/// Compose all `/api/setup/*` routes into a router. Shared by the standalone
/// wizard server and first-run mode (Section 12.1). Carries its own CSRF and
/// session-correlation layers so it is self-contained when mounted without the
/// full application middleware stack.
pub fn setup_router(state: SetupState) -> Router {
    Router::new()
        .route("/api/setup/hardware", get(handlers::hardware_handler))
        .route("/api/setup/scan-binary", post(handlers::scan_binary_handler))
        .route("/api/setup/scan-models", post(handlers::scan_models_handler))
        .route("/api/setup/recommend", post(handlers::recommend_handler))
        .route("/api/setup/preview-config", post(handlers::preview_config_handler))
        .route("/api/setup/save-config", post(handlers::save_config_handler))
        .route("/api/setup/validate-launch", post(handlers::validate_launch_handler))
        .route("/api/setup/build", post(build::build_create_handler))
        .route("/api/setup/build/{id}", get(build::build_status_handler))
        .route("/api/setup/build/{id}", delete(build::build_cancel_handler))
        .route("/api/setup/build/{id}/stream", get(build::build_stream_handler))
        .layer(axum::middleware::from_fn(crate::transport::middleware::csrf_middleware))
        .layer(axum::middleware::from_fn(session_span_middleware))
        .with_state(state)
}

/// Wrap each request in a `setup_request` span carrying the session id and
/// endpoint, defaulting the session to `"anonymous"` when the header is absent.
async fn session_span_middleware(req: Request, next: Next) -> Response {
    let session_id = req
        .headers()
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or("anonymous")
        .to_string();
    let endpoint = req.uri().path().to_string();
    let span =
        tracing::info_span!("setup_request", setup_session_id = %session_id, endpoint = %endpoint);
    next.run(req).instrument(span).await
}

// ---------------------------------------------------------------------------
// Response envelope.
// ---------------------------------------------------------------------------

/// Wraps a serializable payload, flattening it under a top-level
/// `"version": 1` field on serialization.
struct Versioned<T>(T);

impl<T: Serialize> IntoResponse for Versioned<T> {
    fn into_response(self) -> Response {
        match serde_json::to_value(&self.0) {
            Ok(mut value) => {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("version".to_string(), json!(SETUP_API_VERSION));
                    Json(value).into_response()
                } else {
                    // Non-object payload: nest it so the envelope stays an object.
                    Json(json!({ "version": SETUP_API_VERSION, "data": value })).into_response()
                }
            }
            Err(e) => SetupError::Internal(e.to_string()).into_response(),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

/// One failed pre-validation check (08_SETUP_WIZARD.md Section 13.5 Step 1).
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ValidationDetail {
    pub field: String,
    pub error: String,
}

/// Setup endpoint error, rendered as a structured JSON body with the matching
/// HTTP status (Section 11 input-validation contract).
#[derive(Debug)]
pub(crate) enum SetupError {
    /// Invalid input → 400.
    BadRequest(String),
    /// Operation refused (e.g. unwritable path) → 403.
    Forbidden(String),
    /// Conflict (e.g. config exists without `force`) → 409.
    Conflict(String),
    /// Save/preview pre-validation failed → 422 (Section 13.5 Step 1).
    PreValidation(Vec<ValidationDetail>),
    /// Unexpected server-side failure → 500.
    Internal(String),
}

impl IntoResponse for SetupError {
    fn into_response(self) -> Response {
        use axum::http::StatusCode;
        let (status, mut body) = match self {
            SetupError::BadRequest(m) => {
                (StatusCode::BAD_REQUEST, json!({ "error": "invalid_request", "message": m }))
            }
            SetupError::Forbidden(m) => {
                (StatusCode::FORBIDDEN, json!({ "error": "forbidden", "message": m }))
            }
            SetupError::Conflict(m) => {
                (StatusCode::CONFLICT, json!({ "error": "conflict", "message": m }))
            }
            SetupError::PreValidation(details) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "error": "pre_validation_failed", "details": details }),
            ),
            SetupError::Internal(m) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": "internal_error", "message": m }),
            ),
        };
        if let Some(obj) = body.as_object_mut() {
            obj.insert("version".to_string(), json!(SETUP_API_VERSION));
        }
        (status, Json(body)).into_response()
    }
}
