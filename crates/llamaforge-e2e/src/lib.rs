//! End-to-end test harness for LlamaForge.
//!
//! Two entry points:
//!
//! * [`Harness::start`] — boots an in-process `mock-llama` library
//!   instance on an ephemeral port and writes a temp `llamaforge.toml`
//!   pointing at it. Used by tests that exercise the `mock-llama`
//!   contract or wire-up directly, without a running orchestrator.
//!
//! * [`Harness::start_full`] — does not pre-spawn `mock-llama`. Instead
//!   it sets `model_path` to the compiled `mock-llama` binary and lets
//!   the in-process LlamaForge composition root spawn the upstream
//!   server itself via `ServerManager`. Returns an
//!   [`OrchestratorHandle`] exposing `http_url`, `grpc_url`, and
//!   `shutdown_and_wait`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use mock_llama::{MockConfig, MockLlama};

/// Mock-llama-backed harness for tests that only need the upstream
/// shim (no orchestrator).
pub struct Harness {
    pub mock: MockLlama,
    pub _tmp: TempDir,
    pub config_path: PathBuf,
}

/// Handle to the in-process LlamaForge orchestrator started by
/// [`Harness::start_full`]. Owns the temp dir backing the config so it
/// outlives the orchestrator's reads.
pub struct OrchestratorHandle {
    pub http_url: String,
    pub grpc_url: String,
    pub http_addr: SocketAddr,
    pub grpc_addr: SocketAddr,
    pub config_path: PathBuf,
    /// Cancellation token wired into the in-process composition root.
    /// Kept private so tests can't fire it without going through
    /// [`OrchestratorHandle::shutdown_and_wait`], which is the only
    /// path that also drains transports and joins the scheduler.
    shutdown: CancellationToken,
    transport: Option<llamaforge_transport::TransportHandles>,
    server_manager: Arc<llamaforge_server_manager::ServerManager>,
    scheduler: Option<JoinHandle<()>>,
    _tmp: TempDir,
}

impl OrchestratorHandle {
    /// Cancel the shutdown token, stop transports, and wait for the
    /// background scheduler + server-manager to drain. Idempotent —
    /// second call is a no-op since the handle is consumed.
    pub async fn shutdown_and_wait(mut self) -> anyhow::Result<()> {
        self.shutdown.cancel();
        if let Some(transport) = self.transport.take() {
            transport.shutdown();
            transport.wait().await.map_err(|e| anyhow::anyhow!("transport wait: {}", e))?;
        }
        self.server_manager.shutdown_all().await;
        if let Some(handle) = self.scheduler.take() {
            let _ = handle.await;
        }
        Ok(())
    }
}

