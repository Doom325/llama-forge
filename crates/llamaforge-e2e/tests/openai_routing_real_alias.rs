//! Routing keystone — success branch.
//!
//! Verifies the HTTP shim's route validator sees the real model
//! alias from the request body. With a server configured under
//! alias `real-alias` and the request sending `model: "real-alias"`,
//! admission must succeed and the request must produce content.

use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_routing_uses_real_model_alias() {
    let orch = Harness::start_full_with_alias("real-alias").await.expect("orchestrator start");

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "real-alias",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": false,
    });

    let res = client
        .post(format!("{}/v1/chat/completions", orch.http_url))
        .json(&body)
        .send()
        .await
        .expect("send chat completion");
    assert_eq!(
        res.status(),
        200,
        "expected 200 against a real configured alias, got {}: {}",
        res.status(),
        res.text().await.unwrap_or_default()
    );

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
