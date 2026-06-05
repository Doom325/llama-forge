//! Setup API integration tests (08_SETUP_WIZARD.md Section 11).
//!
//! Drives the shared `setup_router` end to end with `tower::oneshot`, covering
//! response shape, the `"version": 1` envelope, CSRF, input validation
//! (400/403/409), the shared hardware cache, and save/preview-config.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use llamaforge_api::{setup_router, SetupState};
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;

const CSRF: (&str, &str) = ("x-llamaforge-request", "1");

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap()
}

fn post(uri: &str, body: &Value, csrf: bool) -> Request<Body> {
    let mut builder =
        Request::builder().method("POST").uri(uri).header("content-type", "application/json");
    if csrf {
        builder = builder.header(CSRF.0, CSRF.1);
    }
    builder.body(Body::from(serde_json::to_vec(body).unwrap())).unwrap()
}

fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, b"GGUF\x00\x00\x00").unwrap();
}

fn draft(model_path: &str, binary_path: &str, port: u16) -> Value {
    json!({
        "model_path": model_path,
        "alias": "test-model",
        "binary_path": binary_path,
        "port": port,
        "gpu_layers": -1,
        "parallel_slots": 2,
        "ctx_size": 8192,
        "cache_type_k": "q8_0",
        "cache_type_v": "q8_0",
        "flash_attn": "auto",
        "estimated_vram_mb": 6000
    })
}

