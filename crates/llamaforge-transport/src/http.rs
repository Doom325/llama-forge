use axum::{
    extract::State,
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post},
    Router,
};
use futures_util::stream::StreamExt;
use llamaforge_config::{ConfigReader, ConfigWriter};
use llamaforge_types::traits::{MetricsExporter, RouteValidator, ServerControl, QueueSubmitter};
use std::sync::Arc;
use tower_http::{
    catch_panic::CatchPanicLayer,
    limit::RequestBodyLimitLayer,
};

use crate::{
    anthropic::{format_anthropic_event, parse_anthropic_request, AnthropicMessagesRequest, AnthropicState},
    error::ApiError,
    middleware::{auth_middleware, large_body_limit_middleware, metrics_middleware, request_id_middleware, LargeBodyConcurrencyLimit},
    openai::{format_openai_chunk, parse_openai_request, OpenAiChatRequest},
    InferenceEngine, TransportError,
};

pub struct HttpAppState<CW> {
    pub config_reader: Arc<dyn ConfigReader>,
    pub route_validator: Arc<dyn RouteValidator>,
    pub inference_engine: Arc<dyn InferenceEngine>,
    pub metrics_exporter: Arc<dyn MetricsExporter>,
    pub server_control: Arc<dyn ServerControl>,
    pub queue_submitter: Arc<dyn QueueSubmitter>,
    pub config_writer: Arc<CW>,
    pub large_body_limit: LargeBodyConcurrencyLimit,
}

impl<CW> Clone for HttpAppState<CW> {
    fn clone(&self) -> Self {
        Self {
            config_reader: self.config_reader.clone(),
            route_validator: self.route_validator.clone(),
            inference_engine: self.inference_engine.clone(),
            metrics_exporter: self.metrics_exporter.clone(),
            server_control: self.server_control.clone(),
            queue_submitter: self.queue_submitter.clone(),
            config_writer: self.config_writer.clone(),
            large_body_limit: self.large_body_limit.clone(),
        }
    }
}

impl<CW> axum::extract::FromRef<HttpAppState<CW>> for LargeBodyConcurrencyLimit {
    fn from_ref(state: &HttpAppState<CW>) -> Self {
        state.large_body_limit.clone()
    }
}

impl<CW> axum::extract::FromRef<HttpAppState<CW>> for Arc<dyn ConfigReader> {
    fn from_ref(state: &HttpAppState<CW>) -> Self {
        state.config_reader.clone()
    }
}

pub(crate) async fn openai_chat_completion_handler<CW>(
    State(state): State<HttpAppState<CW>>,
    axum::extract::Extension(request_id): axum::extract::Extension<llamaforge_types::RequestId>,
    req_bytes: axum::body::Bytes,
) -> Result<axum::response::Response, ApiError> 
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    let body_len = req_bytes.len();
    
    let request: OpenAiChatRequest = if body_len > 1_048_576 {
        tokio::task::spawn_blocking(move || {
            serde_json::from_slice(&req_bytes).map_err(|e| ApiError::InvalidBody(e.to_string()))
        })
        .await
        .map_err(|_| ApiError::Internal("Task join error".to_string()))??
    } else {
        serde_json::from_slice(&req_bytes).map_err(|e| ApiError::InvalidBody(e.to_string()))?
    };

    let stream = request.stream;
    
    let req = if body_len > 1_048_576 {
        tokio::task::spawn_blocking(move || parse_openai_request(request))
            .await
            .map_err(|_| ApiError::Internal("Task join error".to_string()))??
    } else {
        parse_openai_request(request)?
    };

    handle_openai_request(state, request_id, req, stream).await
}

