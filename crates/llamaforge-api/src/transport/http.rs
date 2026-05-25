use axum::{
    extract::State,
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post},
    Router,
};
use futures_util::stream::StreamExt;
use llamaforge_core::config::{ConfigReader, ConfigWriter};
use llamaforge_core::types::traits::{
    MetricsExporter, QueueSubmitter, RouteValidator, ServerControl,
};
use std::sync::Arc;
use std::time::Duration;
use tower_http::{catch_panic::CatchPanicLayer, limit::RequestBodyLimitLayer};

use super::{
    anthropic::{
        format_anthropic_event, parse_anthropic_request, AnthropicMessagesRequest, AnthropicState,
    },
    error::ApiError,
    middleware::{
        auth_middleware, large_body_limit_middleware, metrics_middleware, request_id_middleware,
        LargeBodyConcurrencyLimit,
    },
    openai::{format_openai_chunk, parse_openai_request, OpenAiChatRequest},
    InferenceEngine, TransportError,
};

pub struct HttpAppState<CW, RMS, SM> {
    pub config_reader: Arc<dyn ConfigReader>,
    pub route_validator: Arc<dyn RouteValidator>,
    pub inference_engine: Arc<dyn InferenceEngine>,
    pub metrics_exporter: Arc<dyn MetricsExporter>,
    pub server_control: Arc<dyn ServerControl>,
    pub queue_submitter: Arc<dyn QueueSubmitter>,
    pub config_writer: Arc<CW>,
    pub request_metrics_store: Arc<RMS>,
    pub store_maintenance: Arc<SM>,
    pub large_body_limit: LargeBodyConcurrencyLimit,
}

impl<CW, RMS, SM> Clone for HttpAppState<CW, RMS, SM> {
    fn clone(&self) -> Self {
        Self {
            config_reader: self.config_reader.clone(),
            route_validator: self.route_validator.clone(),
            inference_engine: self.inference_engine.clone(),
            metrics_exporter: self.metrics_exporter.clone(),
            server_control: self.server_control.clone(),
            queue_submitter: self.queue_submitter.clone(),
            config_writer: self.config_writer.clone(),
            request_metrics_store: self.request_metrics_store.clone(),
            store_maintenance: self.store_maintenance.clone(),
            large_body_limit: self.large_body_limit.clone(),
        }
    }
}

impl<CW, RMS, SM> axum::extract::FromRef<HttpAppState<CW, RMS, SM>> for LargeBodyConcurrencyLimit {
    fn from_ref(state: &HttpAppState<CW, RMS, SM>) -> Self {
        state.large_body_limit.clone()
    }
}

impl<CW, RMS, SM> axum::extract::FromRef<HttpAppState<CW, RMS, SM>> for Arc<dyn ConfigReader> {
    fn from_ref(state: &HttpAppState<CW, RMS, SM>) -> Self {
        state.config_reader.clone()
    }
}

/// Project a parsed `ChatCompletionRequest` into the validator's
/// `IncomingRequest` shape, keeping the real model alias / messages /
/// params. Used by both the OpenAI and Anthropic HTTP handlers and the
/// gRPC `stream_inference` handler so routing sees the same view of
/// the request that the inference engine does.
pub(crate) fn build_incoming_request(
    request_id: llamaforge_core::types::RequestId,
    req: &llamaforge_core::types::ChatCompletionRequest,
) -> llamaforge_core::types::traits::IncomingRequest {
    llamaforge_core::types::traits::IncomingRequest {
        request_id,
        requestor_id: llamaforge_core::types::RequestorId::new("anonymous".into()),
        requestor_class: llamaforge_core::types::RequestorClass::new("default".into()),
        model_target: llamaforge_core::types::ModelTarget::Exact(
            llamaforge_core::types::ModelAlias::new(std::sync::Arc::from(req.model.as_str())),
        ),
        capabilities: Default::default(),
        messages: req.messages.clone(),
        params: Default::default(),
        stream: req.stream,
        session_id: None,
        submitted_at: chrono::Utc::now(),
    }
}

