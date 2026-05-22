//! Integration test for `InstrumentedConfigHandle::writeback`'s tracing fields.
//!
//! Lives in its own integration-test binary so the global tracing
//! subscriber it installs cannot clash with the `tracing_test::traced_test`
//! attributes used by other unit tests in `llamaforge-core` (most notably
//! `routing::weights`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use llamaforge_core::config::source::memory::InMemoryConfigSource;
use llamaforge_core::config::{
    ConfigHandle, ConfigMutation, ConfigWriter, InstrumentedConfigHandle, LoraAdapterConfig,
    ServerConfig, ServerId,
};

use tracing::field::{Field, Visit};
use tracing::span::{Id, Record};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

static RECORDED: AtomicBool = AtomicBool::new(false);

struct TestLayer;

impl<S: tracing::Subscriber> Layer<S> for TestLayer {
    fn on_record(&self, _span: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        struct FieldVisitor;
        impl Visit for FieldVisitor {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                let s = format!("{:?}", value);
                if field.name() == "config.write_strategy"
                    && (s.contains("Memory") || s.contains("AtomicRename"))
                {
                    RECORDED.store(true, Ordering::SeqCst);
                }
            }
            fn record_str(&mut self, field: &Field, value: &str) {
                if field.name() == "config.write_strategy"
                    && (value.contains("Memory") || value.contains("AtomicRename"))
                {
                    RECORDED.store(true, Ordering::SeqCst);
                }
            }
        }
        values.record(&mut FieldVisitor);
    }
}

#[tokio::test]
async fn test_instrumented_handle_tracing_fields() {
    // Initialize global subscriber if not already done.
    // (Owns this integration test binary outright, so it cannot collide
    // with other subscribers.)
    let _ = tracing::subscriber::set_global_default(tracing_subscriber::registry().with(TestLayer));

    RECORDED.store(false, Ordering::SeqCst);

    let mut config = llamaforge_core::config::AppConfig::default();
    config.servers.insert(
        ServerId::from_config_id("00000000-0000-0000-0000-000000000001"),
        ServerConfig::default(),
    );
    let source = Arc::new(InMemoryConfigSource::new(config.clone()));
    let handle = ConfigHandle::load(source).unwrap();
    let instrumented = InstrumentedConfigHandle::new(handle);

    let mutation = ConfigMutation::SetLoraAdapters {
        server_id: ServerId::from_config_id("00000000-0000-0000-0000-000000000001"),
        adapters: vec![LoraAdapterConfig {
            path: std::path::PathBuf::from("lora.gguf"),
            scale: 1.0,
        }],
    };

    let _ = instrumented.writeback(mutation).await.unwrap();

    assert!(RECORDED.load(Ordering::SeqCst), "config.write_strategy was not recorded");
}
