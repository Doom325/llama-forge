use futures_core::future::BoxFuture;
use futures_util::StreamExt;
use llamaforge_core::config::{ConfigReader, ConfigWriter};
use llamaforge_core::types::traits::{QueueSubmitter, RouteValidator, ServerControl};
use llamaforge_proto::inference::inference_service_server::{
    InferenceService, InferenceServiceServer,
};
use llamaforge_proto::inference::{InferenceRequest, InferenceResponse};
use llamaforge_proto::management::management_service_server::{
    ManagementService, ManagementServiceServer,
};
use llamaforge_proto::management::{
    CancelRequestRequest, CancelRequestResponse, ListServersRequest, ListServersResponse,
    QueueStatusRequest, QueueStatusResponse, ReloadConfigRequest, ReloadConfigResponse,
    StartServerRequest, StartServerResponse, StopServerRequest, StopServerResponse,
};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{transport::Server, Request, Response, Status};
use tower::{Layer, Service};
use tower_http::catch_panic::CatchPanicLayer;

use super::{HttpAppState, InferenceEngine, TransportError};

pub struct GrpcInferenceService {
    inference_engine: Arc<dyn InferenceEngine>,
    /// Route validator. The gRPC `stream_inference` handler used to
    /// skip admission entirely — unknown models surfaced as
    /// `Status::Internal` via the inference engine's downstream
    /// failure rather than the documented `Code::NotFound`. Mirroring
    /// the HTTP shim's path: validate first, then dispatch.
    route_validator: Arc<dyn RouteValidator>,
}

#[tonic::async_trait]
impl InferenceService for GrpcInferenceService {
    type StreamInferenceStream = Pin<
        Box<dyn futures_core::Stream<Item = Result<InferenceResponse, Status>> + Send + 'static>,
    >;

