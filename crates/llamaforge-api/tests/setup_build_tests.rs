//! Build endpoint integration tests (08_SETUP_WIZARD.md Section 11.4).
//!
//! Drives `POST/GET/GET-stream/DELETE /api/setup/build*` through the shared
//! `setup_router` with mock git/cmake (injected via `with_build_config`),
//! covering the job pattern, event ordering, SSE terminal replay, idempotent
//! POST, and cancellation.

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

fn app_with(commands: BuildCommands) -> axum::Router {
    setup_router(SetupState::new().with_build_config(commands, PhaseTimeouts::default()))
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

#[tokio::test]
async fn build_runs_to_completion_and_replays_terminal_event() {
    let tools = tempfile::tempdir().unwrap();
    let install = tempfile::tempdir().unwrap();
    let app = app_with(mock_commands(tools.path(), MOCK_GIT));

    // Create the build.
    let req = json!({ "backend": "cpu", "install_dir": install.path(), "jobs": 2 });
    let res = app.clone().oneshot(post("/api/setup/build", &req)).await.unwrap();
    assert_eq!(res.status(), StatusCode::ACCEPTED);
    let body = body_json(res).await;
    assert_eq!(body["version"], 1);
    let id = body["build_id"].as_str().unwrap().to_string();

    // Poll status until complete.
    let mut completed = false;
    for _ in 0..100 {
        let res = app.clone().oneshot(get(&format!("/api/setup/build/{id}"))).await.unwrap();
        let status = body_json(res).await;
        if status["status"] == "complete" {
            completed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(completed, "build did not complete");
    assert!(install.path().join("llama-server").exists());

    // Reconnect to the stream: the finished build replays the backlog incl. the
    // terminal `complete` event, then closes.
    let res = app.oneshot(get(&format!("/api/setup/build/{id}/stream"))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let sse = body_text(res).await;
    assert!(sse.contains("event: phase"), "{sse}");
    assert!(sse.contains("event: complete"), "{sse}");
    // phase ordering: cloning precedes building in the replayed stream.
    let cloning = sse.find("cloning");
    let building = sse.find("building");
    assert!(cloning < building, "phases out of order: {sse}");
}

#[tokio::test]
async fn post_build_is_idempotent_per_backend() {
    let tools = tempfile::tempdir().unwrap();
    let install = tempfile::tempdir().unwrap();
    // Slow clone so the job is still running for the second POST.
    let app = app_with(mock_commands(tools.path(), MOCK_GIT_SLOW));

    let req = json!({ "backend": "cpu", "install_dir": install.path(), "jobs": 2 });
    let first = body_json(app.clone().oneshot(post("/api/setup/build", &req)).await.unwrap()).await;
    let second =
        body_json(app.clone().oneshot(post("/api/setup/build", &req)).await.unwrap()).await;
    assert_eq!(first["build_id"], second["build_id"], "expected the same build_id");

    // Clean up: cancel the slow build.
    let id = first["build_id"].as_str().unwrap();
    let _ = app.oneshot(del(&format!("/api/setup/build/{id}"))).await.unwrap();
}

#[tokio::test]
async fn delete_cancels_a_running_build() {
    let tools = tempfile::tempdir().unwrap();
    let install = tempfile::tempdir().unwrap();
    let app = app_with(mock_commands(tools.path(), MOCK_GIT_SLOW));

    let req = json!({ "backend": "cpu", "install_dir": install.path(), "jobs": 2 });
    let body = body_json(app.clone().oneshot(post("/api/setup/build", &req)).await.unwrap()).await;
    let id = body["build_id"].as_str().unwrap().to_string();

    let res = app.clone().oneshot(del(&format!("/api/setup/build/{id}"))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["cancelled"], true);

    // Status eventually reflects cancellation.
    let mut cancelled = false;
    for _ in 0..100 {
        let res = app.clone().oneshot(get(&format!("/api/setup/build/{id}"))).await.unwrap();
        if body_json(res).await["status"] == "cancelled" {
            cancelled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(cancelled, "build was not marked cancelled");
}

#[tokio::test]
async fn status_for_unknown_build_is_an_error() {
    let tools = tempfile::tempdir().unwrap();
    let app = app_with(mock_commands(tools.path(), MOCK_GIT));
    let res =
        app.oneshot(get("/api/setup/build/00000000-0000-0000-0000-000000000000")).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
