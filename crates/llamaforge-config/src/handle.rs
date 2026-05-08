//! ConfigHandle core logic.
//!
//! See design doc Section 2.3.

use crate::diff::{compute_diff, ConfigDiff};
use crate::error::{ConfigLoadError, ConfigReloadError, WritebackError};
use crate::source::ConfigSource;
use crate::traits::{ConfigReader, ConfigWriter};
use crate::types::{AppConfig, ConfigMutation};
use arc_swap::{ArcSwap, Guard};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Central coordinator for configuration state and reloads.
/// See design doc Section 2.3.
pub struct ConfigHandle {
    source: Arc<dyn ConfigSource>,
    inner: ArcSwap<AppConfig>,
    reload_mutex: tokio::sync::Mutex<()>,
    change_tx: broadcast::Sender<Arc<ConfigDiff>>,
}

impl ConfigHandle {
    /// Load the initial configuration and construct the handle.
    pub fn load(source: Arc<dyn ConfigSource>) -> Result<Self, ConfigLoadError> {
        let config = source.load()?;
        let (change_tx, _) = broadcast::channel(16);

        Ok(Self {
            source,
            inner: ArcSwap::from_pointee(config),
            reload_mutex: tokio::sync::Mutex::new(()),
            change_tx,
        })
    }

    /// Export the current configuration as a TOML string.
    pub fn export_toml(&self) -> Result<String, WritebackError> {
        let snapshot = self.inner.load();
        self.source.export(&snapshot)
    }
}

impl crate::traits::private::Sealed for ConfigHandle {}

impl ConfigReader for ConfigHandle {
    fn snapshot(&self) -> Guard<Arc<AppConfig>> {
        self.inner.load()
    }

    fn subscribe(&self) -> broadcast::Receiver<Arc<ConfigDiff>> {
        self.change_tx.subscribe()
    }
}

impl ConfigWriter for ConfigHandle {
    async fn reload(&self) -> Result<ConfigDiff, ConfigReloadError> {
        let _guard = self.reload_mutex.lock().await;

        let source = self.source.clone();
        // Load the new config in a blocking task since source I/O blocks.
        let new_config = tokio::task::spawn_blocking(move || source.load())
            .await
            .map_err(|e| ConfigReloadError::LoadTaskJoin { message: e.to_string() })??;

        let old_config = self.inner.load();
        let diff = compute_diff(&old_config, &new_config);

        if !diff.changes.is_empty() {
            self.inner.store(Arc::new(new_config));
        }
        // Design choice: broadcast even if empty.
        let _ = self.change_tx.send(Arc::new(diff.clone()));

        Ok(diff)
    }

