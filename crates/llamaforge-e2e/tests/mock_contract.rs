//! Verifies the `mock-llama` server honours the llama.cpp endpoints
//! LlamaForge depends on. PR9b uses these as the ground-truth contract
//! against which real `SlotErasure` lands.

use llamaforge_e2e::Harness;
use std::time::Duration;

#[tokio::test]
async fn mock_serves_health_slots_and_completion() {
    let h = Harness::start().await.expect("harness start");
    let base = h.mock_base_url();
    let client = reqwest::Client::new();

    let health = client.get(format!("{}/health", base)).send().await.unwrap();
    assert_eq!(health.status(), 200);

    let slots = client.get(format!("{}/slots", base)).send().await.unwrap();
    assert_eq!(slots.status(), 200);
    let body: serde_json::Value = slots.json().await.unwrap();
    assert!(body.is_array());

    let comp = client
        .post(format!("{}/completion", base))
        .json(&serde_json::json!({
            "prompt": "hi",
            "stream": false,
            "id_slot": 0,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(comp.status(), 200);
    let v: serde_json::Value = comp.json().await.unwrap();
    assert_eq!(v.get("stop").and_then(|x| x.as_bool()), Some(true));

    llamaforge_e2e::shutdown(h).await;
}

#[tokio::test]
async fn slot_erase_works_and_is_counted() {
    let h = Harness::start().await.expect("harness start");
    let base = h.mock_base_url();
    let client = reqwest::Client::new();

    // Populate a slot via a completion.
    let _ = client
        .post(format!("{}/completion", base))
        .json(&serde_json::json!({"prompt":"x","stream":false,"id_slot":0}))
        .send()
        .await
        .unwrap();

    // Erase it.
    let res = client.post(format!("{}/slots/0?action=erase", base)).send().await.unwrap();
    assert_eq!(res.status(), 200);

    // /slots should no longer list slot 0.
    let slots: serde_json::Value =
        client.get(format!("{}/slots", base)).send().await.unwrap().json().await.unwrap();
    let arr = slots.as_array().unwrap();
    assert!(!arr.iter().any(|v| v.get("id") == Some(&serde_json::json!(0))));
    assert_eq!(h.mock.erase_call_count(), 1);

    llamaforge_e2e::shutdown(h).await;
}

#[tokio::test]
async fn slot_erase_with_bad_action_400s() {
    let h = Harness::start().await.expect("harness start");
    let client = reqwest::Client::new();
    let res =
        client.post(format!("{}/slots/0?action=bogus", h.mock_base_url())).send().await.unwrap();
    assert_eq!(res.status(), 400);
    llamaforge_e2e::shutdown(h).await;
}

#[tokio::test]
async fn health_endpoint_responds_quickly() {
    let h = Harness::start().await.expect("harness start");
    let url = format!("{}/health", h.mock_base_url());
    llamaforge_e2e::wait_for_http_200(&url, Duration::from_secs(2)).await.unwrap();
    llamaforge_e2e::shutdown(h).await;
}
