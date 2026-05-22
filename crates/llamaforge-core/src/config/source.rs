//! Config persistence abstraction and implementations.
//!
//! See design doc Section 2.1 for the port trait definition.

#[cfg(any(test, feature = "test-utils"))]
pub mod memory;
#[cfg(not(any(test, feature = "test-utils")))]
pub(crate) mod memory;

pub mod toml_file;

use crate::error::{ConfigLoadError, WritebackError};
use crate::types::{AppConfig, ConfigMutation};
/// Port abstraction for config persistence — separates domain logic from I/O.
///
/// See design doc Section 2.1.
///
/// R5: This trait is Sealed to ensure all implementations are maintained
/// within the crate boundaries for stable v1.0.
pub trait ConfigSource: Send + Sync + private::Sealed {
    /// Load the full config from the backing store.
    fn load(&self) -> Result<AppConfig, ConfigLoadError>;

    /// Persist a mutation to the backing store (crash-safe).
    fn persist(&self, config: &AppConfig, mutation: &ConfigMutation) -> Result<(), WritebackError>;

    /// Export the config as a serialized string (TOML for file-backed).
    fn export(&self, config: &AppConfig) -> Result<String, WritebackError>;
}
/// Sealed trait pattern to prevent external implementations.
pub(crate) mod private {
    pub trait Sealed {}
}

impl private::Sealed for memory::InMemoryConfigSource {}
impl private::Sealed for toml_file::TomlFileConfigSource {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::memory::InMemoryConfigSource;
    use crate::types::ServerId;
    #[test]
    fn test_in_memory_source_load_persist() {
        let mut config = AppConfig::default();
        config.servers.insert(
            crate::types::ServerId::from_config_id("s1"),
            crate::types::ServerConfig::default(),
        );
        let source = InMemoryConfigSource::new(config.clone());

        let loaded = source.load().unwrap();
        assert_eq!(loaded, config);

        let mutation = ConfigMutation::SetLoraAdapters {
            server_id: ServerId::from_config_id("s1"),
            adapters: vec![],
        };

        source.persist(&config, &mutation).unwrap();
        let updated = source.load().unwrap();
        assert_eq!(updated, config); // unchanged because s1 already had empty adapters
    }
}
