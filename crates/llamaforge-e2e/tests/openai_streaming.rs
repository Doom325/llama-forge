//! 10.3.a — OpenAI `/v1/chat/completions` streaming through the
//! orchestrator. Asserts every SSE chunk shares the same `id`, the
//! stream terminates with `data: [DONE]`, at least one
//! `delta.content` chunk arrives before stop, and the final chunk
//! carries `finish_reason: "stop"`.

use futures_util::StreamExt;
use llamaforge_e2e::Harness;
use std::time::Duration;

/// Per-chunk read timeout; mirrors the guard added to
/// `anthropic_streaming.rs` for the same reason (issue #191).
const SSE_READ_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn test_e2e_openai_chat_streaming() {
    let orch = Harness::start_full().await.expect("orchestrator start");

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "mock",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": true,
    });

    let res = client
        .post(format!("{}/v1/chat/completions", orch.http_url))
        .json(&body)
        .send()
        .await
        .expect("send chat completion");
    assert_eq!(res.status(), 200, "expected 200, got {}", res.status());

    let mut id: Option<String> = None;
    let mut saw_done = false;
    let mut saw_content = false;
    let mut final_finish_reason: Option<String> = None;

    let mut stream = res.bytes_stream();
    let mut buffer = String::new();
    loop {
        let next = tokio::time::timeout(SSE_READ_TIMEOUT, stream.next())
            .await
            .unwrap_or_else(|_| panic!(
                "SSE stream did not produce a chunk within {:?}; saw_done={saw_done}, saw_content={saw_content}",
                SSE_READ_TIMEOUT,
            ));
        let Some(chunk) = next else { break };
        let bytes = chunk.expect("stream chunk");
        buffer.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(idx) = buffer.find("\n\n") {
            let event: String = buffer.drain(..idx + 2).collect();
            for line in event.lines() {
                let line = match line.strip_prefix("data: ") {
                    Some(s) => s.trim(),
                    None => continue,
                };
                if line.is_empty() {
                    continue;
                }
                if line == "[DONE]" {
                    saw_done = true;
                    continue;
                }
                let v: serde_json::Value = serde_json::from_str(line).expect("event is JSON");
                let evt_id = v["id"].as_str().expect("id field").to_string();
                match &id {
                    None => id = Some(evt_id),
                    Some(prev) => assert_eq!(prev, &evt_id, "id must be stable across chunks"),
                }
                let choices = &v["choices"];
                if let Some(first) = choices.get(0) {
                    if first["delta"]["content"].as_str().is_some_and(|s| !s.is_empty()) {
                        saw_content = true;
                    }
                    if let Some(fr) = first["finish_reason"].as_str() {
                        final_finish_reason = Some(fr.to_string());
                    }
                }
            }
        }
    }

    assert!(saw_content, "expected at least one delta.content chunk");
    assert!(saw_done, "expected terminal [DONE] marker");
    assert_eq!(
        final_finish_reason.as_deref(),
        Some("stop"),
        "expected final finish_reason = stop, got {:?}",
        final_finish_reason
    );

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
