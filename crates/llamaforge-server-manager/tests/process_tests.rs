#![cfg(feature = "test-utils")]

use chrono::Utc;
use llamaforge_core::config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
use llamaforge_metrics::{create_metrics_pipeline, ManagedProcess, ProcessId, ProcessStore};
use llamaforge_server_manager::{ServerLifecycle, ServerManager, TelemetryError};
use llamaforge_core::types::ServerId;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct MockTelemetry;
impl llamaforge_server_manager::GpuTelemetry for MockTelemetry {
    fn free_vram_bytes(&self, _id: &llamaforge_core::config::GpuId) -> Result<u64, TelemetryError> {
        Ok(10 * 1024 * 1024 * 1024)
    }
    fn utilization_percent(&self, _id: &llamaforge_core::config::GpuId) -> Result<u8, TelemetryError> {
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
binary_path = "{}"
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
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

static NEXT_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(18500);

#[tokio::test]
#[ignore]
async fn test_process_lifecycle_external_kill() {
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

    let srv_id = ServerId::from_config_id("srv_test");

    let mut pid = None;
    for _ in 0..50 {
        if let Some(state) = mgr_started.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Running {
                pid = state.pid;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let pid = pid.unwrap();

    // Kill it
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
    }

    // Wait for restart
    let mut is_restarted = false;
    for _ in 0..50 {
        if let Some(state) = mgr_started.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Restarting
                || (state.lifecycle == ServerLifecycle::Running && state.pid != Some(pid))
            {
                is_restarted = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_restarted);

    token.cancel();
}

#[tokio::test]
#[ignore]
async fn test_process_lifecycle_shutdown() {
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

    let srv_id = ServerId::from_config_id("srv_test");

    let mut pid = None;
    for _ in 0..50 {
        if let Some(state) = mgr_started.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Running {
                pid = state.pid;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let pid = pid.unwrap();

    mgr_started.shutdown_all().await;

    // The process should be killed, we can check if it's dead by sending signal 0
    let mut is_dead = false;
    for _ in 0..50 {
        #[cfg(unix)]
        {
            use nix::sys::signal::kill;
            use nix::unistd::Pid;
            if kill(Pid::from_raw(pid as i32), None).is_err() {
                is_dead = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_dead);

    // Verify rows deleted
    let rows = pipeline.store.list_managed_processes().await.unwrap();
    assert!(rows.is_empty(), "Rows should be deleted on shutdown_all");
}

#[tokio::test]
#[ignore]
async fn test_orphan_recovery() {
    let _ = tracing_subscriber::fmt::try_init();
    let tmp = TempDir::new().unwrap();

    // (a) Successful adoption
    let port_a = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut cmd_a = tokio::process::Command::new(mock_llama_bin());
    cmd_a.arg("--port").arg(port_a.to_string());
    cmd_a.kill_on_drop(true);
    let mut child_a = cmd_a.spawn().unwrap();
    let pid_a = child_a.id().unwrap();
    tokio::spawn(async move {
        let _ = child_a.wait().await;
    });

    // (b) Health fails
    let port_b = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut cmd_b = tokio::process::Command::new(mock_llama_bin());
    cmd_b.arg("--port").arg(port_b.to_string()).arg("--alias").arg("mock,health_fail_after=0");
    cmd_b.kill_on_drop(true);
    let mut child_b = cmd_b.spawn().unwrap();
    let pid_b = child_b.id().unwrap();
    tokio::spawn(async move {
        let _ = child_b.wait().await;
    });

    // (c) Dead PID
    let port_c = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let pid_c = 999999; // Hopefully dead

    // (d) Model mismatch
    let port_d = NEXT_PORT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut cmd_d = tokio::process::Command::new(mock_llama_bin());
    cmd_d.arg("--port").arg(port_d.to_string()).arg("--alias").arg("wrong_model");
    cmd_d.kill_on_drop(true);
    let mut child_d = cmd_d.spawn().unwrap();
    let pid_d = child_d.id().unwrap();
    tokio::spawn(async move {
        let _ = child_d.wait().await;
    });

    // Wait for them to start
    tokio::time::sleep(Duration::from_millis(500)).await;

    let main_path = tmp.path().join("llamaforge.toml");
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    let mut config_str = String::new();
    config_str.push_str("[metrics]\n");
    config_str.push_str(&format!(
        r#"
[servers.srv_a]
binary_path = "{}"
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100

[servers.srv_b]
binary_path = "{}"
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100

[servers.srv_c]
binary_path = "{}"
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100

[servers.srv_d]
binary_path = "{}"
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
"#,
        mock_llama_bin(),
        mock_llama_bin(),
        port_a,
        mock_llama_bin(),
        mock_llama_bin(),
        port_b,
        mock_llama_bin(),
        mock_llama_bin(),
        port_c,
        mock_llama_bin(),
        mock_llama_bin(),
        port_d,
    ));

    std::fs::write(&main_path, config_str).unwrap();
    let paths = ConfigPaths::new(main_path, tmp.path().join("priority_rules.toml"), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    let config = Arc::new(ConfigHandle::load(source).unwrap());

    let db_path = tmp.path().join("metrics_orphan.db");
    let pipeline = create_metrics_pipeline(
        &llamaforge_metrics::MetricsConfig { db_path: db_path.clone(), ..Default::default() },
        config.clone(),
    )
    .unwrap();

    // Pre-populate
    pipeline
        .store
        .upsert_managed_process(&ManagedProcess {
            pid: ProcessId(pid_a),
            server_id: ServerId::from_config_id("srv_a"),
            started_at: Utc::now(),
            cmdline: "".into(),
            port: port_a,
            model_alias: "mock".into(),
        })
        .await
        .unwrap();
    pipeline
        .store
        .upsert_managed_process(&ManagedProcess {
            pid: ProcessId(pid_b),
            server_id: ServerId::from_config_id("srv_b"),
            started_at: Utc::now(),
            cmdline: "".into(),
            port: port_b,
            model_alias: "mock".into(),
        })
        .await
        .unwrap();
    pipeline
        .store
        .upsert_managed_process(&ManagedProcess {
            pid: ProcessId(pid_c),
            server_id: ServerId::from_config_id("srv_c"),
            started_at: Utc::now(),
            cmdline: "".into(),
            port: port_c,
            model_alias: "mock".into(),
        })
        .await
        .unwrap();
    pipeline
        .store
        .upsert_managed_process(&ManagedProcess {
            pid: ProcessId(pid_d),
            server_id: ServerId::from_config_id("srv_d"),
            started_at: Utc::now(),
            cmdline: "".into(),
            port: port_d,
            model_alias: "wrong_model".into(),
        })
        .await
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

    // Give it a moment to process orphans
    tokio::time::sleep(Duration::from_millis(500)).await;

    // A should be running with pid_a
    let state_a = mgr_started.server_state(&ServerId::from_config_id("srv_a")).unwrap();
    assert_eq!(state_a.lifecycle, ServerLifecycle::Running);
    assert_eq!(state_a.pid, Some(pid_a));

    // B should have spawned a NEW child because health failed, so orphan wasn't adopted
    let state_b = mgr_started.server_state(&ServerId::from_config_id("srv_b")).unwrap();
    // B was adopted (since first health check passed), but then it failed health checks and restarted!
    assert!(state_b.lifecycle == ServerLifecycle::Restarting || state_b.pid != Some(pid_b));

    // C should have spawned a NEW child because it was dead
    let state_c = mgr_started.server_state(&ServerId::from_config_id("srv_c")).unwrap();
    assert_ne!(state_c.pid, Some(pid_c));

    // D should have spawned a NEW child because model alias mismatched
    let state_d = mgr_started.server_state(&ServerId::from_config_id("srv_d")).unwrap();
    assert_ne!(state_d.pid, Some(pid_d));

    // Cleanup
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
