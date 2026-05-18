//! Standalone `mock-llama` binary. Spawned as a subprocess by
//! server-manager integration tests; new in-process e2e tests use the
//! library API directly.

use mock_llama::{MockConfig, MockLlama};

#[tokio::main]
async fn main() {
    let mut alias = "mock".to_string();
    let mut port: Option<u16> = None;
    let mut parallel_slots: u32 = 1;
    let mut health_fail_after: u32 = u32::MAX;
    let mut startup_503_duration = std::time::Duration::ZERO;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--alias" => {
                if let Some(a) = args.next() {
                    let mut parts = a.split(',');
                    alias = parts.next().unwrap_or("mock").to_string();
                    for part in parts {
                        if let Some(val) = part.strip_prefix("health_fail_after=") {
                            health_fail_after = val.parse().unwrap_or(u32::MAX);
                        } else if let Some(val) = part.strip_prefix("startup_503_ms=") {
                            if let Ok(ms) = val.parse() {
                                startup_503_duration = std::time::Duration::from_millis(ms);
                            }
                        }
                    }
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

    // `MOCK_LLAMA_CHUNK_DELAY_MS` overrides the per-token decode
    // delay (default 1ms). Useful for cancellation tests that need a
    // window in which to drop the client stream.
    let chunk_delay =
        match std::env::var("MOCK_LLAMA_CHUNK_DELAY_MS").ok().and_then(|s| s.parse::<u64>().ok()) {
            Some(ms) => std::time::Duration::from_millis(ms),
            None => MockConfig::default().chunk_delay,
        };

    let config = MockConfig {
        model_alias: alias,
        parallel_slots,
        chunk_delay,
        health_fail_after,
        startup_503_duration,
        ..Default::default()
    };
    let mock = if let Some(p) = port {
        MockLlama::start_on_port(p, config).await.expect("mock-llama bind failed")
    } else {
        MockLlama::start(config).await.expect("mock-llama bind failed")
    };
    eprintln!("mock-llama listening on {}", mock.base_url());
    let _ = tokio::signal::ctrl_c().await;
    mock.shutdown().await;
}
