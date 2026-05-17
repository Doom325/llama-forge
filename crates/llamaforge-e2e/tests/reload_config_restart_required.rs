//! `POST /api/config/reload` with a restart-required diff must reject
//! with `412 Precondition Failed` **and** leave the running fleet
//! unchanged. v1.0 doesn't ship the staged-apply protocol; the gate
//! inside `ConfigHandle::reload` aborts before the snapshot swap, so
//! the server-manager never sees the diff. This test verifies both
//! the 412 status *and* the no-side-effect invariant — the prior
//! version of this test only asserted the 412 and passed both with
//! and without the bug.

use llamaforge_e2e::Harness;

async fn list_servers(http_url: &str, client: &reqwest::Client) -> serde_json::Value {
    client
        .get(format!("{}/api/servers", http_url))
        .send()
        .await
        .expect("send list")
        .json()
        .await
        .expect("json")
}

#[tokio::test]
async fn test_e2e_reload_restart_required_returns_412_and_no_side_effects() {
    let orch = Harness::start_full().await.expect("orchestrator start");
    let client = reqwest::Client::new();

    // Snapshot the running fleet *before* the reload attempt so we
    // can assert no respawn happens.
    let before = list_servers(&orch.http_url, &client).await;
    let before_started_at: Vec<Option<String>> = before
        .as_array()
        .map(|arr| arr.iter().map(|v| v["started_at"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let before_state: Vec<String> = before
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v["state"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    assert!(
        before_state.iter().any(|s| s == "Running"),
        "fixture should have at least one Running server before reload, got {:?}",
        before_state
    );

    let original = std::fs::read_to_string(&orch.config_path).unwrap();
    let rewritten = original.replace("ctx_size = 1024", "ctx_size = 2048");
    assert_ne!(original, rewritten, "config should have changed");
    std::fs::write(&orch.config_path, &rewritten).unwrap();

    let res = client
        .post(format!("{}/api/config/reload", orch.http_url))
        .send()
        .await
        .expect("send reload");

    assert_eq!(res.status(), 412, "ctx_size is restart-required; reload must 412");
    let body: serde_json::Value = res.json().await.expect("json");
    assert_eq!(body["error"].as_str(), Some("precondition_failed"));

    // Give any (incorrectly-routed) diff-forwarder enough time to
    // respawn the server before we assert the invariants. If the
    // gate is wrong, the diff would have already reached
    // `apply_config_diff` by now.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let after = list_servers(&orch.http_url, &client).await;
    let after_started_at: Vec<Option<String>> = after
        .as_array()
        .map(|arr| arr.iter().map(|v| v["started_at"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let after_state: Vec<String> = after
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v["state"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    assert_eq!(
        before_started_at, after_started_at,
        "started_at must be identical pre/post reload — a change implies a respawn"
    );
    assert_eq!(
        before_state, after_state,
        "lifecycle states must not transition through Stopped on a rejected reload"
    );

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
