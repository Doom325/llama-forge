//! ConfigHandle core logic.
//!
//! See design doc Section 2.3.

use crate::config::diff::{compute_diff, ConfigDiff};
use crate::config::error::{ConfigLoadError, ConfigReloadError, WritebackError};
use crate::config::source::ConfigSource;
use crate::config::traits::{ConfigReader, ConfigWriter};
use crate::config::types::{AppConfig, ConfigMutation};
use arc_swap::ArcSwap;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Maximum length of a `[servers.<id>]` slug, in bytes.
/// Bounded so derived UUIDs, log lines, and Prometheus labels stay sane.
pub const SERVER_ID_MAX_LEN: usize = 64;

/// Validate a `[servers.<id>]` slug: non-empty, ≤ `SERVER_ID_MAX_LEN`,
/// must start with `[A-Za-z0-9]`, body in `[A-Za-z0-9._-]`. UUIDs satisfy
/// this rule unchanged.
///
/// Exposed so the management API (HTTP / gRPC) can reject malformed
/// `server_id` path/body parameters with the same rule that gated load.
pub fn validate_server_id_slug(id: &str) -> Result<(), ConfigLoadError> {
    if id.is_empty() {
        return Err(ConfigLoadError::ValidationError { message: "empty ServerId".to_string() });
    }
    if id.len() > SERVER_ID_MAX_LEN {
        return Err(ConfigLoadError::ValidationError {
            message: format!("ServerId '{}' exceeds {}-byte limit", id, SERVER_ID_MAX_LEN),
        });
    }
    let mut chars = id.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return Err(ConfigLoadError::ValidationError {
            message: format!("ServerId '{}' must start with an ASCII letter or digit", id),
        });
    }
    for c in chars {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
        if !ok {
            return Err(ConfigLoadError::ValidationError {
                message: format!(
                    "ServerId '{}' contains invalid character '{}' (allowed: A-Z a-z 0-9 . _ -)",
                    id, c
                ),
            });
        }
    }
    Ok(())
}

/// Central coordinator for configuration state and reloads.
/// See design doc Section 2.3.
pub struct ConfigHandle {
    source: Arc<dyn ConfigSource>,
    inner: Arc<ArcSwap<AppConfig>>,
    reload_mutex: tokio::sync::Mutex<()>,
    change_tx: broadcast::Sender<Arc<ConfigDiff>>,
}

impl ConfigHandle {
    /// Load the initial configuration and construct the handle.
    pub fn load(source: Arc<dyn ConfigSource>) -> Result<Self, ConfigLoadError> {
        // Slug validation runs in the `servers` deserializer (see
        // `types::deserialize_servers_map`); by the time `source.load()`
        // returns, every key has already been gated.
        let config = source.load()?;

        let (change_tx, _) = broadcast::channel(16);

        Ok(Self {
            source,
            inner: Arc::new(ArcSwap::from_pointee(config)),
            reload_mutex: tokio::sync::Mutex::new(()),
            change_tx,
        })
    }

    /// Construct a handle seeded with `initial` instead of loading from
    /// `source`. Used by first-run / setup mode (08_SETUP_WIZARD.md Section
    /// 13.4), where there is no config file yet: the orchestrator boots with an
    /// empty config and later [`reload`](Self::reload)s from `source` once the
    /// wizard has written the file.
    pub fn with_initial_config(initial: AppConfig, source: Arc<dyn ConfigSource>) -> Self {
        let (change_tx, _) = broadcast::channel(16);
        Self {
            source,
            inner: Arc::new(ArcSwap::from_pointee(initial)),
            reload_mutex: tokio::sync::Mutex::new(()),
            change_tx,
        }
    }

    /// Export the current configuration as a TOML string.
    pub fn export_toml(&self) -> Result<String, WritebackError> {
        let snapshot = self.inner.load();
        self.source.export(&snapshot)
    }

    /// Access the underlying ArcSwap for cached readers.
    pub(crate) fn arc_swap(&self) -> Arc<ArcSwap<AppConfig>> {
        self.inner.clone()
    }
}

impl crate::config::traits::private::Sealed for ConfigHandle {}

impl ConfigReader for ConfigHandle {
    fn snapshot(&self) -> Arc<AppConfig> {
        self.inner.load().clone()
    }

    fn subscribe(&self) -> broadcast::Receiver<Arc<ConfigDiff>> {
        self.change_tx.subscribe()
    }
}

