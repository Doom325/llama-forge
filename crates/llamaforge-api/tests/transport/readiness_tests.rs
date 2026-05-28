#[path = "mocks.rs"]
mod mocks;

use llamaforge_api::transport::{start_servers, TransportDeps};
use llamaforge_core::config::{
    source::memory::InMemoryConfigSource, AppConfig, ConfigHandle, GlobalConfig,
};
use llamaforge_metrics::InMemoryStore;
use reqwest::{Client, StatusCode};
use std::sync::Arc;
use llamaforge_core::types::traits::*;
use llamaforge_core::types::{ChatCompletionChunk, ServerId};

struct MockServerControlState {
    cluster_state: std::sync::atomic::AtomicU8,
    initialized: std::sync::atomic::AtomicBool,
}

struct StatefulMockServerControl {
    state: Arc<MockServerControlState>,
}

#[async_trait::async_trait]
impl ServerControl for StatefulMockServerControl {
    async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError> {
        Ok(vec![])
    }
    async fn start_server(&self, _id: &ServerId) -> Result<(), ServerError> {
        Ok(())
    }
    async fn stop_server(&self, _id: &ServerId) -> Result<(), ServerError> {
        Ok(())
    }
    fn cluster_state(&self) -> u8 {
        self.state.cluster_state.load(std::sync::atomic::Ordering::Acquire)
    }
    fn is_gateway_initialized(&self) -> bool {
        self.state.initialized.load(std::sync::atomic::Ordering::Acquire)
    }
}

async fn setup_test_server(
    server_control: Arc<dyn ServerControl>,
) -> (llamaforge_api::transport::TransportHandles, Arc<ConfigHandle>) {
    let mut config = AppConfig { global: GlobalConfig::default(), ..Default::default() };
    config.global.http_port = 0;
    config.global.grpc_port = 0;

    let source = Arc::new(InMemoryConfigSource::new(config));
    let handle = Arc::new(ConfigHandle::load(source).unwrap());

    let deps = TransportDeps {
        config_reader: handle.clone(),
        route_validator: Arc::new(mocks::MockRouteValidator),
        inference_engine: Arc::new(mocks::MockEngine {
            chunks: std::sync::Mutex::new(vec![Ok(ChatCompletionChunk::Stop {
                finish_reason: "stop".to_string(),
                output_tokens: 0,
                prompt_tokens: 0,
            })]),
            sleep_ms: 0,
        }),
        metrics_exporter: Arc::new(mocks::MockMetricsExporter),
        server_control,
        queue_submitter: Arc::new(mocks::MockQueueSubmitter),
        config_writer: handle.clone(),
        request_metrics_store: Arc::new(InMemoryStore::default()),
        store_maintenance: Arc::new(InMemoryStore::default()),
    };

    let handles = start_servers(deps).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    (handles, handle)
}

#[tokio::test]
async fn test_http_readiness_logic() {
    let state = Arc::new(MockServerControlState {
        cluster_state: std::sync::atomic::AtomicU8::new(0),
        initialized: std::sync::atomic::AtomicBool::new(false),
    });
    let sc = Arc::new(StatefulMockServerControl { state: state.clone() });
    let (handles, _config) = setup_test_server(sc).await;
    let client = Client::new();
    let base_url = format!("http://{}", handles.http_addr);

    // 1. Not initialized -> 503 starting
    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "starting");

    // 2. Initialized, cluster state Unknown (0) -> 200 ok (ready to queue)
    state.initialized.store(true, std::sync::atomic::Ordering::Release);
    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "ok");

    // 3. Initialized, cluster state Healthy (1) -> 200 ok
    state.cluster_state.store(1, std::sync::atomic::Ordering::Release);
    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 4. Initialized, cluster state Failed (2) -> 503 cluster_failed (circuit breaker)
    state.cluster_state.store(2, std::sync::atomic::Ordering::Release);
    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "cluster_failed");

    // 5. /ready/inference - Healthy (1) -> 200
    state.cluster_state.store(1, std::sync::atomic::Ordering::Release);
    let res = client.get(format!("{}/ready/inference", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. /ready/inference - Unknown (0) -> 503 not_ready
    state.cluster_state.store(0, std::sync::atomic::Ordering::Release);
    let res = client.get(format!("{}/ready/inference", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "not_ready");

    // 7. /healthz aliased to health_check -> 200 ok
    let res = client.get(format!("{}/healthz", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "ok");

    handles.shutdown();
    handles.wait().await.unwrap();
}
