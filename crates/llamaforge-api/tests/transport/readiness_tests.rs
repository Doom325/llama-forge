#[path = "mocks.rs"]
mod mocks;

use llamaforge_api::transport::{start_servers, TransportDeps};
use llamaforge_core::config::{
    source::memory::InMemoryConfigSource, AppConfig, ConfigHandle, GlobalConfig,
};
use llamaforge_core::types::traits::*;
use llamaforge_core::types::{ChatCompletionChunk, ClusterState, ClusterStatus, ServerId};
use llamaforge_metrics::InMemoryStore;
use reqwest::{Client, StatusCode};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;

/// `ServerControl` mock backed by atomics so the test can drive
/// `/ready` through the same `Ordering::Acquire` read path that the
/// production `ServerControlAdapter` uses (`design.md:4353`).
struct AtomicMockServerControl {
    cluster_state: Arc<AtomicU8>,
    failed_servers: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl ServerControl for AtomicMockServerControl {
    async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError> {
        Ok(vec![])
    }
    async fn start_server(&self, _id: &ServerId) -> Result<(), ServerError> {
        Ok(())
    }
    async fn stop_server(&self, _id: &ServerId) -> Result<(), ServerError> {
        Ok(())
    }
    fn cluster_status(&self) -> ClusterStatus {
        ClusterStatus {
            state: ClusterState::from_repr(self.cluster_state.load(Ordering::Acquire)),
            failed_servers: self.failed_servers.load(Ordering::Acquire),
        }
    }
}

struct AtomicMockGatewayLifecycle {
    initialized: Arc<AtomicBool>,
}

impl GatewayLifecycle for AtomicMockGatewayLifecycle {
    fn is_gateway_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }
}

