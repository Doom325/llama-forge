//! Build endpoint integration tests (08_SETUP_WIZARD.md Section 11.4).
//!
//! Drives `POST/GET/GET-stream/DELETE /api/setup/build*` through the shared
//! `setup_router` with mock git/cmake (injected via `with_build_config`),
//! covering the job pattern, event ordering, SSE terminal replay, idempotent
//! POST, cancellation, log persistence, the disk-space gate, and the
//! SSE-disconnect grace-kill.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use llamaforge_api::{setup_router, SetupState};
use llamaforge_server::{BuildCommands, PhaseTimeouts};
use serde_json::{json, Value};
use tower::ServiceExt;

const CSRF: (&str, &str) = ("x-llamaforge-request", "1");

const MOCK_GIT: &str = "#!/bin/sh\necho cloning\nmkdir -p \"$4\"\n";
const MOCK_CMAKE: &str = "#!/bin/sh\n\
if [ \"$1\" = \"-B\" ]; then mkdir -p \"$2\"; echo configured;\n\
elif [ \"$1\" = \"--build\" ]; then\n\
  echo '[ 50%] Building'; mkdir -p \"$2/bin\";\n\
  printf '#!/bin/sh\\n' > \"$2/bin/llama-server\"; chmod +x \"$2/bin/llama-server\";\n\
  echo '[100%] Built target llama-server';\n\
else echo bad >&2; exit 1; fi\n";
const MOCK_GIT_SLOW: &str = "#!/bin/sh\nsleep 20\n";

fn write_script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn mock_commands(dir: &Path, git_body: &str) -> BuildCommands {
    let git = dir.join("git");
    let cmake = dir.join("cmake");
    write_script(&git, git_body);
    write_script(&cmake, MOCK_CMAKE);
    BuildCommands { git, cmake, nice_level: None }
}

/// Router with mock commands, a temp log dir, an explicit disconnect grace, and
/// a min-disk threshold.
fn app_with(
    commands: BuildCommands,
    log_dir: &Path,
    grace: Duration,
    min_disk_gb: u64,
) -> axum::Router {
    let state = SetupState::new()
        .with_build_config(commands, PhaseTimeouts::default())
        .with_build_overrides(log_dir.to_path_buf(), grace, min_disk_gb);
    setup_router(state)
}

/// Default test app: long grace (no auto-kill interference), disk gate disabled.
fn app_default(commands: BuildCommands, log_dir: &Path) -> axum::Router {
    app_with(commands, log_dir, Duration::from_secs(60), 1)
}

fn post(uri: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header(CSRF.0, CSRF.1)
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap()
}