    async fn stream_inference(
        &self,
        request: Request<InferenceRequest>,
    ) -> Result<Response<Self::StreamInferenceStream>, Status> {
        let req = request.into_inner();

        let mut messages = Vec::with_capacity(req.messages.len());
        for m in req.messages {
            let role = match m.role.as_str() {
                "system" => llamaforge_core::types::Role::System,
                "assistant" => llamaforge_core::types::Role::Assistant,
                _ => llamaforge_core::types::Role::User,
            };
            messages.push(llamaforge_core::types::Message { role, content: m.content });
        }

        let chat_req = llamaforge_core::types::ChatCompletionRequest {
            model: req.model.clone(),
            messages,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            stream: req.stream,
            logit_bias: if req.logit_bias.is_empty() {
                None
            } else {
                Some(req.logit_bias.into_iter().collect())
            },
            tools: if req.tools.is_empty() { None } else { serde_json::from_str(&req.tools).ok() },
            tool_choice: if req.tool_choice.is_empty() {
                None
            } else {
                serde_json::from_str(&req.tool_choice).ok()
            },
        };

        // Mirror the HTTP shim: validate the model is routable before
        // calling chat_completion. Map admission errors to the
        // documented gRPC codes — `NoServerForModel` → `NotFound`,
        // `QueueFull` → `ResourceExhausted` — rather than the
        // previous "everything is `Internal`" shape.
        let request_id = llamaforge_core::types::RequestId::new(uuid::Uuid::now_v7());
        let incoming = super::http::build_incoming_request(request_id, &chat_req);
        self.route_validator
            .validate_routable(&incoming.model_target, &incoming.capabilities)
            .map_err(map_admission_error)?;

        let mut response_stream = self
            .inference_engine
            .chat_completion(chat_req)
            .await
            .map_err(map_dispatch_error)?;

        let (tx, rx) = tokio::sync::mpsc::channel(128);

        tokio::spawn(async move {
            while let Some(chunk_res) = response_stream.next().await {
                let resp = match chunk_res {
                    Ok(chunk) => match serde_json::to_string(&chunk) {
                        Ok(payload) => Ok(InferenceResponse { payload }),
                        Err(e) => Err(Status::internal(e.to_string())),
                    },
                    Err(_e) => Err(Status::internal("Stream error")),
                };
                if tx.send(resp).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }
}

/// Map a domain `AdmissionError` to a gRPC `Status`. Mirrors the
/// HTTP shim's `From<AdmissionError> for ApiError`.
fn map_admission_error(err: llamaforge_core::types::traits::AdmissionError) -> Status {
    use llamaforge_core::types::traits::AdmissionError as Ae;
    match err {
        Ae::NoServerForModel { ref model } => {
            Status::not_found(format!("model not found: {}", model))
        }
        Ae::QueueFull | Ae::RateLimited { .. } => {
            let mut status = Status::resource_exhausted("queue is full");
            let retry_after = match err {
                Ae::RateLimited { retry_after_secs } => Some(retry_after_secs),
                _ => None,
            };
            if let Some(secs) = retry_after {
                if let Ok(hv) = secs.to_string().parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                    status.metadata_mut().insert("retry-after", hv);
                }
            }
            status
        }
        Ae::InvalidRequest { reason } => Status::invalid_argument(reason),
        Ae::Cancelled => Status::cancelled("request cancelled during admission"),
    }
}

/// Map a domain `DispatchError` to a gRPC `Status`.
fn map_dispatch_error(err: llamaforge_core::types::traits::DispatchError) -> Status {
    use llamaforge_core::types::traits::DispatchError as De;
    match err {
        De::NoServerForModel { ref model } => {
            Status::not_found(format!("model not found: {}", model))
        }
        De::QueueFull | De::RateLimited { .. } => {
            let mut status = Status::resource_exhausted("queue is full");
            let retry_after = match err {
                De::RateLimited { retry_after_secs } => Some(retry_after_secs),
                _ => None,
            };
            if let Some(secs) = retry_after {
                if let Ok(hv) = secs.to_string().parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>() {
                    status.metadata_mut().insert("retry-after", hv);
                }
            }
            status
        }
        De::Internal { reason } => Status::internal(reason),
    }
}

/// Map a domain `ServerError` to a gRPC `Status`. `NotFound` becomes
/// `Code::NotFound` so SDK clients see the documented status rather
/// than `Internal` (which signals "retry" in most SDK retry policies).
fn map_server_error(err: llamaforge_core::types::traits::ServerError) -> Status {
    use llamaforge_core::types::traits::ServerError as Se;
    match err {
        Se::NotFound { .. } => Status::not_found(err.to_string()),
        Se::AlreadyRunning { .. } => Status::already_exists(err.to_string()),
        Se::Unreachable { .. } => Status::unavailable(err.to_string()),
        Se::SpawnFailed { .. } => Status::internal(err.to_string()),
    }
}

pub struct GrpcManagementService<CW> {
    server_control: Arc<dyn ServerControl>,
    queue_submitter: Arc<dyn QueueSubmitter>,
    config_writer: Arc<CW>,
}

#[tonic::async_trait]
impl<CW> ManagementService for GrpcManagementService<CW>
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    async fn list_servers(
        &self,
        _request: Request<ListServersRequest>,
    ) -> Result<Response<ListServersResponse>, Status> {
        let summaries = self
            .server_control
            .list_servers()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let payload =
            serde_json::to_string(&summaries).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(ListServersResponse { payload }))
    }

    async fn start_server(
        &self,
        request: Request<StartServerRequest>,
    ) -> Result<Response<StartServerResponse>, Status> {
        let req = request.into_inner();
        llamaforge_core::config::validate_server_id_slug(&req.server_id)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let server_id = llamaforge_core::types::ServerId::from_config_id(&req.server_id);
        self.server_control.start_server(&server_id).await.map_err(map_server_error)?;
        Ok(Response::new(StartServerResponse {}))
    }

    async fn stop_server(
        &self,
        request: Request<StopServerRequest>,
    ) -> Result<Response<StopServerResponse>, Status> {
        let req = request.into_inner();
        llamaforge_core::config::validate_server_id_slug(&req.server_id)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let server_id = llamaforge_core::types::ServerId::from_config_id(&req.server_id);
        self.server_control.stop_server(&server_id).await.map_err(map_server_error)?;
        Ok(Response::new(StopServerResponse {}))
    }