async fn setup_test_server(
    server_control: Arc<dyn ServerControl>,
    gateway_lifecycle: Arc<dyn GatewayLifecycle>,
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
        gateway_lifecycle,
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
    let cluster_state = Arc::new(AtomicU8::new(ClusterState::SomeStarting as u8));
    let failed_servers = Arc::new(AtomicU32::new(0));
    let initialized = Arc::new(AtomicBool::new(false));

    let sc = Arc::new(AtomicMockServerControl {
        cluster_state: cluster_state.clone(),
        failed_servers: failed_servers.clone(),
    });
    let gl = Arc::new(AtomicMockGatewayLifecycle { initialized: initialized.clone() });
    let (handles, _config) = setup_test_server(sc, gl).await;
    let client = Client::new();
    let base_url = format!("http://{}", handles.http_addr);

    // 1. Not initialized -> 503 starting
    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "starting");

    // 2. Initialized, cluster state SomeStarting -> 200 ok (ready to queue)
    initialized.store(true, Ordering::Release);
    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "ok");

    // 3. Initialized, cluster state SomeRunning -> 200 ok
    cluster_state.store(ClusterState::SomeRunning as u8, Ordering::Release);
    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 4. Initialized, cluster state AllFailed -> 503 cluster_failed (F5: includes failed_servers)
    failed_servers.store(3, Ordering::Release);
    cluster_state.store(ClusterState::AllFailed as u8, Ordering::Release);
    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "cluster_failed");
    assert_eq!(json["failed_servers"], 3);

    // 5. /ready/inference - SomeRunning -> 200
    failed_servers.store(0, Ordering::Release);
    cluster_state.store(ClusterState::SomeRunning as u8, Ordering::Release);
    let res = client.get(format!("{}/ready/inference", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. /ready/inference - SomeStarting (incl. empty cluster) -> 503 (F4)
    cluster_state.store(ClusterState::SomeStarting as u8, Ordering::Release);
    let res = client.get(format!("{}/ready/inference", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "not_ready");

    // 7. /health (aliased to health_check) -> 200 ok
    let res = client.get(format!("{}/health", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "ok");

    handles.shutdown();
    handles.wait().await.unwrap();
}

// ----- F10 regression tests: drive ServerControlAdapter on top of a
// real ServerManager so we exercise recalculate_cluster_state instead
// of bypassing it. -----

use llamaforge_api::adapters::server_control::{AtomicGatewayLifecycle, ServerControlAdapter};
use llamaforge_core::config::{ConfigPaths, TomlFileConfigSource};
use llamaforge_core::types::ServerLifecycle;
use llamaforge_server::ServerManager;

fn dummy_telemetry() -> Arc<dyn llamaforge_server::GpuTelemetry> {
    struct DummyTelemetry;
    impl llamaforge_server::GpuTelemetry for DummyTelemetry {
        fn utilization_percent(
            &self,
            _: &llamaforge_core::config::GpuId,
        ) -> Result<u8, llamaforge_server::TelemetryError> {
            Ok(0)
        }
        fn list_devices(
            &self,
        ) -> Result<Vec<llamaforge_server::GpuDevice>, llamaforge_server::TelemetryError> {
            Ok(vec![])
        }
        fn free_vram_bytes(
            &self,
            _: &llamaforge_core::config::GpuId,
        ) -> Result<u64, llamaforge_server::TelemetryError> {
            Ok(0)
        }
    }
    Arc::new(DummyTelemetry)
}

fn fresh_manager() -> Arc<ServerManager> {
    let tmp = tempfile::TempDir::new().unwrap();
    let toml_path = tmp.path().join("llamaforge.toml");
    std::fs::write(&toml_path, "[metrics]\n").unwrap();
    let paths = ConfigPaths::new(
        toml_path,
        tmp.path().join("priority_rules.toml"),
        tmp.path().to_path_buf(),
    );
    let source = Arc::new(TomlFileConfigSource::new(paths));
    let config = Arc::new(ConfigHandle::load(source).unwrap());

    let db_path = tmp.path().join("metrics.db");
    let pipeline = llamaforge_metrics::create_metrics_pipeline(
        &llamaforge_metrics::MetricsConfig { db_path, ..Default::default() },
        config.clone(),
    )
    .unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();

    // Keep tmp dir alive for the manager's lifetime. We leak the
    // TempDir since the test is short-lived and the OS reclaims at
    // process exit.
    std::mem::forget(tmp);

    Arc::new(ServerManager::new(
        dummy_telemetry(),
        config,
        pipeline.ingest,
        pipeline.store.clone(),
        shutdown,
    ))
}

/// F10 regression: the `/ready` 503 body's `failed_servers` field is
/// populated by the same `ServerManager` atomic that
/// `recalculate_cluster_state` updates. Drives the manager through a
/// 2-server all-Failed transition end-to-end.
#[tokio::test]
async fn test_ready_failed_servers_count_via_manager() {
    let manager = fresh_manager();
    manager
        .insert_synthetic_server_for_test(ServerId::from_config_id("s1"), ServerLifecycle::Failed);
    manager
        .insert_synthetic_server_for_test(ServerId::from_config_id("s2"), ServerLifecycle::Failed);

    let sc: Arc<dyn ServerControl> = Arc::new(ServerControlAdapter::new(manager.clone()));
    let init = Arc::new(AtomicBool::new(true));
    let gl: Arc<dyn GatewayLifecycle> = Arc::new(AtomicGatewayLifecycle::new(init));
    let (handles, _config) = setup_test_server(sc, gl).await;
    let client = Client::new();
    let base_url = format!("http://{}", handles.http_addr);

    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(json["status"], "cluster_failed");
    assert_eq!(json["failed_servers"], 2);

    handles.shutdown();
    handles.wait().await.unwrap();
}

/// F10 regression: all-Stopped must NOT trip the circuit breaker.
/// Operator-stopped fleets stay "ready" so kubelet doesn't evict.
#[tokio::test]
async fn test_ready_all_stopped_does_not_trip_breaker() {
    let manager = fresh_manager();
    manager
        .insert_synthetic_server_for_test(ServerId::from_config_id("s1"), ServerLifecycle::Stopped);
    manager
        .insert_synthetic_server_for_test(ServerId::from_config_id("s2"), ServerLifecycle::Stopped);

    let sc: Arc<dyn ServerControl> = Arc::new(ServerControlAdapter::new(manager.clone()));
    let init = Arc::new(AtomicBool::new(true));
    let gl: Arc<dyn GatewayLifecycle> = Arc::new(AtomicGatewayLifecycle::new(init));
    let (handles, _config) = setup_test_server(sc, gl).await;
    let client = Client::new();
    let base_url = format!("http://{}", handles.http_addr);

    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK, "all-Stopped must not 503");

    handles.shutdown();
    handles.wait().await.unwrap();
}

/// F10 regression: an empty cluster reports `SomeStarting`, so
/// `/ready/inference` returns 503 (no Running backends) while
/// `/ready` still returns 200 (the gateway can queue).
#[tokio::test]
async fn test_empty_cluster_inference_returns_503() {
    // No servers inserted at all: the manager's initial atomics
    // already encode `SomeStarting / failed_servers = 0` per the
    // constructor, which is the empty-cluster steady state.
    let manager = fresh_manager();

    let sc: Arc<dyn ServerControl> = Arc::new(ServerControlAdapter::new(manager.clone()));
    let init = Arc::new(AtomicBool::new(true));
    let gl: Arc<dyn GatewayLifecycle> = Arc::new(AtomicGatewayLifecycle::new(init));
    let (handles, _config) = setup_test_server(sc, gl).await;
    let client = Client::new();
    let base_url = format!("http://{}", handles.http_addr);

    let res = client.get(format!("{}/ready", base_url)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK, "empty cluster: /ready should be 200");
    let res = client.get(format!("{}/ready/inference", base_url)).send().await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "empty cluster: /ready/inference should be 503"
    );

    handles.shutdown();
    handles.wait().await.unwrap();
}
