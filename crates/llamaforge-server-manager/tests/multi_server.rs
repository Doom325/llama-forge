#![cfg(feature = "test-utils")]

use llamaforge_config::{ConfigHandle, ConfigPaths, ConfigWriter, ServerId, TomlFileConfigSource};
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
        Ok(100)
    }
    fn list_devices(&self) -> Result<Vec<llamaforge_server_manager::GpuDevice>, TelemetryError> {
        Ok(vec![])
    }
}

fn get_multi_config(tmp: &TempDir, port_base: u16) -> Arc<ConfigHandle> {
    let main_path = tmp.path().join("llamaforge.toml");
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    let mut config_str = String::new();
    config_str.push_str("[global]\nrestart_stagger_secs = 2\n");
    config_str.push_str("[metrics]\n");

    // Server 1 on GPU 0
    config_str.push_str(&format!(
        r#"
[servers.srv_0_a]
devices = [{{gpu_id = 0, layer_range = [0, 10]}}]
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
"#,
        mock_llama_bin(),
        port_base
    ));

    // Server 2 on GPU 0
    config_str.push_str(&format!(
        r#"
[servers.srv_0_b]
devices = [{{gpu_id = 0, layer_range = [0, 10]}}]
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
"#,
        mock_llama_bin(),
        port_base + 1
    ));

    // Server 3 on GPU 1
    config_str.push_str(&format!(
        r#"
[servers.srv_1]
devices = [{{gpu_id = 1, layer_range = [0, 10]}}]
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
"#,
        mock_llama_bin(),
        port_base + 2
    ));

    std::fs::write(&main_path, config_str).unwrap();
    let paths = ConfigPaths::new(main_path, tmp.path().join("priority_rules.toml"), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

static NEXT_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(19100);

#[tokio::test]
#[ignore]
async fn test_multi_server_orchestration() {
    let _ = tracing_subscriber::fmt::try_init();
    let tmp = TempDir::new().unwrap();
    let port = NEXT_PORT.fetch_add(3, std::sync::atomic::Ordering::SeqCst);

    let config = get_multi_config(&tmp, port);
    let db_path = tmp.path().join("metrics.db");

    let pipeline = create_metrics_pipeline(
        &llamaforge_metrics::MetricsConfig { db_path, ..Default::default() },
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
    let start_time = tokio::time::Instant::now();
    let mgr_started = mgr.clone().start().await.unwrap();

    let srv_0_a = ServerId("srv_0_a".into());
    let srv_0_b = ServerId("srv_0_b".into());
    let srv_1 = ServerId("srv_1".into());

    let mut a_running_at = None;
    let mut b_running_at = None;
    let mut s1_running_at = None;

    // Wait until all are running
    for _ in 0..100 {
        if a_running_at.is_none() {
            if let Some(state) = mgr_started.server_state(&srv_0_a) {
                if state.lifecycle == ServerLifecycle::Running {
                    a_running_at = Some(tokio::time::Instant::now());
                }
            }
        }
        if b_running_at.is_none() {
            if let Some(state) = mgr_started.server_state(&srv_0_b) {
                if state.lifecycle == ServerLifecycle::Running {
                    b_running_at = Some(tokio::time::Instant::now());
                }
            }
        }
        if s1_running_at.is_none() {
            if let Some(state) = mgr_started.server_state(&srv_1) {
                if state.lifecycle == ServerLifecycle::Running {
                    s1_running_at = Some(tokio::time::Instant::now());
                }
            }
        }

        if a_running_at.is_some() && b_running_at.is_some() && s1_running_at.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(a_running_at.is_some(), "srv_0_a did not start");
    assert!(b_running_at.is_some(), "srv_0_b did not start");
    assert!(s1_running_at.is_some(), "srv_1 did not start");

    let t_a = a_running_at.unwrap();
    let t_b = b_running_at.unwrap();
    let _t_1 = s1_running_at.unwrap();

    // Verify GPU 0 stagger >= 2s
    let diff_gpu0 = if t_a > t_b { t_a.duration_since(t_b) } else { t_b.duration_since(t_a) };
    assert!(diff_gpu0.as_secs_f64() >= 1.5, "GPU0 servers not properly staggered: {:?}", diff_gpu0);

    // Kill srv_0_a
    let state_a = mgr_started.server_state(&srv_0_a).unwrap();
    let pid_a = state_a.pid.unwrap();
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        let _ = kill(Pid::from_raw(pid_a as i32), Signal::SIGKILL);
    }

    // Wait for it to restart, then remove srv_1 via config diff
    let mut is_restarting = false;
    for _ in 0..50 {
        if let Some(state) = mgr_started.server_state(&srv_0_a) {
            if state.lifecycle == ServerLifecycle::Restarting {
                is_restarting = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_restarting);

    // Check that srv_0_b is still running undisturbed
    let state_b = mgr_started.server_state(&srv_0_b).unwrap();
    assert_eq!(state_b.lifecycle, ServerLifecycle::Running);

    // Diff that removes srv_1
    let main_path = tmp.path().join("llamaforge.toml");
    let _config_str = std::fs::read_to_string(&main_path).unwrap();
    // Re-write to only include srv_0_a and srv_0_b
    let new_config_str = format!(
        r#"
[global]
restart_stagger_secs = 2
[metrics]

[servers.srv_0_a]
devices = [{{gpu_id = 0, layer_range = [0, 10]}}]
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100

[servers.srv_0_b]
devices = [{{gpu_id = 0, layer_range = [0, 10]}}]
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100
"#,
        mock_llama_bin(),
        port,
        mock_llama_bin(),
        port + 1
    );
    std::fs::write(&main_path, new_config_str).unwrap();

    let diff = mgr.config().reload().await.unwrap();
    mgr_started.apply_config_diff(&diff).await;

    // Verify srv_1 removed cleanly
    let mut is_s1_removed = false;
    for _ in 0..50 {
        if mgr_started.server_state(&srv_1).is_none() {
            is_s1_removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_s1_removed, "srv_1 not removed");

    mgr_started.shutdown_all().await;

    let rows = pipeline.store.list_managed_processes().await.unwrap();
    assert_eq!(rows.len(), 0);

    let ledger = mgr_started.vram_ledger();
    assert!(ledger.is_empty() || ledger.values().all(|&v| v == 0));

    assert!(start_time.elapsed() < Duration::from_secs(60), "Test took too long");
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