impl Harness {
    /// Boot the mock-llama lib on an ephemeral port and write a temp
    /// `llamaforge.toml` pointing at it.
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_with_alias("e2e-mock").await
    }

    pub async fn start_with_alias(alias: &str) -> anyhow::Result<Self> {
        let cfg = MockConfig {
            model_alias: alias.to_string(),
            completion_chunks: 3,
            chunk_delay: Duration::from_millis(1),
            parallel_slots: 1,
            ..Default::default()
        };
        let mock = MockLlama::start(cfg).await?;
        let port = mock.addr.port();
        let server_id = uuid::Uuid::now_v7();

        let tmp = tempfile::tempdir()?;
        let model_path = tmp.path().join("model.gguf");
        std::fs::write(&model_path, b"")?;

        let toml = format!(
            r#"
[global]
bind_address = "127.0.0.1"
http_port = 0
grpc_port = 0

[gateway]

[servers.{sid}]
binary_path = "{binary}"
model_path = "{model}"
port = {port}
ctx_size = 1024
parallel_slots = 1
alias = "{alias}"
"#,
            sid = server_id,
            binary = "/usr/bin/true",
            model = model_path.display(),
            port = port,
            alias = alias,
        );

        let config_path = tmp.path().join("llamaforge.toml");
        std::fs::write(&config_path, toml)?;

        Ok(Self { mock, _tmp: tmp, config_path })
    }

    pub fn mock_base_url(&self) -> String {
        self.mock.base_url()
    }

    /// Boot the full LlamaForge orchestrator in-process, pointed at the
    /// compiled `mock-llama` binary. The `ServerManager` spawns
    /// `mock-llama` itself; the test drives the orchestrator's HTTP /
    /// gRPC endpoints via the returned [`OrchestratorHandle`].
    pub async fn start_full() -> anyhow::Result<OrchestratorHandle> {
        // Default server alias used by the e2e tests. Anything that
        // sends `{"model": "mock"}` (most of the suite) will route to
        // the spawned mock-llama; tests that want a different alias
        // should use `start_full_with_alias`.
        Self::start_full_with_alias("mock").await
    }

    pub async fn start_full_with_alias(alias: &str) -> anyhow::Result<OrchestratorHandle> {
        // Best-effort tracing: makes orchestrator logs visible when the
        // test runner is invoked with `--nocapture` / `RUST_LOG=...`.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .with_test_writer()
            .try_init();

        let mock_bin = mock_llama_bin();
        anyhow::ensure!(
            mock_bin.exists(),
            "mock-llama binary not found at {} — run `cargo build -p mock-llama` first",
            mock_bin.display()
        );

        // Pre-allocate a port for the spawned mock-llama. We bind on
        // 127.0.0.1:0, capture the port, then drop the listener so the
        // child process can rebind. Small race window, acceptable for
        // tests.
        let probe = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let mock_port = probe.local_addr()?.port();
        drop(probe);

        let server_id = uuid::Uuid::now_v7();
        let tmp = tempfile::tempdir()?;
        let state_dir = tmp.path().join("state");
        std::fs::create_dir_all(&state_dir)?;

        // mock-llama ignores --model; create a placeholder GGUF file so the
        // value passed via --model is a real path on disk.
        let model_path = tmp.path().join("model.gguf");
        std::fs::write(&model_path, b"")?;

        let metrics_db_path = tmp.path().join("metrics.db");

        let toml = format!(
            r#"
[global]
bind_address = "127.0.0.1"
http_port = 0
grpc_port = 0
restart_cooldown_secs = 1
metrics_db_path = "{metrics_db}"

[gateway]

[servers.{sid}]
binary_path = "{binary}"
model_path = "{model}"
port = {port}
ctx_size = 1024
parallel_slots = 4
alias = "{alias}"
estimated_vram_mb = 100
"#,
            sid = server_id,
            binary = mock_bin.display(),
            model = model_path.display(),
            port = mock_port,
            alias = alias,
            metrics_db = metrics_db_path.display(),
        );

        let config_path = tmp.path().join("llamaforge.toml");
        std::fs::write(&config_path, toml)?;

        // Load via the file source so hot-reload and writeback paths
        // match production.
        let paths = llamaforge_config::ConfigPaths::new(
            config_path.clone(),
            config_path.with_file_name("priority_rules.toml"),
            state_dir,
        );
        let source = Arc::new(llamaforge_config::TomlFileConfigSource::new(paths));
        let handle = Arc::new(llamaforge_config::ConfigHandle::load(source)?);

        // Build the orchestrator. Mirrors `llamaforge::serve::run` minus
        // the signal-handling loop and shutdown sequence — those live on
        // the returned `OrchestratorHandle` so the test owns the
        // lifecycle.
        let shutdown = CancellationToken::new();
        let (deps, bg) = llamaforge::build(handle.clone(), shutdown.clone()).await?;
        let llamaforge::BackgroundTasks { scheduler, server_manager } = bg;

        server_manager
            .clone()
            .start()
            .await
            .map_err(|e| anyhow::anyhow!("server-manager start: {}", e))?;

        // Mirror the production wiring in `llamaforge::serve::run`:
        // forward config-change diffs into the server-manager so a
        // `ReloadConfig` actually propagates into the running fleet.
        // Without this, hot-reloadable edits (alias / lora_adapters)
        // would broadcast on a channel nobody subscribed to.
        let config_for_forwarder = handle.clone();
        let sm_for_forwarder = server_manager.clone();
        let _diff_task = tokio::spawn(async move {
            use llamaforge_config::ConfigReader;
            let mut rx = config_for_forwarder.subscribe();
            while let Ok(diff) = rx.recv().await {
                if diff.changes.is_empty() {
                    continue;
                }
                sm_for_forwarder.apply_config_diff(&diff).await;
            }
        });

        let transport = llamaforge_transport::start_servers(deps)
            .await
            .map_err(|e| anyhow::anyhow!("transport start_servers: {}", e))?;

        let http_addr = transport.http_addr;
        let grpc_addr = transport.grpc_addr;
        let http_url = format!("http://{}", http_addr);
        let grpc_url = format!("http://{}", grpc_addr);

        // Probe `/health` until it answers. The HTTP server needs a
        // moment after `bind()` before it accepts connections.
        wait_for_http_200(&format!("{}/health", http_url), Duration::from_secs(5)).await?;

        // Wait for the spawned mock-llama upstream to be reachable, then
        // for server-manager to observe it as Running. Without the
        // direct probe the wait_for_running_server polling can race
        // with the upstream spawn.
        let mock_url = format!("http://127.0.0.1:{}/health", mock_port);
        wait_for_http_200(&mock_url, Duration::from_secs(15)).await?;
        wait_for_running_server(&server_manager, Duration::from_secs(15)).await?;

        Ok(OrchestratorHandle {
            http_url,
            grpc_url,
            http_addr,
            grpc_addr,
            shutdown,
            config_path,
            transport: Some(transport),
            server_manager,
            scheduler: Some(scheduler),
            _tmp: tmp,
        })
    }
}

/// Resolve the compiled `mock-llama` binary path. Mirrors the helper
/// `crates/llamaforge-server-manager/tests/*.rs` use: respect
/// `CARGO_TARGET_DIR` if set, otherwise walk up from this crate's
/// manifest dir to the workspace `target/{debug|release}/mock-llama`.
fn mock_llama_bin() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("target"))
            .unwrap_or_else(|| PathBuf::from("target"))
    });
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    target.join(profile).join("mock-llama")
}

/// Poll the server-manager until at least one configured server has
/// reached `Running` lifecycle (i.e. health-check passed at least once).
async fn wait_for_running_server(
    sm: &llamaforge_server_manager::ServerManager,
    deadline: Duration,
) -> anyhow::Result<()> {
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > deadline {
            anyhow::bail!("timeout waiting for any server to reach Running lifecycle");
        }
        let any_running = sm.all_servers().iter().any(|(_, s)| {
            matches!(s.lifecycle, llamaforge_server_manager::ServerLifecycle::Running)
        });
        if any_running {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Wait for an HTTP probe to return 200 OK or timeout.
pub async fn wait_for_http_200(url: &str, deadline: Duration) -> anyhow::Result<()> {
    let start = tokio::time::Instant::now();
    let client = reqwest::Client::new();
    loop {
        if start.elapsed() > deadline {
            anyhow::bail!("timeout waiting for 200 from {}", url);
        }
        if let Ok(res) = client.get(url).send().await {
            if res.status() == 200 {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Convenience: shutdown a mock-llama-only harness in the canonical order.
pub async fn shutdown(h: Harness) {
    let Harness { mock, .. } = h;
    mock.shutdown().await;
}
