//! Thread-local cached reader for hot paths.
//!
//! See design doc Section 2.4.
use crate::traits::ConfigReader;
use crate::types::AppConfig;
use arc_swap::cache::Cache;
use arc_swap::ArcSwap;
use std::sync::Arc;

/// High-performance configuration reader for the hot path.
/// Uses a thread-local cache to avoid atomic load contention on the `ArcSwap`.
pub struct CachedConfigReader {
    cache: Cache<Arc<ArcSwap<AppConfig>>, Arc<AppConfig>>,
}

impl CachedConfigReader {
    /// Create a new CachedConfigReader from any ConfigReader.
    pub fn new(reader: &dyn ConfigReader) -> Self {
        Self { cache: Cache::new(reader.arc_swap()) }
    }
    /// Load the configuration from the local cache.
    /// Performs an atomic load only if the underlying ArcSwap has advanced.
    pub fn load(&mut self) -> &Arc<AppConfig> {
        self.cache.load()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::ConfigHandle;
    use crate::source::memory::InMemoryConfigSource;

    #[test]
    fn test_cached_config_reader() {
        let config = AppConfig::default();
        let source = Arc::new(InMemoryConfigSource::new(config.clone()));
        let handle = ConfigHandle::load(source).unwrap();

        let mut cached_reader = CachedConfigReader::new(&handle);

        let ptr1 = Arc::as_ptr(cached_reader.load());
        let ptr2 = Arc::as_ptr(cached_reader.load());

        assert_eq!(
            ptr1, ptr2,
            "should return the same underlying Arc without atomic ops if unchanged"
        );
    }
}
