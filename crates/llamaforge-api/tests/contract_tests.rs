//! Contract tests (08_SETUP_WIZARD.md Section 10.7, PLAN PR12): pin the JSON
//! shape — field names, types, and nesting — of every `/api/setup/*` response
//! and each build SSE event, so a backend change that would break the frontend
//! types (`ui/src/lib/api/types.ts`) fails here.
//!
//! ts-rs decision (task 12.9): we deliberately do NOT derive TypeScript types
//! from the Rust structs via `ts-rs`. The setup response types are small, stable,
//! and already hand-written on the frontend; adding a build-time codegen dep +
//! `#[derive(TS)]` on every struct (and wiring its generation into CI) is more
//! machinery than the contract warrants. These shape assertions pin the same
//! field-name/type/nesting contract that ts-rs would, with no new dependency.
//!
//! TS type sync (task 12.10): with ts-rs deferred, **this file is the
//! source of truth for the `/api/setup/*` contract**. Each test below is
//! annotated `// pins ui types.ts: <TypeName>` naming the TypeScript type
//! (`ui/src/lib/api/types.ts` / the `queries.ts` response interfaces) it pins.
//! When a backend response struct changes, the matching test fails here, which
//! is the signal to update the corresponding TS type. There is no automated
//! cross-language diff (would require parsing TS); the annotations make the
//! one-to-one mapping explicit and greppable instead.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use axum::body::Body;
use axum::http::Request;
use llamaforge_api::{setup_router, SetupState};
use llamaforge_server::{BuildComplete, BuildEvent, BuildFailed, BuildPhase};
use serde_json::{json, Value};
use tower::ServiceExt;

const CSRF: (&str, &str) = ("x-llamaforge-request", "1");

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap()
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

