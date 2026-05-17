//! Routing keystone — rejection branch.
//!
//! Unknown model name must surface as `404 model_not_found` (the
//! OpenAI / Anthropic SDK contract), not `429 queue_full` which
//! triggers SDK retry.

use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_unknown_model_returns_404_not_429() {
    let orch = Harness::start_full().await.expect("orchestrator start");

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "no-such-alias",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": false,
    });

    let res = client
        .post(format!("{}/v1/chat/completions", orch.http_url))
        .json(&body)
        .send()
        .await
        .expect("send chat completion");

    let status = res.status();
    let body: serde_json::Value = res.json().await.expect("response body is JSON");

    assert_eq!(status, 404, "expected 404 for unknown model, got {} (body={})", status, body);
    assert_eq!(
        body["error"].as_str(),
        Some("model_not_found"),
        "expected `model_not_found` error code, got {}",
        body
    );

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
