//! Dual-protocol transport gateway: gRPC on :50051 (canonical) plus an
//! HTTP/SSE shim for OpenAI- and Anthropic-compatible SDK clients.
//!
//! Implements design.md §5.1 (Transport Layer) and §7 (API
//! Specification). Component spec:
//! `docs/component-designs/07_TRANSPORT_LAYER.md`.

pub(crate) mod anthropic;
pub(crate) mod error;
pub(crate) mod grpc;
pub(crate) mod http;
pub(crate) mod management;
pub(crate) mod middleware;
pub(crate) mod openai;

pub use error::{AnthropicProtocolError, ApiError, TransportError};
pub use grpc::start_grpc_server;
pub use http::{start_http_server, HttpAppState};
pub use llamaforge_core::types::traits::{
    MetricsExporter, QueueSubmitter, RouteValidator, ServerControl,
};
pub use middleware::LargeBodyConcurrencyLimit;

use futures_core::Stream;
use llamaforge_core::config::{ConfigReader, ConfigWriter};
use llamaforge_core::types::{ChatCompletionChunk, ChatCompletionRequest, StreamError};
use llamaforge_metrics::{RequestMetricsStore, StoreMaintenance};
use std::pin::Pin;
use std::sync::Arc;

/// InferenceEngine protocol-agnostic core trait.
#[async_trait::async_trait]
pub trait InferenceEngine: Send + Sync + 'static {
    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>,
        llamaforge_core::types::traits::DispatchError,
    >;
}

pub struct TransportDeps<CW, RMS, SM>
where
    CW: ConfigWriter + 'static,
    RMS: RequestMetricsStore + 'static,
    SM: StoreMaintenance + 'static,
{
    pub config_reader: Arc<dyn ConfigReader>,
    pub config_writer: Arc<CW>,
    pub server_control: Arc<dyn ServerControl>,
    pub queue_submitter: Arc<dyn QueueSubmitter>,
    pub metrics_exporter: Arc<dyn MetricsExporter>,
    pub request_metrics_store: Arc<RMS>,
    pub store_maintenance: Arc<SM>,
    pub route_validator: Arc<dyn RouteValidator>,
    pub inference_engine: Arc<dyn InferenceEngine>,
}

pub struct TransportHandles {
    http_handle: tokio::task::JoinHandle<()>,
    grpc_handle: tokio::task::JoinHandle<()>,
    shutdown_tx: tokio::sync::watch::Sender<()>,
    pub http_addr: std::net::SocketAddr,
    pub grpc_addr: std::net::SocketAddr,
}

impl TransportHandles {
    pub fn new(
        http_handle: tokio::task::JoinHandle<()>,
        grpc_handle: tokio::task::JoinHandle<()>,
        shutdown_tx: tokio::sync::watch::Sender<()>,
        http_addr: std::net::SocketAddr,
        grpc_addr: std::net::SocketAddr,
    ) -> Self {
        Self { http_handle, grpc_handle, shutdown_tx, http_addr, grpc_addr }
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }

    pub async fn wait(self) -> Result<(), TransportError> {
        let (http_res, grpc_res) = tokio::join!(self.http_handle, self.grpc_handle);
        http_res.map_err(TransportError::ServerTaskFailed)?;
        grpc_res.map_err(TransportError::ServerTaskFailed)?;
        Ok(())
    }
}

