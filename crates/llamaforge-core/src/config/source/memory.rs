//! In-memory implementation of ConfigSource for unit testing.
//!
//! See design doc Section 2.1.

use crate::config::error::{ConfigLoadError, WritebackError};
use crate::config::source::ConfigSource;
use crate::config::types::{AppConfig, ConfigMutation};
use parking_lot::Mutex;

/// In-memory implementation of ConfigSource for internal testing.
///
/// R17/R20: This is pub(crate) to keep the stable API focused on production sources.
#[cfg_attr(not(any(test, feature = "test-utils")), allow(dead_code))]
pub struct InMemoryConfigSource {
    pub(crate) config: Mutex<AppConfig>,
}

impl InMemoryConfigSource {
    /// Create a new InMemoryConfigSource with the given config.
    #[cfg_attr(not(any(test, feature = "test-utils")), allow(dead_code))]
    pub fn new(config: AppConfig) -> Self {
        Self { config: Mutex::new(config) }
    }

    #[cfg(any(test, feature = "test-utils"))]
    pub fn replace_config(&self, new_config: AppConfig) {
        *self.config.lock() = new_config;
    }
}

impl ConfigSource for InMemoryConfigSource {
    fn load(&self) -> Result<AppConfig, ConfigLoadError> {
        Ok(self.config.lock().clone())
    }

    fn persist(&self, config: &AppConfig, mutation: &ConfigMutation) -> Result<(), WritebackError> {
        let mut inner = self.config.lock();
        let mut new_config = config.clone();

        if mutation.apply(&mut new_config) == crate::config::types::MutationOutcome::TargetMissing {
            return Err(WritebackError::ServerNotFound);
        }

        *inner = new_config;
        tracing::Span::current().record("config.write_strategy", "Memory");
        Ok(())
    }

    fn export(&self, config: &AppConfig) -> Result<String, WritebackError> {
        toml::to_string(config).map_err(|e| WritebackError::SerializeError { source: Box::new(e) })
    }
}
