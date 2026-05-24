#![cfg(feature = "test-utils")]

use llamaforge_core::config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
use llamaforge_core::types::ServerId;
use llamaforge_metrics::create_metrics_pipeline;
use llamaforge_server::{ServerLifecycle, ServerManager, TelemetryError};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct MockTelemetry;
impl llamaforge_server::GpuTelemetry for MockTelemetry {
    fn free_vram_bytes(&self, _id: &llamaforge_core::config::GpuId) -> Result<u64, TelemetryError> {
        Ok(10 * 1024 * 1024 * 1024)
    }
    fn utilization_percent(
        &self,
        _id: &llamaforge_core::config::GpuId,
    ) -> Result<u8, TelemetryError> {
        Ok(100)
    }
    fn list_devices(&self) -> Result<Vec<llamaforge_server::GpuDevice>, TelemetryError> {
        Ok(vec![])
    }
}

fn get_test_config(
    tmp: &TempDir,
    mock_port: u16,
    restart_on_degradation: bool,
) -> Arc<ConfigHandle> {
    let main_path = tmp.path().join(format!("llamaforge_{}.toml", mock_port));
    let state_dir = tmp.path().join(format!("state_{}", mock_port));
    std::fs::create_dir_all(&state_dir).unwrap();

    let mut config_str = String::new();
    config_str.push_str("[metrics]\n");
    config_str.push_str(&format!(
        r#"
[servers.srv_deg]
binary_path = "{}"
model_path = "{}"
alias = "mock,tps=0.1"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
restart_on_degradation = {}
"#,
        mock_llama_bin(),
        mock_llama_bin(),
        mock_port,
        restart_on_degradation
    ));

    std::fs::write(&main_path, config_str).unwrap();
    let paths = ConfigPaths::new(main_path, tmp.path().join("priority_rules.toml"), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

static NEXT_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(19000);

#[tokio::test]
#[ignore]
async fn test_degradation_e2e_restart() {
    let _ = tracing_subscriber::fmt::try_init();
    let tmp = TempDir::new().unwrap();
    let port = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let config = get_test_config(&tmp, port, true);
    let db_path = tmp.path().join(format!("metrics_{}.db", port));

    let pipeline = create_metrics_pipeline(
        &llamaforge_metrics::MetricsConfig { db_path: db_path.clone(), ..Default::default() },
        config.clone(),
    )
    .unwrap();

    let token = CancellationToken::new();
    let manager = ServerManager::new(
        Arc::new(MockTelemetry),
        config.clone(),
        pipeline.ingest,
        pipeline.store.clone(),
        token.clone(),
    );

    let mgr = Arc::new(manager);
    let mgr_started = mgr.clone().start().await.unwrap();
    let srv_id = ServerId::from_config_id("srv_deg");

    // Wait for Running
    let is_running = mgr_started
        .wait_for_lifecycle(&srv_id, ServerLifecycle::Running, Duration::from_secs(10))
        .await;
    assert!(is_running);

    // Warm up TPS window
    mgr_started.force_warm_tps_window_for_test(&srv_id, 30, 10.0);

    // Wait for restart (3 ticks * 10s = ~30s)
    let restarted = mgr_started
        .wait_for_lifecycle(&srv_id, ServerLifecycle::Restarting, Duration::from_secs(40))
        .await;
    assert!(restarted, "Server should have restarted due to degradation");

    mgr_started.shutdown_all().await;
}

#[tokio::test]
#[ignore]
async fn test_degradation_e2e_no_restart() {
    let _ = tracing_subscriber::fmt::try_init();
    let tmp = TempDir::new().unwrap();
    let port = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let config = get_test_config(&tmp, port, false);
    let db_path = tmp.path().join(format!("metrics_{}.db", port));

    let pipeline = create_metrics_pipeline(
        &llamaforge_metrics::MetricsConfig { db_path: db_path.clone(), ..Default::default() },
        config.clone(),
    )
    .unwrap();

    let token = CancellationToken::new();
    let manager = ServerManager::new(
        Arc::new(MockTelemetry),
        config.clone(),
        pipeline.ingest,
        pipeline.store.clone(),
        token.clone(),
    );

    let mgr = Arc::new(manager);
    let mgr_started = mgr.clone().start().await.unwrap();
    let srv_id = ServerId::from_config_id("srv_deg");

    // Wait for Running
    let is_running = mgr_started
        .wait_for_lifecycle(&srv_id, ServerLifecycle::Running, Duration::from_secs(10))
        .await;
    assert!(is_running);

    // Warm up TPS window
    mgr_started.force_warm_tps_window_for_test(&srv_id, 30, 10.0);

    // Wait for 35s to ensure no restart happens (advance virtual time to run instantly)
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(35)).await;
    tokio::time::resume();

    let state = mgr_started.server_state(&srv_id).unwrap();
    assert_eq!(state.lifecycle, ServerLifecycle::Running, "Server should not restart");

    mgr_started.shutdown_all().await;
}

/// See `config_tests::mock_llama_bin` — same rationale (env! is only
/// set for tests in the same package as the bin).
fn mock_llama_bin() -> String {
    let target =
        std::env::var("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("target")
        });
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    target.join(profile).join("mock-llama").to_string_lossy().into_owned()
}
