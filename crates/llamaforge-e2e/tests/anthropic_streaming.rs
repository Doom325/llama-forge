//! 10.3.b — Anthropic `/v1/messages` streaming through the orchestrator.
//! Asserts event ordering: `message_start` is first, `content_block_start`
//! precedes every `content_block_delta`, and `message_stop` is last. The
//! G-11 invariant — no `content_block_delta` before a
//! `content_block_start` — must hold.

use futures_util::StreamExt;
use llamaforge_e2e::Harness;
use std::time::Duration;

/// Per-chunk read timeout. Without this the test hangs indefinitely if the
/// orchestrator's SSE writer fails to propagate stream close (issue #191).
/// 30s is generous — under normal load chunks arrive in <1s.
const SSE_READ_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn test_e2e_anthropic_messages_streaming() {
    let orch = Harness::start_full().await.expect("orchestrator start");

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "mock",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": true,
        "max_tokens": 16,
    });

    let res = client
        .post(format!("{}/v1/messages", orch.http_url))
        .json(&body)
        .send()
        .await
        .expect("send messages");
    assert_eq!(res.status(), 200, "expected 200, got {}", res.status());

    let mut events: Vec<String> = Vec::new();
    let mut stream = res.bytes_stream();
    let mut buffer = String::new();
    loop {
        let next = tokio::time::timeout(SSE_READ_TIMEOUT, stream.next())
            .await
            .unwrap_or_else(|_| panic!(
                "SSE stream did not produce a chunk within {:?}; events so far: {:?}",
                SSE_READ_TIMEOUT, events,
            ));
        let Some(chunk) = next else { break };
        let bytes = chunk.expect("stream chunk");
        buffer.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(idx) = buffer.find("\n\n") {
            let block: String = buffer.drain(..idx + 2).collect();
            // SSE event blocks may carry both an `event:` and a `data:` line.
            // We track by the `event:` value where present, falling back to
            // the JSON `type` field.
            let mut event_name: Option<String> = None;
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("event: ") {
                    event_name = Some(rest.trim().to_string());
                } else if let Some(rest) = line.strip_prefix("data: ") {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest.trim()) {
                        if event_name.is_none() {
                            if let Some(t) = v["type"].as_str() {
                                event_name = Some(t.to_string());
                            }
                        }
                    }
                }
            }
            if let Some(name) = event_name {
                events.push(name);
            }
        }
    }

    assert!(!events.is_empty(), "expected at least one Anthropic event");
    // `ping` events are Anthropic's stream keepalive and may appear
    // anywhere; ignore them when checking the protocol-mandated event
    // skeleton.
    let non_ping: Vec<&str> =
        events.iter().filter(|e| e.as_str() != "ping").map(String::as_str).collect();
    assert_eq!(
        non_ping.first().copied(),
        Some("message_start"),
        "first non-ping event must be message_start, got {:?}",
        events
    );
    assert_eq!(
        non_ping.last().copied(),
        Some("message_stop"),
        "last non-ping event must be message_stop, got {:?}",
        events
    );

    // G-11: every content_block_delta must follow a content_block_start.
    let mut block_open = false;
    let mut saw_delta = false;
    for e in &events {
        match e.as_str() {
            "content_block_start" => block_open = true,
            "content_block_stop" => block_open = false,
            "content_block_delta" => {
                assert!(
                    block_open,
                    "content_block_delta arrived without preceding content_block_start: {:?}",
                    events
                );
                saw_delta = true;
            }
            _ => {}
        }
    }
    assert!(saw_delta, "expected at least one content_block_delta event");

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
