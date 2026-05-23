use llamaforge_core::types::traits::MetricsExporter;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

pub struct MetricsExporterAdapter {
    handle: PrometheusHandle,
}

impl MetricsExporterAdapter {
    /// Installs the global Prometheus recorder and returns the adapter holding the handle.
    /// This is a process-global operation and should only be called once at boot.
    pub fn install_global() -> Result<Self, metrics_exporter_prometheus::BuildError> {
        let builder = PrometheusBuilder::new();
        let handle = builder.install_recorder()?;
        Ok(Self { handle })
    }
}

impl MetricsExporter for MetricsExporterAdapter {
    fn gather_text(&self) -> String {
        self.handle.render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics::counter;

    #[test]
    fn test_metrics_exporter_renders_text() {
        // Using install_recorder() in tests can be tricky if multiple tests try to install it.
        // But since this is the only test that does it, it should be fine. If it fails due to
        // already installed, we can ignore the error and use a standalone builder.
        // Actually, we can just use `PrometheusBuilder::new().build_recorder()` which returns
        // `(PrometheusRecorder, PrometheusHandle)` without installing it globally!

        let builder = PrometheusBuilder::new();
        let recorder = builder.build_recorder();
        let handle = recorder.handle();

        let adapter = MetricsExporterAdapter { handle };

        // We can't use the `metrics::counter!` macro without installing a recorder globally,
        // unless we use `metrics::with_local_recorder`.
        metrics::with_local_recorder(&recorder, || {
            counter!("llamaforge_test_metric").increment(1);
        });

        let rendered = adapter.gather_text();
        assert!(rendered.contains("llamaforge_test_metric"));
    }
}
