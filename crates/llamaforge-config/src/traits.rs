//! Configuration interface traits.
//!
//! See design doc Section 2.2 for the port definitions.

use crate::diff::ConfigDiff;
use crate::error::{ConfigReloadError, WritebackError};
use crate::types::{AppConfig, ConfigMutation};
use arc_swap::Guard;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Read-only access to the configuration.
pub trait ConfigReader: Send + Sync {
    /// Get a zero-cost snapshot of the current configuration.
    fn snapshot(&self) -> Guard<Arc<AppConfig>>;

    /// Subscribe to configuration changes.
    fn subscribe(&self) -> broadcast::Receiver<Arc<ConfigDiff>>;
}

/// Write access for reloading or mutating configuration.
#[async_trait::async_trait]
pub trait ConfigWriter: Send + Sync {
    /// Reload configuration from the backing store.
    async fn reload(&self) -> Result<ConfigDiff, ConfigReloadError>;

    /// Apply and persist a runtime mutation.
    async fn writeback(&self, mutation: ConfigMutation) -> Result<(), WritebackError>;
}
