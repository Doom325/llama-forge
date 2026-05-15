//! Standalone `mock-llama` binary. Used by legacy subprocess-style
//! tests; new tests use the library API directly.

use mock_llama::{MockConfig, MockLlama};

#[tokio::main]
async fn main() {
    let mut alias = "mock".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--alias" => {
                if let Some(a) = args.next() {
                    alias = a.split(',').next().unwrap_or("mock").to_string();
                }
            }
            // --port is accepted for legacy compatibility but ignored —
            // the standalone binary always binds on an ephemeral port
            // and prints it to stderr.
            "--port" => {
                let _ = args.next();
            }
            _ => {}
        }
    }

    let config = MockConfig { model_alias: alias, ..Default::default() };
    let mock = MockLlama::start(config).await.expect("mock-llama bind failed");
    eprintln!("mock-llama listening on {}", mock.base_url());
    let _ = tokio::signal::ctrl_c().await;
    mock.shutdown().await;
}
