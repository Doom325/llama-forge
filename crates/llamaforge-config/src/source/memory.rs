//! In-memory implementation of ConfigSource for unit testing.
//!
//! See design doc Section 2.1.

use crate::error::{ConfigLoadError, WritebackError};
use crate::source::{private, ConfigSource};
use crate::types::{AppConfig, ConfigMutation};
use parking_lot::Mutex;

/// In-memory implementation of ConfigSource for internal testing.
///
/// R17/R20: This is pub(crate) to keep the stable API focused on production sources.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct InMemoryConfigSource {
    config: Mutex<AppConfig>,
}

impl InMemoryConfigSource {
    /// Create a new InMemoryConfigSource with the given config.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(config: AppConfig) -> Self {
        Self { config: Mutex::new(config) }
    }

    #[cfg(test)]
    pub fn replace_config(&self, new_config: AppConfig) {
        *self.config.lock() = new_config;
    }
}
impl private::Sealed for InMemoryConfigSource {}

impl ConfigSource for InMemoryConfigSource {
    fn load(&self) -> Result<AppConfig, ConfigLoadError> {
        Ok(self.config.lock().clone())
    }
    fn persist(
        &self,
        config: &AppConfig,
        mutation: &ConfigMutation,
    ) -> Result<crate::source::WriteStrategy, WritebackError> {
        let mut inner = self.config.lock();
        let mut new_config = config.clone();

        if mutation.apply(&mut new_config) == crate::types::MutationOutcome::TargetMissing {
            return Err(WritebackError::ServerNotFound);
        }

        *inner = new_config;
        Ok(crate::source::WriteStrategy::Memory)
    }
    fn export(&self, config: &AppConfig) -> Result<String, WritebackError> {
        toml::to_string(config).map_err(|e| WritebackError::SerializeError { source: Box::new(e) })
    }
}
