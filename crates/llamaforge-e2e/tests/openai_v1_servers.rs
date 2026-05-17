//! `GET /v1/servers` — operator status surface aliased on top of
//! `/api/servers`. The two endpoints must serve the same payload.

use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_v1_servers_alias_matches_api_servers() {
    let orch = Harness::start_full().await.expect("orchestrator start");
    let client = reqwest::Client::new();

    let v1: serde_json::Value = client
        .get(format!("{}/v1/servers", orch.http_url))
        .send()
        .await
        .expect("send /v1/servers")
        .json()
        .await
        .expect("v1 json");

    let api: serde_json::Value = client
        .get(format!("{}/api/servers", orch.http_url))
        .send()
        .await
        .expect("send /api/servers")
        .json()
        .await
        .expect("api json");

    assert_eq!(v1, api, "/v1/servers and /api/servers must serve the same payload");

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
