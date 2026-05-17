//! `GET /v1/models` — OpenAI SDK discovery surface.
//!
//! Returns the list envelope `{"object":"list","data":[{"id": "<alias>", ...}]}`
//! sourced from the config snapshot's aliased servers. SDK clients
//! probe this endpoint at construction time to validate the
//! configured model.

use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_openai_v1_models_lists_configured_aliases() {
    let orch = Harness::start_full_with_alias("mock").await.expect("orchestrator start");
    let client = reqwest::Client::new();

    let payload: serde_json::Value = client
        .get(format!("{}/v1/models", orch.http_url))
        .send()
        .await
        .expect("send /v1/models")
        .json()
        .await
        .expect("json");

    assert_eq!(payload["object"].as_str(), Some("list"));
    let data = payload["data"].as_array().expect("data array");
    let ids: Vec<&str> = data.iter().filter_map(|v| v["id"].as_str()).collect();
    assert!(ids.contains(&"mock"), "expected `mock` alias in /v1/models, got {:?}", ids);
    for entry in data {
        assert_eq!(entry["object"].as_str(), Some("model"));
        assert_eq!(entry["owned_by"].as_str(), Some("llamaforge"));
        assert!(entry["created"].is_number(), "`created` must be a unix ts, got {:?}", entry);
    }

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
