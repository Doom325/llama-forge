#![cfg(feature = "test-utils")]

use llamaforge_config::{ConfigHandle, ConfigPaths, ServerId, TomlFileConfigSource};
use llamaforge_metrics::{create_metrics_pipeline, ProcessStore};
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
        Ok(0)
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
[servers.srv_e2e]
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
"#,
        env!("CARGO_BIN_EXE_mock-llama"),
        mock_port,
    ));

    std::fs::write(&main_path, config_str).unwrap();
    let paths = ConfigPaths::new(main_path, tmp.path().join("priority_rules.toml"), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

static NEXT_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(18600);

#[tokio::test]
#[ignore]
async fn test_e2e_lifecycle() {
    let _ = tracing_subscriber::fmt::try_init();
    let tmp = TempDir::new().unwrap();
    let port = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let config = get_test_config(&tmp, port);
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

    let srv_id = ServerId("srv_e2e".into());

    // Wait for Running
    let mut is_running = false;
    let mut pid = None;
    for _ in 0..50 {
        if let Some(state) = mgr_started.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Running {
                is_running = true;
                pid = state.pid;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_running);
    let pid = pid.unwrap();

    // Dispatch fake increments
    mgr_started.increment_pending(&srv_id);
    assert_eq!(mgr_started.pending_dispatches(&srv_id), 1);

    // Check row exists
    let rows = pipeline.store.list_managed_processes().await.unwrap();
    assert_eq!(rows.len(), 1);

    // Stop server
    mgr_started.stop_server(&srv_id).await;

    // Verify Draining
    let mut is_draining = false;
    for _ in 0..20 {
        if let Some(state) = mgr_started.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Draining {
                is_draining = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_draining);

    // Decrement pending so it can finish draining
    mgr_started.decrement_pending(&srv_id);

    // Wait for it to be stopped
    let mut is_stopped = false;
    for _ in 0..150 {
        if let Some(state) = mgr_started.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Stopped {
                is_stopped = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_stopped);

    // Verify reaped and row deleted
    let mut is_dead = false;
    #[cfg(unix)]
    {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;
        if kill(Pid::from_raw(pid as i32), None).is_err() {
            is_dead = true;
        }
    }
    assert!(is_dead, "Process was not reaped");

    let rows = pipeline.store.list_managed_processes().await.unwrap();
    assert_eq!(rows.len(), 0, "SQLite row was not deleted");

    token.cancel();
}