/// An ephemeral port that is free at call time (released immediately).
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Create real model + binary files in `dir` so save/preview pre-validation
/// (which checks the paths exist) passes. Returns (model_path, binary_path).
fn real_paths(dir: &Path) -> (String, String) {
    let model = dir.join("model.gguf");
    let binary = dir.join("llama-server");
    fs::write(&model, b"GGUF\x00").unwrap();
    fs::write(&binary, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    (model.display().to_string(), binary.display().to_string())
}

// --- 5.14: hardware endpoint ----------------------------------------------

#[tokio::test]
async fn hardware_endpoint_returns_versioned_summary() {
    let app = setup_router(SetupState::new());
    let res = app.oneshot(get("/api/setup/hardware")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = body_json(res).await;
    assert_eq!(body["version"], 1);
    assert!(body["cpu"].is_object());
    assert!(body["gpus"].is_array());
    assert!(body["recommended_backend"].is_string());
}

// --- 5.15: scan-models endpoint -------------------------------------------

#[tokio::test]
async fn scan_models_endpoint_discovers_files() {
    let dir = tempfile::tempdir().unwrap();
    touch(&dir.path().join("llama-3-8b-Q4_K_M.gguf"));

    let app = setup_router(SetupState::new());
    let req_body = json!({ "source": "local", "directories": [dir.path()] });
    let res = app.oneshot(post("/api/setup/scan-models", &req_body, true)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = body_json(res).await;
    assert_eq!(body["version"], 1);
    let models = body["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["quantization_hint"], "Q4_K_M");
}

// --- 5.16: scan-binary endpoint -------------------------------------------

#[tokio::test]
async fn scan_binary_endpoint_finds_mock_via_extra_paths() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("llama-server");
    fs::write(&script, "#!/bin/sh\necho 'b4567'\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let app = setup_router(SetupState::new());
    let req_body = json!({ "extra_paths": [dir.path()] });
    let res = app.oneshot(post("/api/setup/scan-binary", &req_body, true)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = body_json(res).await;
    assert_eq!(body["version"], 1);
    assert_eq!(body["found"], true);
    assert_eq!(body["binary_version"], "b4567");
    assert!(body.get("version").is_some()); // envelope, separate from binary_version
}

// --- 5.17: recommend endpoint (shared hardware cache) ----------------------

#[tokio::test]
async fn recommend_endpoint_uses_cached_hardware() {
    let app = setup_router(SetupState::new());

    // Populate the cache via the hardware endpoint, then recommend (shared
    // Arc state across the cloned router).
    let warm = app.clone().oneshot(get("/api/setup/hardware")).await.unwrap();
    assert_eq!(warm.status(), StatusCode::OK);

    let req_body = json!({ "models": [{ "path": "/models/llama-3-8b.gguf", "size_bytes": 4_400_000_000u64 }] });
    let res = app.oneshot(post("/api/setup/recommend", &req_body, true)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body = body_json(res).await;
    assert_eq!(body["version"], 1);
    let servers = body["servers"].as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["recommended"]["port"], 8080);
    assert!(servers[0]["rationale"].is_object());
}

// --- 5.18: input validation -----------------------------------------------

#[tokio::test]
async fn validation_rejects_bad_input() {
    let app = setup_router(SetupState::new());

    // Empty directories → 400.
    let res = app
        .clone()
        .oneshot(post(
            "/api/setup/scan-models",
            &json!({ "source": "local", "directories": [] }),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Null byte in extra_paths → 400.
    let res = app
        .clone()
        .oneshot(post("/api/setup/scan-binary", &json!({ "extra_paths": ["/bad\u{0}path"] }), true))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Empty models → 400.
    let res = app
        .clone()
        .oneshot(post("/api/setup/recommend", &json!({ "models": [] }), true))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Missing CSRF header on a mutation → 403.
    let res = app
        .oneshot(post(
            "/api/setup/scan-models",
            &json!({ "source": "local", "directories": ["/tmp"] }),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

// --- 5.19: preview-config + save-config ------------------------------------

#[tokio::test]
#[serial]
async fn preview_and_save_config_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("llamaforge.toml");
    let (model, binary) = real_paths(dir.path());
    let app = setup_router(SetupState::new());

    let request = json!({
        "path": config_path,
        "force": false,
        "servers": [draft(&model, &binary, free_port())],
    });

    // Preview returns TOML, writes nothing.
    let res = app.clone().oneshot(post("/api/setup/preview-config", &request, true)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert_eq!(body["version"], 1);
    assert!(body["toml"].as_str().unwrap().contains("[servers"));
    assert!(!config_path.exists(), "preview must not write the file");

    // Save writes the file.
    let res = app.clone().oneshot(post("/api/setup/save-config", &request, true)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(config_path.exists());

    // Saving again without force → 409.
    let res = app.oneshot(post("/api/setup/save-config", &request, true)).await.unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
}

#[tokio::test]
#[serial]
async fn save_config_unwritable_path_is_forbidden() {
    let dir = tempfile::tempdir().unwrap();
    let (model, binary) = real_paths(dir.path());
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap(); // read+exec, no write

    let app = setup_router(SetupState::new());
    let request = json!({
        "path": locked.join("llamaforge.toml"),
        "force": false,
        "servers": [draft(&model, &binary, free_port())],
    });
    let res = app.oneshot(post("/api/setup/save-config", &request, true)).await.unwrap();

    let _ = fs::set_permissions(&locked, fs::Permissions::from_mode(0o755));

    // Pre-validation passes (real files, free port); unwritable dir → 403.
    // Root may bypass the permission; tolerate that.
    if res.status() != StatusCode::OK {
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }
}

// --- 5.19b: pre-validation failure → 422 with details ---------------------

#[tokio::test]
#[serial]
async fn save_config_pre_validation_returns_422_with_details() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("llamaforge.toml");
    let app = setup_router(SetupState::new());

    // Nonexistent binary + model paths → pre-validation collects both failures.
    let request = json!({
        "path": config_path,
        "force": false,
        "servers": [draft("/nope/model.gguf", "/nope/llama-server", free_port())],
    });
    let res = app.oneshot(post("/api/setup/save-config", &request, true)).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body = body_json(res).await;
    assert_eq!(body["error"], "pre_validation_failed");
    let details = body["details"].as_array().unwrap();
    assert!(details.iter().any(|d| d["field"] == "model_path"));
    assert!(details.iter().any(|d| d["field"] == "binary_path"));
    assert!(!config_path.exists(), "must not write on pre-validation failure");
}

// --- 5.20: session correlation header is accepted --------------------------

#[tokio::test]
async fn session_id_header_is_accepted() {
    let app = setup_router(SetupState::new());
    let req = Request::builder()
        .method("GET")
        .uri("/api/setup/hardware")
        .header("x-setup-session-id", "sess-abc123")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

// --- #273: reconfigure exempts the active config's own ports ----------------

#[tokio::test]
#[serial]
async fn reconfigure_save_exempts_active_config_ports() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("llamaforge.toml");
    let (model, binary) = real_paths(dir.path());
    let port = free_port();
    let app = setup_router(SetupState::new());

    let request = json!({
        "path": config_path,
        "force": true,
        "servers": [draft(&model, &binary, port)],
    });

    // First save while the port is free → writes the active config.
    let res = app.clone().oneshot(post("/api/setup/save-config", &request, true)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Hold the port the way the running server would, so is_port_available == false.
    let _held = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();

    // Re-saving the same active server must NOT 422 on its own in-use port (#273).
    let res = app.oneshot(post("/api/setup/save-config", &request, true)).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "a port held by the active config must be exempt from the availability probe"
    );
}

#[tokio::test]
#[serial]
async fn save_config_still_rejects_in_use_port_for_a_new_server() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("llamaforge.toml");
    let (model, binary) = real_paths(dir.path());
    // Hold the listener for the whole test (rather than free_port()'s bind-then-
    // release) so the port is *continuously* taken — no TOCTOU window for another
    // test/process to grab the just-freed ephemeral port. No existing config on
    // disk → the port is genuinely unavailable → 422.
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let app = setup_router(SetupState::new());

    let request =
        json!({ "path": config_path, "force": false, "servers": [draft(&model, &binary, port)] });
    let res = app.oneshot(post("/api/setup/save-config", &request, true)).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
