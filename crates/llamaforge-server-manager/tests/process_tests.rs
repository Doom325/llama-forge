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
[servers.srv_test]
model_path = "{}"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
"#,
        env!("CARGO_BIN_EXE_mock-llama"),
        mock_port
    ));

    std::fs::write(&main_path, config_str).unwrap();
    let paths = ConfigPaths::new(main_path, tmp.path().join("priority_rules.toml"), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

#[tokio::test]
async fn test_process_lifecycle_external_kill() {
    let _ = tracing_subscriber::fmt::try_init();
    let tmp = TempDir::new().unwrap();
    let config = get_test_config(&tmp, 18080);
    let db_path = tmp.path().join("metrics.db");

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

    // Start the manager
    let mgr_started = mgr.clone().start().await.unwrap();

    let srv_id = ServerId("srv_test".into());

    // Wait for the task to spawn and process to be Running/Starting
    // and wait for PID to be populated.
    let mut pid = None;
    for _ in 0..50 {
        if let Some(state) = mgr_started.server_state(&srv_id) {
            if let Some(p) = state.pid {
                pid = Some(p);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let pid = pid.expect("Failed to get PID");

    // Verify managed_processes row exists
    let procs = pipeline.store.list_managed_processes().await.unwrap();
    assert!(procs.iter().any(|p| p.pid.0 == pid));

    // Send external SIGKILL
    #[cfg(unix)]
    {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGKILL,
        )
        .unwrap();
    }

    // It should transition to Restarting
    let mut restarted = false;
    for _ in 0..50 {
        if let Some(state) = mgr_started.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Restarting {
                restarted = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(restarted, "Failed to transition to Restarting");

    // The ledger should be cleared on exit
    let _gpu = llamaforge_config::GpuId(0);
    // Note: The ledger might be immediately re-reserved by the next start attempt,
    // but we can check if the row was removed before the next start.
    // It's hard to definitively check a transient state like ledger cleared between restarts without injecting delays.
    // Instead we can just check if row was removed.
    let procs_after = pipeline.store.list_managed_processes().await.unwrap();
    assert!(!procs_after.iter().any(|p| p.pid.0 == pid), "managed_processes row was not removed");
}

#[tokio::test]
async fn test_process_lifecycle_shutdown() {
    let tmp = TempDir::new().unwrap();
    let config = get_test_config(&tmp, 18081);
    let db_path = tmp.path().join("metrics2.db");

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

    let srv_id = ServerId("srv_test".into());

    let mut pid = None;
    for _ in 0..50 {
        if let Some(state) = mgr_started.server_state(&srv_id) {
            if let Some(p) = state.pid {
                pid = Some(p);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let pid = pid.expect("Failed to get PID");

    // Cancel token
    token.cancel();

    // The process should be killed, we can check if it's dead by sending signal 0
    let mut is_dead = false;
    for _ in 0..50 {
        #[cfg(unix)]
        {
            if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_err() {
                is_dead = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_dead, "Process was not killed on shutdown");
}
