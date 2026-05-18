use llamaforge_server_manager::ServerManager;
use llamaforge_types::traits::{ServerControl, ServerError, ServerSummary};
use std::sync::Arc;

pub struct ServerControlAdapter {
    manager: Arc<ServerManager>,
}

impl ServerControlAdapter {
    pub fn new(manager: Arc<ServerManager>) -> Self {
        Self { manager }
    }

    /// Reverse `types::ServerId` → `config::ServerId` by iterating
    /// the manager's fleet and matching the UUIDv5-derived id. Slug
    /// configs can no longer be looked up by stringifying the UUID,
    /// so iteration is the only option short of a side-table.
    fn resolve_config_id(
        &self,
        id: &llamaforge_types::ServerId,
    ) -> Option<llamaforge_config::ServerId> {
        self.manager.all_servers().into_iter().find_map(|(config_id, _)| {
            (llamaforge_types::ServerId::from_config_id(&config_id.0) == *id).then_some(config_id)
        })
    }
}

#[async_trait::async_trait]
impl ServerControl for ServerControlAdapter {
    async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError> {
        let mut summaries = Vec::new();
        for (config_id, state) in self.manager.all_servers() {
            let busy_slots = state.slots.iter().filter(|s| s.is_processing).count() as u32;

            let types_id = llamaforge_types::ServerId::from_config_id(&config_id.0);

            summaries.push(ServerSummary {
                id: types_id,
                alias: llamaforge_types::ModelAlias::new(state.model_alias.into()),
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

    async fn start_server(&self, id: &llamaforge_types::ServerId) -> Result<(), ServerError> {
        let config_id = self
            .resolve_config_id(id)
            .ok_or(ServerError::NotFound { id: *id })?;
        match self.manager.request_start(&config_id).await {
            Ok(_) => Ok(()),
            Err(llamaforge_server_manager::ServerManagerError::ServerNotFound { id: _ }) => {
                Err(ServerError::NotFound { id: *id })
            }
            Err(llamaforge_server_manager::ServerManagerError::StartFailed { id: _, reason }) => {
                if reason.contains("already running") {
                    Err(ServerError::AlreadyRunning { id: *id })
                } else {
                    Err(ServerError::SpawnFailed { reason })
                }
            }
            Err(e) => Err(ServerError::SpawnFailed { reason: e.to_string() }),
        }
    }

    async fn stop_server(&self, id: &llamaforge_types::ServerId) -> Result<(), ServerError> {
        let config_id = self
            .resolve_config_id(id)
            .ok_or(ServerError::NotFound { id: *id })?;
        match self.manager.request_stop(&config_id).await {
            Ok(_) => Ok(()),
            Err(llamaforge_server_manager::ServerManagerError::ServerNotFound { id: _ }) => {
                Err(ServerError::NotFound { id: *id })
            }
            Err(e) => Err(ServerError::SpawnFailed { reason: e.to_string() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llamaforge_config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};

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
        impl llamaforge_server_manager::GpuTelemetry for DummyTelemetry {
            fn utilization_percent(
                &self,
                _: &llamaforge_config::GpuId,
            ) -> Result<u8, llamaforge_server_manager::TelemetryError> {
                Ok(0)
            }
            fn list_devices(
                &self,
            ) -> Result<
                Vec<llamaforge_server_manager::GpuDevice>,
                llamaforge_server_manager::TelemetryError,
            > {
                Ok(vec![])
            }
            fn free_vram_bytes(
                &self,
                _: &llamaforge_config::GpuId,
            ) -> Result<u64, llamaforge_server_manager::TelemetryError> {
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

        let id = llamaforge_types::ServerId::new(
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        );
        let start_res = adapter.start_server(&id).await;
        assert!(start_res.is_ok());

        let start_again = adapter.start_server(&id).await;
        assert!(matches!(start_again, Err(ServerError::AlreadyRunning { .. })));

        let not_found_id = llamaforge_types::ServerId::new(
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
        );
        let start_res_not_found = adapter.start_server(&not_found_id).await;
        assert!(matches!(start_res_not_found, Err(ServerError::NotFound { .. })));

        let stop_res = adapter.stop_server(&not_found_id).await;
        assert!(matches!(stop_res, Err(ServerError::NotFound { .. })));
    }
}
