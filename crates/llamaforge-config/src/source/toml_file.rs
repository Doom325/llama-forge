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
        if !config.priority_rules.fair_share.weights.is_empty() {
            let sum: u32 = config.priority_rules.fair_share.weights.values().sum();
            if sum != 100 && sum > 0 {
                tracing::warn!("fair_share weights sum to {}, auto-normalizing to 100", sum);
            }
        }

        // 2-5, 8. Server rules
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
        }

        // Rule 7: queue_default_timeout_secs > 0
        if config.global.queue_default_timeout_secs == 0 {
            return Err(ConfigLoadError::ValidationError {
                message: "queue_default_timeout_secs must be > 0".to_string(),
            });
        }

        Ok(())
    }

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
                    .ok_or_else(|| WritebackError::SerializeError {
                        source: format!("server '{}' not found in config", server_id).into(),
                    })?;

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
        if self.paths.priority_rules.exists() {
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
        let lock_file = File::create(&self.paths.lock_file)
            .map_err(|e| WritebackError::WriteFailed { source: e })?;
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
        let temp_path = self.paths.state_dir.join("config.tmp");
        let mut temp_file =
            File::create(&temp_path).map_err(|e| WritebackError::WriteFailed { source: e })?;

        temp_file
            .write_all(doc.to_string().as_bytes())
            .map_err(|e| WritebackError::WriteFailed { source: e })?;
        temp_file.sync_all().map_err(|e| WritebackError::WriteFailed { source: e })?;

        match fs::rename(&temp_path, &self.paths.main_config) {
            Ok(()) => Ok(()),
            Err(e)
                if e.kind() == std::io::ErrorKind::PermissionDenied
                    || e.raw_os_error() == Some(nix::libc::EXDEV) =>
            {
                // Phase 2: Fallback to truncation
                tracing::warn!("atomic rename failed ({:?}), falling back to truncation", e);
                let mut f = File::create(&self.paths.main_config)
                    .map_err(|e| WritebackError::WriteFailed { source: e })?;
                f.write_all(doc.to_string().as_bytes())
                    .map_err(|e| WritebackError::WriteFailed { source: e })?;
                f.sync_all().map_err(|e| WritebackError::WriteFailed { source: e })?;
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
    use crate::types::{LoraAdapterConfig, ServerId};
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
    fn test_toml_file_source_validation() {
        let dir = tempdir().unwrap();
        let main_path = dir.path().join("llamaforge.toml");
        let paths =
            ConfigPaths::new(main_path.clone(), PathBuf::from(""), dir.path().to_path_buf());

        fs::write(
            &main_path,
            r#"
[servers.s1]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
[servers.s2]
model_path = "m2.gguf"
port = 8080
ctx_size = 2048
"#,
        )
        .unwrap();

        let source = TomlFileConfigSource::new(paths);
        let result = source.load();
        assert!(result.is_err());
        match result.unwrap_err() {
            ConfigLoadError::ValidationError { message } => {
                assert!(message.contains("duplicate server port"))
            }
            _ => panic!("expected ValidationError"),
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

        let paths = ConfigPaths::new(main_path.clone(), PathBuf::from(""), state_dir);
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
}