fn del(uri: &str) -> Request<Body> {
    Request::builder().method("DELETE").uri(uri).header(CSRF.0, CSRF.1).body(Body::empty()).unwrap()
}

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(res.into_body(), 4 * 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn body_text(res: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(res.into_body(), 4 * 1024 * 1024).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn build_req(install: &Path) -> Value {
    json!({ "backend": "cpu", "install_dir": install, "jobs": 2 })
}

/// Poll a build's status until it matches `want` (or give up).
async fn await_status(app: &axum::Router, id: &str, want: &str) -> bool {
    for _ in 0..100 {
        let res = app.clone().oneshot(get(&format!("/api/setup/build/{id}"))).await.unwrap();
        if body_json(res).await["status"] == want {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn build_runs_to_completion_replays_terminal_and_persists_log() {
    let tools = tempfile::tempdir().unwrap();
    let install = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let app = app_default(mock_commands(tools.path(), MOCK_GIT), logs.path());

    let res =
        app.clone().oneshot(post("/api/setup/build", &build_req(install.path()))).await.unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let body = body_json(res).await;
    assert_eq!(body["version"], 1);
    let id = body["build_id"].as_str().unwrap().to_string();

    assert!(await_status(&app, &id, "complete").await, "build did not complete");
    assert!(install.path().join("llama-server").exists());

    // A build log was persisted (§11.7).
    let log_count = fs::read_dir(logs.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("build-"))
        .count();
    assert_eq!(log_count, 1, "expected one persisted build log");

    // Reconnect: the finished build replays the backlog incl. the terminal
    // `complete` event (with a log_path), then closes.
    let res = app.oneshot(get(&format!("/api/setup/build/{id}/stream"))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let sse = body_text(res).await;
    assert!(sse.contains("event: phase"), "{sse}");
    assert!(sse.contains("event: complete"), "{sse}");
    assert!(sse.contains("log_path"), "complete event should carry log_path: {sse}");
    assert!(sse.find("cloning") < sse.find("building"), "phases out of order: {sse}");
}

#[tokio::test]
async fn post_build_is_idempotent_per_backend() {
    let tools = tempfile::tempdir().unwrap();
    let install = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    // Slow clone so the job is still running for the second POST.
    let app = app_default(mock_commands(tools.path(), MOCK_GIT_SLOW), logs.path());

    let first = body_json(
        app.clone().oneshot(post("/api/setup/build", &build_req(install.path()))).await.unwrap(),
    )
    .await;
    let second = body_json(
        app.clone().oneshot(post("/api/setup/build", &build_req(install.path()))).await.unwrap(),
    )
    .await;
    assert_eq!(first["build_id"], second["build_id"], "expected the same build_id");

    let id = first["build_id"].as_str().unwrap();
    let _ = app.oneshot(del(&format!("/api/setup/build/{id}"))).await.unwrap();
}

#[tokio::test]
async fn delete_cancels_a_running_build() {
    let tools = tempfile::tempdir().unwrap();
    let install = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let app = app_default(mock_commands(tools.path(), MOCK_GIT_SLOW), logs.path());

    let body = body_json(
        app.clone().oneshot(post("/api/setup/build", &build_req(install.path()))).await.unwrap(),
    )
    .await;
    let id = body["build_id"].as_str().unwrap().to_string();

    let res = app.clone().oneshot(del(&format!("/api/setup/build/{id}"))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["cancelled"], true);
    assert!(await_status(&app, &id, "cancelled").await, "build was not marked cancelled");
}

#[tokio::test]
async fn status_for_unknown_build_is_an_error() {
    let tools = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let app = app_default(mock_commands(tools.path(), MOCK_GIT), logs.path());
    let res =
        app.oneshot(get("/api/setup/build/00000000-0000-0000-0000-000000000000")).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn build_refused_when_disk_below_threshold() {
    let tools = tempfile::tempdir().unwrap();
    let install = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    // Require more disk than any machine has → insufficient_disk.
    let app = app_with(
        mock_commands(tools.path(), MOCK_GIT),
        logs.path(),
        Duration::from_secs(60),
        u64::MAX,
    );

    let res = app.oneshot(post("/api/setup/build", &build_req(install.path()))).await.unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let body = body_json(res).await;
    assert_eq!(body["error"], "insufficient_disk");
    assert_eq!(body["required_gb"], json!(u64::MAX));
}

#[tokio::test]
async fn abandoned_build_is_killed_after_disconnect_grace() {
    let tools = tempfile::tempdir().unwrap();
    let install = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    // Slow build + short grace so the disconnect-kill path is exercised fast.
    let app = app_with(
        mock_commands(tools.path(), MOCK_GIT_SLOW),
        logs.path(),
        Duration::from_millis(400),
        1,
    );

    let body = body_json(
        app.clone().oneshot(post("/api/setup/build", &build_req(install.path()))).await.unwrap(),
    )
    .await;
    let id = body["build_id"].as_str().unwrap().to_string();

    // Connect to the stream (subscriber count → 1) and hold it open long enough
    // for the monitor to observe the subscriber, then drop it (→ 0).
    let res = app.clone().oneshot(get(&format!("/api/setup/build/{id}/stream"))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(250)).await;
    drop(res);

    // With no subscriber, the build is cancelled after the grace window.
    assert!(
        await_status(&app, &id, "cancelled").await,
        "abandoned build was not killed after the disconnect grace"
    );
}