/// Derive a `Retry-After` (seconds) hint from the queue's current
/// depth / throughput estimate. Falls back to 30s if either reading is
/// unavailable. Used only on the `QueueFull` branch; model-mismatch
/// rejections must not surface a `Retry-After` since the model name
/// will still be unknown on retry.
fn compute_retry_after(qs: &dyn QueueSubmitter) -> u32 {
    if let Ok(status) = qs.status() {
        if status.estimated_throughput_req_sec > 0.0 {
            return (status.depth as f32 / status.estimated_throughput_req_sec).ceil() as u32;
        }
    }
    30
}

pub(crate) async fn openai_chat_completion_handler<CW, RMS, SM>(
    State(state): State<HttpAppState<CW, RMS, SM>>,
    axum::extract::Extension(request_id): axum::extract::Extension<
        llamaforge_core::types::RequestId,
    >,
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

async fn handle_openai_request<CW, RMS, SM>(
    state: HttpAppState<CW, RMS, SM>,
    request_id: llamaforge_core::types::RequestId,
    req: llamaforge_core::types::ChatCompletionRequest,
    stream: bool,
) -> Result<axum::response::Response, ApiError>
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    let model_echo = req.model.clone();
    // Build the validator's `IncomingRequest` from the parsed body so
    // routing sees the real model alias / capabilities / params /
    // messages, not a placeholder.
    let incoming = build_incoming_request(request_id, &req);
    state
        .route_validator
        .validate_routable(&incoming.model_target, &incoming.capabilities)
        .map_err(|e| {
            // Pattern-match instead of collapsing every error to `QueueFull`:
            // `NoServerForModel` must surface as `404 model_not_found`, not
            // `429 queue_full` with a wrong `Retry-After`.
            match e {
                llamaforge_core::types::traits::AdmissionError::QueueFull => {
                    let retry_after_secs = compute_retry_after(&*state.queue_submitter);
                    ApiError::QueueFull { retry_after_secs }
                }
                other => ApiError::from(other),
            }
        })?;

    let mut response_stream =
        state.inference_engine.chat_completion(req).await.map_err(|_| ApiError::BackendError)?;

    let req_id_str = format!("chatcmpl-{}", request_id.as_uuid());

    if stream {
        let qs = state.queue_submitter.clone();
        let req_id = request_id;
        // Per-chunk timeout — matches the non-streaming path's bound.
        // Without this an upstream stall holds the SSE connection open
        // indefinitely (issue #191).
        let chunk_timeout =
            Duration::from_secs(state.config_reader.snapshot().global.queue_default_timeout_secs);
        let req_id_str_for_stream = req_id_str.clone();
        let sse_stream = async_stream::stream! {
            let mut response_stream = response_stream;
            loop {
                match tokio::time::timeout(chunk_timeout, response_stream.next()).await {
                    Err(_elapsed) => {
                        tracing::warn!(
                            request_id = %req_id,
                            timeout_secs = chunk_timeout.as_secs(),
                            "openai.sse.upstream_stalled — closing client stream"
                        );
                        let sse_data = serde_json::json!({
                            "error": {
                                "message": "Upstream stalled; stream closed",
                                "type": "timeout",
                                "param": null,
                                "code": null
                            }
                        })
                        .to_string();
                        yield Ok::<_, axum::Error>(Event::default().data(sse_data));
                        break;
                    }
                    Ok(None) => break,
                    Ok(Some(Ok(chunk))) => {
                        for data in format_openai_chunk(&req_id_str_for_stream, &chunk) {
                            yield Ok(Event::default().data(data));
                        }
                    }
                    Ok(Some(Err(_))) => {
                        let sse_data = serde_json::json!({
                            "error": {
                                "message": "Stream error",
                                "type": "internal_error",
                                "param": null,
                                "code": null
                            }
                        })
                        .to_string();
                        yield Ok(Event::default().data(sse_data));
                        break;
                    }
                }
            }
        };

        struct StreamGuard<S> {
            inner: S,
            request_id: llamaforge_core::types::RequestId,
            queue_submitter: std::sync::Arc<dyn llamaforge_core::types::traits::QueueSubmitter>,
            finished: bool,
        }
        impl<S> Drop for StreamGuard<S> {
            fn drop(&mut self) {
                if !self.finished {
                    tracing::debug!("Client disconnected, cancelling request {}", self.request_id);
                    let _ = self.queue_submitter.cancel(&self.request_id);
                }
            }
        }
        impl<S: futures_core::Stream + Unpin> futures_core::Stream for StreamGuard<S> {
            type Item = S::Item;
            fn poll_next(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Option<Self::Item>> {
                let res = std::pin::Pin::new(&mut self.inner).poll_next(cx);
                if let std::task::Poll::Ready(None) = res {
                    self.finished = true;
                }
                res
            }
        }
        let guarded_stream = StreamGuard {
            inner: Box::pin(sse_stream),
            request_id: req_id,
            queue_submitter: qs,
            finished: false,
        };

        Ok(Sse::new(guarded_stream).into_response())
    } else {
        // Non-streaming: collect `Token` chunks into the assistant
        // content and read `finish_reason` / `output_tokens` from the
        // terminal `Stop` chunk. The full OpenAI response shape
        // (`object`, `created`, `model`, `choices[].index`,
        // `choices[].finish_reason`, `usage`) is required by the
        // official SDKs — see design.md §7.2.
        let timeout_secs = state.config_reader.snapshot().global.queue_default_timeout_secs;
        let aggregated =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async move {
                let mut full_text = String::new();
                let mut finish_reason: Option<String> = None;
                let mut completion_tokens: u32 = 0;
                let mut prompt_tokens_acc: u32 = 0;
                let mut saw_stop = false;
                while let Some(chunk_res) = response_stream.next().await {
                    match chunk_res {
                        Ok(llamaforge_core::types::ChatCompletionChunk::Token(t)) => {
                            full_text.push_str(&t);
                        }
                        Ok(llamaforge_core::types::ChatCompletionChunk::Stop {
                            finish_reason: reason,
                            output_tokens,
                            prompt_tokens,
                        }) => {
                            finish_reason = Some(reason);
                            completion_tokens = output_tokens;
                            prompt_tokens_acc = prompt_tokens;
                            saw_stop = true;
                        }
                        _ => {}
                    }
                }
                (full_text, finish_reason, completion_tokens, prompt_tokens_acc, saw_stop)
            })
            .await
            .map_err(|_| ApiError::Timeout)?;
        let (content, finish_reason, completion_tokens, prompt_tokens, saw_stop) = aggregated;
        // Warming-window guard: if the stream closed without ever
        // observing a terminal `Stop` chunk, the backend went away
        // mid-response (no Running server yet, or a crash). Return
        // 503 with `Retry-After` instead of a successful empty body
        // that the client would interpret as `finish_reason="stop"
        // tokens=0`. A *legitimately* empty completion arrives as
        // `Stop { output_tokens: 0, .. }` and is still a 200.
        if !saw_stop {
            return Err(ApiError::ServiceUnavailable { retry_after_secs: 1 });
        }
        let finish_reason = finish_reason.unwrap_or_else(|| "stop".to_string());
        let created_at = chrono::Utc::now().timestamp();

        Ok(axum::Json(serde_json::json!({
            "id": req_id_str,
            "object": "chat.completion",
            "created": created_at,
            "model": model_echo,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content,
                },
                "finish_reason": finish_reason,
            }],
            // `prompt_tokens` is the upstream `tokens_evaluated`
            // captured on the terminal stop chunk (#134). 0 only when
            // the upstream didn't surface a value.
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens.saturating_add(completion_tokens),
            }
        }))
        .into_response())
    }
}

