use llamaforge_types::traits::MetricsExporter;

#[derive(Default)]
pub struct MetricsExporterAdapter {}

impl MetricsExporterAdapter {
    pub fn new() -> Self {
        Self {}
    }
}

impl MetricsExporter for MetricsExporterAdapter {
    fn gather_text(&self) -> String {
        unimplemented!("Implemented in PR5")
    }
}
