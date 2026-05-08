//! Config Manager — see component-designs/01_CONFIG_MANAGER.md

pub(crate) mod cached;
pub(crate) mod diff;
pub(crate) mod error;
pub(crate) mod handle;
pub(crate) mod instrumented;
pub(crate) mod source;
pub(crate) mod traits;
pub(crate) mod types;

pub use cached::CachedConfigReader;
pub use diff::{ConfigChange, ConfigDiff, ServerConfigDiff, ServerConfigField};
pub use error::{ConfigLoadError, ConfigReloadError, WritebackError};
pub use handle::ConfigHandle;
pub use instrumented::InstrumentedConfigHandle;
pub use source::toml_file::TomlFileConfigSource;
pub use source::ConfigSource;
pub use traits::{ConfigReader, ConfigWriter};
pub use types::{
    AppConfig, Backend, CacheType, ConfigMutation, ConfigPaths, DeviceSpec, FairShareConfig,
    FlashAttnMode, GlobalConfig, GpuId, GrpcConfig, LogFormat, LogLevel, LoraAdapterConfig,
    ModelArchitecture, PreemptionConfig, PriorityRulesConfig, PrometheusConfig, RequestorId,
    ServerConfig, ServerId, SpeculativeMethod, StarvationConfig, TaskClassName, TaskTypeId,
    WebUiConfig, MutationOutcome,
};
