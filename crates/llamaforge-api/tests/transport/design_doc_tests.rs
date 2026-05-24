#[path = "mocks.rs"]
mod mocks;

use llamaforge_api::transport::{start_servers, TransportDeps};
use llamaforge_core::config::{
    source::memory::InMemoryConfigSource, AppConfig, ConfigHandle, GlobalConfig,
};
use llamaforge_metrics::InMemoryStore;
use reqwest::{Client, StatusCode};
use std::sync::Arc;

use llamaforge_core::types::{
    traits::{
        AdmissionError, IncomingRequest, QueueError, QueueSnapshot, QueueSubmitter, QueueTicket,
        RouteValidator,
    },
    ChatCompletionChunk,
};

/// Bind on ephemeral ports; tests read `handles.http_addr` /
/// `handles.grpc_addr` to format URLs.
async fn setup_test_server(
    config_modifier: impl FnOnce(&mut AppConfig),
    route_validator: Arc<dyn RouteValidator>,
    queue_submitter: Arc<dyn QueueSubmitter>,
    inference_engine: Arc<dyn llamaforge_api::transport::InferenceEngine>,
) -> llamaforge_api::transport::TransportHandles {
    let mut config = AppConfig { global: GlobalConfig::default(), ..Default::default() };
    config.global.http_port = 0;
    config.global.grpc_port = 0;
    config_modifier(&mut config);

    let source = Arc::new(InMemoryConfigSource::new(config));
    let handle = Arc::new(ConfigHandle::load(source).unwrap());

    let deps = TransportDeps {
        config_reader: handle.clone(),
        route_validator,
        inference_engine,
        metrics_exporter: Arc::new(mocks::MockMetricsExporter),
        server_control: Arc::new(mocks::MockServerControl),
        queue_submitter,
        config_writer: handle,
        request_metrics_store: Arc::new(InMemoryStore::default()),
        store_maintenance: Arc::new(InMemoryStore::default()),
    };

    let handles = start_servers(deps).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    handles
}

fn default_deps() -> (
    Arc<dyn RouteValidator>,
    Arc<dyn QueueSubmitter>,
    Arc<dyn llamaforge_api::transport::InferenceEngine>,
) {
    (
        Arc::new(mocks::MockRouteValidator),
        Arc::new(mocks::MockQueueSubmitter),
        // Default mock engine emits a terminal `Stop` so the
        // non-streaming aggregator's `saw_stop` warming-window guard
        // doesn't 503 fixtures whose only intent is exercising the
        // HTTP surface (CORS, status codes, header validation).
        // Per-test overrides supply their own chunks where the stream
        // shape matters.
        Arc::new(mocks::MockEngine {
            chunks: std::sync::Mutex::new(vec![Ok(ChatCompletionChunk::Stop {
                finish_reason: "stop".to_string(),
                output_tokens: 0,
                prompt_tokens: 0,
            })]),
            sleep_ms: 0,
        }),
    )
}

fn http_url(handles: &llamaforge_api::transport::TransportHandles, path: &str) -> String {
    format!("http://{}{}", handles.http_addr, path)
}

