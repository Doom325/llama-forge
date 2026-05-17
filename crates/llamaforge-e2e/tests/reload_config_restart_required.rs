//! `POST /api/config/reload` with a restart-required diff must reject
//! with `412 Precondition Failed`. v1.0 doesn't ship the staged-apply
//! protocol; the operator must restart. See `docs/v1.1-deferred.md`.

use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_reload_restart_required_returns_412() {
    let orch = Harness::start_full().await.expect("orchestrator start");
    let client = reqwest::Client::new();

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

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
