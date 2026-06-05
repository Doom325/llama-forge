//! `GET /api/status` — lightweight, unauthenticated status used by the
//! frontend to detect first-run mode (08_SETUP_WIZARD.md Section 13.6).
//!
//! Mounted on the main HTTP router (outside `setup_router` and CSRF) so the
//! SvelteKit root layout can poll it without credentials. `setup_required` is
//! `true` while the orchestrator is in `Setup`/`Transitioning` mode and `false`
//! once it is `Running` (or whenever first-run mode is not in play).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

/// Shared flag set while the orchestrator is in setup mode. `None` in normal
/// (non-first-run) boots, where `setup_required` is always `false`.
pub(crate) type SetupActive = Option<Arc<AtomicBool>>;

pub(crate) async fn status_handler(State(setup_active): State<SetupActive>) -> impl IntoResponse {
    let setup_required =
        setup_active.as_ref().map(|flag| flag.load(Ordering::Relaxed)).unwrap_or(false);
    Json(json!({
        "setup_required": setup_required,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
