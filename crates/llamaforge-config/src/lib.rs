//! Config Manager — see component-designs/01_CONFIG_MANAGER.md

pub(crate) mod cached;
pub mod diff;
pub mod error;
pub(crate) mod handle;
pub(crate) mod instrumented;
pub(crate) mod source;
pub mod traits;
pub mod types;

pub use cached::CachedConfigReader;
pub use handle::ConfigHandle;
pub use instrumented::InstrumentedConfigHandle;
pub use source::toml_file::TomlFileConfigSource;
pub use source::ConfigSource;
pub use traits::{ConfigReader, ConfigWriter};