async fn handle_openai_request<CW>(
    state: HttpAppState<CW>,
    request_id: llamaforge_types::RequestId,
    req: llamaforge_types::ChatCompletionRequest,
    stream: bool,
) -> Result<axum::response::Response, ApiError> 
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    state.route_validator.validate_routable(&llamaforge_types::traits::IncomingRequest {})
        .map_err(|_| ApiError::QueueFull)?;

    let mut response_stream = state.inference_engine.chat_completion(req).await
        .map_err(|_| ApiError::BackendError)?;

    let req_id_str = format!("chatcmpl-{}", request_id.as_uuid());

    if stream {
        let sse_stream = response_stream.map(move |res| {
            match res {
                Ok(chunk) => {
                    let sse_data = format_openai_chunk(&req_id_str, &chunk);
                    Ok(Event::default().data(sse_data))
                }
                Err(_) => Err(axum::Error::new("Stream error")),
            }
        });
        
        Ok(Sse::new(sse_stream).into_response())
    } else {
        let timeout_secs = state.config_reader.snapshot().global.queue_default_timeout_secs;
        let res = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async move {
            let mut full_text = String::new();
            while let Some(chunk_res) = response_stream.next().await {
                if let Ok(llamaforge_types::ChatCompletionChunk::Token(t)) = chunk_res {
                    full_text.push_str(&t);
                }
            }
            full_text
        }).await.map_err(|_| ApiError::Timeout)?;
        
        Ok(axum::Json(serde_json::json!({
            "id": req_id_str,
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": res
                }
            }]
        })).into_response())
    }
}

pub(crate) async fn openai_completion_handler<CW>(
    State(state): State<HttpAppState<CW>>,
    axum::extract::Extension(request_id): axum::extract::Extension<llamaforge_types::RequestId>,
    req_bytes: axum::body::Bytes,
) -> Result<axum::response::Response, ApiError> 
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    #[derive(serde::Deserialize)]
    struct OpenAiCompletionRequest {
        model: String,
        prompt: String,
        #[serde(default)]
        max_tokens: Option<u32>,
        #[serde(default)]
        temperature: Option<f32>,
        #[serde(default)]
        stream: bool,
    }

    let body_len = req_bytes.len();
    let req_json: OpenAiCompletionRequest = if body_len > 1_048_576 {
        tokio::task::spawn_blocking(move || {
            serde_json::from_slice(&req_bytes).map_err(|e| ApiError::InvalidBody(e.to_string()))
        })
        .await
        .map_err(|_| ApiError::Internal("Task join error".to_string()))??
    } else {
        serde_json::from_slice(&req_bytes).map_err(|e| ApiError::InvalidBody(e.to_string()))?
    };

    let chat_req = llamaforge_types::ChatCompletionRequest {
        model: req_json.model,
        messages: vec![llamaforge_types::Message {
            role: llamaforge_types::Role::User,
            content: req_json.prompt,
        }],
        max_tokens: req_json.max_tokens,
        temperature: req_json.temperature,
        stream: req_json.stream,
        logit_bias: None,
        tools: None,
        tool_choice: None,
    };

    handle_openai_request(state, request_id, chat_req, req_json.stream).await
}

