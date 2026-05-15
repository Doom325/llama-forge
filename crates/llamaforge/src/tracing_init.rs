//! Tracing-subscriber bootstrap for the orchestrator binary.
//!
//! Honours `RUST_LOG` (default `info`). Writes JSON to stdout when the config's
//! `log_format` is `json`; otherwise uses the pretty formatter. See design doc
//! §7 (Observability) for the layered subscriber strategy.

use llamaforge_config::LogFormat;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let registry = tracing_subscriber::registry().with(filter);
    match format {
        LogFormat::Json => {
            registry.with(tracing_subscriber::fmt::layer().json()).init();
        }
        LogFormat::Pretty | LogFormat::Compact => {
            registry.with(tracing_subscriber::fmt::layer()).init();
        }
    }
}
