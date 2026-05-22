//! Config Manager — parses `llamaforge.toml`, arbitrates UI overrides,
//! emits hot-reload diff events via `ArcSwap`.
//!
//! Implements the Config Manager half of design.md §5.5; the Web UI
//! routes that share §5.5 live in `crates/llamaforge-transport/src/management.rs`.
//! Component spec: `docs/component-designs/01_CONFIG_MANAGER.md`.

pub(crate) mod cached;
pub(crate) mod diff;
pub(crate) mod error;
pub(crate) mod handle;
pub(crate) mod instrumented;

#[cfg(any(test, feature = "test-utils"))]
pub mod source;
#[cfg(not(any(test, feature = "test-utils")))]
pub(crate) mod source;

pub(crate) mod traits;
pub(crate) mod types;

pub use cached::CachedConfigReader;
pub use diff::{compute_diff, ConfigChange, ConfigDiff, ServerConfigDiff, ServerConfigField};
pub use error::{ConfigLoadError, ConfigReloadError, WritebackError};
pub use handle::{validate_server_id_slug, ConfigHandle, SERVER_ID_MAX_LEN};
pub use instrumented::InstrumentedConfigHandle;
pub use source::toml_file::TomlFileConfigSource;
pub use source::ConfigSource;
pub use traits::{ConfigReader, ConfigWriter};
pub use types::{
    AllowedPriorityRange, AppConfig, Backend, CacheType, ConfigMutation, ConfigPaths, DeviceSpec,
    FairShareConfig, FlashAttnMode, GatewayConfig, GlobalConfig, GpuId, GrpcConfig, LogFormat,
    LogLevel, LoraAdapterConfig, ModelArchitecture, MutationOutcome, PreemptionConfig,
    PriorityRulesConfig, PrometheusConfig, RequestorClassPolicy, RequestorId, RoutingConfig,
    RoutingWeightsConfig, ServerConfig, ServerId, SpeculativeMethod, StarvationConfig,
    TaskClassName, TaskTypeId, WalCheckpointMode, WebUiConfig,
};