pub(crate) async fn openai_completion_handler<CW, RMS, SM>(
    State(state): State<HttpAppState<CW, RMS, SM>>,
    axum::extract::Extension(request_id): axum::extract::Extension<
        llamaforge_core::types::RequestId,
    >,
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

    let chat_req = llamaforge_core::types::ChatCompletionRequest {
        model: req_json.model,
        messages: vec![llamaforge_core::types::Message {
            role: llamaforge_core::types::Role::User,
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

pub(crate) async fn anthropic_messages_handler<CW, RMS, SM>(
    State(state): State<HttpAppState<CW, RMS, SM>>,
    axum::extract::Extension(request_id): axum::extract::Extension<
        llamaforge_core::types::RequestId,
    >,
    headers: axum::http::HeaderMap,
    req_bytes: axum::body::Bytes,
) -> Result<axum::response::Response, ApiError>
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    // Anthropic SDK mandates an `anthropic-version: YYYY-MM-DD`
    // header. v1.0 is permissive — log a structured warning and
    // continue, mirroring the header back if it was supplied so SDK
    // clients can keep their version-pinning code paths working.
    // A strict-mode `gateway.anthropic_strict_version_header` knob
    // is tracked in `docs/v1.1-deferred.md`.
    let anthropic_version =
        headers.get("anthropic-version").and_then(|v| v.to_str().ok()).map(str::to_string);
    if anthropic_version.is_none() {
        tracing::warn!(api = "anthropic", "api.missing_anthropic_version_header");
    }
    let inject_version = |mut resp: axum::response::Response| -> axum::response::Response {
        if let Some(ref v) = anthropic_version {
            if let Ok(hv) = axum::http::HeaderValue::from_str(v) {
                resp.headers_mut().insert("anthropic-version", hv);
            }
        }
        resp
    };

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

    let incoming = build_incoming_request(request_id, &req);
    state
        .route_validator
        .validate_routable(&incoming.model_target, &incoming.capabilities)
        .map_err(|e| match e {
            llamaforge_core::types::traits::AdmissionError::QueueFull => {
                let retry_after_secs = compute_retry_after(&*state.queue_submitter);
                ApiError::QueueFull { retry_after_secs }
            }
            other => ApiError::from(other),
        })?;

    let mut response_stream =
        state.inference_engine.chat_completion(req).await.map_err(|_| ApiError::BackendError)?;

    let req_id_str = format!("msg_{}", request_id.as_uuid());

    if stream {
        let qs = state.queue_submitter.clone();
        let req_id = request_id;
        // Per-chunk timeout — matches the non-streaming path's bound.
        // Without this an upstream stall holds the SSE connection open
        // indefinitely (issue #191).
        let chunk_timeout =
            Duration::from_secs(state.config_reader.snapshot().global.queue_default_timeout_secs);
        let req_id_str_for_stream = req_id_str.clone();
        let model_for_stream = model.clone();
        let sse_stream = async_stream::stream! {
            let mut response_stream = response_stream;
            let mut anthropic_state = AnthropicState::new();
            loop {
                match tokio::time::timeout(chunk_timeout, response_stream.next()).await {
                    Err(_elapsed) => {
                        tracing::warn!(
                            request_id = %req_id,
                            timeout_secs = chunk_timeout.as_secs(),
                            "anthropic.sse.upstream_stalled — closing client stream"
                        );
                        let err_json = serde_json::json!({
                            "type": "error",
                            "error": {
                                "type": "timeout_error",
                                "message": "Upstream stalled; stream closed"
                            }
                        })
                        .to_string();
                        yield Ok::<_, axum::Error>(Event::default().event("error").data(err_json));
                        break;
                    }
                    Ok(None) => break,
                    Ok(Some(Ok(chunk))) => {
                        match format_anthropic_event(
                            &req_id_str_for_stream,
                            &model_for_stream,
                            &chunk,
                            &mut anthropic_state,
                        ) {
                            Ok(events) => {
                                for evt in events {
                                    yield Ok(Event::from(evt));
                                }
                            }
                            Err(e) => {
                                yield Err(axum::Error::new(e));
                                break;
                            }
                        }
                    }
                    Ok(Some(Err(_))) => {
                        let err_json = serde_json::json!({
                            "type": "error",
                            "error": {
                                "type": "api_error",
                                "message": "Stream error"
                            }
                        })
                        .to_string();
                        yield Ok(Event::default().event("error").data(err_json));
                        break;
                    }
                }
            }
        };

        struct StreamGuard<S> {
            inner: S,
            request_id: llamaforge_core::types::RequestId,
            queue_submitter: std::sync::Arc<dyn llamaforge_core::types::traits::QueueSubmitter>,
            finished: bool,
        }
        impl<S> Drop for StreamGuard<S> {
            fn drop(&mut self) {
                if !self.finished {
                    tracing::debug!("Client disconnected, cancelling request {}", self.request_id);
                    let _ = self.queue_submitter.cancel(&self.request_id);
                }
            }
        }
        impl<S: futures_core::Stream + Unpin> futures_core::Stream for StreamGuard<S> {
            type Item = S::Item;
            fn poll_next(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Option<Self::Item>> {
                let res = std::pin::Pin::new(&mut self.inner).poll_next(cx);
                if let std::task::Poll::Ready(None) = res {
                    self.finished = true;
                }
                res
            }
        }
        let guarded_stream = StreamGuard {
            inner: Box::pin(sse_stream),
            request_id: req_id,
            queue_submitter: qs,
            finished: false,
        };

        Ok(inject_version(Sse::new(guarded_stream).into_response()))
    } else {
        // Non-streaming: same aggregation pattern as the OpenAI shim
        // — collect `Token` content, read `stop_reason` /
        // `output_tokens` from the terminal `Stop` chunk, and 503 if
        // the stream closes without ever observing one (warming-window
        // guard — see the OpenAI handler comment above).
        let timeout_secs = state.config_reader.snapshot().global.queue_default_timeout_secs;
        let aggregated =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async move {
                let mut full_text = String::new();
                let mut stop_reason: Option<String> = None;
                let mut output_tokens: u32 = 0;
                let mut input_tokens: u32 = 0;
                let mut saw_stop = false;
                while let Some(chunk_res) = response_stream.next().await {
                    match chunk_res {
                        Ok(llamaforge_core::types::ChatCompletionChunk::Token(t)) => {
                            full_text.push_str(&t);
                        }
                        Ok(llamaforge_core::types::ChatCompletionChunk::Stop {
                            finish_reason,
                            output_tokens: tokens,
                            prompt_tokens,
                        }) => {
                            stop_reason = Some(finish_reason);
                            output_tokens = tokens;
                            input_tokens = prompt_tokens;
                            saw_stop = true;
                        }
                        _ => {}
                    }
                }
                (full_text, stop_reason, output_tokens, input_tokens, saw_stop)
            })
            .await
            .map_err(|_| ApiError::Timeout)?;
        let (text, stop_reason, output_tokens, input_tokens, saw_stop) = aggregated;
        if !saw_stop {
            return Err(ApiError::ServiceUnavailable { retry_after_secs: 1 });
        }
        let stop_reason = stop_reason.unwrap_or_else(|| "end_turn".to_string());

        Ok(inject_version(
            axum::Json(serde_json::json!({
                "id": req_id_str,
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": text,
                }],
                "model": model,
                "stop_reason": stop_reason,
                "stop_sequence": null,
                // `input_tokens` is the upstream `tokens_evaluated` (#134).
                "usage": {"input_tokens": input_tokens, "output_tokens": output_tokens}
            }))
            .into_response(),
        ))
    }
}

pub(crate) async fn health_check() -> axum::response::Response {
    axum::Json(serde_json::json!({"status": "ok"})).into_response()
}

/// `GET /v1/models` — OpenAI-compatible model discovery. Emits the
/// list envelope `{"object":"list","data":[...]}` over the distinct
/// server aliases in the config snapshot. Servers without an alias
/// (default `"default"`) are skipped to keep the discovery surface
/// useful.
pub(crate) async fn list_openai_models<CW, RMS, SM>(
    State(state): State<HttpAppState<CW, RMS, SM>>,
) -> axum::response::Response {
    let snap = state.config_reader.snapshot();
    let now_unix = chrono::Utc::now().timestamp();
    let mut seen = std::collections::BTreeSet::new();
    let mut data: Vec<serde_json::Value> = Vec::new();
    for (_id, cfg) in snap.servers.iter() {
        if let Some(alias) = cfg.alias.clone() {
            if alias.is_empty() {
                continue;
            }
            if seen.insert(alias.clone()) {
                data.push(serde_json::json!({
                    "id": alias,
                    "object": "model",
                    "created": now_unix,
                    "owned_by": "llamaforge",
                }));
            }
        }
    }
    axum::Json(serde_json::json!({
        "object": "list",
        "data": data,
    }))
    .into_response()
}

/// `GET /v1/servers` — operator status surface aliased on top of
/// `/api/servers`. The payload is mapped to `ServerStatusResponse`.
pub(crate) async fn list_v1_servers<CW, RMS, SM>(
    State(state): State<HttpAppState<CW, RMS, SM>>,
) -> Result<axum::response::Response, ApiError> {
    let summaries =
        state.server_control.list_servers().await.map_err(|e| ApiError::Internal(e.to_string()))?;

    let dtos: Vec<super::management::ServerStatusResponse> = summaries
        .into_iter()
        .map(|s| super::management::ServerStatusResponse {
            id: s.id.to_string(),
            model: s.alias.into_inner().to_string(),
            state: format!("{:?}", s.state),
            slots_total: s.slots_total,
            slots_busy: s.slots_busy,
        })
        .collect();

    Ok(axum::Json(dtos).into_response())
}

pub(crate) async fn prometheus_export<CW, RMS, SM>(
    State(state): State<HttpAppState<CW, RMS, SM>>,
) -> axum::response::Response
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    let text = state.metrics_exporter.gather_text();
    (axum::http::StatusCode::OK, text).into_response()
}

