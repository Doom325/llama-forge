//! Regression test for #101 — non-streaming `/v1/chat/completions`
//! returned empty `content` because the adapter forwarded the client's
//! `stream: false` flag verbatim to the backend, which then returned a
//! unary JSON body that the SSE-only dispatch proxy dropped.
//!
//! Per design.md §7 the gateway always streams from the backend and
//! aggregates at the shim boundary. This test drives the orchestrator
//! end-to-end with `stream: false` and asserts the aggregated
//! `choices[0].message.content` is non-empty (mock-llama emits three
//! `tok<n>` chunks by default).

use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_openai_chat_non_streaming_returns_content() {
    let orch = Harness::start_full().await.expect("orchestrator start");

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "dummy",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": false,
    });

    let res = client
        .post(format!("{}/v1/chat/completions", orch.http_url))
        .json(&body)
        .send()
        .await
        .expect("send chat completion");
    assert_eq!(res.status(), 200, "expected 200, got {}", res.status());

    let payload: serde_json::Value = res.json().await.expect("response body is JSON");

    let content = payload["choices"][0]["message"]["content"]
        .as_str()
        .expect("choices[0].message.content is a string");
    assert!(
        !content.is_empty(),
        "expected non-empty content from aggregated stream, got {:?}",
        payload
    );
    // mock-llama emits tok0/tok1/tok2 by default; the shim aggregator
    // joins them into a single content string.
    assert!(content.contains("tok"), "expected `tok` token in content, got {:?}", content);

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