pub(crate) async fn anthropic_messages_handler<CW>(
    State(state): State<HttpAppState<CW>>,
    axum::extract::Extension(request_id): axum::extract::Extension<llamaforge_types::RequestId>,
    req_bytes: axum::body::Bytes,
) -> Result<axum::response::Response, ApiError> 
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    let body_len = req_bytes.len();
    
    let request: AnthropicMessagesRequest = if body_len > 1_048_576 {
        tokio::task::spawn_blocking(move || {
            serde_json::from_slice(&req_bytes).map_err(|e| ApiError::InvalidBody(e.to_string()))
        })
        .await
        .map_err(|_| ApiError::Internal("Task join error".to_string()))??
    } else {
        serde_json::from_slice(&req_bytes).map_err(|e| ApiError::InvalidBody(e.to_string()))?
    };

    let stream = request.stream;
    let model = request.model.clone();

    let req = if body_len > 1_048_576 {
        tokio::task::spawn_blocking(move || parse_anthropic_request(request))
            .await
            .map_err(|_| ApiError::Internal("Task join error".to_string()))??
    } else {
        parse_anthropic_request(request)?
    };

    state.route_validator.validate_routable(&llamaforge_types::traits::IncomingRequest {})
        .map_err(|_| ApiError::QueueFull)?;

    let mut response_stream = state.inference_engine.chat_completion(req).await
        .map_err(|_| ApiError::BackendError)?;

    let req_id_str = format!("msg_{}", request_id.as_uuid());

    if stream {
        let mut anthropic_state = AnthropicState::new();
        
        let sse_stream = response_stream.flat_map(move |res| {
            let chunks = match res {
                Ok(chunk) => {
                    match format_anthropic_event(&req_id_str, &model, &chunk, &mut anthropic_state) {
                        Ok(events) => events.into_iter().map(|evt| Ok(Event::from(evt))).collect::<Vec<_>>(),
                        Err(e) => vec![Err(axum::Error::new(e))],
                    }
                }
                Err(_) => vec![Err(axum::Error::new("Stream error"))],
            };
            
            futures_util::stream::iter(chunks)
        });
        
        Ok(Sse::new(sse_stream).into_response())
    } else {
        let timeout_secs = state.config_reader.snapshot().global.queue_default_timeout_secs;
        let res = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async move {
            let mut full_text = String::new();
            while let Some(chunk_res) = response_stream.next().await {
                if let Ok(llamaforge_types::ChatCompletionChunk::Token(t)) = chunk_res {
                    full_text.push_str(&t);
                }
            }
            full_text
        }).await.map_err(|_| ApiError::Timeout)?;
        
        Ok(axum::Json(serde_json::json!({
            "id": req_id_str,
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": res
            }],
            "model": model,
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {"input_tokens": 0, "output_tokens": 0}
        })).into_response())
    }
}

pub(crate) async fn health_check() -> axum::response::Response {
    axum::Json(serde_json::json!({"status": "ok"})).into_response()
}

pub(crate) async fn prometheus_export<CW>(
    State(state): State<HttpAppState<CW>>,
) -> axum::response::Response 
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    let text = state.metrics_exporter.gather_text();
    axum::response::Response::builder()
        .status(200)
        .body(axum::body::Body::from(text))
        .unwrap()
}

pub(crate) fn build_router<CW>(state: HttpAppState<CW>) -> Router 
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    Router::new()
        .route("/v1/chat/completions", post(openai_chat_completion_handler::<CW>))
        .route("/v1/completions", post(openai_completion_handler::<CW>))
        .route("/v1/messages", post(anthropic_messages_handler::<CW>))
        .route("/health", get(health_check))
        .route("/metrics", get(prometheus_export::<CW>))
        .layer(axum::middleware::from_fn(metrics_middleware))
        .layer(axum::middleware::from_fn_with_state(state.config_reader.clone(), auth_middleware))
        .layer(axum::middleware::from_fn_with_state(state.large_body_limit.clone(), large_body_limit_middleware))
        .layer(RequestBodyLimitLayer::new(50 * 1024 * 1024))
        .layer(axum::middleware::from_fn(request_id_middleware))
        .layer(CatchPanicLayer::new())
        .with_state(state)
}