pub(crate) fn build_router<CW, RMS, SM>(state: HttpAppState<CW, RMS, SM>) -> Router
where
    CW: ConfigWriter + Send + Sync + 'static,
    RMS: llamaforge_metrics::RequestMetricsStore + Send + Sync + 'static,
    SM: llamaforge_metrics::StoreMaintenance + Send + Sync + 'static,
{
    let management_state = super::management::ManagementState {
        server_control: state.server_control.clone(),
        queue_submitter: state.queue_submitter.clone(),
        config_reader: state.config_reader.clone(),
        config_writer: state.config_writer.clone(),
        request_metrics_store: state.request_metrics_store.clone(),
        store_maintenance: state.store_maintenance.clone(),
    };

    let management_router = super::management::management_routes(management_state);

    let max_body_bytes = state.config_reader.snapshot().gateway.max_request_body_bytes as usize;

    let inference_router = Router::new()
        .route("/v1/chat/completions", post(openai_chat_completion_handler::<CW, RMS, SM>))
        .route("/v1/completions", post(openai_completion_handler::<CW, RMS, SM>))
        .route("/v1/messages", post(anthropic_messages_handler::<CW, RMS, SM>))
        // OpenAI SDKs hit `/v1/models` at construction time for
        // discovery. Sourced from the config snapshot's aliased
        // servers, reshaped to the OpenAI list envelope.
        .route("/v1/models", get(list_openai_models::<CW, RMS, SM>))
        // `USER_JOURNEYS.md` J3 documents `/v1/servers` as the
        // operator status surface. Same payload as `/api/servers`.
        .route("/v1/servers", get(list_v1_servers::<CW, RMS, SM>))
        .route("/health", get(health_check))
        .route("/metrics", get(prometheus_export::<CW, RMS, SM>))
        .with_state(state.clone());

    inference_router
        .merge(management_router)
        .layer(axum::middleware::from_fn(metrics_middleware))
        .layer(axum::middleware::from_fn(request_id_middleware))
        .layer(axum::middleware::from_fn_with_state(state.config_reader.clone(), auth_middleware))
        .layer(axum::middleware::from_fn_with_state(
            state.large_body_limit.clone(),
            large_body_limit_middleware,
        ))
        .layer(RequestBodyLimitLayer::new(max_body_bytes))
        .layer(super::middleware::build_cors_layer(state.config_reader.clone()))
        .layer(CatchPanicLayer::new())
}

