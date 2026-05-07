//! Core configuration types and structs.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;

/// Newtype wrapper for Server Identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServerId(pub String);

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ServerId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for ServerId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Newtype wrapper for Task Type Identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskTypeId(pub String);

impl fmt::Display for TaskTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for TaskTypeId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for TaskTypeId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Newtype wrapper for Task Class Name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskClassName(pub String);

impl fmt::Display for TaskClassName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for TaskClassName {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for TaskClassName {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Newtype wrapper for Requestor Identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestorId(pub String);

impl fmt::Display for RequestorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for RequestorId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for RequestorId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Structured logging level.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

/// Structured logging format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Pretty,
    Json,
    Compact,
}

/// Inference backend type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    #[default]
    LlamaServer,
    Custom,
}

/// KV cache quantization type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheType {
    #[default]
    F16,
    Q8_0,
    Q4_0,
}

/// Flash attention mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlashAttnMode {
    #[default]
    Auto,
    Enabled,
    Disabled,
}

/// Speculative decoding method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SpeculativeMethod {
    #[default]
    None,
    DraftModel {
        path: std::path::PathBuf,
    },
    Lookup {
        #[serde(default = "default_lookup_ngram_size")]
        ngram_size: u32,
    },
}

fn default_lookup_ngram_size() -> u32 {
    2
}

/// Model architecture classification.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelArchitecture {
    #[default]
    Transformer,
    Recurrent,
}

/// GPU identity (usually index).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GpuId(pub u32);

/// GPU device specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceSpec {
    pub gpu_id: GpuId,
    #[serde(default)]
    pub layer_range: Option<std::ops::Range<u32>>,
}

/// LoRA adapter configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoraAdapterConfig {
    pub path: PathBuf,
    pub scale: f32,
}

/// Top-level configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    #[serde(default)]
    pub global: GlobalConfig,
    #[serde(default)]
    pub grpc: GrpcConfig,
    #[serde(default)]
    pub prometheus: PrometheusConfig,
    #[serde(default)]
    pub web_ui: WebUiConfig,
    #[serde(default)]
    pub servers: HashMap<ServerId, ServerConfig>,
    #[serde(default)]
    pub priority_rules: PriorityRulesConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_bind_address")]
    pub bind_address: IpAddr,
    #[serde(default = "default_grpc_port")]
    pub grpc_port: u16,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default)]
    pub log_level: LogLevel,
    #[serde(default)]
    pub log_format: LogFormat,
    #[serde(default = "default_metrics_db_path")]
    pub metrics_db_path: PathBuf,
    #[serde(default = "default_retention_days")]
    pub metrics_retention_days: u32,
    #[serde(default = "default_queue_max_depth")]
    pub queue_max_depth: u32,
    #[serde(default = "default_queue_timeout_secs")]
    pub queue_default_timeout_secs: u64,
    #[serde(default = "default_restart_stagger_secs")]
    pub restart_stagger_secs: u64,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            grpc_port: default_grpc_port(),
            http_port: default_http_port(),
            log_level: LogLevel::default(),
            log_format: LogFormat::default(),
            metrics_db_path: default_metrics_db_path(),
            metrics_retention_days: default_retention_days(),
            queue_max_depth: default_queue_max_depth(),
            queue_default_timeout_secs: default_queue_timeout_secs(),
            restart_stagger_secs: default_restart_stagger_secs(),
            api_key: None,
            state_dir: default_state_dir(),
        }
    }
}

