//! 10.3.d — graceful shutdown. Issues a streaming `chat_completion`,
//! waits for at least one chunk, then triggers `shutdown_and_wait`.
//! Asserts the in-flight stream completes cleanly and a new POST after
//! shutdown fails to connect.

use futures_util::StreamExt;
use llamaforge_e2e::Harness;
use std::time::Duration;

#[tokio::test]
async fn test_e2e_graceful_shutdown() {
    let orch = Harness::start_full().await.expect("orchestrator start");

    let http_url = orch.http_url.clone();
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "mock",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": true,
    });

    let res = client
        .post(format!("{}/v1/chat/completions", http_url))
        .json(&body)
        .send()
        .await
        .expect("send chat completion");
    assert_eq!(res.status(), 200);

    let mut stream = res.bytes_stream();

    // Wait for the first chunk before triggering shutdown.
    let first =
        tokio::time::timeout(Duration::from_secs(5), stream.next()).await.expect("first chunk");
    let first = first.expect("stream produced something").expect("first chunk ok");
    assert!(!first.is_empty());

    // Trigger shutdown; the in-flight stream must drain cleanly.
    // Per-chunk timeout protects the join-pair against an SSE close-propagation
    // hang on shutdown (issue #191).
    let drain_stream = async {
        loop {
            let next = tokio::time::timeout(Duration::from_secs(30), stream.next())
                .await
                .expect("SSE stream did not drain within 30s during graceful shutdown");
            match next {
                Some(chunk) => {
                    chunk.expect("mid-stream error during graceful drain");
                }
                None => break,
            }
        }
    };
    let shutdown_task = orch.shutdown_and_wait();
    let (_, shutdown_result) = tokio::join!(drain_stream, shutdown_task);
    shutdown_result.expect("clean shutdown");

    // A new request after shutdown must fail to connect.
    let res_new = client
        .post(format!("{}/v1/chat/completions", http_url))
        .json(&body)
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    assert!(res_new.is_err(), "expected connection refused after shutdown, got {:?}", res_new);
}