    async fn writeback(&self, mutation: ConfigMutation) -> Result<ConfigDiff, WritebackError> {
        let _guard = self.reload_mutex.lock().await;

        let current_config = (**self.inner.load()).clone();

        let source = self.source.clone();
        let mutation_clone = mutation.clone();
        let config_clone = current_config.clone();

        tokio::task::spawn_blocking(move || source.persist(&config_clone, &mutation_clone))
            .await
            .map_err(|e| WritebackError::SerializeError { source: Box::new(e) })??;

        let mut new_config = current_config.clone();
        match mutation {
            ConfigMutation::SetLoraAdapters { server_id, adapters } => {
                if let Some(server) = new_config.servers.get_mut(&server_id) {
                    server.lora_adapters = adapters;
                } else {
                    return Err(WritebackError::SerializeError {
                        source: format!("server '{}' not found", server_id).into(),
                    });
                }
            }
        }

        let diff = compute_diff(&current_config, &new_config);

        if !diff.changes.is_empty() {
            self.inner.store(Arc::new(new_config));
            let _ = self.change_tx.send(Arc::new(diff.clone()));
        }

        Ok(diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::ConfigChange;
    use crate::source::memory::InMemoryConfigSource;
    use crate::types::{ServerConfig, ServerId};

    #[tokio::test]
    async fn test_config_handle_load_and_snapshot() {
        let config = AppConfig::default();
        let source = Arc::new(InMemoryConfigSource::new(config.clone()));

        let handle = ConfigHandle::load(source).unwrap();

        let start = std::time::Instant::now();
        for _ in 0..1000 {
            let _snap = handle.snapshot();
        }
        let duration = start.elapsed();
        assert!(duration.as_millis() <= 5, "snapshot should be very fast");

        assert_eq!(**handle.snapshot(), config);
    }

    #[tokio::test]
    async fn test_config_handle_reload_with_changes() {
        let config = AppConfig::default();
        let source = Arc::new(InMemoryConfigSource::new(config.clone()));
        let handle = ConfigHandle::load(source.clone()).unwrap();

        let mut rx = handle.subscribe();

        // Mutate the underlying source out-of-band (simulating file edit)
        let mut new_config = config.clone();
        new_config.servers.insert(
            ServerId::from("new_server"),
            ServerConfig { port: 8080, ..Default::default() },
        );
        // Replace inner directly for the test
        source.replace_config(new_config.clone());

        let diff = handle.reload().await.unwrap();
        assert_eq!(diff.changes.len(), 1);
        assert!(
            matches!(&diff.changes[0], ConfigChange::ServerAdded { id } if id.0 == "new_server")
        );

        // Verify snapshot updated
        assert_eq!(**handle.snapshot(), new_config);

        // Verify broadcast received
        let received_diff = rx.recv().await.unwrap();
        assert_eq!(received_diff.changes.len(), 1);
    }

    #[tokio::test]
    async fn test_config_handle_reload_no_changes() {
        let config = AppConfig::default();
        let source = Arc::new(InMemoryConfigSource::new(config.clone()));
        let handle = ConfigHandle::load(source).unwrap();

        let mut rx = handle.subscribe();

        let diff = handle.reload().await.unwrap();
        assert!(diff.changes.is_empty());

        // Verify broadcast received even if empty
        let received_diff = rx.recv().await.unwrap();
        assert!(received_diff.changes.is_empty());
    }

    #[tokio::test]
    async fn test_config_handle_reload_error() {
        let config = AppConfig::default();
        use std::sync::atomic::{AtomicBool, Ordering};
        struct FailOnSecondSource {
            config: AppConfig,
            failed: AtomicBool,
        }
        impl crate::source::private::Sealed for FailOnSecondSource {}
        impl ConfigSource for FailOnSecondSource {
            fn load(&self) -> Result<AppConfig, ConfigLoadError> {
                if self.failed.swap(true, Ordering::SeqCst) {
                    Err(ConfigLoadError::ValidationError { message: "fail".into() })
                } else {
                    Ok(self.config.clone())
                }
            }
            fn persist(&self, _: &AppConfig, _: &ConfigMutation) -> Result<(), WritebackError> {
                Ok(())
            }
            fn export(&self, _: &AppConfig) -> Result<String, WritebackError> {
                Ok("".into())
            }
        }

        let source = Arc::new(FailOnSecondSource { config, failed: AtomicBool::new(false) });
        let handle = ConfigHandle::load(source).unwrap();
        let mut rx = handle.subscribe();

        let err = handle.reload().await.unwrap_err();
        assert!(matches!(
            err,
            ConfigReloadError::LoadError(ConfigLoadError::ValidationError { .. })
        ));

        // Verify snapshot unchanged
        assert_eq!(**handle.snapshot(), AppConfig::default());

        // Verify NO broadcast received
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_config_handle_concurrent_reloads() {
        let config = AppConfig::default();
        let source = Arc::new(InMemoryConfigSource::new(config.clone()));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());

        let mut tasks = vec![];
        for _ in 0..10 {
            let h = handle.clone();
            tasks.push(tokio::spawn(async move {
                h.reload().await.unwrap();
            }));
        }

        for t in tasks {
            t.await.unwrap();
        }

        assert_eq!(**handle.snapshot(), AppConfig::default());
    }

    #[tokio::test]
    async fn test_config_handle_broadcast_lagged() {
        let config = AppConfig::default();
        let source = Arc::new(InMemoryConfigSource::new(config.clone()));
        let handle = ConfigHandle::load(source).unwrap();

        let mut rx = handle.subscribe();

        // Broadcast channel capacity is 16. Send 20 reloads.
        for _ in 0..20 {
            handle.reload().await.unwrap();
        }

        let result = rx.try_recv();
        assert!(matches!(result.unwrap_err(), broadcast::error::TryRecvError::Lagged(_)));
    }

    #[tokio::test]
    async fn test_config_handle_writeback_happy_path() {
        let mut config = AppConfig::default();
        config.servers.insert(ServerId::from("s1"), ServerConfig::default());
        let source = Arc::new(InMemoryConfigSource::new(config.clone()));
        let handle = ConfigHandle::load(source).unwrap();
        let mut rx = handle.subscribe();

        let mutation = ConfigMutation::SetLoraAdapters {
            server_id: ServerId::from("s1"),
            adapters: vec![crate::types::LoraAdapterConfig {
                path: std::path::PathBuf::from("lora.gguf"),
                scale: 1.0,
            }],
        };

        let diff = handle.writeback(mutation).await.unwrap();
        assert_eq!(diff.changes.len(), 1);

        match &diff.changes[0] {
            ConfigChange::ServerChanged { id, diff } => {
                assert_eq!(id.0, "s1");
                assert!(diff.hot_reloadable());
            }
            _ => panic!("Expected ServerChanged"),
        }

        let snap = handle.snapshot();
        assert_eq!(snap.servers.get(&ServerId::from("s1")).unwrap().lora_adapters.len(), 1);

        let broadcast_diff = rx.recv().await.unwrap();
        assert_eq!(broadcast_diff.changes.len(), 1);
    }

    #[tokio::test]
    async fn test_config_handle_writeback_invalid_server_id() {
        let config = AppConfig::default();
        let source = Arc::new(InMemoryConfigSource::new(config.clone()));
        let handle = ConfigHandle::load(source).unwrap();
        let mut rx = handle.subscribe();

        let mutation = ConfigMutation::SetLoraAdapters {
            server_id: ServerId::from("invalid"),
            adapters: vec![],
        };

        let err = handle.writeback(mutation).await.unwrap_err();
        assert!(matches!(err, WritebackError::ServerNotFound));

        let snap = handle.snapshot();
        assert_eq!(**snap, config); // Unchanged

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_config_handle_writeback_reload_serialized() {
        // Implement a source with a slow reload
        let config = AppConfig::default();
        struct SlowSource {
            config: AppConfig,
        }
        impl crate::source::private::Sealed for SlowSource {}
        impl ConfigSource for SlowSource {
            fn load(&self) -> Result<AppConfig, ConfigLoadError> {
                std::thread::sleep(std::time::Duration::from_millis(100));
                Ok(self.config.clone())
            }
            fn persist(&self, _: &AppConfig, _: &ConfigMutation) -> Result<(), WritebackError> {
                Ok(())
            }
            fn export(&self, _: &AppConfig) -> Result<String, WritebackError> {
                Ok("".into())
            }
        }

        let source = Arc::new(SlowSource { config });
        let handle = Arc::new(ConfigHandle::load(source).unwrap());

        let h1 = handle.clone();
        let task1 = tokio::spawn(async move {
            h1.reload().await.unwrap();
        });

        // Let task1 start
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let h2 = handle.clone();
        let task2 = tokio::spawn(async move {
            let mutation = ConfigMutation::SetLoraAdapters {
                server_id: ServerId::from("invalid"), // will fail fast if not serialized
                adapters: vec![],
            };
            let start = std::time::Instant::now();
            let _ = h2.writeback(mutation).await;
            assert!(start.elapsed().as_millis() >= 80, "Writeback didn't wait for reload");
        });

        let _ = tokio::join!(task1, task2);
    }
}
