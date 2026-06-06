use llamaforge_core::types::traits::{ServerControl, ServerError, ServerSummary};
use llamaforge_server::ServerManager;
use std::sync::Arc;

pub struct ServerControlAdapter {
    manager: Arc<ServerManager>,
}

impl ServerControlAdapter {
    pub fn new(manager: Arc<ServerManager>) -> Self {
        Self { manager }
    }
}

#[async_trait::async_trait]
impl ServerControl for ServerControlAdapter {
    async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError> {
        let mut summaries = Vec::new();
        for (id, state) in self.manager.all_servers() {
            let busy_slots = state.slots.iter().filter(|s| s.is_processing).count() as u32;

            summaries.push(ServerSummary {
                id,
                alias: llamaforge_core::types::ModelAlias::new(state.model_alias.into()),
                state: state.lifecycle,
                port: state.port,
                slots_total: state.parallel_slots,
                slots_busy: busy_slots,
                started_at: state.started_at,
                last_restart_at: state.last_restart_at,
            });
        }
        Ok(summaries)
    }

    async fn start_server(&self, id: &llamaforge_core::types::ServerId) -> Result<(), ServerError> {
        match self.manager.request_start(id).await {
            Ok(_) => Ok(()),
            Err(llamaforge_server::ServerManagerError::ServerNotFound { id: _ }) => {
                Err(ServerError::NotFound { id: *id })
            }
            Err(llamaforge_server::ServerManagerError::StartFailed { id: _, reason }) => {
                if reason.contains("already running") {
                    Err(ServerError::AlreadyRunning { id: *id })
                } else {
                    Err(ServerError::SpawnFailed { reason })
                }
            }
            Err(e) => Err(ServerError::SpawnFailed { reason: e.to_string() }),
        }
    }

    async fn stop_server(&self, id: &llamaforge_core::types::ServerId) -> Result<(), ServerError> {
        match self.manager.request_stop(id).await {
            Ok(_) => Ok(()),
            Err(llamaforge_server::ServerManagerError::ServerNotFound { id: _ }) => {
                Err(ServerError::NotFound { id: *id })
            }
            Err(e) => Err(ServerError::SpawnFailed { reason: e.to_string() }),
        }
    }

    async fn restart_server(
        &self,
        id: &llamaforge_core::types::ServerId,
    ) -> Result<(), ServerError> {
        match self.manager.request_restart(id).await {
            Ok(_) => Ok(()),
            Err(llamaforge_server::ServerManagerError::ServerNotFound { id: _ }) => {
                Err(ServerError::NotFound { id: *id })
            }
            Err(llamaforge_server::ServerManagerError::StartFailed { id: _, reason }) => {
                Err(ServerError::SpawnFailed { reason })
            }
            Err(e) => Err(ServerError::SpawnFailed { reason: e.to_string() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llamaforge_core::config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};

    fn dummy_manager() -> Arc<ServerManager> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let toml_config = "[global]\n[servers.\"00000000-0000-0000-0000-000000000001\"]\nbinary_path=\"llama-server\"\nmodel_path=\"m1\"\nport=8080\nctx_size=1\n";
        std::fs::write(tmp.path(), toml_config).unwrap();
        let paths = ConfigPaths::new(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
        );
        let source = Arc::new(TomlFileConfigSource::new(paths));
        let config = Arc::new(ConfigHandle::load(source).unwrap());

        let tmp_dir = tempfile::TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("metrics.db");

        let pipeline = llamaforge_metrics::create_metrics_pipeline(
            &llamaforge_metrics::MetricsConfig { db_path, ..Default::default() },
            config.clone(),
        )
        .unwrap();

        let shutdown = tokio_util::sync::CancellationToken::new();

        struct DummyTelemetry;
        impl llamaforge_server::GpuTelemetry for DummyTelemetry {
            fn utilization_percent(
                &self,
                _: &llamaforge_core::config::GpuId,
            ) -> Result<u8, llamaforge_server::TelemetryError> {
                Ok(0)
            }
            fn list_devices(
                &self,
            ) -> Result<Vec<llamaforge_server::GpuDevice>, llamaforge_server::TelemetryError>
            {
                Ok(vec![])
            }
            fn free_vram_bytes(
                &self,
                _: &llamaforge_core::config::GpuId,
            ) -> Result<u64, llamaforge_server::TelemetryError> {
                Ok(1000000)
            }
        }

        Arc::new(ServerManager::new(
            Arc::new(DummyTelemetry),
            config,
            pipeline.ingest,
            pipeline.store.clone(),
            shutdown,
        ))
    }

    #[tokio::test]
    async fn test_server_control_mapping() {
        let mgr = dummy_manager();
        let mgr = mgr.start().await.unwrap();

        let adapter = ServerControlAdapter::new(mgr.clone());

        // Wait a bit for the spawned tasks to fail (since 'm1' is not a valid binary)
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let servers = adapter.list_servers().await.unwrap();
        assert_eq!(servers.len(), 1);

        let id = llamaforge_core::types::ServerId::new(
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        );
        let start_res = adapter.start_server(&id).await;
        assert!(start_res.is_ok());

        let start_again = adapter.start_server(&id).await;
        assert!(matches!(start_again, Err(ServerError::AlreadyRunning { .. })));

        let not_found_id = llamaforge_core::types::ServerId::new(
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
        );
        let start_res_not_found = adapter.start_server(&not_found_id).await;
        assert!(matches!(start_res_not_found, Err(ServerError::NotFound { .. })));

        let stop_res = adapter.stop_server(&not_found_id).await;
        assert!(matches!(stop_res, Err(ServerError::NotFound { .. })));
    }
}