pub async fn start_servers<CW, RMS, SM>(
    deps: TransportDeps<CW, RMS, SM>,
) -> Result<TransportHandles, TransportError>
where
    CW: ConfigWriter + Send + Sync + 'static,
    RMS: RequestMetricsStore + Send + Sync + 'static,
    SM: StoreMaintenance + Send + Sync + 'static,
{
    // Configure gracefully shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());

    // Extract gateway concurrency settings from config at boot. Hot-reload of
    // the gateway section is observable via `ConfigChange::GatewayConfigChanged`
    // but isn't propagated into the running limiter today; subscribers can
    // restart the gateway layer if they need a new bound. See design doc §6.2.
    let gateway = deps.config_reader.snapshot().gateway.clone();
    let large_body_limit = middleware::LargeBodyConcurrencyLimit::new(
        gateway.max_concurrent_large_requests,
        gateway.large_body_threshold_mb,
    );

    // Prepare HttpAppState
    let http_state = HttpAppState {
        config_reader: deps.config_reader.clone(),
        route_validator: deps.route_validator.clone(),
        inference_engine: deps.inference_engine.clone(),
        metrics_exporter: deps.metrics_exporter.clone(),
        server_control: deps.server_control.clone(),
        queue_submitter: deps.queue_submitter.clone(),
        config_writer: deps.config_writer.clone(),
        request_metrics_store: deps.request_metrics_store.clone(),
        store_maintenance: deps.store_maintenance.clone(),
        large_body_limit,
    };

    let grpc_state = http_state.clone();

    // Bind both listeners up-front so we can expose the resolved (possibly
    // ephemeral) ports on TransportHandles. http_port = 0 / grpc_port = 0
    // in config means "let the OS pick", which the e2e harness relies on.
    let snap = http_state.config_reader.snapshot();
    let http_bind = std::net::SocketAddr::new(snap.global.bind_address, snap.global.http_port);
    let grpc_bind = std::net::SocketAddr::new(snap.global.bind_address, snap.global.grpc_port);
    drop(snap);

    let http_listener = tokio::net::TcpListener::bind(&http_bind)
        .await
        .map_err(|e| TransportError::Bootstrap(format!("bind http {}: {}", http_bind, e)))?;
    let http_addr = http_listener
        .local_addr()
        .map_err(|e| TransportError::Bootstrap(format!("http local_addr: {}", e)))?;

    let grpc_listener = tokio::net::TcpListener::bind(&grpc_bind)
        .await
        .map_err(|e| TransportError::Bootstrap(format!("bind grpc {}: {}", grpc_bind, e)))?;
    let grpc_addr = grpc_listener
        .local_addr()
        .map_err(|e| TransportError::Bootstrap(format!("grpc local_addr: {}", e)))?;

    tracing::info!(
        bind_address = %http_addr.ip(),
        port = http_addr.port(),
        tls = false,
        "transport.started"
    );

    // Start HTTP server
    let http_shutdown_rx = shutdown_rx.clone();
    let http_handle = tokio::spawn(async move {
        if let Err(e) = http::start_http_server(http_state, http_listener, http_shutdown_rx).await {
            tracing::error!(
                error = %e,
                "transport.http_server_failed"
            );
        }
    });

    // Start gRPC server
    let grpc_shutdown_rx = shutdown_rx.clone();
    let grpc_handle = tokio::spawn(async move {
        if let Err(e) = grpc::start_grpc_server(grpc_state, grpc_listener, grpc_shutdown_rx).await {
            tracing::error!(
                error = %e,
                "transport.grpc_server_failed"
            );
        }
    });

    Ok(TransportHandles::new(http_handle, grpc_handle, shutdown_tx, http_addr, grpc_addr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_core::Stream;
    use futures_util::stream;
    use llamaforge_core::config::{
        source::memory::InMemoryConfigSource, AppConfig, ConfigHandle, GlobalConfig,
    };
    use llamaforge_core::types::{traits::*, ChatCompletionChunk, StreamError};

    struct MockEngine {
        sleep_ms: u64,
    }
    #[async_trait::async_trait]
    impl InferenceEngine for MockEngine {
        async fn chat_completion(
            &self,
            _req: ChatCompletionRequest,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>,
            DispatchError,
        > {
            if self.sleep_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
            }
            // Emit a terminal `Stop` chunk so the non-streaming
            // aggregator's `saw_stop` warming-window guard doesn't
            // 503 this fixture. The test's intent — graceful shutdown
            // drains in-flight requests — is unchanged.
            Ok(Box::pin(stream::iter(vec![
                Ok(ChatCompletionChunk::Token("test".into())),
                Ok(ChatCompletionChunk::Stop {
                    finish_reason: "stop".to_string(),
                    output_tokens: 42,
                    prompt_tokens: 10,
                }),
            ])))
        }
    }

    struct MockRouteValidator;
    impl RouteValidator for MockRouteValidator {
        fn validate_routable(
            &self,
            _model: &llamaforge_core::types::ModelTarget,
            _capabilities: &llamaforge_core::types::RequestCapabilities,
        ) -> Result<(), AdmissionError> {
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
        async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError> {
            Ok(vec![])
        }
        async fn start_server(
            &self,
            _id: &llamaforge_core::types::ServerId,
        ) -> Result<(), ServerError> {
            Ok(())
        }
        async fn stop_server(
            &self,
            _id: &llamaforge_core::types::ServerId,
        ) -> Result<(), ServerError> {
            Ok(())
        }
    }

    struct MockQueueSubmitter;
    impl QueueSubmitter for MockQueueSubmitter {
        fn submit(&self, _req: IncomingRequest) -> Result<QueueTicket, AdmissionError> {
            Ok(QueueTicket {
                request_id: llamaforge_core::types::RequestId::new(uuid::Uuid::now_v7()),
                admitted_at: chrono::Utc::now(),
                cancel_token: tokio_util::sync::CancellationToken::new(),
                events: tokio::sync::mpsc::channel(1).1,
            })
        }
        fn cancel(&self, _id: &llamaforge_core::types::RequestId) -> Result<(), QueueError> {
            Ok(())
        }
        fn status(&self) -> Result<QueueSnapshot, QueueError> {
            Ok(QueueSnapshot { depth: 0, estimated_throughput_req_sec: 1.0 })
        }
    }

    fn build_test_deps(
        sleep_ms: u64,
    ) -> TransportDeps<
        ConfigHandle,
        llamaforge_metrics::InMemoryStore,
        llamaforge_metrics::InMemoryStore,
    > {
        // Bind on ephemeral ports; `TransportHandles` carries the
        // resolved addresses back to the test.
        let global = GlobalConfig { http_port: 0, grpc_port: 0, ..Default::default() };
        let config = AppConfig { global, ..Default::default() };
        let source = Arc::new(InMemoryConfigSource::new(config));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());
        let store = Arc::new(llamaforge_metrics::InMemoryStore::default());

        TransportDeps {
            config_reader: handle.clone(),
            config_writer: handle.clone(),
            route_validator: Arc::new(MockRouteValidator),
            inference_engine: Arc::new(MockEngine { sleep_ms }),
            metrics_exporter: Arc::new(MockMetricsExporter),
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitter),
            request_metrics_store: store.clone(),
            store_maintenance: store,
        }
    }

    #[tokio::test]
    async fn test_dual_server_startup_and_shutdown() {
        let deps = build_test_deps(0);
        let handles = start_servers(deps).await.unwrap();
        let http_addr = handles.http_addr;
        let grpc_addr = handles.grpc_addr;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let http_resp = reqwest::get(format!("http://{}/health", http_addr)).await.unwrap();
        assert_eq!(http_resp.status(), 200);

        let channel = tonic::transport::Endpoint::new(format!("http://{}", grpc_addr))
            .unwrap()
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

        handles.shutdown();
        handles.wait().await.unwrap();
    }

    #[tokio::test]
    async fn test_graceful_shutdown() {
        let deps = build_test_deps(500); // 500ms sleep in engine
        let handles = start_servers(deps).await.unwrap();
        let url = format!("http://{}/v1/chat/completions", handles.http_addr);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

        let body = serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": false
        });

        let url_for_task = url.clone();
        let req_future =
            tokio::spawn(async move { client.post(&url_for_task).json(&body).send().await });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        handles.shutdown();

        let res = req_future.await.unwrap().unwrap();
        assert_eq!(res.status(), 200);

        let client2 = reqwest::Client::new();
        let res_new = client2
            .post(&url)
            .json(&serde_json::json!({
                "model": "test",
                "messages": [{"role": "user", "content": "hi"}],
                "stream": false
            }))
            .send()
            .await;
        assert!(res_new.is_err());

        handles.wait().await.unwrap();
    }

    #[tokio::test]
    async fn test_prometheus_endpoint() {
        let deps = build_test_deps(0);
        let handles = start_servers(deps).await.unwrap();
        let http_addr = handles.http_addr;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let res = reqwest::get(format!("http://{}/metrics", http_addr)).await.unwrap();
        assert_eq!(res.status(), 200);

        let text = res.text().await.unwrap();
        assert_eq!(text, ""); // Mock exporter returns empty string

        handles.shutdown();
        handles.wait().await.unwrap();
    }
}
