//! `POST /api/config/reload` with a hot-reloadable diff (alias).
//!
//! Before this PR the reload broadcast on a channel nobody subscribed
//! to — the diff applied to the snapshot but the running fleet never
//! saw it. With the diff forwarder wired in `serve.rs` (and mirrored
//! in the e2e harness), the alias change must propagate to
//! `/api/servers` after a 200 reload.

use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_reload_alias_change_succeeds() {
    let orch = Harness::start_full_with_alias("mock").await.expect("orchestrator start");
    let client = reqwest::Client::new();

    let original = std::fs::read_to_string(&orch.config_path).unwrap();
    let rewritten = original.replace(r#"alias = "mock""#, r#"alias = "mock-renamed""#);
    assert_ne!(original, rewritten, "config should have changed");
    std::fs::write(&orch.config_path, &rewritten).unwrap();

    let res = client
        .post(format!("{}/api/config/reload", orch.http_url))
        .send()
        .await
        .expect("send reload");
    assert_eq!(res.status(), 200, "alias is hot-reloadable; reload must succeed");

    // The diff forwarder applies asynchronously; poll briefly.
    let mut applied = false;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let listing: serde_json::Value = client
            .get(format!("{}/api/servers", orch.http_url))
            .send()
            .await
            .expect("send list")
            .json()
            .await
            .expect("json");
        let aliases: Vec<String> = listing
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v["alias"].as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        if aliases.iter().any(|a| a == "mock-renamed") {
            applied = true;
            break;
        }
    }
    assert!(applied, "alias change must propagate to /api/servers after reload");

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
