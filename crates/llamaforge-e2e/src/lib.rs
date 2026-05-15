//! End-to-end test harness for LlamaForge.
//!
//! Spins up a `mock-llama` server on an ephemeral port, writes a temp
//! `llamaforge.toml` pointing at that port, and exposes a `Harness`
//! that builds the full composition root in-process. Tests poke the
//! resulting HTTP / gRPC endpoints with real clients.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

use mock_llama::{MockConfig, MockLlama};

/// Live end-to-end harness. Holds a running `mock-llama` plus the temp
/// state-dir backing it. The actual LlamaForge composition root is
/// constructed via the binary's `serve::run` (today inaccessible from
/// outside the bin); tests speak directly to mock-llama plus
/// orchestrator endpoints if the orchestrator is also under test.
pub struct Harness {
    pub mock: MockLlama,
    pub _tmp: TempDir,
    pub config_path: PathBuf,
}

impl Harness {
    /// Boot the mock with a known alias and write a llamaforge.toml
    /// pointing at it.
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_with_alias("e2e-mock").await
    }

    pub async fn start_with_alias(alias: &str) -> anyhow::Result<Self> {
        let cfg = MockConfig {
            model_alias: alias.to_string(),
            completion_chunks: 3,
            chunk_delay: Duration::from_millis(1),
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
model_path = "{model}"
port = {port}
ctx_size = 1024
parallel_slots = 1
alias = "{alias}"
"#,
            sid = server_id,
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
}

/// Convenience: wait for an HTTP probe to return 200 OK or timeout.
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

/// Convenience: shutdown a harness in the canonical order.
pub async fn shutdown(h: Harness) {
    let Harness { mock, .. } = h;
    mock.shutdown().await;
}

#[allow(dead_code)]
fn _ensure_components_reachable() {
    // Tickle the imports so the workspace deps are exercised at compile time
    // even before the in-process orchestrator build is wired (PR10).
    let _ = std::any::TypeId::of::<llamaforge_adapters::queue::QueueSubmitterAdapter>();
    let _ = std::any::TypeId::of::<llamaforge_transport::TransportError>();
    let _: Arc<()> = Arc::new(());
    let _ = std::marker::PhantomData::<llamaforge_types::ChatCompletionRequest>;
    let _ = std::marker::PhantomData::<llamaforge_config::AppConfig>;
    let _ = std::marker::PhantomData::<llamaforge_metrics::MetricsConfig>;
    let _ = std::marker::PhantomData::<llamaforge_server_manager::ServerManager>;
    let _ = std::marker::PhantomData::<llamaforge_routing::Router>;
    let _ = std::marker::PhantomData::<llamaforge_dispatch::dispatcher::Dispatcher>;
    let _ = std::marker::PhantomData::<llamaforge_queue::pipeline::RequestPipeline>;
}
