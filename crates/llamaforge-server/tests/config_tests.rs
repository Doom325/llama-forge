#![cfg(feature = "test-utils")]

use llamaforge_core::config::ConfigWriter;
use llamaforge_core::config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
use llamaforge_core::types::ServerId;
use llamaforge_metrics::create_metrics_pipeline;
use llamaforge_server::{ServerLifecycle, ServerManager, TelemetryError};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// Resolves the workspace `mock-llama` binary path at runtime.
///
/// `env!("CARGO_BIN_EXE_*")` is only populated for integration tests in
/// the same package as the binary; since `mock-llama` is now its own
/// workspace member, we walk from this crate's manifest dir up to the
/// workspace root and into `target/{debug,release}/mock-llama`.
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

fn get_test_config(tmp: &TempDir, mock_port: u16) -> Arc<ConfigHandle> {
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
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
restart_on_degradation = true
"#,
        mock_llama_bin(),
        mock_llama_bin(),
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

    let srv_id = ServerId::from_config_id("srv_test");

    // Wait for Running
    assert!(
        mgr.wait_for_lifecycle(&srv_id, ServerLifecycle::Running, Duration::from_secs(10)).await
    );

    // Test (a): Add a server
    let main_path = tmp.path().join("llamaforge.toml");
    let mut config_str = std::fs::read_to_string(&main_path).unwrap();
    config_str.push_str(&format!(
        r#"
[servers.srv_added]
binary_path = "{}"
model_path = "{}"
alias = "added"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
"#,
        mock_llama_bin(),
        mock_llama_bin(),
        port + 1,
    ));
    std::fs::write(&main_path, &config_str).unwrap();

    let diff = mgr.config().reload().await.unwrap();
    mgr.apply_config_diff(&diff).await;

    let srv_added = ServerId::from_config_id("srv_added");
    assert!(
        mgr.wait_for_lifecycle(&srv_added, ServerLifecycle::Running, Duration::from_secs(10)).await
    );

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
    assert!(
        mgr.wait_for_lifecycle(&srv_id, ServerLifecycle::Running, Duration::from_secs(10)).await
    );

    // Test (b): Apply diff removing a server
    // Remove srv_added
    // Re-write without srv_added section
    let mut config_str2 = String::new();
    config_str2.push_str("[metrics]\n");
    config_str2.push_str(&format!(
        r#"
[servers.srv_test]
devices = [{{gpu_id = 1, layer_range = [0, 10]}}]
binary_path = "{}"
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
restart_on_degradation = true
"#,
        mock_llama_bin(),
        mock_llama_bin(),
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
