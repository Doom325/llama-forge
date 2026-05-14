#![allow(dead_code)]
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

pub(crate) struct HttpAppState<CW> {
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
    req_bytes: axum::body::Bytes, // Use Bytes to check length before parsing
) -> Result<axum::response::Response, ApiError> 
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    let body_len = req_bytes.len();
    
    // For bodies > 1MB use spawn_blocking
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
    
    // For spawn_blocking constraint
    let req = if body_len > 1_048_576 {
        tokio::task::spawn_blocking(move || parse_openai_request(request))
            .await
            .map_err(|_| ApiError::Internal("Task join error".to_string()))??
    } else {
        parse_openai_request(request)?
    };

    state.route_validator.validate_routable(&llamaforge_types::traits::IncomingRequest {})
        .map_err(|_| ApiError::QueueFull)?;

    let mut response_stream = state.inference_engine.chat_completion(req).await
        .map_err(|_| ApiError::BackendError)?;

    if stream {
        let sse_stream = response_stream.map(|res| {
            match res {
                Ok(chunk) => {
                    let sse_data = format_openai_chunk("chatcmpl-123", &chunk);
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
            "id": "chatcmpl-123",
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
    State(_state): State<HttpAppState<CW>>,
    _req_bytes: axum::body::Bytes,
) -> Result<axum::response::Response, ApiError> 
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    Err(ApiError::Internal("Not Implemented".to_string()))
}

pub(crate) async fn anthropic_messages_handler<CW>(
    State(state): State<HttpAppState<CW>>,
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

    let response_stream = state.inference_engine.chat_completion(req).await
        .map_err(|_| ApiError::BackendError)?;

    if stream {
        let mut anthropic_state = AnthropicState::new();
        
        let sse_stream = response_stream.flat_map(move |res| {
            let chunks = match res {
                Ok(chunk) => {
                    format_anthropic_event("msg_123", &model, &chunk, &mut anthropic_state).unwrap_or_default()
                }
                Err(_) => vec![],
            };
            
            tokio_stream::iter(chunks.into_iter().map(|evt| {
                Ok::<_, axum::Error>(axum::response::sse::Event::from(evt))
            }))
        });
        
        Ok(Sse::new(sse_stream).into_response())
    } else {
        Err(ApiError::Internal("Non-streaming not fully implemented".to_string()))
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
        .layer(axum::middleware::from_fn(request_id_middleware))
        .layer(axum::middleware::from_fn_with_state(state.config_reader.clone(), auth_middleware))
        .layer(axum::middleware::from_fn_with_state(state.large_body_limit.clone(), large_body_limit_middleware))
        .layer(RequestBodyLimitLayer::new(50 * 1024 * 1024))
        .layer(CatchPanicLayer::new())
        .with_state(state)
}

pub(crate) async fn start_http_server<CW>(
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
