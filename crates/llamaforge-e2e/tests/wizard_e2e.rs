//! Setup-wizard end-to-end tests (08_SETUP_WIZARD.md §13, PLAN PR14). Spawns a
//! real `llamaforge serve` with NO config so it boots into first-run setup mode,
//! drives the `/api/setup/*` HTTP surface the wizard uses, and verifies the
//! setup→running transition happens in-place (no restart) — the first-run serve
//! E2E deferred from PR7.
//!
//! First-run defaults to HTTP port 8000, but honours `LLAMAFORGE_HTTP_PORT`
//! (issue #281); this test binds an ephemeral free port so it neither collides on
//! the fixed default nor needs serialization.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::process::Command;

const CSRF: (&str, &str) = ("x-llamaforge-request", "1");

/// An ephemeral port that is free at call time (released immediately, then handed
/// to the spawned server via `LLAMAFORGE_HTTP_PORT`).
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn get_bin(name: &str) -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("target"))
            .unwrap_or_else(|| PathBuf::from("target"))
    });
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    target.join(profile).join(name)
}

/// Write an executable stub that answers `--version` like `llama-server` so the
/// binary scanner reports it as found.
fn fake_llama_server(dir: &Path) -> PathBuf {
    let bin = dir.join("llama-server");
    std::fs::write(&bin, "#!/bin/sh\necho 'b4567'\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

async fn poll_status(client: &reqwest::Client, base: &str, want_setup_required: bool) -> bool {
    for _ in 0..60 {
        if let Ok(resp) = client.get(format!("{base}/api/status")).send().await {
            if let Ok(body) = resp.json::<Value>().await {
                if body["setup_required"] == json!(want_setup_required) {
                    return true;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

#[tokio::test]
async fn test_first_run_setup_to_running_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("llamaforge.toml"); // does NOT exist yet → first-run
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    let bin_dir = tmp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    // A stub that answers `--version` for the binary scanner...
    fake_llama_server(&bin_dir);
    // ...but the server we actually launch + run is `mock-llama`, which serves a
    // real /health endpoint (the fake stub exits immediately and can't pass the
    // validate-launch health check that drives the setup→running transition).
    let binary = get_bin("mock-llama");

    let model_dir = tmp.path().join("models");
    std::fs::create_dir_all(&model_dir).unwrap();
    let model = model_dir.join("llama-3-8b-Q4_K_M.gguf");
    std::fs::write(&model, b"GGUF\x00").unwrap();

    // Boot in first-run setup mode (no config present), on an ephemeral port.
    let port = free_port();
    let base = format!("http://127.0.0.1:{port}");
    let mut cmd = Command::new(get_bin("llamaforge"));
    cmd.arg("serve").arg("--config").arg(&config_path).arg("--state-dir").arg(&state_dir);
    cmd.env("LLAMAFORGE_HTTP_PORT", port.to_string());
    cmd.process_group(0);
    // Kill the server even if an assertion panics, so a failed run can't orphan it.
    cmd.kill_on_drop(true);
    let mut child = cmd.spawn().unwrap();

    let client = reqwest::Client::builder().timeout(Duration::from_secs(35)).build().unwrap();
    assert!(
        poll_status(&client, &base, true).await,
        "server should report setup_required=true on boot"
    );

    // --- Drive the wizard's /api/setup/* surface --------------------------
    let scan_bin: Value = client
        .post(format!("{base}/api/setup/scan-binary"))
        .header(CSRF.0, CSRF.1)
        .json(&json!({ "extra_paths": [bin_dir] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(scan_bin["found"], true, "fake llama-server should be found");

    let _models: Value = client
        .post(format!("{base}/api/setup/scan-models"))
        .header(CSRF.0, CSRF.1)
        .json(&json!({ "source": "local", "directories": [model_dir] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let _rec: Value = client
        .post(format!("{base}/api/setup/recommend"))
        .header(CSRF.0, CSRF.1)
        .json(&json!({ "models": [{ "path": model, "size_bytes": 4_400_000_000u64 }] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let save_req = json!({
        "path": config_path,
        "force": false,
        "servers": [{
            "model_path": model,
            "alias": "llama-3-8b",
            "binary_path": binary,
            "port": 18234,
            "gpu_layers": -1,
            "parallel_slots": 2,
            "ctx_size": 4096,
            "cache_type_k": "f16",
            "cache_type_v": "f16",
            "flash_attn": "auto",
            "estimated_vram_mb": 6000
        }]
    });

    let save = client
        .post(format!("{base}/api/setup/save-config"))
        .header(CSRF.0, CSRF.1)
        .json(&save_req)
        .send()
        .await
        .unwrap();
    assert_eq!(save.status(), reqwest::StatusCode::OK, "save-config should succeed");

    // 14.5 — a second save without force conflicts (the config now exists).
    let conflict = client
        .post(format!("{base}/api/setup/save-config"))
        .header(CSRF.0, CSRF.1)
        .json(&save_req)
        .send()
        .await
        .unwrap();
    assert_eq!(
        conflict.status(),
        reqwest::StatusCode::CONFLICT,
        "second save without force should 409"
    );

    // validate-launch spawns the server and health-checks it; on success it
    // signals setup completion, which drives the transition (§13.5 Step 3).
    let validate: Value = client
        .post(format!("{base}/api/setup/validate-launch"))
        .header(CSRF.0, CSRF.1)
        .json(&json!({ "servers": save_req["servers"] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(validate["success"], true, "validate-launch should pass mock-llama health check");

    // --- The completion signal triggers the in-place setup→running transition.
    assert!(
        poll_status(&client, &base, false).await,
        "server should transition to setup_required=false after launch (no restart)"
    );

    // Clean shutdown.
    std::process::Command::new("kill")
        .arg("-15")
        .arg(child.id().unwrap().to_string())
        .status()
        .unwrap();
    let status =
        tokio::time::timeout(Duration::from_secs(15), child.wait()).await.unwrap().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert!(
        status.success() || status.code() == Some(0) || status.signal() == Some(15),
        "first-run process should exit cleanly on SIGTERM"
    );
}
