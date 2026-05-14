use futures_core::future::BoxFuture;
use futures_util::StreamExt;
use llamaforge_config::{ConfigReader, ConfigWriter};
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
use llamaforge_types::traits::{QueueSubmitter, ServerControl};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{transport::Server, Request, Response, Status};
use tower::{Layer, Service};
use tower_http::catch_panic::CatchPanicLayer;

use crate::{openai::OpenAiChatRequest, HttpAppState, InferenceEngine, TransportError};

pub struct GrpcInferenceService {
    inference_engine: Arc<dyn InferenceEngine>,
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

        let openai_req: OpenAiChatRequest = serde_json::from_str(&req.payload)
            .map_err(|e| Status::invalid_argument(format!("Invalid payload: {}", e)))?;

        let mut chat_req =
            crate::openai::parse_openai_request(openai_req).map_err(tonic::Status::from)?;

        chat_req.model = req.model;

        let mut response_stream = self
            .inference_engine
            .chat_completion(chat_req)
            .await
            .map_err(|e| tonic::Status::internal(e.to_string()))?;

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
        let uuid = uuid::Uuid::parse_str(&req.server_id)
            .map_err(|_| Status::invalid_argument("Invalid server_id"))?;
        let server_id = llamaforge_types::ServerId::new(uuid);
        self.server_control
            .start_server(&server_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(StartServerResponse {}))
    }

    async fn stop_server(
        &self,
        request: Request<StopServerRequest>,
    ) -> Result<Response<StopServerResponse>, Status> {
        let req = request.into_inner();
        let uuid = uuid::Uuid::parse_str(&req.server_id)
            .map_err(|_| Status::invalid_argument("Invalid server_id"))?;
        let server_id = llamaforge_types::ServerId::new(uuid);
        self.server_control
            .stop_server(&server_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
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
        let request_id = llamaforge_types::RequestId::new(uuid);
        self.queue_submitter.cancel(&request_id).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(CancelRequestResponse {}))
    }

    async fn reload_config(
        &self,
        _request: Request<ReloadConfigRequest>,
    ) -> Result<Response<ReloadConfigResponse>, Status> {
        self.config_writer.reload().await.map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(ReloadConfigResponse {}))
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

