//! Production implementation of ConfigSource backed by TOML files on disk.
//!
//! See design doc Section 2.1 and 3.2 for the implementation strategy.

use crate::error::{ConfigLoadError, WritebackError};
use crate::source::ConfigSource;
use crate::types::{AppConfig, ConfigMutation, ConfigPaths, PriorityRulesConfig};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};
use toml_edit::DocumentMut;

/// Filesystem-backed configuration source.
/// Handles merging llamaforge.toml and priority_rules.toml.
pub struct TomlFileConfigSource {
    paths: ConfigPaths,
}

impl TomlFileConfigSource {
    /// Create a new TomlFileConfigSource with the given paths.
    pub fn new(paths: ConfigPaths) -> Self {
        Self { paths }
    }

    fn validate(config: &mut AppConfig) -> Result<(), ConfigLoadError> {
        let mut ports = HashSet::new();

        // 1. Fair share weights normalization (Soft validation)
        // See design doc Section 6.1.
        if !config.priority_rules.fair_share.weights.is_empty() {
            let sum: u32 = config.priority_rules.fair_share.weights.values().sum();
            if sum != 100 && sum > 0 {
                // R24: Structured logging
                tracing::warn!(sum, "fair_share weights do not sum to 100, auto-normalizing");

                // Proportional normalization
                for weight in config.priority_rules.fair_share.weights.values_mut() {
                    *weight = ((*weight as f64 / sum as f64) * 100.0).round() as u32;
                }

                // Fix rounding errors to ensure exact 100 sum
                let new_sum: u32 = config.priority_rules.fair_share.weights.values().sum();
                if new_sum != 100 && !config.priority_rules.fair_share.weights.is_empty() {
                    // Find the key with the maximum weight
                    if let Some(max_id) = config
                        .priority_rules
                        .fair_share
                        .weights
                        .iter()
                        .max_by_key(|&(_, w)| *w)
                        .map(|(k, _)| k.clone())
                    {
                        if let Some(w) = config.priority_rules.fair_share.weights.get_mut(&max_id) {
                            if new_sum < 100 {
                                *w += 100 - new_sum;
                            } else {
                                *w = w.saturating_sub(new_sum - 100);
                            }
                        }
                    }
                }
            }
        }

        // 2-6, 8. Server rules
        // Rule 8: No duplicate server IDs is structurally enforced by HashMap during parsing,
        // but we ensure all servers pass validation.
        for (id, server) in &config.servers {
            // Rule 2: Unique server ports
            if !ports.insert(server.port) {
                return Err(ConfigLoadError::ValidationError {
                    message: format!("duplicate server port: {}", server.port),
                });
            }

            // Rule 3: model_path non-empty
            if server.model_path.as_os_str().is_empty() {
                return Err(ConfigLoadError::ValidationError {
                    message: format!("empty model_path for server '{}'", id),
                });
            }

            // Rule 4: ctx_size > 0
            if server.ctx_size == 0 {
                return Err(ConfigLoadError::ValidationError {
                    message: format!("ctx_size must be > 0 for server '{}'", id),
                });
            }

            // Rule 5: parallel_slots > 0
            if server.parallel_slots == 0 {
                return Err(ConfigLoadError::ValidationError {
                    message: format!("parallel_slots must be > 0 for server '{}'", id),
                });
            }

            // Rule 6: Device specs reference valid GPU IDs (Structural check)
            let mut gpus = HashSet::new();
            for dev in &server.devices {
                if !gpus.insert(dev.gpu_id.0) {
                    return Err(ConfigLoadError::ValidationError {
                        message: format!("duplicate gpu_id {} for server '{}'", dev.gpu_id.0, id),
                    });
                }
            }
        }

        // Rule 7: queue_default_timeout_secs > 0
        if config.global.queue_default_timeout_secs == 0 {
            return Err(ConfigLoadError::ValidationError {
                message: "queue_default_timeout_secs must be > 0".to_string(),
            });
        }

        Ok(())
    }

