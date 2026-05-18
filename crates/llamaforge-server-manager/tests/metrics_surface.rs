#![cfg(feature = "test-utils")]

use llamaforge_config::{ConfigHandle, ConfigPaths, ServerId, TomlFileConfigSource};
use llamaforge_metrics::create_metrics_pipeline;
use llamaforge_server_manager::{
    InstrumentedServerManager, ServerLifecycle, ServerManager, TelemetryError,
};
use metrics_util::debugging::{DebuggingRecorder, Snapshot};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// See `config_tests::mock_llama_bin` — same rationale.
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
    let main_path = tmp.path().join(format!("llamaforge_{}.toml", mock_port));
    let state_dir = tmp.path().join(format!("state_{}", mock_port));
    std::fs::create_dir_all(&state_dir).unwrap();

    let mut config_str = String::new();
    config_str.push_str("[metrics]\n");
    config_str.push_str(&format!(
        r#"
[servers.srv_metrics]
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

static NEXT_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(19200);

#[tokio::test]
#[ignore]
#[serial_test::serial]
async fn test_metrics_surface() {
    let _ = tracing_subscriber::fmt::try_init();

    // Install global recorder
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _ = metrics::set_global_recorder(recorder);

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
    llamaforge_server_manager::spawn_metrics_poller(mgr_started.clone());
    let instrumented = InstrumentedServerManager::new(mgr_started.clone());

    let srv_id = ServerId("srv_metrics".into());

    // Wait for Running
    let mut is_running = false;
    for _ in 0..100 {
        if let Some(state) = instrumented.server_state(&srv_id) {
            if state.lifecycle == ServerLifecycle::Running {
                is_running = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(is_running);

    // Call instrumented functions
    for _ in 0..5 {
        instrumented.increment_requests(&srv_id);
    }

    for _ in 0..5 {
        instrumented.increment_pending(&srv_id);
    }

    println!("PENDING AFTER INCREMENTS: {}", instrumented.pending_dispatches(&srv_id));

    for _ in 0..5 {
        instrumented.decrement_pending(&srv_id);
    }

    println!("PENDING AFTER DECREMENTS: {}", instrumented.pending_dispatches(&srv_id));
    // Wait a tick for metrics to update (they update every 10s via poller, we need to wait for poller to run)
    tokio::time::sleep(Duration::from_secs(12)).await;

    let snapshot: Snapshot = snapshotter.snapshot();

    // Verification
    let metrics = snapshot.into_vec();
    let mut metric_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (key, _, _, _) in &metrics {
        metric_names.insert(key.key().name().to_string());
    }

    let expected = vec![
        "llamaforge_server_status",
        "llamaforge_server_restarts_total",
        "llamaforge_server_uptime_seconds",
        "llamaforge_vram_reserved_bytes",
        "llamaforge_vram_free_bytes",
        "llamaforge_gpu_utilization_pct",
        "llamaforge_server_cumulative_requests",
        "llamaforge_pending_dispatches",
        "llamaforge_health_check_duration_seconds",
    ];

    for e in expected {
        if !metric_names.contains(e) {
            println!("FIXME: emit {}", e);
        }
    }

    for (key, _, _, value) in &metrics {
        println!(
            "METRIC: {} {:?} = {:?}",
            key.key().name(),
            key.key().labels().collect::<Vec<_>>(),
            value
        );
    }

    // Verify llamaforge_server_cumulative_requests is emitted
    let has_requests = metrics.iter().any(|(k, _, _, _)| {
        k.key().name() == "llamaforge_server_cumulative_requests"
            && k.key().labels().any(|l| l.key() == "server_id" && l.value() == "srv_metrics")
    });
    assert!(has_requests, "llamaforge_server_cumulative_requests not emitted");

    // Verify llamaforge_pending_dispatches is emitted
    let has_pending = metrics.iter().any(|(k, _, _, _)| {
        k.key().name() == "llamaforge_pending_dispatches"
            && k.key().labels().any(|l| l.key() == "server_id" && l.value() == "srv_metrics")
    });
    assert!(has_pending, "llamaforge_pending_dispatches not emitted");

    mgr_started.shutdown_all().await;
}
