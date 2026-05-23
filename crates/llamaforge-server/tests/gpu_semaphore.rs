#![cfg(feature = "test-utils")]

use llamaforge_core::config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
use llamaforge_core::types::ServerId;
use llamaforge_metrics::create_metrics_pipeline;
use llamaforge_server::{ServerManager, TelemetryError};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::Instant;
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
        Ok(0)
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
[global]
restart_stagger_secs = 2

[servers.srv1]
binary_path = "{}"
model_path = "{}"
alias = "mock"
port = {}
ctx_size = 4096
parallel_slots = 1
estimated_vram_mb = 100

[servers.srv2]
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
        mock_llama_bin(),
        mock_llama_bin(),
        mock_port + 1
    ));

    std::fs::write(&main_path, config_str).unwrap();
    let paths = ConfigPaths::new(main_path, tmp.path().join("priority_rules.toml"), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

#[tokio::test]
#[ignore]
async fn test_gpu_semaphore() {
    let _ = tracing_subscriber::fmt::try_init();
    let tmp = TempDir::new().unwrap();
    let config = get_test_config(&tmp, 18300);
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

    let _start_time = tokio::time::Instant::now();
    let mgr_started = mgr.clone().start().await.unwrap();

    let srv1_id = ServerId::from_config_id("srv1");
    let srv2_id = ServerId::from_config_id("srv2");

    let mut srv1_running_time = None;
    let mut srv2_running_time = None;

    // Wait until both are Running
    for _ in 0..100 {
        if srv1_running_time.is_none() {
            if let Some(state) = mgr_started.server_state(&srv1_id) {
                if state.lifecycle == llamaforge_server::ServerLifecycle::Running {
                    srv1_running_time = Some(Instant::now());
                }
            }
        }
        if srv2_running_time.is_none() {
            if let Some(state) = mgr_started.server_state(&srv2_id) {
                if state.lifecycle == llamaforge_server::ServerLifecycle::Running {
                    srv2_running_time = Some(Instant::now());
                }
            }
        }
        if srv1_running_time.is_some() && srv2_running_time.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(srv1_running_time.is_some(), "srv1 did not reach Running");
    assert!(srv2_running_time.is_some(), "srv2 did not reach Running");

    // Because they use the same GPU and restart_stagger_secs = 2,
    // the time between their Running states (plus the time to spawn) must be at least 2 seconds.
    // Wait, the semaphore is released with stagger. So if S1 acquires it and starts, it releases the semaphore 2 seconds later.
    // So S2 must acquire it at least 2 seconds after S1's health passed.
    // Thus the difference between their running times must be >= 2.0.
    let diff = if srv1_running_time.unwrap() > srv2_running_time.unwrap() {
        srv1_running_time.unwrap().duration_since(srv2_running_time.unwrap())
    } else {
        srv2_running_time.unwrap().duration_since(srv1_running_time.unwrap())
    };

    assert!(diff.as_secs_f64() >= 1.5, "Stagger not enforced: diff was {:?}", diff);

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
