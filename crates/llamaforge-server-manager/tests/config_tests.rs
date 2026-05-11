#![cfg(feature = "test-utils")]

use llamaforge_config::ConfigWriter;
use llamaforge_config::{ConfigHandle, ConfigPaths, ServerId, TomlFileConfigSource};
use llamaforge_metrics::create_metrics_pipeline;
use llamaforge_server_manager::{ServerLifecycle, ServerManager, TelemetryError};
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
        Ok(100)
    }
    fn list_devices(&self) -> Result<Vec<llamaforge_server_manager::GpuDevice>, TelemetryError> {
        Ok(vec![])
    }
}

fn get_test_config(tmp: &TempDir, mock_port: u16) -> Arc<ConfigHandle> {
    let main_path = tmp.path().join("llamaforge.toml");
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    let mut config_str = String::new();
    config_str.push_str("[metrics]\n");
    config_str.push_str(&format!(
        r#"
[servers.srv_test]
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
restart_on_degradation = true
"#,
        env!("CARGO_BIN_EXE_mock-llama"),
        mock_port,
    ));

    std::fs::write(&main_path, config_str).unwrap();
    let paths = ConfigPaths::new(main_path, tmp.path().join("priority_rules.toml"), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

async fn setup_manager(tmp: &TempDir, port: u16) -> (Arc<ServerManager>, CancellationToken) {
    let config = get_test_config(tmp, port);
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

    (mgr_started, token)
}

static NEXT_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(18400);

#[tokio::test]
#[ignore]
async fn test_apply_config_diff() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let tmp = TempDir::new().unwrap();
    let port = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (mgr, token) = setup_manager(&tmp, port).await;

    let srv_id = ServerId("srv_test".into());

    // Wait for Running
    let mut is_running = false;
    for _ in 0..50 {
        if let Some(state) = mgr.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Running {
                is_running = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_running);

    // Test (a): Add a server
    let main_path = tmp.path().join("llamaforge.toml");
    let mut config_str = std::fs::read_to_string(&main_path).unwrap();
    config_str.push_str(&format!(
        r#"
[servers.srv_added]
model_path = "{}"
alias = "added"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
"#,
        env!("CARGO_BIN_EXE_mock-llama"),
        port + 1,
    ));
    std::fs::write(&main_path, &config_str).unwrap();

    let diff = mgr.config().reload().await.unwrap();
    mgr.apply_config_diff(&diff).await;

    let srv_added = ServerId("srv_added".into());
    let mut is_added_running = false;
    for _ in 0..100 {
        if let Some(state) = mgr.server_state(&srv_added) {
            if state.lifecycle == ServerLifecycle::Running {
                is_added_running = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_added_running, "Added server did not reach Running state");

    // Test (c): Apply diff with requires_restart=false (hot reloadable)
    // We will change alias for srv_added
    config_str = config_str.replace("alias = \"added\"", "alias = \"hot_reloaded\"");
    std::fs::write(&main_path, &config_str).unwrap();

    let diff = mgr.config().reload().await.unwrap();
    mgr.apply_config_diff(&diff).await;

    // Verify it changed in-place without restart
    let state = mgr.server_state(&srv_added).unwrap();
    assert_eq!(state.model_alias, "hot_reloaded");
    assert_eq!(state.lifecycle, ServerLifecycle::Running);

    // Test (d): FM-5: Change devices on running server
    // Change gpu for srv_test
    config_str = config_str.replace(
        "[servers.srv_test]\nmodel_path",
        "[servers.srv_test]\ndevices = [{gpu_id = 1, layer_range = [0, 10]}]\nmodel_path",
    );
    std::fs::write(&main_path, &config_str).unwrap();

    let diff = mgr.config().reload().await.unwrap();
    mgr.apply_config_diff(&diff).await;

    // Wait for Srv_test to restart and become Running again
    let mut is_srv_test_running = false;
    for _ in 0..100 {
        if let Some(state) = mgr.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Running {
                is_srv_test_running = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_srv_test_running, "Server did not restart and reach Running on new GPU");

    // Test (b): Apply diff removing a server
    // Remove srv_added
    // Re-write without srv_added section
    let mut config_str2 = String::new();
    config_str2.push_str("[metrics]\n");
    config_str2.push_str(&format!(
        r#"
[servers.srv_test]
devices = [{{gpu_id = 1, layer_range = [0, 10]}}]
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
restart_on_degradation = true
"#,
        env!("CARGO_BIN_EXE_mock-llama"),
        port,
    ));
    std::fs::write(&main_path, &config_str2).unwrap();

    let diff = mgr.config().reload().await.unwrap();
    mgr.apply_config_diff(&diff).await;

    let mut is_added_removed = false;
    for _ in 0..100 {
        if mgr.server_state(&srv_added).is_none() {
            is_added_removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_added_removed, "Removed server was not deleted from state");

    // srv_test is untouched
    let state = mgr.server_state(&srv_id).unwrap();
    assert_eq!(state.lifecycle, ServerLifecycle::Running);

    token.cancel();
}
