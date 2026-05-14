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
pub use llamaforge_types::traits::{
    MetricsExporter, QueueSubmitter, RouteValidator, ServerControl,
};
pub use middleware::LargeBodyConcurrencyLimit;

use futures_core::Stream;
use llamaforge_config::{ConfigReader, ConfigWriter};
use llamaforge_metrics::{RequestMetricsStore, StoreMaintenance};
use llamaforge_types::{ChatCompletionChunk, ChatCompletionRequest, StreamError};
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
        llamaforge_types::traits::DispatchError,
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
}

impl TransportHandles {
    pub fn new(
        http_handle: tokio::task::JoinHandle<()>,
        grpc_handle: tokio::task::JoinHandle<()>,
        shutdown_tx: tokio::sync::watch::Sender<()>,
    ) -> Self {
        Self { http_handle, grpc_handle, shutdown_tx }
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

    // Extract configurations
    // Local invariant: max_concurrent_large_requests and large_body_threshold_mb will be added to the schema in v1.1.
    // For now, we enforce safe defaults matching the design bounds.
    let max_concurrent_large_requests = 5;
    let large_body_threshold_mb = 1;
    let large_body_limit = middleware::LargeBodyConcurrencyLimit::new(
        max_concurrent_large_requests,
        large_body_threshold_mb,
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

    // Start HTTP server
    let http_shutdown_rx = shutdown_rx.clone();
    let http_handle = tokio::spawn(async move {
        if let Err(e) = http::start_http_server(http_state, http_shutdown_rx).await {
            tracing::error!(
                error = %e,
                "transport.http_server_failed"
            );
        }
    });

    // Start gRPC server
    let grpc_shutdown_rx = shutdown_rx.clone();
    let grpc_handle = tokio::spawn(async move {
        if let Err(e) = grpc::start_grpc_server(grpc_state, grpc_shutdown_rx).await {
            tracing::error!(
                error = %e,
                "transport.grpc_server_failed"
            );
        }
    });

    Ok(TransportHandles::new(http_handle, grpc_handle, shutdown_tx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_core::Stream;
    use futures_util::stream;
    use llamaforge_config::{
        source::memory::InMemoryConfigSource, AppConfig, ConfigHandle, GlobalConfig,
    };
    use llamaforge_types::{traits::*, ChatCompletionChunk, StreamError};

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
            Ok(Box::pin(stream::iter(vec![Ok(ChatCompletionChunk::Token("test".into()))])))
        }
    }

    struct MockRouteValidator;
    impl RouteValidator for MockRouteValidator {
        fn validate_routable(&self, _req: &IncomingRequest) -> Result<(), AdmissionError> {
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
        async fn start_server(&self, _id: &llamaforge_types::ServerId) -> Result<(), ServerError> {
            Ok(())
        }
        async fn stop_server(&self, _id: &llamaforge_types::ServerId) -> Result<(), ServerError> {
            Ok(())
        }
    }

    struct MockQueueSubmitter;
    impl QueueSubmitter for MockQueueSubmitter {
        fn submit(&self, _req: IncomingRequest) -> Result<QueueTicket, AdmissionError> {
            Ok(QueueTicket {})
        }
        fn cancel(&self, _id: &llamaforge_types::RequestId) -> Result<(), QueueError> {
            Ok(())
        }
        fn status(&self) -> Result<QueueSnapshot, QueueError> {
            Ok(QueueSnapshot { depth: 0, estimated_throughput_req_sec: 1.0 })
        }
    }

    fn build_test_deps(
        http_port: u16,
        grpc_port: u16,
        sleep_ms: u64,
    ) -> TransportDeps<
        ConfigHandle,
        llamaforge_metrics::InMemoryStore,
        llamaforge_metrics::InMemoryStore,
    > {
        let global = GlobalConfig { http_port, grpc_port, ..Default::default() };
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
        let deps = build_test_deps(50058, 50059, 0);
        let handles = start_servers(deps).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let http_resp = reqwest::get("http://127.0.0.1:50058/health").await.unwrap();
        assert_eq!(http_resp.status(), 200);

        let channel = tonic::transport::Channel::from_static("http://127.0.0.1:50059")
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
        let deps = build_test_deps(50060, 50061, 500); // 500ms sleep in engine
        let handles = start_servers(deps).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

        let body = serde_json::json!({
            "model": "test",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": false
        });

        let req_future = tokio::spawn(async move {
            client.post("http://127.0.0.1:50060/v1/chat/completions").json(&body).send().await
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        handles.shutdown();

        let res = req_future.await.unwrap().unwrap();
        assert_eq!(res.status(), 200);

        let client2 = reqwest::Client::new();
        let res_new = client2
            .post("http://127.0.0.1:50060/v1/chat/completions")
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
        let deps = build_test_deps(50062, 50063, 0);
        let handles = start_servers(deps).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let res = reqwest::get("http://127.0.0.1:50062/metrics").await.unwrap();
        assert_eq!(res.status(), 200);

        let text = res.text().await.unwrap();
        assert_eq!(text, ""); // Mock exporter returns empty string

        handles.shutdown();
        handles.wait().await.unwrap();
    }
}
