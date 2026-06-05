//! Input-validation boundary tests (08_SETUP_WIZARD.md Section 10.6, PLAN
//! 12.11) for the `/api/setup/*` endpoints: malformed paths, empty inputs, and
//! out-of-range build parameters must be handled gracefully (never a 500, never
//! a panic).
//!
//! The hardware-probe boundary cases (gibberish `nvidia-smi`/`system_profiler`
//! output, multi-GPU, Apple-Silicon unified memory, probe timeout/partial
//! failure) are covered at the source level — they need the injectable
//! `CommandRunner`, which is not reachable through the HTTP surface — in
//! `crates/llamaforge-server/tests/hardware_tests.rs` and `hardware.rs` unit
//! tests (PR1).

#![cfg(unix)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use llamaforge_api::{setup_router, SetupState};
use serde_json::{json, Value};
use tower::ServiceExt;

const CSRF: (&str, &str) = ("x-llamaforge-request", "1");

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
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

// --- Model scanner boundaries ---------------------------------------------

#[tokio::test]
async fn scan_models_empty_directories_is_rejected() {
    // The scanner requires at least one directory (ModelScanError::NoDirectories)
    // rather than silently scanning nothing.
    let app = setup_router(SetupState::new());
    let res = app
        .oneshot(post("/api/setup/scan-models", &json!({ "source": "local", "directories": [] })))
        .await
        .unwrap();
    assert!(res.status().is_client_error(), "got {}", res.status());
}

#[tokio::test]
async fn scan_models_nonexistent_directory_is_graceful() {
    let app = setup_router(SetupState::new());
    let res = app
        .oneshot(post(
            "/api/setup/scan-models",
            &json!({ "source": "local", "directories": ["/no/such/dir/xyzzy"] }),
        ))
        .await
        .unwrap();
    // Missing dirs are skipped, not fatal.
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_json(res).await["models"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn scan_models_path_traversal_is_treated_literally() {
    let app = setup_router(SetupState::new());
    let res = app
        .oneshot(post(
            "/api/setup/scan-models",
            &json!({ "source": "local", "directories": ["../../../../../../etc"] }),
        ))
        .await
        .unwrap();
    // A relative/traversal path is just a path to scan — no escape, no crash.
    assert!(res.status() == StatusCode::OK || res.status().is_client_error());
}

#[tokio::test]
async fn scan_models_null_byte_path_is_handled() {
    let app = setup_router(SetupState::new());
    let res = app
        .oneshot(post(
            "/api/setup/scan-models",
            &json!({ "source": "local", "directories": ["/bad\u{0}path"] }),
        ))
        .await
        .unwrap();
    assert_ne!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

// --- Build parameter boundaries -------------------------------------------

#[tokio::test]
async fn build_invalid_backend_is_rejected() {
    let app = setup_router(SetupState::new());
    let res = app
        .oneshot(post(
            "/api/setup/build",
            &json!({ "backend": "quantum", "install_dir": "/tmp/x", "jobs": 1 }),
        ))
        .await
        .unwrap();
    assert!(res.status().is_client_error(), "got {}", res.status());
}

#[tokio::test]
async fn build_negative_jobs_is_rejected() {
    let app = setup_router(SetupState::new());
    let res = app
        .oneshot(post(
            "/api/setup/build",
            &json!({ "backend": "cpu", "install_dir": "/tmp/x", "jobs": -4 }),
        ))
        .await
        .unwrap();
    assert!(res.status().is_client_error(), "got {}", res.status());
}

// --- Recommendation boundary ----------------------------------------------

#[tokio::test]
async fn recommend_empty_models_is_rejected() {
    let app = setup_router(SetupState::new());
    let res = app.oneshot(post("/api/setup/recommend", &json!({ "models": [] }))).await.unwrap();
    assert!(res.status().is_client_error());
}