    async fn queue_status(
        &self,
        _request: Request<QueueStatusRequest>,
    ) -> Result<Response<QueueStatusResponse>, Status> {
        let status = self.queue_submitter.status().map_err(|e| Status::internal(e.to_string()))?;
        let payload =
            serde_json::to_string(&status).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(QueueStatusResponse { payload }))
    }

    async fn cancel_request(
        &self,
        request: Request<CancelRequestRequest>,
    ) -> Result<Response<CancelRequestResponse>, Status> {
        let req = request.into_inner();
        let uuid = uuid::Uuid::parse_str(&req.request_id)
            .map_err(|_| Status::invalid_argument("Invalid request_id format"))?;
        let request_id = llamaforge_core::types::RequestId::new(uuid);
        self.queue_submitter.cancel(&request_id).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(CancelRequestResponse {}))
    }

    async fn reload_config(
        &self,
        _request: Request<ReloadConfigRequest>,
    ) -> Result<Response<ReloadConfigResponse>, Status> {
        // The gate lives inside `ConfigHandle::reload`: if the on-disk
        // diff touches a restart-required field, reload returns
        // `RestartRequired` *before* swapping the snapshot or
        // broadcasting. We map that to `failed_precondition` so the
        // operator sees an honest signal and no half-applied state.
        // Hot-reloadable diffs swap + broadcast normally; the
        // subscriber in `serve.rs` calls `apply_config_diff` on the
        // server-manager.
        match self.config_writer.reload().await {
            Ok(_) => Ok(Response::new(ReloadConfigResponse {})),
            Err(llamaforge_core::config::ConfigReloadError::RestartRequired { fields }) => {
                Err(Status::failed_precondition(format!(
                    "config diff touches restart-required server fields ({:?}); \
                     restart the orchestrator to apply",
                    fields
                )))
            }
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }
}

#[derive(Clone)]
pub struct AuthInterceptor {
    config_reader: Arc<dyn ConfigReader>,
}

impl tonic::service::Interceptor for AuthInterceptor {
    fn call(&mut self, request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        let config = self.config_reader.snapshot();
        if let Some(expected_key) = &config.global.api_key {
            if expected_key.is_empty() {
                return Ok(request);
            }

            match request.metadata().get("authorization") {
                Some(token)
                    if token.to_str().unwrap_or_default() == format!("Bearer {}", expected_key) =>
                {
                    Ok(request)
                }
                _ => {
                    metrics::counter!("llamaforge_auth_failures_total").increment(1);
                    Err(Status::unauthenticated("Authentication failed"))
                }
            }
        } else {
            Ok(request)
        }
    }
}

#[derive(Clone)]
pub struct GrpcMetricsLayer;

impl<S> Layer<S> for GrpcMetricsLayer {
    type Service = GrpcMetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcMetricsService { inner }
    }
}

#[derive(Clone)]
pub struct GrpcMetricsService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<axum::http::Request<ReqBody>> for GrpcMetricsService<S>
where
    S: Service<axum::http::Request<ReqBody>, Response = axum::http::Response<ResBody>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: axum::http::Request<ReqBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        std::mem::swap(&mut self.inner, &mut inner);

        let path = req.uri().path().to_string();

        // Skip metrics for non-grpc paths (like health checks if we want, but we'll include all)
        metrics::gauge!("llamaforge_active_connections", "transport" => "grpc").increment(1.0);
        let start = std::time::Instant::now();

        Box::pin(async move {
            let response = inner.call(req).await;

            let duration = start.elapsed().as_secs_f64();
            metrics::gauge!("llamaforge_active_connections", "transport" => "grpc").decrement(1.0);

            // Tonic sets the grpc-status in headers or trailers, but often it's ok HTTP 200 with grpc-status inside.
            // For simplicity, we just record "ok" or "error" if we can't extract grpc-status easily.
            let status = match &response {
                Ok(res) => {
                    if let Some(grpc_status) = res.headers().get("grpc-status") {
                        grpc_status.to_str().unwrap_or("unknown").to_string()
                    } else {
                        "0".to_string() // OK
                    }
                }
                Err(_) => "error".to_string(),
            };

            metrics::counter!("llamaforge_grpc_requests_total", "method" => path.clone(), "status" => status).increment(1);
            metrics::histogram!("llamaforge_grpc_request_duration_seconds", "method" => path)
                .record(duration);

            response
        })
    }
}