pub async fn start_http_server<CW, RMS, SM>(
    state: HttpAppState<CW, RMS, SM>,
    listener: tokio::net::TcpListener,
    shutdown_rx: tokio::sync::watch::Receiver<()>,
) -> Result<(), TransportError>
where
    CW: ConfigWriter + Send + Sync + 'static,
    RMS: llamaforge_metrics::RequestMetricsStore + Send + Sync + 'static,
    SM: llamaforge_metrics::StoreMaintenance + Send + Sync + 'static,
{
    let router = build_router(state);

    let mut rx = shutdown_rx.clone();
    axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>())
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
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use futures_core::Stream;
    use llamaforge_core::config::{source::memory::InMemoryConfigSource, AppConfig, ConfigHandle};
    use llamaforge_core::types::{
        traits::{
            AdmissionError, IncomingRequest, QueueError, QueueSnapshot, QueueTicket, ServerSummary,
        },
        ChatCompletionChunk, RequestId, ServerId, StreamError,
    };
    use std::pin::Pin;
    use std::sync::Arc;
    use tower::ServiceExt;

    // A mock InferenceEngine to inject into the app state
    struct MockEngine {
        chunks: std::sync::Mutex<Vec<Result<ChatCompletionChunk, StreamError>>>,
        sleep_ms: u64,
    }
    #[async_trait::async_trait]
    impl InferenceEngine for MockEngine {
        async fn chat_completion(
            &self,
            _req: llamaforge_core::types::ChatCompletionRequest,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>,
            llamaforge_core::types::traits::DispatchError,
        > {
            let chunks = std::mem::take(&mut *self.chunks.lock().unwrap());
            let sleep_time = self.sleep_ms;

            let stream = futures_util::stream::unfold(
                (chunks.into_iter(), sleep_time),
                |(mut iter, delay)| async move {
                    if delay > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    }
                    iter.next().map(|item| (item, (iter, delay)))
                },
            );

            Ok(Box::pin(stream))
        }
    }

    struct MockRouteValidator;
    impl RouteValidator for MockRouteValidator {
        fn validate_routable(
            &self,
            _model: &llamaforge_core::types::ModelTarget,
            _capabilities: &llamaforge_core::types::RequestCapabilities,
        ) -> Result<(), llamaforge_core::types::traits::AdmissionError> {
            Ok(())
        }
    }

    struct MockMetricsExporter;
    impl MetricsExporter for MockMetricsExporter {
        fn gather_text(&self) -> String {
            "".into()
        }
    }

    struct MockServerControl;
    #[async_trait::async_trait]
    impl ServerControl for MockServerControl {
        async fn list_servers(
            &self,
        ) -> Result<Vec<ServerSummary>, llamaforge_core::types::traits::ServerError> {
            Ok(vec![])
        }
        async fn start_server(
            &self,
            _id: &ServerId,
        ) -> Result<(), llamaforge_core::types::traits::ServerError> {
            Ok(())
        }
        async fn stop_server(
            &self,
            _id: &ServerId,
        ) -> Result<(), llamaforge_core::types::traits::ServerError> {
            Ok(())
        }
    }

    struct MockQueueSubmitter;
    impl QueueSubmitter for MockQueueSubmitter {
        fn submit(
            &self,
            _req: IncomingRequest,
        ) -> Result<QueueTicket, llamaforge_core::types::traits::AdmissionError> {
            Ok(QueueTicket {
                request_id: llamaforge_core::types::RequestId::new(uuid::Uuid::now_v7()),
                admitted_at: chrono::Utc::now(),
                cancel_token: tokio_util::sync::CancellationToken::new(),
                events: tokio::sync::mpsc::channel(1).1,
            })
        }
        fn cancel(&self, _id: &RequestId) -> Result<(), QueueError> {
            Ok(())
        }
        fn status(&self) -> Result<QueueSnapshot, QueueError> {
            Ok(QueueSnapshot { depth: 0, estimated_throughput_req_sec: 1.0 })
        }
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
            request_metrics_store: Arc::new(llamaforge_metrics::InMemoryStore::default()),
            store_maintenance: Arc::new(llamaforge_metrics::InMemoryStore::default()),
            large_body_limit: LargeBodyConcurrencyLimit::new(10, 1),
        };

        build_router(state)
    }

    #[tokio::test]
    async fn test_body_limit_rejection() {
        let engine = MockEngine { chunks: std::sync::Mutex::new(vec![]), sleep_ms: 0 };
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
            Ok(ChatCompletionChunk::Stop {
                finish_reason: "stop".to_string(),
                output_tokens: 0,
                prompt_tokens: 0,
            }),
        ];
        let engine = MockEngine { chunks: std::sync::Mutex::new(chunks), sleep_ms: 0 };
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
    async fn test_non_streaming_no_stop_chunk_returns_503() {
        // Warming-window guard: a stream that closes without ever
        // emitting a terminal `Stop` chunk represents a backend that
        // went away mid-response. The aggregator must surface this as
        // `503 Service Unavailable` with a `Retry-After` header, not
        // as a successful empty 200.
        let chunks: Vec<Result<ChatCompletionChunk, StreamError>> = vec![]; // closes without Stop
        let engine = MockEngine { chunks: std::sync::Mutex::new(chunks), sleep_ms: 0 };
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
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(res.headers().get("retry-after").is_some(), "503 must carry Retry-After");
    }

    #[tokio::test]
    async fn test_non_streaming_empty_completion_returns_200() {
        // Counter-case: a *legitimately* empty completion arrives as a
        // single `Stop { output_tokens: 0, .. }` chunk with no preceding
        // `Token`s. The aggregator must return `200 OK` with empty
        // `content`, not 503 — `saw_stop` distinguishes "backend went
        // away" from "model chose to say nothing".
        let chunks = vec![Ok(ChatCompletionChunk::Stop {
            finish_reason: "stop".to_string(),
            output_tokens: 0,
            prompt_tokens: 0,
        })];
        let engine = MockEngine { chunks: std::sync::Mutex::new(chunks), sleep_ms: 0 };
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
        assert_eq!(res.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json["choices"][0]["message"]["content"].as_str(), Some(""));
        assert_eq!(json["choices"][0]["finish_reason"].as_str(), Some("stop"));
        assert_eq!(json["usage"]["completion_tokens"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn test_non_streaming_timeout() {
        // Sleep for 2000ms > 1 second timeout configured above
        let engine = MockEngine { chunks: std::sync::Mutex::new(vec![]), sleep_ms: 2000 };
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

    #[tokio::test]
    async fn test_json_error_format() {
        let engine = MockEngine { chunks: std::sync::Mutex::new(vec![]), sleep_ms: 0 };
        let app = build_test_router(engine);

        // Slugs starting with `-` are rejected by validate_server_id_slug,
        // producing the canonical 400 JSON error body we want to inspect.
        let req = Request::builder()
            .method("POST")
            .uri("/api/servers/-bad/start")
            .body(Body::empty())
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        let body_bytes = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(json["error"].is_string());
        assert!(json["message"].is_string());
        assert!(json["request_id"].is_string());
    }

    #[tokio::test]
    async fn test_mid_stream_error_event() {
        let chunks =
            vec![Ok(ChatCompletionChunk::Token("Hel".into())), Err(StreamError::Cancelled)];
        let engine = MockEngine { chunks: std::sync::Mutex::new(chunks), sleep_ms: 0 };
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

        let body_bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
        assert!(body_str.contains("Hel"));
        assert!(body_str.contains("\"error\":{")); // Check for error object
        assert!(body_str.contains("Stream error"));
    }

    #[tokio::test]
    async fn test_queue_full_returns_429_with_retry_after() {
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

        let mut config = AppConfig::default();
        config.global.queue_default_timeout_secs = 1;
        let source = Arc::new(InMemoryConfigSource::new(config));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());

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

        let state = HttpAppState {
            config_reader: handle.clone(),
            config_writer: handle.clone(),
            route_validator: Arc::new(MockRouteValidatorRejects),
            inference_engine: Arc::new(MockEngine {
                chunks: std::sync::Mutex::new(vec![]),
                sleep_ms: 0,
            }),
            metrics_exporter: Arc::new(MockMetricsExporter),
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitterFull),
            request_metrics_store: Arc::new(llamaforge_metrics::InMemoryStore::default()),
            store_maintenance: Arc::new(llamaforge_metrics::InMemoryStore::default()),
            large_body_limit: crate::transport::middleware::LargeBodyConcurrencyLimit::new(10, 1),
        };

        let app = build_router(state);

        let body = serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": false
        });

        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);

        // Throughput is 10.0, depth is 100 -> 100 / 10 = 10s
        let retry_after = res.headers().get("retry-after").unwrap();
        assert_eq!(retry_after, "10");
    }
}