fn default_bind_address() -> IpAddr {
    "0.0.0.0".parse().unwrap()
}
fn default_grpc_port() -> u16 {
    50051
}
fn default_http_port() -> u16 {
    8000
}
fn default_metrics_db_path() -> PathBuf {
    PathBuf::from("llamaforge.db")
}
fn default_retention_days() -> u32 {
    30
}
fn default_queue_max_depth() -> u32 {
    1000
}
fn default_queue_timeout_secs() -> u64 {
    300
}
fn default_restart_stagger_secs() -> u64 {
    5
}
fn default_state_dir() -> PathBuf {
    PathBuf::from("/var/run/llamaforge/")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GrpcConfig {
    #[serde(default)]
    pub tls_cert: Option<PathBuf>,
    #[serde(default)]
    pub tls_key: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PrometheusConfig {
    #[serde(default = "default_prometheus_port")]
    pub port: u16,
    #[serde(default = "default_prometheus_path")]
    pub path: String,
}
fn default_prometheus_port() -> u16 {
    9090
}
fn default_prometheus_path() -> String {
    "/metrics".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WebUiConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerConfig {
    pub model_path: PathBuf,
    #[serde(default)]
    pub alias: Option<String>,
    pub port: u16,
    #[serde(default)]
    pub backend: Backend,
    #[serde(default)]
    pub devices: Vec<DeviceSpec>,
    #[serde(default = "default_gpu_layers")]
    pub gpu_layers: i32,
    #[serde(default = "default_parallel_slots")]
    pub parallel_slots: u32,
    #[serde(default = "default_ctx_size")]
    pub ctx_size: u32,
    #[serde(default)]
    pub cache_type_k: CacheType,
    #[serde(default)]
    pub cache_type_v: CacheType,
    #[serde(default)]
    pub flash_attn: FlashAttnMode,
    #[serde(default)]
    pub lora_adapters: Vec<LoraAdapterConfig>,
    #[serde(default)]
    pub estimated_vram_mb: Option<u64>,
    #[serde(default)]
    pub speculative_method: SpeculativeMethod,
    #[serde(default)]
    pub model_architecture: ModelArchitecture,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::new(),
            alias: None,
            port: 0,
            backend: Backend::default(),
            devices: Vec::new(),
            gpu_layers: default_gpu_layers(),
            parallel_slots: default_parallel_slots(),
            ctx_size: default_ctx_size(),
            cache_type_k: CacheType::default(),
            cache_type_v: CacheType::default(),
            flash_attn: FlashAttnMode::default(),
            lora_adapters: Vec::new(),
            estimated_vram_mb: None,
            speculative_method: SpeculativeMethod::default(),
            model_architecture: ModelArchitecture::default(),
        }
    }
}

fn default_gpu_layers() -> i32 {
    -1
}
fn default_parallel_slots() -> u32 {
    1
}
fn default_ctx_size() -> u32 {
    0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PriorityRulesConfig {
    #[serde(default)]
    pub task_types: HashMap<TaskTypeId, u32>,
    #[serde(default)]
    pub custom_task_types: HashMap<TaskTypeId, u32>,
    #[serde(default)]
    pub task_type_classes: HashMap<TaskTypeId, TaskClassName>,
    #[serde(default)]
    pub requestor_bonuses: HashMap<RequestorId, i32>,
    #[serde(default)]
    pub starvation: StarvationConfig,
    #[serde(default)]
    pub fair_share: FairShareConfig,
    #[serde(default)]
    pub preemption: PreemptionConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StarvationConfig {
    #[serde(default = "default_starvation_threshold")]
    pub threshold_secs: u64,
    #[serde(default = "default_starvation_boost")]
    pub boost_per_interval: u32,
}
impl Default for StarvationConfig {
    fn default() -> Self {
        Self {
            threshold_secs: default_starvation_threshold(),
            boost_per_interval: default_starvation_boost(),
        }
    }
}
fn default_starvation_threshold() -> u64 {
    30
}
fn default_starvation_boost() -> u32 {
    50
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FairShareConfig {
    #[serde(default)]
    pub weights: HashMap<RequestorId, u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PreemptionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_preempt_min_tokens")]
    pub min_tokens_before_preempt: u32,
}
fn default_preempt_min_tokens() -> u32 {
    10
}

/// Paths to all configuration and state files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    pub main_config: PathBuf,
    pub priority_rules: PathBuf,
    pub state_dir: PathBuf,
    pub lock_file: PathBuf,
}

impl ConfigPaths {
    pub fn new(main_config: PathBuf, priority_rules: PathBuf, state_dir: PathBuf) -> Self {
        let lock_file = state_dir.join("config.lock");
        Self {
            main_config,
            priority_rules,
            state_dir,
            lock_file,
        }
    }
}

/// Mutations that can be applied to the configuration at runtime.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigMutation {
    /// Replace the LoRA adapter list for a specific server.
    SetLoraAdapters {
        server_id: ServerId,
        adapters: Vec<LoraAdapterConfig>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_config_serde_roundtrip() {
        let mut config = AppConfig::default();
        config.servers.insert(
            ServerId::from("llama-3"),
            ServerConfig {
                model_path: PathBuf::from("models/llama-3.gguf"),
                port: 8080,
                ..Default::default()
            },
        );

        let toml = toml::to_string(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&toml).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn test_global_config_defaults() {
        let toml = "[global]\n";
        let config: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.global.grpc_port, 50051);
        assert_eq!(config.global.http_port, 8000);
        assert_eq!(config.global.bind_address, "0.0.0.0".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn test_unknown_keys_ignored() {
        let toml = "[global]\nbogus_key = 42\n";
        let result: Result<AppConfig, _> = toml::from_str(toml);
        assert!(result.is_ok());
    }
}

