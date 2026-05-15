//! Standalone `mock-llama` binary. Spawned as a subprocess by
//! server-manager integration tests; new in-process e2e tests use the
//! library API directly.

use mock_llama::{MockConfig, MockLlama};

#[tokio::main]
async fn main() {
    let mut alias = "mock".to_string();
    let mut port: Option<u16> = None;
    let mut parallel_slots: u32 = 1;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--alias" => {
                if let Some(a) = args.next() {
                    alias = a.split(',').next().unwrap_or("mock").to_string();
                }
            }
            "--port" => {
                if let Some(p) = args.next() {
                    port = p.parse().ok();
                }
            }
            "--parallel" => {
                if let Some(p) = args.next() {
                    parallel_slots = p.parse().unwrap_or(1);
                }
            }
            _ => {}
        }
    }

    let config = MockConfig { model_alias: alias, parallel_slots, ..Default::default() };
    let mock = if let Some(p) = port {
        MockLlama::start_on_port(p, config).await.expect("mock-llama bind failed")
    } else {
        MockLlama::start(config).await.expect("mock-llama bind failed")
    };
    eprintln!("mock-llama listening on {}", mock.base_url());
    let _ = tokio::signal::ctrl_c().await;
    mock.shutdown().await;
}
