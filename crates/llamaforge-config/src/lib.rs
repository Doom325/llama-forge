//! Config Manager — see component-designs/01_CONFIG_MANAGER.md

pub(crate) mod cached;
pub mod diff;
pub mod error;
pub(crate) mod handle;
pub(crate) mod instrumented;
pub mod source;
pub mod types;

pub use source::toml_file::TomlFileConfigSource;
pub use source::ConfigSource;