    /// Acquires an exclusive flock with a retry loop.
    ///
    /// # Rationale for Raw FD & Deprecated flock:
    /// The newer `nix::fcntl::Flock` RAII guard consumes ownership of the `File` object,
    /// which complicates our retry loop and subsequent use of the file for read/write.
    /// Since this is a specialized persistence component with explicit lifecycle control,
    /// raw FD locking provides the necessary flexibility for the 30s timeout pattern
    /// defined in design doc Section 3.2.
    #[allow(deprecated)]
    fn acquire_flock_with_timeout(fd: &File, timeout: Duration) -> Result<(), WritebackError> {
        let deadline = Instant::now() + timeout;
        loop {
            match nix::fcntl::flock(fd.as_raw_fd(), nix::fcntl::FlockArg::LockExclusiveNonblock) {
                Ok(()) => return Ok(()),
                Err(nix::errno::Errno::EWOULDBLOCK) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(nix::errno::Errno::EWOULDBLOCK) => {
                    return Err(WritebackError::LockTimeout { elapsed: timeout });
                }
                Err(e) => {
                    return Err(WritebackError::WriteFailed { source: std::io::Error::from(e) })
                }
            }
        }
    }

    fn apply_mutation_to_doc(
        doc: &mut DocumentMut,
        mutation: &ConfigMutation,
    ) -> Result<(), WritebackError> {
        match mutation {
            ConfigMutation::SetLoraAdapters { server_id, adapters } => {
                let servers = doc
                    .get_mut("servers")
                    .and_then(|v| v.as_table_like_mut())
                    .ok_or_else(|| WritebackError::SerializeError {
                        source: "missing [servers] table".into(),
                    })?;

                let server = servers
                    .get_mut(&server_id.0)
                    .and_then(|v| v.as_table_like_mut())
                    .ok_or_else(|| WritebackError::ServerNotFound)?;

                let mut lora_array = toml_edit::Array::new();
                for adapter in adapters {
                    let mut item = toml_edit::InlineTable::new();
                    item.insert(
                        "path",
                        toml_edit::Value::from(adapter.path.to_string_lossy().to_string()),
                    );
                    item.insert("scale", toml_edit::Value::from(adapter.scale as f64));
                    lora_array.push(item);
                }
                server.insert("lora_adapters", toml_edit::value(lora_array));
            }
        }
        Ok(())
    }
}

