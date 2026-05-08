//! Observability decorator for ConfigHandle.
//!
//! See design doc Section 2.5 and 7.1.

use crate::diff::ConfigDiff;
use crate::error::{ConfigReloadError, WritebackError};
use crate::handle::ConfigHandle;
use crate::traits::{ConfigReader, ConfigWriter};
use crate::types::{AppConfig, ConfigMutation};
use arc_swap::Guard;
use metrics::{counter, describe_counter, describe_histogram, histogram};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, instrument, warn};

/// Observability decorator for ConfigHandle.
/// Enforces the domain/observability boundary by keeping telemetry out of the core logic.
pub struct InstrumentedConfigHandle {
    inner: ConfigHandle,
}

impl InstrumentedConfigHandle {
    /// Wrap a ConfigHandle with observability features.
    pub fn new(inner: ConfigHandle) -> Self {
        describe_counter!(
            "llamaforge_config_reloads_total",
            "Total number of configuration reloads attempted"
        );
        describe_histogram!(
            "llamaforge_config_reload_duration_seconds",
            "Time taken to reload the configuration"
        );
        describe_counter!(
            "llamaforge_config_writebacks_total",
            "Total number of configuration writebacks attempted"
        );
        describe_histogram!(
            "llamaforge_config_writeback_duration_seconds",
            "Time taken to persist configuration mutations"
        );

        Self { inner }
    }
}

impl crate::traits::private::Sealed for InstrumentedConfigHandle {}

impl ConfigReader for InstrumentedConfigHandle {
    // Hot path: zero instrumentation overhead.
    fn snapshot(&self) -> Guard<Arc<AppConfig>> {
        self.inner.snapshot()
    }

    fn subscribe(&self) -> broadcast::Receiver<Arc<ConfigDiff>> {
        self.inner.subscribe()
    }

    fn arc_swap(&self) -> Arc<arc_swap::ArcSwap<AppConfig>> {
        self.inner.arc_swap()
    }
}

impl ConfigWriter for InstrumentedConfigHandle {
    #[instrument(
        name = "config.reload",
        skip(self),
        fields(
            config.server_count = tracing::field::Empty,
            config.diff.servers_added = tracing::field::Empty,
            config.diff.servers_removed = tracing::field::Empty,
            config.diff.servers_changed = tracing::field::Empty
        )
    )]
    async fn reload(&self) -> Result<ConfigDiff, ConfigReloadError> {
        let start = std::time::Instant::now();
        let result = self.inner.reload().await;
        let duration = start.elapsed();

        histogram!("llamaforge_config_reload_duration_seconds").record(duration.as_secs_f64());

        match &result {
            Ok(diff) => {
                let snap = self.inner.snapshot();
                let span = tracing::Span::current();
                span.record("config.server_count", snap.servers.len());
                span.record("config.diff.servers_added", diff.servers_added().count());
                span.record("config.diff.servers_removed", diff.servers_removed().count());
                span.record("config.diff.servers_changed", diff.servers_changed().count());

                info!(
                    config.diff.servers_added = diff.servers_added().count(),
                    config.diff.servers_removed = diff.servers_removed().count(),
                    config.diff.servers_changed = diff.servers_changed().count(),
                    config.reload_duration_ms = duration.as_millis(),
                    "config.reloaded"
                );
                counter!("llamaforge_config_reloads_total", "result" => "success").increment(1);
            }
            Err(e) => {
                warn!(
                    error = ?e,
                    config.reload_duration_ms = duration.as_millis(),
                    "config.reload_failed"
                );
                counter!("llamaforge_config_reloads_total", "result" => "error").increment(1);
            }
        }

        result
    }

    #[instrument(
        name = "config.writeback",
        skip(self, mutation),
        fields(
            config.mutation_type = tracing::field::Empty,
            config.server_id = tracing::field::Empty,
            config.diff.servers_added = tracing::field::Empty,
            config.diff.servers_removed = tracing::field::Empty,
            config.diff.servers_changed = tracing::field::Empty
        )
    )]
    async fn writeback(&self, mutation: ConfigMutation) -> Result<ConfigDiff, WritebackError> {
        let start = std::time::Instant::now();

        let span = tracing::Span::current();
        let server_id_string = match &mutation {
            ConfigMutation::SetLoraAdapters { server_id, .. } => server_id.0.clone(),
        };
        span.record("config.mutation_type", "SetLoraAdapters");
        span.record("config.server_id", server_id_string.as_str());

        let result = self.inner.writeback(mutation).await;
        let duration = start.elapsed();

        histogram!("llamaforge_config_writeback_duration_seconds").record(duration.as_secs_f64());

        match &result {
            Ok(diff) => {
                span.record("config.diff.servers_added", diff.servers_added().count());
                span.record("config.diff.servers_removed", diff.servers_removed().count());
                span.record("config.diff.servers_changed", diff.servers_changed().count());

                info!(
                    config.server_id = server_id_string.as_str(),
                    config.diff.servers_added = diff.servers_added().count(),
                    config.diff.servers_removed = diff.servers_removed().count(),
                    config.diff.servers_changed = diff.servers_changed().count(),
                    config.writeback_duration_ms = duration.as_millis(),
                    "config.writeback"
                );
                counter!("llamaforge_config_writebacks_total", "result" => "success").increment(1);
            }
            Err(e @ WritebackError::ReadOnly { .. }) => {
                warn!(
                    error = ?e,
                    config.server_id = server_id_string.as_str(),
                    config.writeback_duration_ms = duration.as_millis(),
                    "config.writeback_failed_readonly"
                );
                counter!("llamaforge_config_writebacks_total", "result" => "error_readonly")
                    .increment(1);
            }
            Err(e) => {
                warn!(
                    error = ?e,
                    config.server_id = server_id_string.as_str(),
                    config.writeback_duration_ms = duration.as_millis(),
                    "config.writeback_failed"
                );
                counter!("llamaforge_config_writebacks_total", "result" => "error").increment(1);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::memory::InMemoryConfigSource;
    use crate::types::ServerId;
    use tracing_subscriber::EnvFilter;

    // A simple test to ensure it compiles and executes without panic.
    // Real tracing assertions are tricky in unit tests without a specific capture setup,
    // but we can verify the behavior delegates correctly.
    #[tokio::test]
    async fn test_instrumented_handle_delegates() {
        let _ = tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).try_init();

        let config = AppConfig::default();
        let source = Arc::new(InMemoryConfigSource::new(config.clone()));
        let handle = ConfigHandle::load(source).unwrap();
        let instrumented = InstrumentedConfigHandle::new(handle);

        // Snapshot delegates
        assert_eq!(**instrumented.snapshot(), config);

        // Reload delegates
        let diff = instrumented.reload().await.unwrap();
        assert!(diff.changes.is_empty());

        // Writeback delegates
        let mutation = ConfigMutation::SetLoraAdapters {
            server_id: ServerId::from("missing"),
            adapters: vec![],
        };
        let err = instrumented.writeback(mutation).await.unwrap_err();
        assert!(matches!(err, WritebackError::ServerNotFound));
    }
}