pub async fn start_grpc_server<CW>(
    state: HttpAppState<CW>, // Reuse HttpAppState or pass individual arcs
    shutdown_rx: tokio::sync::watch::Receiver<()>,
) -> Result<(), TransportError>
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    let config = state.config_reader.snapshot();
    let addr = std::net::SocketAddr::new(config.global.bind_address, config.global.grpc_port);

    let grpc_config = &config.grpc;
    let max_message_size = (grpc_config.max_message_size_mb * 1024 * 1024) as usize;
    let keepalive_interval =
        std::time::Duration::from_secs(grpc_config.keepalive_interval_secs as u64);
    let keepalive_timeout =
        std::time::Duration::from_secs(grpc_config.keepalive_timeout_secs as u64);
    let max_concurrent_streams = grpc_config.max_concurrent_streams;

    let inference_service = InferenceServiceServer::new(GrpcInferenceService {
        inference_engine: state.inference_engine.clone(),
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

    tracing::info!("transport.started gRPC server on {}", addr);

    let mut rx = shutdown_rx.clone();

    // Use builder
    let mut builder = Server::builder()
        .layer(CatchPanicLayer::new())
        .layer(GrpcMetricsLayer)
        .layer(tonic::service::interceptor::InterceptorLayer::new(auth_interceptor))
        .http2_keepalive_interval(Some(keepalive_interval))
        .http2_keepalive_timeout(Some(keepalive_timeout))
        .max_concurrent_streams(max_concurrent_streams);

    if let (Some(_cert_path), Some(_key_path)) = (&grpc_config.tls_cert, &grpc_config.tls_key) {
        // TODO: Load certificates and configure TLS
    }

    builder
        .add_service(inference_service)
        .add_service(management_service)
        .add_service(health_service) // Health service might not need auth
        .serve_with_shutdown(addr, async move {
            let _ = rx.changed().await;
        })
        .await
        .map_err(|e| TransportError::Bootstrap(e.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ApiError;
    use llamaforge_config::source::memory::InMemoryConfigSource;
    use llamaforge_config::ConfigHandle;
    use llamaforge_config::{AppConfig, GlobalConfig};
    use tonic::Code;

    #[test]
    fn test_grpc_error_status_mapping() {
        assert_eq!(Status::from(ApiError::InvalidBody("err".into())).code(), Code::InvalidArgument);
        assert_eq!(Status::from(ApiError::AuthFailure).code(), Code::Unauthenticated);
        assert_eq!(Status::from(ApiError::ModelNotFound).code(), Code::NotFound);
        assert_eq!(Status::from(ApiError::QueueFull).code(), Code::ResourceExhausted);
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
    use llamaforge_proto::inference::inference_service_client::InferenceServiceClient;
    use llamaforge_proto::inference::InferenceRequest;
    use llamaforge_types::{ChatCompletionChunk, StreamError};

    struct MockEngine {
        chunks: Vec<Result<ChatCompletionChunk, StreamError>>,
    }
    #[tonic::async_trait]
    impl InferenceEngine for MockEngine {
        async fn chat_completion(
            &self,
            _req: llamaforge_types::ChatCompletionRequest,
        ) -> Result<
            Pin<
                Box<
                    dyn futures_core::Stream<Item = Result<ChatCompletionChunk, StreamError>>
                        + Send
                        + 'static,
                >,
            >,
            llamaforge_types::traits::DispatchError,
        > {
            let chunks = self.chunks.clone();
            Ok(Box::pin(stream::iter(chunks)))
        }
    }

    struct MockRouteValidator;
    impl llamaforge_types::traits::RouteValidator for MockRouteValidator {
        fn validate_routable(
            &self,
            _req: &llamaforge_types::traits::IncomingRequest,
        ) -> Result<(), llamaforge_types::traits::AdmissionError> {
            Ok(())
        }
    }

    struct MockMetricsExporter;
    impl llamaforge_types::traits::MetricsExporter for MockMetricsExporter {
        fn gather_text(&self) -> String {
            "".into()
        }
    }

    struct MockServerControl;
    #[tonic::async_trait]
    impl llamaforge_types::traits::ServerControl for MockServerControl {
        async fn list_servers(
            &self,
        ) -> Result<
            Vec<llamaforge_types::traits::ServerSummary>,
            llamaforge_types::traits::ServerError,
        > {
            Ok(vec![])
        }
        async fn start_server(
            &self,
            _id: &llamaforge_types::ServerId,
        ) -> Result<(), llamaforge_types::traits::ServerError> {
            Ok(())
        }
        async fn stop_server(
            &self,
            _id: &llamaforge_types::ServerId,
        ) -> Result<(), llamaforge_types::traits::ServerError> {
            Ok(())
        }
    }

    struct MockQueueSubmitter;
    impl llamaforge_types::traits::QueueSubmitter for MockQueueSubmitter {
        fn submit(
            &self,
            _req: llamaforge_types::traits::IncomingRequest,
        ) -> Result<llamaforge_types::traits::QueueTicket, llamaforge_types::traits::AdmissionError>
        {
            Ok(llamaforge_types::traits::QueueTicket {})
        }
        fn cancel(
            &self,
            _id: &llamaforge_types::RequestId,
        ) -> Result<(), llamaforge_types::traits::QueueError> {
            Ok(())
        }
        fn status(
            &self,
        ) -> Result<llamaforge_types::traits::QueueSnapshot, llamaforge_types::traits::QueueError>
        {
            Ok(llamaforge_types::traits::QueueSnapshot {})
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
            Ok(ChatCompletionChunk::Stop("stop".into())),
        ];

        let state = HttpAppState {
            config_reader: handle.clone(),
            config_writer: handle.clone(),
            route_validator: Arc::new(MockRouteValidator),
            inference_engine: Arc::new(MockEngine { chunks }),
            metrics_exporter: Arc::new(MockMetricsExporter),
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitter),
            large_body_limit: crate::middleware::LargeBodyConcurrencyLimit::new(10, 1),
        };

        // We need a specific port to connect to. We can just use a fixed port for this test or let it fail if in use.
        // Or we can just start it and let it run.
        let mut config2 = AppConfig::default();
        config2.global.grpc_port = 50055;
        let source2 = Arc::new(InMemoryConfigSource::new(config2));
        let handle2 = Arc::new(ConfigHandle::load(source2).unwrap());
        let state2 = HttpAppState { config_reader: handle2.clone(), ..state.clone() };

        let server_task = tokio::spawn(async move {
            let _ = start_grpc_server(state2, rx).await;
        });

        // Give it a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut client = InferenceServiceClient::connect("http://127.0.0.1:50055").await.unwrap();

        let payload = serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        });

        let req = tonic::Request::new(InferenceRequest {
            model: "test".to_string(),
            payload: payload.to_string(),
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
        let global = GlobalConfig { api_key: None, grpc_port: 50056, ..Default::default() };
        let config = AppConfig { global, ..Default::default() };
        let source = Arc::new(InMemoryConfigSource::new(config));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());

        let state = HttpAppState {
            config_reader: handle.clone(),
            config_writer: handle.clone(),
            route_validator: Arc::new(MockRouteValidator),
            inference_engine: Arc::new(MockEngine { chunks: vec![] }),
            metrics_exporter: Arc::new(MockMetricsExporter),
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitter),
            large_body_limit: crate::middleware::LargeBodyConcurrencyLimit::new(10, 1),
        };

        let server_task = tokio::spawn(async move {
            let _ = start_grpc_server(state, rx).await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let channel = tonic::transport::Channel::from_static("http://127.0.0.1:50056")
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
}