impl ConfigSource for TomlFileConfigSource {
    fn load(&self) -> Result<AppConfig, ConfigLoadError> {
        let main_path = self.paths.main_config.clone();
        let main_content = fs::read_to_string(&main_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ConfigLoadError::NotFound { path: main_path }
            } else {
                ConfigLoadError::IoError { path: main_path, source: e }
            }
        })?;

        if main_content.trim().is_empty() {
            return Err(ConfigLoadError::EmptyFile { path: self.paths.main_config.clone() });
        }

        let mut config: AppConfig =
            toml::from_str(&main_content).map_err(|e| ConfigLoadError::ParseError {
                path: self.paths.main_config.clone(),
                source: Box::new(e),
            })?;

        // Merge priority rules if the file exists
        if self.paths.priority_rules.exists() && self.paths.priority_rules.as_os_str() != "" {
            let rules_content = fs::read_to_string(&self.paths.priority_rules).map_err(|e| {
                ConfigLoadError::IoError { path: self.paths.priority_rules.clone(), source: e }
            })?;

            if !rules_content.trim().is_empty() {
                let rules: PriorityRulesConfig =
                    toml::from_str(&rules_content).map_err(|e| ConfigLoadError::ParseError {
                        path: self.paths.priority_rules.clone(),
                        source: Box::new(e),
                    })?;

                config.priority_rules = rules;
            }
        }

        Self::validate(&mut config)?;

        Ok(config)
    }

    fn persist(
        &self,
        _config: &AppConfig,
        mutation: &ConfigMutation,
    ) -> Result<(), WritebackError> {
        // 1. Open and lock
        let lock_file = match File::create(&self.paths.lock_file) {
            Ok(f) => f,
            Err(e)
                if e.kind() == std::io::ErrorKind::PermissionDenied
                    || e.raw_os_error() == Some(nix::libc::EROFS) =>
            {
                return Err(WritebackError::ReadOnly { path: self.paths.lock_file.clone() });
            }
            Err(e) => return Err(WritebackError::WriteFailed { source: e }),
        };
        Self::acquire_flock_with_timeout(&lock_file, Duration::from_secs(30))?;

        // 2. Read existing main config to preserve comments
        let main_content = fs::read_to_string(&self.paths.main_config)
            .map_err(|e| WritebackError::WriteFailed { source: e })?;
        let mut doc: DocumentMut = main_content
            .parse()
            .map_err(|e| WritebackError::SerializeError { source: Box::new(e) })?;

        // 3. Apply mutation to doc
        Self::apply_mutation_to_doc(&mut doc, mutation)?;

        // 4. Atomic writeback (Phase 1: Temp file + Rename)
        // Write via temporary file and atomic rename to guarantee we never leave a partially
        // written config on disk in case of a crash or power loss.
        let temp_path = self.paths.state_dir.join("config.tmp");
        let mut temp_file = match File::create(&temp_path) {
            Ok(f) => f,
            Err(e)
                if e.kind() == std::io::ErrorKind::PermissionDenied
                    || e.raw_os_error() == Some(nix::libc::EROFS) =>
            {
                return Err(WritebackError::ReadOnly { path: temp_path.clone() });
            }
            Err(e) => return Err(WritebackError::WriteFailed { source: e }),
        };

        temp_file
            .write_all(doc.to_string().as_bytes())
            .map_err(|e| WritebackError::WriteFailed { source: e })?;
        temp_file.sync_all().map_err(|e| {
            tracing::warn!(error = ?e, "sync_all failed during writeback");
            WritebackError::WriteFailed { source: e }
        })?;

        match fs::rename(&temp_path, &self.paths.main_config) {
            Ok(()) => {
                tracing::Span::current().record("config.write_strategy", "AtomicRename");
                Ok(())
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::PermissionDenied
                    || e.raw_os_error() == Some(nix::libc::EROFS) =>
            {
                Err(WritebackError::ReadOnly { path: self.paths.main_config.clone() })
            }
            Err(e) if e.raw_os_error() == Some(nix::libc::EXDEV) => {
                // Phase 2: Fallback to truncation
                tracing::warn!(error = ?e, "atomic rename failed, falling back to truncation");

                let mut f = match File::create(&self.paths.main_config) {
                    Ok(file) => file,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::PermissionDenied
                            || e.raw_os_error() == Some(nix::libc::EROFS) =>
                    {
                        return Err(WritebackError::ReadOnly {
                            path: self.paths.main_config.clone(),
                        });
                    }
                    Err(e) => return Err(WritebackError::WriteFailed { source: e }),
                };
                f.write_all(doc.to_string().as_bytes())
                    .map_err(|e| WritebackError::WriteFailed { source: e })?;
                f.sync_all().map_err(|e| WritebackError::WriteFailed { source: e })?;
                tracing::Span::current().record("config.write_strategy", "TruncateFallback");
                Ok(())
            }
            Err(e) => Err(WritebackError::WriteFailed { source: e }),
        }
    }

    fn export(&self, config: &AppConfig) -> Result<String, WritebackError> {
        toml::to_string(config).map_err(|e| WritebackError::SerializeError { source: Box::new(e) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{LoraAdapterConfig, RequestorId, ServerId};
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_toml_file_source_load_and_merge() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("llamaforge.toml");
        let rules_path = dir.path().join("priority_rules.toml");
        let state_dir = dir.path().to_path_buf();

        fs::write(
            &main_path,
            r#"
[global]
http_port = 9000
[servers.s1]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
        )
        .unwrap();

        fs::write(
            &rules_path,
            r#"
[starvation]
threshold_secs = 60
"#,
        )
        .unwrap();

        let paths = ConfigPaths::new(main_path, rules_path, state_dir);
        let source = TomlFileConfigSource::new(paths);
        let config = source.load().unwrap();

        assert_eq!(config.global.http_port, 9000);
        assert_eq!(config.servers.get(&ServerId::from("s1")).unwrap().port, 8080);
        assert_eq!(config.priority_rules.starvation.threshold_secs, 60);
    }

    #[test]
    fn test_toml_file_source_load_errors() {
        let dir = tempdir().unwrap();
        let state_dir = dir.path().to_path_buf();

        // 1. NotFound
        let paths =
            ConfigPaths::new(dir.path().join("missing.toml"), PathBuf::new(), state_dir.clone());
        let source = TomlFileConfigSource::new(paths);
        assert!(matches!(source.load().unwrap_err(), ConfigLoadError::NotFound { .. }));

        // 2. EmptyFile
        let empty_path = dir.path().join("empty.toml");
        fs::write(&empty_path, "   \n").unwrap();
        let paths = ConfigPaths::new(empty_path, PathBuf::new(), state_dir.clone());
        let source = TomlFileConfigSource::new(paths);
        assert!(matches!(source.load().unwrap_err(), ConfigLoadError::EmptyFile { .. }));

        // 3. ParseError
        let junk_path = dir.path().join("junk.toml");
        fs::write(&junk_path, "this is not toml").unwrap();
        let paths = ConfigPaths::new(junk_path, PathBuf::new(), state_dir.clone());
        let source = TomlFileConfigSource::new(paths);
        assert!(matches!(source.load().unwrap_err(), ConfigLoadError::ParseError { .. }));
    }

    #[test]
    fn test_toml_file_source_validation_rules() {
        let dir = tempdir().unwrap();
        let state_dir = dir.path().to_path_buf();
        let main_path = dir.path().join("llamaforge.toml");

        let check_val = |content: &str, expected: &str| {
            fs::write(&main_path, content).unwrap();
            let paths = ConfigPaths::new(main_path.clone(), PathBuf::new(), state_dir.clone());
            let source = TomlFileConfigSource::new(paths);
            let err = source.load().unwrap_err();
            if let ConfigLoadError::ValidationError { message } = err {
                assert!(message.contains(expected), "expected '{}' in '{}'", expected, message);
            } else {
                panic!("expected ValidationError, got {:?}", err);
            }
        };

        // Rule 2: Duplicate port
        check_val("[servers.s1]\nmodel_path='m1'\nport=8080\nctx_size=1\n[servers.s2]\nmodel_path='m2'\nport=8080\nctx_size=1", "duplicate server port");

        // Rule 3: Empty model_path
        check_val("[servers.s1]\nmodel_path=''\nport=8080\nctx_size=1", "empty model_path");

        // Rule 4: ctx_size 0
        check_val("[servers.s1]\nmodel_path='m1'\nport=8080\nctx_size=0", "ctx_size must be > 0");

        // Rule 5: parallel_slots 0
        check_val(
            "[servers.s1]\nmodel_path='m1'\nport=8080\nctx_size=1\nparallel_slots=0",
            "parallel_slots must be > 0",
        );

        // Rule 6: Duplicate GPU ID
        check_val("[servers.s1]\nmodel_path='m1'\nport=8080\nctx_size=1\ndevices=[{gpu_id=0}, {gpu_id=0}]", "duplicate gpu_id 0");

        // Rule 7: queue_default_timeout_secs 0
        check_val(
            "[global]\nqueue_default_timeout_secs=0",
            "queue_default_timeout_secs must be > 0",
        );
    }

    #[test]
    fn test_fair_share_normalization() {
        let mut config = AppConfig::default();
        config.priority_rules.fair_share.weights.insert(RequestorId::from("a"), 10);
        config.priority_rules.fair_share.weights.insert(RequestorId::from("b"), 10);

        TomlFileConfigSource::validate(&mut config).unwrap();

        let sum: u32 = config.priority_rules.fair_share.weights.values().sum();
        assert_eq!(sum, 100);
        assert_eq!(
            *config.priority_rules.fair_share.weights.get(&RequestorId::from("a")).unwrap(),
            50
        );
        assert_eq!(
            *config.priority_rules.fair_share.weights.get(&RequestorId::from("b")).unwrap(),
            50
        );
    }

    #[test]
    fn test_fair_share_normalization_bug_101_with_zero() {
        let mut config = AppConfig::default();
        config.priority_rules.fair_share.weights.insert(RequestorId::from("a"), 3);
        config.priority_rules.fair_share.weights.insert(RequestorId::from("b"), 3);
        config.priority_rules.fair_share.weights.insert(RequestorId::from("c"), 2);
        config.priority_rules.fair_share.weights.insert(RequestorId::from("d"), 0);

        TomlFileConfigSource::validate(&mut config).unwrap();

        let sum: u32 = config.priority_rules.fair_share.weights.values().sum();
        assert_eq!(sum, 100);
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_fair_share_normalization_proptest(
            weights in prop::collection::hash_map("[a-z]", 0..1000u32, 1..20)
        ) {
            let mut config = AppConfig::default();
            for (k, v) in weights {
                config.priority_rules.fair_share.weights.insert(RequestorId::from(k), v);
            }

            let orig_sum: u32 = config.priority_rules.fair_share.weights.values().sum();

            TomlFileConfigSource::validate(&mut config).unwrap();

            let sum: u32 = config.priority_rules.fair_share.weights.values().sum();
            if orig_sum > 0 {
                prop_assert_eq!(sum, 100);
            } else {
                prop_assert_eq!(sum, 0);
            }
        }
    }

    #[test]
    fn test_toml_file_source_persist_lora() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("llamaforge.toml");
        let state_dir = dir.path().to_path_buf();

        fs::write(
            &main_path,
            r#"
# This is a comment we want to preserve
[servers.s1]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
        )
        .unwrap();

        let paths = ConfigPaths::new(main_path.clone(), PathBuf::new(), state_dir);
        let source = TomlFileConfigSource::new(paths);
        let config = source.load().unwrap();

        let mutation = ConfigMutation::SetLoraAdapters {
            server_id: ServerId::from("s1"),
            adapters: vec![LoraAdapterConfig { path: PathBuf::from("lora.gguf"), scale: 1.0 }],
        };

        source.persist(&config, &mutation).unwrap();

        let updated_content = fs::read_to_string(&main_path).unwrap();
        assert!(updated_content.contains("# This is a comment"));
        assert!(updated_content.contains("lora_adapters"));
        assert!(updated_content.contains("lora.gguf"));
        assert!(updated_content.contains("1.0"));

        let updated_config = source.load().unwrap();
        let s1 = updated_config.servers.get(&ServerId::from("s1")).unwrap();
        assert_eq!(s1.lora_adapters.len(), 1);
        assert_eq!(s1.lora_adapters[0].path, PathBuf::from("lora.gguf"));
    }

    #[tokio::test]
    async fn test_flock_contention_timeout() {
        let dir = tempdir().unwrap();
        let lock_path = dir.path().join("config.lock");

        // 1. Manually acquire an exclusive lock
        let f = File::create(&lock_path).unwrap();
        #[allow(deprecated)]
        nix::fcntl::flock(f.as_raw_fd(), nix::fcntl::FlockArg::LockExclusive).unwrap();
        // 2. Try to acquire it via acquire_flock_with_timeout in another thread
        let handle = std::thread::spawn(move || {
            let f2 = File::open(&lock_path).unwrap();
            TomlFileConfigSource::acquire_flock_with_timeout(&f2, Duration::from_millis(500))
        });

        let result = handle.join().unwrap();
        assert!(matches!(result.unwrap_err(), WritebackError::LockTimeout { .. }));
    }
}