#[tokio::test]
async fn test_openai_null_max_tokens_accepted() {
    let (rv, qs, ie) = default_deps();
    let handles = setup_test_server(|_| {}, rv, qs, ie).await;

    let client = Client::new();
    let res = client
        .post(http_url(&handles, "/v1/chat/completions"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": null
        }))
        .send()
        .await
        .unwrap();

    if res.status() != StatusCode::OK {
        panic!("Status: {}, Body: {}", res.status(), res.text().await.unwrap());
    }
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_openai_logit_bias_negative_keys() {
    let (rv, qs, ie) = default_deps();
    let handles = setup_test_server(|_| {}, rv, qs, ie).await;

    let client = Client::new();
    let res = client
        .post(http_url(&handles, "/v1/chat/completions"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "logit_bias": {
                "-1": 5.0,
                "123": -2.0
            }
        }))
        .send()
        .await
        .unwrap();

    if res.status() != StatusCode::OK {
        panic!("Status: {}, Body: {}", res.status(), res.text().await.unwrap());
    }
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_anthropic_state_machine_ordering() {
    let chunks = vec![
        Ok(ChatCompletionChunk::Token("Tok1".into())),
        Ok(ChatCompletionChunk::Token("Tok2".into())),
        Ok(ChatCompletionChunk::Token("Tok3".into())),
        Ok(ChatCompletionChunk::Stop {
            finish_reason: "stop".to_string(),
            output_tokens: 0,
            prompt_tokens: 0,
        }),
    ];
    let ie = Arc::new(mocks::MockEngine { chunks: std::sync::Mutex::new(chunks), sleep_ms: 0 });
    let (rv, qs, _) = default_deps();
    let handles = setup_test_server(|_| {}, rv, qs, ie).await;

    let client = Client::new();
    let res = client
        .post(http_url(&handles, "/v1/messages"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 100,
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    if res.status() != StatusCode::OK {
        panic!("Status: {}, Body: {}", res.status(), res.text().await.unwrap());
    }
    assert_eq!(res.status(), StatusCode::OK);

    let body_str = res.text().await.unwrap();

    let message_start_idx = body_str.find("message_start").expect("Missing message_start");
    let content_block_start_idx =
        body_str.find("content_block_start").expect("Missing content_block_start");
    let content_block_delta_idx1 = body_str.find("Tok1").expect("Missing Tok1");
    let content_block_delta_idx2 = body_str.find("Tok2").expect("Missing Tok2");
    let content_block_delta_idx3 = body_str.find("Tok3").expect("Missing Tok3");
    let content_block_stop_idx =
        body_str.find("content_block_stop").expect("Missing content_block_stop");
    let message_delta_idx = body_str.find("message_delta").expect("Missing message_delta");
    let message_stop_idx = body_str.find("message_stop").expect("Missing message_stop");

    assert!(message_start_idx < content_block_start_idx);
    assert!(content_block_start_idx < content_block_delta_idx1);
    assert!(content_block_delta_idx1 < content_block_delta_idx2);
    assert!(content_block_delta_idx2 < content_block_delta_idx3);
    assert!(content_block_delta_idx3 < content_block_stop_idx);
    assert!(content_block_stop_idx < message_delta_idx);
    assert!(message_delta_idx < message_stop_idx);
}

#[tokio::test]
async fn test_anthropic_no_token_before_preamble() {
    let chunks = vec![Ok(ChatCompletionChunk::Token("Tok1".into()))];
    let ie = Arc::new(mocks::MockEngine { chunks: std::sync::Mutex::new(chunks), sleep_ms: 0 });
    let (rv, qs, _) = default_deps();
    let handles = setup_test_server(|_| {}, rv, qs, ie).await;

    let client = Client::new();
    let res = client
        .post(http_url(&handles, "/v1/messages"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 100,
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    let body_str = res.text().await.unwrap();
    let lines: Vec<&str> = body_str.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines[0].starts_with("event: message_start"));
}

struct MockRouteValidatorRejects;
impl RouteValidator for MockRouteValidatorRejects {
    fn validate_routable(
        &self,
        _model: &llamaforge_core::types::ModelTarget,
        _capabilities: &llamaforge_core::types::RequestCapabilities,
    ) -> Result<(), AdmissionError> {
        Err(AdmissionError::QueueFull)
    }
}

struct MockQueueSubmitterFull;
impl QueueSubmitter for MockQueueSubmitterFull {
    fn submit(&self, _req: IncomingRequest) -> Result<QueueTicket, AdmissionError> {
        Err(AdmissionError::QueueFull)
    }
    fn cancel(&self, _id: &llamaforge_core::types::RequestId) -> Result<(), QueueError> {
        Ok(())
    }
    fn status(&self) -> Result<QueueSnapshot, QueueError> {
        Ok(QueueSnapshot { depth: 100, estimated_throughput_req_sec: 10.0 })
    }
}

#[tokio::test]
async fn test_queue_full_returns_429_with_retry_after() {
    let rv = Arc::new(MockRouteValidatorRejects);
    let qs = Arc::new(MockQueueSubmitterFull);
    let (_, _, ie) = default_deps();
    let handles = setup_test_server(|c| c.global.queue_default_timeout_secs = 1, rv, qs, ie).await;

    let client = Client::new();
    let res = client
        .post(http_url(&handles, "/v1/chat/completions"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);

    let retry_after = res.headers().get("retry-after").unwrap().to_str().unwrap();
    assert_eq!(retry_after, "10");
}

#[tokio::test]
async fn test_auth_required_when_configured() {
    let (rv, qs, ie) = default_deps();
    let handles =
        setup_test_server(|c| c.global.api_key = Some("secret123".into()), rv, qs, ie).await;

    let client = Client::new();
    let res = client
        .post(http_url(&handles, "/v1/chat/completions"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_prometheus_endpoint_always_200() {
    let (rv, qs, ie) = default_deps();
    let handles = setup_test_server(|_| {}, rv, qs, ie).await;

    let client = Client::new();
    let res = client.get(http_url(&handles, "/metrics")).send().await.unwrap();

    if res.status() != StatusCode::OK {
        panic!("Status: {}, Body: {}", res.status(), res.text().await.unwrap());
    }
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_streaming_per_chunk_timeout_closes_stalled_upstream() {
    // Regression guard for issue #191: when an upstream stall exceeds
    // `queue_default_timeout_secs`, the streaming path must close the
    // SSE body with a `type: "timeout"` event instead of holding the
    // connection open indefinitely.
    //
    // Mock upstream sleeps 2s before emitting its first chunk; the
    // configured per-chunk timeout is 1s — so the timeout must fire
    // first and the chunk must never reach the client.
    let chunks = vec![Ok(ChatCompletionChunk::Token("Tok1".into()))];
    let ie = Arc::new(mocks::MockEngine { chunks: std::sync::Mutex::new(chunks), sleep_ms: 2000 });
    let (rv, qs, _) = default_deps();

    let handles = setup_test_server(|c| c.global.queue_default_timeout_secs = 1, rv, qs, ie).await;

    let client = Client::new();
    let res = client
        .post(http_url(&handles, "/v1/chat/completions"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 100,
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK, "SSE upgrade must succeed before timeout fires");
    let body_str = res.text().await.unwrap();
    assert!(
        !body_str.contains("Tok1"),
        "stalled chunk must not have been forwarded; body was: {body_str}"
    );
    assert!(
        body_str.contains("\"type\":\"timeout\""),
        "expected per-chunk timeout error event in body; was: {body_str}"
    );
}
