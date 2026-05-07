//! Config persistence abstraction and implementations.
//!
//! See design doc Section 2.1 for the port trait definition.

pub mod toml_file;

use crate::error::{ConfigLoadError, WritebackError};
use crate::types::{AppConfig, ConfigMutation};
use parking_lot::Mutex;

/// Port abstraction for config persistence — separates domain logic from I/O.
/// See design doc Section 2.1.
pub trait ConfigSource: Send + Sync {
    /// Load the full config from the backing store.
    fn load(&self) -> Result<AppConfig, ConfigLoadError>;

    /// Persist a mutation to the backing store (crash-safe).
    fn persist(&self, config: &AppConfig, mutation: &ConfigMutation) -> Result<(), WritebackError>;

    /// Export the config as a serialized string (TOML for file-backed).
    fn export(&self, config: &AppConfig) -> Result<String, WritebackError>;
}

/// In-memory implementation of ConfigSource for unit testing.
/// Holds a Mutex-protected AppConfig.
pub struct InMemoryConfigSource {
    config: Mutex<AppConfig>,
}

impl InMemoryConfigSource {
    /// Create a new InMemoryConfigSource with the given config.
    pub fn new(config: AppConfig) -> Self {
        Self { config: Mutex::new(config) }
    }
}

impl ConfigSource for InMemoryConfigSource {
    fn load(&self) -> Result<AppConfig, ConfigLoadError> {
        Ok(self.config.lock().clone())
    }

    fn persist(&self, config: &AppConfig, mutation: &ConfigMutation) -> Result<(), WritebackError> {
        let mut inner = self.config.lock();
        let mut new_config = config.clone();

        // Basic mutation application for testing.
        // Real implementation in PR5 handles the full SetLoraAdapters logic.
        match mutation {
            ConfigMutation::SetLoraAdapters { server_id, adapters } => {
                if let Some(server) = new_config.servers.get_mut(server_id) {
                    server.lora_adapters = adapters.clone();
                } else {
                    // For InMemory we just ignore if not found.
                }
            }
        }

        *inner = new_config;
        Ok(())
    }

    fn export(&self, config: &AppConfig) -> Result<String, WritebackError> {
        toml::to_string(config).map_err(|e| WritebackError::SerializeError { source: Box::new(e) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ServerId;

    #[test]
    fn test_in_memory_source_load_persist() {
        let config = AppConfig::default();
        let source = InMemoryConfigSource::new(config.clone());

        let loaded = source.load().unwrap();
        assert_eq!(loaded, config);

        let mutation =
            ConfigMutation::SetLoraAdapters { server_id: ServerId::from("s1"), adapters: vec![] };

        source.persist(&config, &mutation).unwrap();
        let updated = source.load().unwrap();
        assert_eq!(updated, config); // unchanged because s1 doesn't exist
    }
}