#[async_trait::async_trait]
impl ConfigWriter for ConfigHandle {
    async fn reload(&self) -> Result<ConfigDiff, ConfigReloadError> {
        let _guard = self.reload_mutex.lock().await;

        let source = self.source.clone();
        // Offload source loading to a blocking task to prevent starvation of the async executor,
        // as file I/O operations will block the current thread.
        let new_config = tokio::task::spawn_blocking(move || source.load())
            .await
            .map_err(|e| ConfigReloadError::LoadTaskJoin { message: e.to_string() })??;

        let old_config = self.inner.load();
        let diff = compute_diff(&old_config, &new_config);

        if old_config.global.metrics_db_path != new_config.global.metrics_db_path {
            return Err(ConfigReloadError::GlobalRestartRequired {
                message: "metrics_db_path changed".to_string(),
            });
        }
        if old_config.global.telemetry_backend != new_config.global.telemetry_backend {
            return Err(ConfigReloadError::GlobalRestartRequired {
                message: "telemetry_backend changed".to_string(),
            });
        }

        // Pre-swap gate: if the on-disk file changed any field whose
        // semantics require a server restart, abort here. v1.0 doesn't
        // ship a staged-apply protocol, so partially applying (or
        // even swapping the snapshot without applying) would leave
        // operators with a half-consistent fleet. Return an error
        // *before* the swap + broadcast so callers see no side
        // effects — the on-disk file is the new truth, the running
        // orchestrator keeps the old config until restart.
        let restart_required: Vec<crate::config::diff::ServerConfigField> = diff
            .servers_changed()
            .flat_map(|(_, d)| d.fields_changed.iter().copied())
            .filter(|f| f.requires_restart())
            .collect();
        if !restart_required.is_empty() {
            return Err(ConfigReloadError::RestartRequired { fields: restart_required });
        }

        if !diff.changes.is_empty() {
            self.inner.store(Arc::new(new_config));
        }
        // Design choice: Broadcast an empty diff even if no changes occurred.
        // This explicitly signals to subscribers that a requested reload cycle has completed,
        // allowing them to unblock or verify synchronization state.
        let _ = self.change_tx.send(Arc::new(diff.clone()));

        Ok(diff)
    }

