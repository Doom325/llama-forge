//! Configuration interface traits.
//!
//! See design doc Section 2.2 for the port definitions.

use crate::config::diff::ConfigDiff;
use crate::config::error::{ConfigReloadError, WritebackError};
use crate::config::types::{AppConfig, ConfigMutation};
use arc_swap::Guard;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Read-only access to the configuration.
pub trait ConfigReader: Send + Sync + private::Sealed {
    /// Get a zero-cost snapshot of the current configuration.
    ///
    /// Implementation contract: returns a Guard from `arc_swap::ArcSwap::load`.
    fn snapshot(&self) -> Guard<Arc<AppConfig>>;

    /// Subscribe to configuration changes.
    fn subscribe(&self) -> broadcast::Receiver<Arc<ConfigDiff>>;

    /// Access the underlying ArcSwap for cached readers.
    fn arc_swap(&self) -> Arc<arc_swap::ArcSwap<AppConfig>>;
}

/// Write access for reloading or mutating configuration.
#[async_trait::async_trait]
pub trait ConfigWriter: Send + Sync + private::Sealed {
    /// Reload configuration from the backing store.
    async fn reload(&self) -> Result<ConfigDiff, ConfigReloadError>;

    /// Apply and persist a runtime mutation.
    async fn writeback(&self, mutation: ConfigMutation) -> Result<ConfigDiff, WritebackError>;
}

pub(crate) mod private {
    pub trait Sealed {}
}

// Ensure trait remains object-safe (can be used as Box<dyn ConfigReader>)
// so that tests or dynamic configurations can inject mock implementations,
// and so production features like prune.rs and cached.rs can utilize trait objects.
const _: Option<&dyn ConfigReader> = None;