pub async fn start_http_server<CW>(
    state: HttpAppState<CW>,
    shutdown_rx: tokio::sync::watch::Receiver<()>,
) -> Result<(), TransportError> 
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    let config = state.config_reader.snapshot();
    let addr = std::net::SocketAddr::new(config.global.bind_address, config.global.http_port);
    let router = build_router(state);
    
    tracing::info!("transport.started HTTP server on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| TransportError::Bootstrap(e.to_string()))?;
    
    let mut rx = shutdown_rx.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = rx.changed().await;
        })
        .await
        .map_err(|e| TransportError::Bootstrap(e.to_string()))?;
        
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::{Request, StatusCode}};
    use llamaforge_config::{AppConfig, ConfigHandle, source::memory::InMemoryConfigSource};
    use llamaforge_types::{
        traits::{IncomingRequest, QueueError, QueueSnapshot, QueueTicket, ServerSummary},
        RequestId, ServerId, ChatCompletionChunk, StreamError,
    };
    use std::pin::Pin;
    use std::sync::Arc;
    use tower::ServiceExt;
    use futures_core::Stream;

    // A mock InferenceEngine to inject into the app state
    struct MockEngine {
        chunks: Vec<Result<ChatCompletionChunk, StreamError>>,
        sleep_ms: u64,
    }
    #[async_trait::async_trait]
    impl InferenceEngine for MockEngine {
        async fn chat_completion(
            &self,
            _req: llamaforge_types::ChatCompletionRequest,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>, llamaforge_types::traits::DispatchError> {
            let chunks = self.chunks.clone();
            let sleep_time = self.sleep_ms;
            
            let stream = futures_util::stream::unfold((chunks.into_iter(), sleep_time), |(mut iter, delay)| async move {
                if delay > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
                iter.next().map(|item| (item, (iter, delay)))
            });
            
            Ok(Box::pin(stream))
        }
    }

    struct MockRouteValidator;
    impl RouteValidator for MockRouteValidator {
        fn validate_routable(&self, _req: &IncomingRequest) -> Result<(), llamaforge_types::traits::AdmissionError> {
            Ok(())
        }
    }

    struct MockMetricsExporter;
    impl MetricsExporter for MockMetricsExporter {
        fn gather_text(&self) -> String { "".into() }
    }

    struct MockServerControl;
    #[async_trait::async_trait]
    impl ServerControl for MockServerControl {
        async fn list_servers(&self) -> Result<Vec<ServerSummary>, llamaforge_types::traits::ServerError> { Ok(vec![]) }
        async fn start_server(&self, _id: &ServerId) -> Result<(), llamaforge_types::traits::ServerError> { Ok(()) }
        async fn stop_server(&self, _id: &ServerId) -> Result<(), llamaforge_types::traits::ServerError> { Ok(()) }
    }

    struct MockQueueSubmitter;
    impl QueueSubmitter for MockQueueSubmitter {
        fn submit(&self, _req: IncomingRequest) -> Result<QueueTicket, llamaforge_types::traits::AdmissionError> { Ok(QueueTicket {}) }
        fn cancel(&self, _id: &RequestId) -> Result<(), QueueError> { Ok(()) }
        fn status(&self) -> Result<QueueSnapshot, QueueError> { Ok(QueueSnapshot {}) }
    }

    fn build_test_router(engine: MockEngine) -> Router {
        let mut config = AppConfig::default();
        config.global.queue_default_timeout_secs = 1; // Used for timeout tests
        let source = Arc::new(InMemoryConfigSource::new(config));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());

        let state = HttpAppState {
            config_reader: handle.clone(),
            config_writer: handle.clone(),
            route_validator: Arc::new(MockRouteValidator),
            inference_engine: Arc::new(engine),
            metrics_exporter: Arc::new(MockMetricsExporter),
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitter),
            large_body_limit: LargeBodyConcurrencyLimit::new(10, 1),
        };

        build_router(state)
    }

    #[tokio::test]
    async fn test_body_limit_rejection() {
        let engine = MockEngine { chunks: vec![], sleep_ms: 0 };
        let app = build_test_router(engine);

        // Limit is 50MB. Make a dummy body of 51MB.
        // For oneshot testing, we can just use a large body, or mock the content length header and an infinite stream if allowed,
        // but the RequestBodyLimitLayer relies on the actual body stream size or Content-Length.
        let body_str = "x".repeat(50 * 1024 * 1024 + 10);
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(body_str))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn test_full_streaming_path() {
        let chunks = vec![
            Ok(ChatCompletionChunk::Token("Hel".into())),
            Ok(ChatCompletionChunk::Token("lo".into())),
            Ok(ChatCompletionChunk::Stop("stop".into())),
        ];
        let engine = MockEngine { chunks, sleep_ms: 0 };
        let app = build_test_router(engine);

        let body = serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Read the body
        let body_bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(body_str.contains("Hel"));
        assert!(body_str.contains("lo"));
        assert!(body_str.contains("[DONE]"));
    }

    #[tokio::test]
    async fn test_non_streaming_timeout() {
        // Sleep for 2000ms > 1 second timeout configured above
        let engine = MockEngine { chunks: vec![], sleep_ms: 2000 };
        let app = build_test_router(engine);

        let body = serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": false
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::GATEWAY_TIMEOUT);
    }
}