    async fn writeback(&self, mutation: ConfigMutation) -> Result<ConfigDiff, WritebackError> {
        let _guard = self.reload_mutex.lock().await;

        let current_guard = self.inner.load();
        let mut new_config = (**current_guard).clone();

        match mutation.apply(&mut new_config) {
            crate::config::types::MutationOutcome::TargetMissing => {
                return Err(WritebackError::ServerNotFound)
            }
            crate::config::types::MutationOutcome::Unchanged => {
                let diff = ConfigDiff::new(vec![]);
                let _ = self.change_tx.send(Arc::new(diff.clone()));
                return Ok(diff);
            }
            crate::config::types::MutationOutcome::Applied => {}
        }

        // TODO(PR5/Design §3.2): In the full protocol, we should first attempt to apply
        // the hot-reload via the `llama-server` API before doing this disk writeback.
        // For now, this just persists the configuration.

        let source = self.source.clone();
        let current_config_arc = current_guard.clone();
        let mutation_clone = mutation.clone();

        let parent_span = tracing::Span::current();
        tokio::task::spawn_blocking(move || {
            parent_span.in_scope(|| source.persist(&current_config_arc, &mutation_clone))
        })
        .await
        .map_err(|e| WritebackError::PersistTaskJoin { message: e.to_string() })??;

        let diff = compute_diff(&current_guard, &new_config);

        if !diff.changes.is_empty() {
            self.inner.store(Arc::new(new_config));
        }
        let _ = self.change_tx.send(Arc::new(diff.clone()));

        Ok(diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::diff::ConfigChange;
    use crate::config::source::memory::InMemoryConfigSource;
    use crate::config::types::{ServerConfig, ServerId};

    /// Round-trip a candidate slug through TOML deserialization so the
    /// `validate_server_id_slug` rule in `deserialize_servers_map` is
    /// exercised. Returns the inner `ConfigLoadError::ValidationError`
    /// (re-wrapped through `toml::de::Error`) on rejection.
    fn make_handle_with_server_id(id: &str) -> Result<ConfigHandle, ConfigLoadError> {
        // Build TOML by hand so the [servers.<slug>] table carries the
        // raw, un-escaped key. `toml::Value::to_string` quotes the key
        // and would mask shape issues we want the validator to catch.
        let toml_str = format!(
            "[servers.\"{}\"]\nbinary_path = \"/usr/local/bin/llama-server\"\nmodel_path = \"/tmp/model.gguf\"\nport = 8080\nctx_size = 1024\nparallel_slots = 1\n",
            id.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let config: AppConfig = toml::from_str(&toml_str)
            .map_err(|e| ConfigLoadError::ValidationError { message: e.to_string() })?;
        ConfigHandle::load(Arc::new(InMemoryConfigSource::new(config)))
    }

    #[test]
    fn slug_server_ids_are_accepted() {
        for ok in &[
            "gemma3-27b",
            "qwen-coder-7b",
            "phi.mini",
            "server_01",
            "Server-A",
            "a",
            "00000000-0000-0000-0000-000000000001",
        ] {
            let res = make_handle_with_server_id(ok);
            assert!(res.is_ok(), "expected slug {:?} to be accepted", ok);
        }
    }

    #[test]
    fn invalid_server_ids_are_rejected() {
        for bad in &["", "-leading-hyphen", ".leading-dot", "has space", "has/slash", "emoji😀"] {
            match make_handle_with_server_id(bad) {
                Err(ConfigLoadError::ValidationError { .. }) => {}
                Err(other) => panic!("expected ValidationError for {:?}, got {:?}", bad, other),
                Ok(_) => panic!("expected slug {:?} to be rejected", bad),
            }
        }
        // 65-byte slug exceeds SERVER_ID_MAX_LEN.
        let long = "a".repeat(SERVER_ID_MAX_LEN + 1);
        assert!(matches!(
            make_handle_with_server_id(&long),
            Err(ConfigLoadError::ValidationError { .. })
        ));
    }

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

        assert_eq!(*handle.snapshot(), config);
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
            ServerId::from_config_id("new_server"),
            ServerConfig { port: 8080, ..Default::default() },
        );
        // Replace inner directly for the test
        source.replace_config(new_config.clone());

        let diff = handle.reload().await.unwrap();
        assert_eq!(diff.changes.len(), 1);
        assert!(
            matches!(&diff.changes[0], ConfigChange::ServerAdded { id } if *id == ServerId::from_config_id("new_server"))
        );

        // Verify snapshot updated
        assert_eq!(*handle.snapshot(), new_config);

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
        impl crate::config::source::private::Sealed for FailOnSecondSource {}
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
        assert_eq!(*handle.snapshot(), AppConfig::default());

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

        assert_eq!(*handle.snapshot(), AppConfig::default());
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
        config.servers.insert(
            ServerId::from_config_id("00000000-0000-0000-0000-000000000001"),
            ServerConfig::default(),
        );
        let source = Arc::new(InMemoryConfigSource::new(config.clone()));
        let handle = ConfigHandle::load(source).unwrap();
        let mut rx = handle.subscribe();

        let mutation = ConfigMutation::SetLoraAdapters {
            server_id: ServerId::from_config_id("00000000-0000-0000-0000-000000000001"),
            adapters: vec![crate::config::types::LoraAdapterConfig {
                path: std::path::PathBuf::from("lora.gguf"),
                scale: 1.0,
            }],
        };

        let diff = handle.writeback(mutation).await.unwrap();
        assert_eq!(diff.changes.len(), 1);

        match &diff.changes[0] {
            ConfigChange::ServerChanged { id, diff } => {
                assert_eq!(*id, ServerId::from_config_id("00000000-0000-0000-0000-000000000001"));
                assert!(diff.hot_reloadable());
            }
            _ => panic!("Expected ServerChanged"),
        }

        let snap = handle.snapshot();
        assert_eq!(
            snap.servers
                .get(&ServerId::from_config_id("00000000-0000-0000-0000-000000000001"))
                .unwrap()
                .lora_adapters
                .len(),
            1
        );

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
            server_id: ServerId::from_config_id("invalid"),
            adapters: vec![],
        };

        let err = handle.writeback(mutation).await.unwrap_err();
        assert!(matches!(err, WritebackError::ServerNotFound));

        let snap = handle.snapshot();
        assert_eq!(*snap, config); // Unchanged

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_config_handle_writeback_reload_serialized() {
        // Implement a source with a slow reload
        let config = AppConfig::default();
        struct SlowSource {
            config: AppConfig,
        }
        impl crate::config::source::private::Sealed for SlowSource {}
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
                server_id: ServerId::from_config_id("invalid"), // will fail fast if not serialized
                adapters: vec![],
            };
            let start = std::time::Instant::now();
            let _ = h2.writeback(mutation).await;
            assert!(start.elapsed().as_millis() >= 80, "Writeback didn't wait for reload");
        });

        let _ = tokio::join!(task1, task2);
    }
}
