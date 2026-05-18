#![cfg(feature = "test-utils")]

use llamaforge_config::{ConfigHandle, ConfigPaths, ServerId, TomlFileConfigSource};
use llamaforge_metrics::create_metrics_pipeline;
use llamaforge_server_manager::{
    ClientRefreshEvent, ServerLifecycle, ServerManager, TelemetryError,
};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct MockTelemetry;
impl llamaforge_server_manager::GpuTelemetry for MockTelemetry {
    fn free_vram_bytes(&self, _id: &llamaforge_config::GpuId) -> Result<u64, TelemetryError> {
        Ok(10 * 1024 * 1024 * 1024)
    }
    fn utilization_percent(&self, _id: &llamaforge_config::GpuId) -> Result<u8, TelemetryError> {
        Ok(0)
    }
    fn list_devices(&self) -> Result<Vec<llamaforge_server_manager::GpuDevice>, TelemetryError> {
        Ok(vec![])
    }
}

fn get_test_config(
    tmp: &TempDir,
    mock_port: u16,
    restart_on_deg: bool,
    ctx_size: u32,
    alias: &str,
) -> Arc<ConfigHandle> {
    let main_path = tmp.path().join("llamaforge.toml");
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    let mut config_str = String::new();
    config_str.push_str("[metrics]\n");
    config_str.push_str(&format!(
        r#"
[servers.srv_test]
binary_path = "{}"
model_path = "{}"
alias = "{}"
port = {}
ctx_size = {}
parallel_slots = 1
estimated_vram_mb = 100
restart_on_degradation = {}
"#,
        mock_llama_bin(),
        mock_llama_bin(),
        alias,
        mock_port,
        ctx_size,
        restart_on_deg
    ));

    std::fs::write(&main_path, config_str).unwrap();
    let paths = ConfigPaths::new(main_path, tmp.path().join("priority_rules.toml"), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

async fn setup_manager(
    tmp: &TempDir,
    port: u16,
    restart_on_deg: bool,
    ctx_size: u32,
    alias: &str,
) -> (Arc<ServerManager>, tokio::sync::broadcast::Receiver<ClientRefreshEvent>, CancellationToken) {
    let config = get_test_config(tmp, port, restart_on_deg, ctx_size, alias);
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

    let refresh_rx = manager.subscribe_client_refresh();
    let mgr = Arc::new(manager);
    let mgr_started = mgr.clone().start().await.unwrap();

    (mgr_started, refresh_rx, token)
}

static NEXT_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(18100);

#[tokio::test]
#[ignore]
async fn test_health_transition_and_refresh() {
    let _ = tracing_subscriber::fmt::try_init();
    let tmp = TempDir::new().unwrap();
    let port = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (mgr, mut refresh_rx, token) = setup_manager(&tmp, port, true, 4096, "mock").await;
    let srv_id = ServerId("srv_test".into());

    // Wait for it to become Running
    let mut is_running = false;
    for _ in 0..100 {
        if let Some(state) = mgr.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Running {
                is_running = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_running, "Failed to reach Running state");

    // Check refresh event
    let event = refresh_rx.recv().await.expect("Failed to receive ClientRefreshEvent");
    assert_eq!(event.server_id, srv_id);
    assert_eq!(event.new_port, port);

    token.cancel();
}

#[tokio::test]
#[ignore]
async fn test_consecutive_health_failures() {
    let _ = tracing_subscriber::fmt::try_init();

    let tmp = TempDir::new().unwrap();
    let port = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (mgr, _, token) = setup_manager(&tmp, port, true, 4096, "mock,health_fail_after=1").await;
    let srv_id = ServerId("srv_test".into());

    // Should reach Restarting
    let mut is_restarting = false;
    // Need to wait for at least 4 ticks (0s, 2s, 4s, 6s) + jitter + backoff
    for _ in 0..200 {
        if let Some(state) = mgr.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Restarting {
                is_restarting = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_restarting, "Failed to reach Restarting state on health failure");

    token.cancel();
}

#[tokio::test]
async fn test_startup_grace_window() {
    let _ = tracing_subscriber::fmt::try_init();

    let tmp = TempDir::new().unwrap();
    let port = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    // Return 503 for 8 seconds. This would normally cause 3 consecutive health
    // failures (at 2s, 4s, 6s) and trigger a restart if not for the grace window.
    let (mgr, _, token) = setup_manager(&tmp, port, true, 4096, "mock,startup_503_ms=8000").await;
    let srv_id = ServerId("srv_test".into());

    let mut is_running = false;
    let mut restarted = false;
    // Wait up to 15 seconds to allow the 8s 503 window to expire and state to transition
    for _ in 0..150 {
        if let Some(state) = mgr.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Restarting {
                restarted = true;
            }
            if state.lifecycle == ServerLifecycle::Running {
                is_running = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!restarted, "Server incorrectly restarted during the 503 grace window");
    assert!(is_running, "Failed to reach Running state after the grace window");

    token.cancel();
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