pub async fn start_grpc_server<CW, RMS, SM>(
    state: HttpAppState<CW, RMS, SM>, // Reuse HttpAppState or pass individual arcs
    listener: tokio::net::TcpListener,
    shutdown_rx: tokio::sync::watch::Receiver<()>,
) -> Result<(), TransportError>
where
    CW: ConfigWriter + Send + Sync + 'static,
    RMS: llamaforge_metrics::RequestMetricsStore + Send + Sync + 'static,
    SM: llamaforge_metrics::StoreMaintenance + Send + Sync + 'static,
{
    let config = state.config_reader.snapshot();

    let grpc_config = &config.grpc;
    let max_message_size = (grpc_config.max_message_size_mb * 1024 * 1024) as usize;
    let keepalive_interval =
        std::time::Duration::from_secs(grpc_config.keepalive_interval_secs as u64);
    let keepalive_timeout =
        std::time::Duration::from_secs(grpc_config.keepalive_timeout_secs as u64);
    let max_concurrent_streams = grpc_config.max_concurrent_streams;

    let inference_service = InferenceServiceServer::new(GrpcInferenceService {
        inference_engine: state.inference_engine.clone(),
        route_validator: state.route_validator.clone(),
    })
    .max_decoding_message_size(max_message_size)
    .max_encoding_message_size(max_message_size);

    let management_service = ManagementServiceServer::new(GrpcManagementService {
        server_control: state.server_control.clone(),
        queue_submitter: state.queue_submitter.clone(),
        config_writer: state.config_writer.clone(),
    })
    .max_decoding_message_size(max_message_size)
    .max_encoding_message_size(max_message_size);

    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter.clone().set_serving::<InferenceServiceServer<GrpcInferenceService>>().await;
    health_reporter
        .clone()
        .set_serving::<ManagementServiceServer<GrpcManagementService<CW>>>()
        .await;

    let auth_interceptor = AuthInterceptor { config_reader: state.config_reader.clone() };

    let bound = listener
        .local_addr()
        .map_err(|e| TransportError::Bootstrap(format!("grpc local_addr: {}", e)))?;
    tracing::info!(
        bind_address = %bound.ip(),
        port = bound.port(),
        tls = grpc_config.tls_enabled,
        "transport.started"
    );

    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let mut rx = shutdown_rx.clone();

    // Use builder
    let mut builder = Server::builder()
        .layer(CatchPanicLayer::new())
        .layer(GrpcMetricsLayer)
        .layer(tonic::service::interceptor::InterceptorLayer::new(auth_interceptor))
        .http2_keepalive_interval(Some(keepalive_interval))
        .http2_keepalive_timeout(Some(keepalive_timeout))
        .max_concurrent_streams(max_concurrent_streams);

    if grpc_config.tls_enabled {
        let cert_path = grpc_config.tls_cert.as_ref().ok_or_else(|| {
            TransportError::Bootstrap("grpc.tls_enabled=true requires grpc.tls_cert".to_string())
        })?;
        let key_path = grpc_config.tls_key.as_ref().ok_or_else(|| {
            TransportError::Bootstrap("grpc.tls_enabled=true requires grpc.tls_key".to_string())
        })?;
        let cert = std::fs::read(cert_path).map_err(|e| {
            TransportError::Bootstrap(format!("read tls_cert {}: {}", cert_path.display(), e))
        })?;
        let key = std::fs::read(key_path).map_err(|e| {
            TransportError::Bootstrap(format!("read tls_key {}: {}", key_path.display(), e))
        })?;
        let identity = tonic::transport::Identity::from_pem(cert, key);
        builder = builder
            .tls_config(tonic::transport::ServerTlsConfig::new().identity(identity))
            .map_err(|e| TransportError::Bootstrap(format!("tls config: {}", e)))?;
    }

    let server = builder
        .add_service(inference_service)
        .add_service(management_service)
        .add_service(health_service); // Health service might not need auth

    #[cfg(feature = "dev-reflection")]
    let server = {
        // `tonic-reflection` reads the `FileDescriptorSet` emitted by
        // `llamaforge-proto/build.rs` and serves the gRPC reflection
        // API. SDK-less clients (`grpcurl`, `evans`, gRPC UI) can
        // then discover services without shipping `.proto` files.
        // Gated behind the `dev-reflection` cargo feature (default-on
        // for dev / staging; production builds can opt out with
        // `--no-default-features`).
        let reflection = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(llamaforge_proto::FILE_DESCRIPTOR_SET)
            .build_v1()
            .map_err(|e| TransportError::Bootstrap(format!("reflection: {}", e)))?;
        server.add_service(reflection)
    };

    server
        .serve_with_incoming_shutdown(incoming, async move {
            let _ = rx.changed().await;
        })
        .await
        .map_err(|e| TransportError::Bootstrap(e.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::error::ApiError;
    use llamaforge_core::config::source::memory::InMemoryConfigSource;
    use llamaforge_core::config::ConfigHandle;
    use llamaforge_core::config::{AppConfig, GlobalConfig};
    use tonic::Code;

    #[test]
    fn test_grpc_error_status_mapping() {
        assert_eq!(Status::from(ApiError::InvalidBody("err".into())).code(), Code::InvalidArgument);
        assert_eq!(Status::from(ApiError::AuthFailure).code(), Code::Unauthenticated);
        assert_eq!(Status::from(ApiError::ModelNotFound).code(), Code::NotFound);
        assert_eq!(
            Status::from(ApiError::QueueFull { retry_after_secs: 30 }).code(),
            Code::ResourceExhausted
        );
        assert_eq!(Status::from(ApiError::BackendError).code(), Code::Internal);
        assert_eq!(Status::from(ApiError::Timeout).code(), Code::DeadlineExceeded);
        assert_eq!(Status::from(ApiError::Internal("err".into())).code(), Code::Internal);
    }

    #[tokio::test]
    async fn test_grpc_auth_interceptor() {
        let global = GlobalConfig { api_key: Some("secret".to_string()), ..Default::default() };
        let config = AppConfig { global, ..Default::default() };
        let source = Arc::new(InMemoryConfigSource::new(config));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());

        let mut interceptor = AuthInterceptor { config_reader: handle.clone() };

        // Request without token
        let req = tonic::Request::new(());
        let res = tonic::service::Interceptor::call(&mut interceptor, req);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code(), Code::Unauthenticated);

        // Request with wrong token
        let mut req = tonic::Request::new(());
        req.metadata_mut().insert("authorization", "Bearer wrong".parse().unwrap());
        let res = tonic::service::Interceptor::call(&mut interceptor, req);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().code(), Code::Unauthenticated);

        // Request with correct token
        let mut req = tonic::Request::new(());
        req.metadata_mut().insert("authorization", "Bearer secret".parse().unwrap());
        let res = tonic::service::Interceptor::call(&mut interceptor, req);
        assert!(res.is_ok());
    }

    use futures_util::stream;
    use llamaforge_core::types::{ChatCompletionChunk, StreamError};
    use llamaforge_proto::inference::inference_service_client::InferenceServiceClient;
    use llamaforge_proto::inference::InferenceRequest;

    struct MockEngine {
        chunks: std::sync::Mutex<Vec<Result<ChatCompletionChunk, StreamError>>>,
    }
    #[tonic::async_trait]
    impl InferenceEngine for MockEngine {
        async fn chat_completion(
            &self,
            _req: llamaforge_core::types::ChatCompletionRequest,
        ) -> Result<
            Pin<
                Box<
                    dyn futures_core::Stream<Item = Result<ChatCompletionChunk, StreamError>>
                        + Send
                        + 'static,
                >,
            >,
            llamaforge_core::types::traits::DispatchError,
        > {
            let chunks = std::mem::take(&mut *self.chunks.lock().unwrap());
            Ok(Box::pin(stream::iter(chunks)))
        }
    }

    struct MockRouteValidator;
    impl llamaforge_core::types::traits::RouteValidator for MockRouteValidator {
        fn validate_routable(
            &self,
            _model: &llamaforge_core::types::ModelTarget,
            _capabilities: &llamaforge_core::types::RequestCapabilities,
        ) -> Result<(), llamaforge_core::types::traits::AdmissionError> {
            Ok(())
        }
    }

    struct MockMetricsExporter;
    impl llamaforge_core::types::traits::MetricsExporter for MockMetricsExporter {
        fn gather_text(&self) -> String {
            "".into()
        }
    }

    struct MockServerControl;
    #[tonic::async_trait]
    impl llamaforge_core::types::traits::ServerControl for MockServerControl {
        async fn list_servers(
            &self,
        ) -> Result<
            Vec<llamaforge_core::types::traits::ServerSummary>,
            llamaforge_core::types::traits::ServerError,
        > {
            Ok(vec![])
        }
        async fn start_server(
            &self,
            _id: &llamaforge_core::types::ServerId,
        ) -> Result<(), llamaforge_core::types::traits::ServerError> {
            Ok(())
        }
        async fn stop_server(
            &self,
            _id: &llamaforge_core::types::ServerId,
        ) -> Result<(), llamaforge_core::types::traits::ServerError> {
            Ok(())
        }
        fn cluster_state(&self) -> u8 {
            1
        }
        fn is_gateway_initialized(&self) -> bool {
            true
        }
    }

    struct MockQueueSubmitter;
    impl llamaforge_core::types::traits::QueueSubmitter for MockQueueSubmitter {
        fn submit(
            &self,
            _req: llamaforge_core::types::traits::IncomingRequest,
        ) -> Result<
            llamaforge_core::types::traits::QueueTicket,
            llamaforge_core::types::traits::AdmissionError,
        > {
            Ok(llamaforge_core::types::traits::QueueTicket {
                request_id: llamaforge_core::types::RequestId::new(uuid::Uuid::now_v7()),
                admitted_at: chrono::Utc::now(),
                cancel_token: tokio_util::sync::CancellationToken::new(),
                events: tokio::sync::mpsc::channel(1).1,
            })
        }
        fn cancel(
            &self,
            _id: &llamaforge_core::types::RequestId,
        ) -> Result<(), llamaforge_core::types::traits::QueueError> {
            Ok(())
        }
        fn status(
            &self,
        ) -> Result<
            llamaforge_core::types::traits::QueueSnapshot,
            llamaforge_core::types::traits::QueueError,
        > {
            Ok(llamaforge_core::types::traits::QueueSnapshot {
                depth: 0,
                estimated_throughput_req_sec: 1.0,
            })
        }
    }

    #[tokio::test]
    async fn test_grpc_streaming_integration() {
        let (tx, rx) = tokio::sync::watch::channel(());
        let global = GlobalConfig { api_key: None, grpc_port: 0, ..Default::default() };
        let config = AppConfig { global, ..Default::default() };
        let source = Arc::new(InMemoryConfigSource::new(config));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());

        let chunks = vec![
            Ok(ChatCompletionChunk::Token("Hel".into())),
            Ok(ChatCompletionChunk::Token("lo".into())),
            Ok(ChatCompletionChunk::Stop {
                finish_reason: "stop".to_string(),
                output_tokens: 0,
                prompt_tokens: 0,
            }),
        ];

        let state = HttpAppState {
            config_reader: handle.clone(),
            config_writer: handle.clone(),
            route_validator: Arc::new(MockRouteValidator),
            inference_engine: Arc::new(MockEngine { chunks: std::sync::Mutex::new(chunks) }),
            metrics_exporter: Arc::new(MockMetricsExporter),
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitter),
            request_metrics_store: Arc::new(llamaforge_metrics::InMemoryStore::default()),
            store_maintenance: Arc::new(llamaforge_metrics::InMemoryStore::default()),
            large_body_limit: crate::transport::middleware::LargeBodyConcurrencyLimit::new(10, 1),
        };

        // Bind ephemeral port so the test isn't flaky on port-collisions.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bound = listener.local_addr().unwrap();
        let state2 = HttpAppState { config_reader: handle.clone(), ..state.clone() };

        let server_task = tokio::spawn(async move {
            let _ = start_grpc_server(state2, listener, rx).await;
        });

        // Give it a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let url = format!("http://{}", bound);
        let mut client = InferenceServiceClient::connect(url).await.unwrap();

        let req = tonic::Request::new(InferenceRequest {
            model: "test".to_string(),
            messages: vec![llamaforge_proto::inference::ChatMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            max_tokens: None,
            temperature: None,
            stream: true,
            logit_bias: std::collections::HashMap::new(),
            tools: String::new(),
            tool_choice: String::new(),
        });

        let mut stream = client.stream_inference(req).await.unwrap().into_inner();

        let mut responses = Vec::new();
        while let Some(res) = stream.message().await.unwrap() {
            responses.push(res.payload);
        }

        assert_eq!(responses.len(), 3);
        assert!(responses[0].contains("Hel"));
        assert!(responses[1].contains("lo"));
        assert!(responses[2].contains("stop"));

        let _ = tx.send(());
        let _ = server_task.await;
    }

    #[tokio::test]
    async fn test_grpc_health_check() {
        let (tx, rx) = tokio::sync::watch::channel(());
        let global = GlobalConfig { api_key: None, grpc_port: 0, ..Default::default() };
        let config = AppConfig { global, ..Default::default() };
        let source = Arc::new(InMemoryConfigSource::new(config));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());

        let state = HttpAppState {
            config_reader: handle.clone(),
            config_writer: handle.clone(),
            route_validator: Arc::new(MockRouteValidator),
            inference_engine: Arc::new(MockEngine { chunks: std::sync::Mutex::new(vec![]) }),
            metrics_exporter: Arc::new(MockMetricsExporter),
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitter),
            request_metrics_store: Arc::new(llamaforge_metrics::InMemoryStore::default()),
            store_maintenance: Arc::new(llamaforge_metrics::InMemoryStore::default()),
            large_body_limit: crate::transport::middleware::LargeBodyConcurrencyLimit::new(10, 1),
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bound = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            let _ = start_grpc_server(state, listener, rx).await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let url = format!("http://{}", bound);
        let channel = tonic::transport::Channel::from_static(Box::leak(url.into_boxed_str()))
            .connect()
            .await
            .unwrap();
        let mut client = tonic_health::pb::health_client::HealthClient::new(channel);
        let request = tonic::Request::new(tonic_health::pb::HealthCheckRequest {
            service: "llamaforge.inference.InferenceService".to_string(),
        });

        let response = client.check(request).await.unwrap().into_inner();
        assert_eq!(
            response.status,
            tonic_health::pb::health_check_response::ServingStatus::Serving as i32
        );

        let _ = tx.send(());
        let _ = server_task.await;
    }

    #[test]
    fn test_grpc_rate_limited_mapping() {
        use llamaforge_core::types::traits::{AdmissionError, DispatchError};

        // Case 1: DispatchError::RateLimited
        let de = DispatchError::RateLimited { retry_after_secs: 123 };
        let status = map_dispatch_error(de);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
        assert_eq!(status.metadata().get("retry-after").unwrap(), "123");

        // Case 2: AdmissionError::RateLimited
        let ae = AdmissionError::RateLimited { retry_after_secs: 456 };
        let status = map_admission_error(ae);
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);
        assert_eq!(status.metadata().get("retry-after").unwrap(), "456");
    }
}