/// Assert an object has *exactly* the given keys (pins names + that nothing
/// extra leaked in). `optional` keys may or may not be present.
fn assert_object_keys(value: &Value, required: &[&str], optional: &[&str]) {
    let obj = value.as_object().unwrap_or_else(|| panic!("expected object, got {value}"));
    for key in required {
        assert!(obj.contains_key(*key), "missing required key `{key}` in {value}");
    }
    for key in obj.keys() {
        let known = required.contains(&key.as_str()) || optional.contains(&key.as_str());
        assert!(known, "unexpected key `{key}` in {value}");
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Real model + binary files so save/preview pre-validation (which checks the
/// paths exist) passes. Returns a server-config-draft JSON value.
fn server_draft(dir: &Path) -> Value {
    let model = dir.join("model.gguf");
    let binary = dir.join("llama-server");
    fs::write(&model, b"GGUF\x00").unwrap();
    fs::write(&binary, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    json!({
        "model_path": model.display().to_string(),
        "alias": "test-model",
        "binary_path": binary.display().to_string(),
        "port": free_port(),
        "gpu_layers": -1,
        "parallel_slots": 2,
        "ctx_size": 8192,
        "cache_type_k": "q8_0",
        "cache_type_v": "q8_0",
        "flash_attn": "auto",
        "estimated_vram_mb": 6000
    })
}

// --- Hardware -------------------------------------------------------------

// pins ui types.ts: HardwareInfo (GpuInfo, CpuInfo)
#[tokio::test]
async fn hardware_response_shape() {
    let app = setup_router(SetupState::new());
    let body = body_json(app.oneshot(get("/api/setup/hardware")).await.unwrap()).await;

    assert_object_keys(&body, &["version", "gpus", "cpu", "ram_mb", "recommended_backend"], &[]);
    assert_eq!(body["version"], 1);
    assert!(body["gpus"].is_array());
    assert!(body["ram_mb"].is_number());
    assert!(body["recommended_backend"].is_string());
    assert_object_keys(&body["cpu"], &["name", "cores_physical", "cores_logical"], &[]);
    assert!(body["cpu"]["cores_physical"].is_number());
    for gpu in body["gpus"].as_array().unwrap() {
        assert_object_keys(gpu, &["index", "name", "vram_mb"], &[]);
        assert!(gpu["vram_mb"].is_number());
    }
}

// --- Binary scanner (both variants of the discriminated union) ------------

// pins ui types.ts: BinaryScanResult (found variant)
#[tokio::test]
async fn scan_binary_found_shape() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("llama-server");
    fs::write(&script, "#!/bin/sh\necho 'b1234'\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let app = setup_router(SetupState::new());
    let body = body_json(
        app.oneshot(post("/api/setup/scan-binary", &json!({ "extra_paths": [dir.path()] })))
            .await
            .unwrap(),
    )
    .await;

    assert_object_keys(&body, &["version", "found", "path", "binary_version", "backends"], &[]);
    assert_eq!(body["found"], true);
    assert!(body["path"].is_string());
    assert!(body["binary_version"].is_string());
    assert!(body["backends"].is_array());
}

// pins ui types.ts: BinaryScanResult (not-found variant)
#[tokio::test]
async fn scan_binary_not_found_shape() {
    let dir = tempfile::tempdir().unwrap();
    let app = setup_router(SetupState::new());
    // An empty dir with PATH search yielding nothing → the not-found variant.
    let body = body_json(
        app.oneshot(post("/api/setup/scan-binary", &json!({ "extra_paths": [dir.path()] })))
            .await
            .unwrap(),
    )
    .await;

    if body["found"] == json!(false) {
        assert_object_keys(&body, &["version", "found", "searched_paths"], &[]);
        assert!(body["searched_paths"].is_array());
    }
    // (If a real llama-server is on PATH the host returns `found: true`; the
    // found shape is pinned by the test above.)
}

// --- Model scanner --------------------------------------------------------

// pins ui types.ts: ModelScanResult (DiscoveredModel)
#[tokio::test]
async fn scan_models_response_shape() {
    let dir = tempfile::tempdir().unwrap();
    let model = dir.path().join("llama-3-8b-Q4_K_M.gguf");
    fs::write(&model, b"gguf").unwrap();

    let app = setup_router(SetupState::new());
    let body = body_json(
        app.oneshot(post(
            "/api/setup/scan-models",
            &json!({ "source": "local", "directories": [dir.path()] }),
        ))
        .await
        .unwrap(),
    )
    .await;

    assert_object_keys(
        &body,
        &[
            "version",
            "models",
            "scan_duration_ms",
            "files_scanned",
            "truncated",
            "truncation_reason",
            "warnings",
        ],
        &[],
    );
    assert!(body["truncated"].is_boolean());
    assert!(body["warnings"].is_array());
    let models = body["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_object_keys(
        &models[0],
        &["id", "path", "filename", "size_bytes", "quantization_hint", "architecture_hint"],
        &["modified"],
    );
}

// --- Recommendation -------------------------------------------------------

// pins ui types.ts: RecommendationResult (ServerRecommendation, ConfigRecommendation)
#[tokio::test]
async fn recommend_response_shape() {
    let app = setup_router(SetupState::new());
    app.clone().oneshot(get("/api/setup/hardware")).await.unwrap(); // warm cache

    let body = body_json(
        app.oneshot(post(
            "/api/setup/recommend",
            &json!({ "models": [{ "path": "/m/llama.gguf", "size_bytes": 4_400_000_000u64 }] }),
        ))
        .await
        .unwrap(),
    )
    .await;

    assert_object_keys(&body, &["version", "servers", "warnings"], &[]);
    let servers = body["servers"].as_array().unwrap();
    assert_object_keys(&servers[0], &["model_path", "recommended", "rationale"], &[]);
    assert!(servers[0]["rationale"].is_object());
    assert_object_keys(
        &servers[0]["recommended"],
        &[
            "gpu_layers",
            "ctx_size",
            "parallel_slots",
            "cache_type_k",
            "cache_type_v",
            "flash_attn",
            "estimated_vram_mb",
            "port",
        ],
        &[],
    );
}

// --- Preview + Save config (success shapes the wizard's Review step reads) --

// pins ui queries.ts: PreviewConfigResponse
#[tokio::test]
async fn preview_config_response_shape() {
    let dir = tempfile::tempdir().unwrap();
    let app = setup_router(SetupState::new());
    let request = json!({
        "path": dir.path().join("llamaforge.toml"),
        "force": false,
        "servers": [server_draft(dir.path())]
    });
    let body =
        body_json(app.oneshot(post("/api/setup/preview-config", &request)).await.unwrap()).await;
    assert_object_keys(&body, &["version", "toml"], &[]);
    assert!(body["toml"].is_string());
}

// pins ui queries.ts: SaveConfigResponse
#[tokio::test]
async fn save_config_response_shape() {
    let dir = tempfile::tempdir().unwrap();
    let app = setup_router(SetupState::new());
    let request = json!({
        "path": dir.path().join("llamaforge.toml"),
        "force": false,
        "servers": [server_draft(dir.path())]
    });
    let body =
        body_json(app.oneshot(post("/api/setup/save-config", &request)).await.unwrap()).await;
    // backup_path is omitted when no prior file was replaced (skip_serializing_if).
    assert_object_keys(&body, &["version", "path", "replaced_existing"], &["backup_path"]);
    assert!(body["path"].is_string());
    assert!(body["replaced_existing"].is_boolean());
}

// --- Build SSE events (the `{event, data}` shapes the frontend parses) -----

// pins ui types.ts: BuildStatus + buildStatus.ts BuildEventInput
#[test]
fn build_sse_event_shapes() {
    let phase = serde_json::to_value(BuildEvent::Phase {
        phase: BuildPhase::Cloning,
        message: "cloning".into(),
    })
    .unwrap();
    assert_eq!(phase["event"], "phase");
    assert_object_keys(&phase["data"], &["phase", "message"], &[]);

    let progress =
        serde_json::to_value(BuildEvent::Progress { phase: BuildPhase::Building, percent: 50 })
            .unwrap();
    assert_eq!(progress["event"], "progress");
    assert_object_keys(&progress["data"], &["phase", "percent"], &[]);

    let log = serde_json::to_value(BuildEvent::Log { lines: vec!["x".into()] }).unwrap();
    assert_eq!(log["event"], "log");
    assert_object_keys(&log["data"], &["lines"], &[]);

    let complete = serde_json::to_value(BuildEvent::Complete(BuildComplete {
        binary_path: "/x".into(),
        duration_secs: 1,
        log_path: Some("/l".into()),
        replaced_existing: false,
        backup_path: None,
    }))
    .unwrap();
    assert_eq!(complete["event"], "complete");
    assert_object_keys(
        &complete["data"],
        &["binary_path", "duration_secs", "replaced_existing", "backup_path"],
        &["log_path"],
    );

    let failed = serde_json::to_value(BuildEvent::Failed(BuildFailed {
        phase: BuildPhase::Configuring,
        error: "boom".into(),
        last_lines: vec!["e".into()],
        log_path: None,
    }))
    .unwrap();
    assert_eq!(failed["event"], "failed");
    assert_object_keys(&failed["data"], &["phase", "error", "last_lines"], &["log_path"]);
}

// --- Error envelope -------------------------------------------------------

// pins ui api/client.ts: ApiError envelope (error/message)
#[tokio::test]
async fn error_responses_are_json_with_message() {
    let app = setup_router(SetupState::new());
    // Empty models → 422 pre-validation (a representative error path).
    let res = app.oneshot(post("/api/setup/recommend", &json!({ "models": [] }))).await.unwrap();
    assert!(res.status().is_client_error());
    assert_eq!(
        res.headers().get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    let body = body_json(res).await;
    assert!(
        body.get("error").is_some() || body.get("message").is_some(),
        "error response must carry error/message: {body}"
    );
}
