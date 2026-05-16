//! Non-streaming `/v1/chat/completions` end-to-end coverage.
//!
//! Asserts the gateway's shim-aggregation contract from design.md §7:
//! when the client sends `stream: false` we still stream from the
//! backend (the dispatch proxy is SSE-only) and aggregate at the shim
//! boundary into a complete OpenAI-shaped response. Without this the
//! adapter previously forwarded `stream: false` upstream, the backend
//! returned a unary JSON body, and the SSE parser dropped it —
//! surfacing as `content: ""`.
//!
//! Also asserts the response carries the full OpenAI non-streaming
//! shape (`object`, `created`, `model`, `choices[].index`,
//! `choices[].finish_reason`, `usage`) — the official Python/JS SDKs
//! treat `finish_reason` and `usage` as non-optional.

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

    // OpenAI non-streaming shape: `object`, `created`, `model`,
    // `choices[].index`, `choices[].finish_reason`, and `usage` are
    // all required by the SDK types.
    assert_eq!(payload["object"].as_str(), Some("chat.completion"));
    assert!(
        payload["created"].as_i64().is_some_and(|c| c > 0),
        "created should be a unix ts, got {:?}",
        payload["created"]
    );
    assert_eq!(payload["model"].as_str(), Some("dummy"));
    assert_eq!(payload["choices"][0]["index"].as_u64(), Some(0));
    assert_eq!(payload["choices"][0]["finish_reason"].as_str(), Some("stop"));
    let usage = &payload["usage"];
    assert!(usage["prompt_tokens"].as_u64().is_some(), "usage.prompt_tokens missing");
    let completion_tokens =
        usage["completion_tokens"].as_u64().expect("usage.completion_tokens present");
    assert!(completion_tokens > 0, "expected completion_tokens > 0, got {}", completion_tokens);
    let total_tokens = usage["total_tokens"].as_u64().expect("usage.total_tokens present");
    assert_eq!(
        total_tokens,
        usage["prompt_tokens"].as_u64().unwrap() + completion_tokens,
        "total_tokens should equal prompt_tokens + completion_tokens"
    );

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
