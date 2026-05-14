//! Core configuration types and structs.
//!
//! See design doc Section 2.6 for field details and defaults.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

/// Newtype wrapper for Server Identity.
/// See design doc Section 2.6.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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
/// See design doc Section 2.6.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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
/// See design doc Section 2.6.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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
/// See design doc Section 2.6.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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
/// See design doc Section 2.6.
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
/// See design doc Section 2.6.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Pretty,
    Json,
    Compact,
}

/// Inference backend type.
/// See design doc Section 2.6.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    #[default]
    LlamaServer,
    Custom,
}

/// KV cache quantization type.
/// See design doc Section 2.6.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheType {
    #[default]
    F16,
    Q8_0,
    Q4_0,
}

/// Flash attention mode.
/// See design doc Section 2.6.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlashAttnMode {
    #[default]
    Auto,
    Enabled,
    Disabled,
}

/// Speculative decoding method.
/// See design doc Section 2.6.
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
/// See design doc Section 2.6.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelArchitecture {
    #[default]
    Transformer,
    Recurrent,
}

/// GPU identity (usually index).
/// See design doc Section 2.6.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct GpuId(pub u32);

/// GPU device specification.
/// See design doc Section 2.6.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeviceSpec {
    pub gpu_id: GpuId,
    #[serde(default)]
    pub layer_range: Option<std::ops::Range<u32>>,
}

/// LoRA adapter configuration.
/// See design doc Section 2.6.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LoraAdapterConfig {
    pub path: PathBuf,
    pub scale: f32,
}

/// Top-level configuration.
/// See design doc Section 2.6.
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
    pub cors: CorsConfig,
    #[serde(default)]
    pub servers: HashMap<ServerId, ServerConfig>,
    #[serde(default)]
    pub priority_rules: PriorityRulesConfig,
    #[serde(default)]
    pub routing: RoutingConfig,
}

/// CORS configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CorsConfig {
    #[serde(default = "default_allowed_origins")]
    pub allowed_origins: Vec<String>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self { allowed_origins: default_allowed_origins() }
    }
}

fn default_allowed_origins() -> Vec<String> {
    vec!["*".to_string()]
}

/// Routing configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingConfig {
    #[serde(default = "default_session_affinity_max_entries")]
    pub session_affinity_max_entries: u32,
    #[serde(default = "default_startup_wait_secs")]
    pub startup_wait_secs: u64,
    #[serde(default)]
    pub weights: RoutingWeightsConfig,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            session_affinity_max_entries: default_session_affinity_max_entries(),
            startup_wait_secs: default_startup_wait_secs(),
            weights: RoutingWeightsConfig::default(),
        }
    }
}

fn default_session_affinity_max_entries() -> u32 {
    10_000
}

fn default_startup_wait_secs() -> u64 {
    30
}

/// Routing weights configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingWeightsConfig {
    #[serde(default = "default_w_slots")]
    pub w_slots: f64,
    #[serde(default = "default_w_kv")]
    pub w_kv: f64,
    #[serde(default = "default_w_queue")]
    pub w_queue: f64,
    #[serde(default = "default_w_affinity")]
    pub w_affinity: f64,
}

impl Default for RoutingWeightsConfig {
    fn default() -> Self {
        Self {
            w_slots: default_w_slots(),
            w_kv: default_w_kv(),
            w_queue: default_w_queue(),
            w_affinity: default_w_affinity(),
        }
    }
}

fn default_w_slots() -> f64 {
    0.4
}
fn default_w_kv() -> f64 {
    0.2
}
fn default_w_queue() -> f64 {
    0.2
}
fn default_w_affinity() -> f64 {
    0.2
}

/// Global system configuration.
/// See design doc Section 2.6.
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
    IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))
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

/// gRPC listener configuration.
/// See design doc Section 2.6.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GrpcConfig {
    #[serde(default = "default_grpc_max_message_size_mb")]
    pub max_message_size_mb: u32,
    #[serde(default = "default_grpc_keepalive_interval_secs")]
    pub keepalive_interval_secs: u32,
    #[serde(default = "default_grpc_keepalive_timeout_secs")]
    pub keepalive_timeout_secs: u32,
    #[serde(default = "default_grpc_max_concurrent_streams")]
    pub max_concurrent_streams: u32,
    #[serde(default)]
    pub tls_cert: Option<PathBuf>,
    #[serde(default)]
    pub tls_key: Option<PathBuf>,
}

impl Default for GrpcConfig {
    fn default() -> Self {
        Self {
            max_message_size_mb: default_grpc_max_message_size_mb(),
            keepalive_interval_secs: default_grpc_keepalive_interval_secs(),
            keepalive_timeout_secs: default_grpc_keepalive_timeout_secs(),
            max_concurrent_streams: default_grpc_max_concurrent_streams(),
            tls_cert: None,
            tls_key: None,
        }
    }
}

fn default_grpc_max_message_size_mb() -> u32 {
    16
}
fn default_grpc_keepalive_interval_secs() -> u32 {
    30
}
fn default_grpc_keepalive_timeout_secs() -> u32 {
    10
}
fn default_grpc_max_concurrent_streams() -> u32 {
    1000
}

/// Prometheus metrics configuration.
/// See design doc Section 2.6.
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

/// Web UI configuration.
/// See design doc Section 2.6.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WebUiConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}
fn default_true() -> bool {
    true
}

/// Single llama-server instance configuration.
/// See design doc Section 2.6.
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
    pub shutdown_drain_timeout_secs: Option<u64>,
    #[serde(default)]
    pub speculative_method: SpeculativeMethod,
    #[serde(default)]
    pub model_architecture: ModelArchitecture,
    #[serde(default = "default_max_restart_attempts")]
    pub max_restart_attempts: u32,
    #[serde(default = "default_restart_on_degradation")]
    pub restart_on_degradation: bool,
    #[serde(default = "default_restart_cooldown_secs")]
    pub restart_cooldown_secs: u64,
}

fn default_max_restart_attempts() -> u32 {
    3
}

fn default_restart_on_degradation() -> bool {
    true
}

fn default_restart_cooldown_secs() -> u64 {
    10
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
            shutdown_drain_timeout_secs: None,
            speculative_method: SpeculativeMethod::default(),
            model_architecture: ModelArchitecture::default(),
            max_restart_attempts: default_max_restart_attempts(),
            restart_on_degradation: default_restart_on_degradation(),
            restart_cooldown_secs: default_restart_cooldown_secs(),
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

/// Priority-aware queuing and scheduling rules.
/// See design doc Section 2.6.
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
    pub requestor_classes: HashMap<String, RequestorClassPolicy>,
    #[serde(default)]
    pub starvation: StarvationConfig,
    #[serde(default)]
    pub fair_share: FairShareConfig,
    #[serde(default)]
    pub preemption: PreemptionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AllowedPriorityRange {
    pub min: u32,
    pub max: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct RequestorClassPolicy {
    pub max_concurrent: u32,
    pub allowed_task_types: HashMap<TaskTypeId, AllowedPriorityRange>,
    pub priority_bonus: i32,
    pub queue_timeout_secs: Option<u32>,
}

/// Request starvation boost configuration.
/// See design doc Section 2.6.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StarvationConfig {
    #[serde(default = "default_check_interval_secs")]
    pub check_interval_secs: u32,
    #[serde(default = "default_boost_scale")]
    pub boost_scale: f64,
    #[serde(default = "default_boost_start_after_secs")]
    pub boost_start_after_secs: u32,
    #[serde(default = "default_max_boost")]
    pub max_boost: u32,
}
impl Default for StarvationConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: default_check_interval_secs(),
            boost_scale: default_boost_scale(),
            boost_start_after_secs: default_boost_start_after_secs(),
            max_boost: default_max_boost(),
        }
    }
}
fn default_check_interval_secs() -> u32 {
    5
}
fn default_boost_scale() -> f64 {
    50.0
}
fn default_boost_start_after_secs() -> u32 {
    30
}
fn default_max_boost() -> u32 {
    400
}

/// Fair share scheduling weights.
/// See design doc Section 2.6.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FairShareConfig {
    #[serde(default)]
    pub weights: HashMap<RequestorId, u32>,
}

/// Queue preemption configuration.
/// See design doc Section 2.6.
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
/// See design doc Section 3.3.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigPaths {
    pub main_config: PathBuf,
    pub priority_rules: PathBuf,
    pub state_dir: PathBuf,
    pub lock_file: PathBuf,
}

impl ConfigPaths {
    pub fn new(main_config: PathBuf, priority_rules: PathBuf, state_dir: PathBuf) -> Self {
        let lock_file = state_dir.join("config.lock");
        Self { main_config, priority_rules, state_dir, lock_file }
    }
}
/// Mutations that can be applied to the configuration at runtime.
/// See design doc Section 2.6.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigMutation {
    /// Replace the LoRA adapter list for a specific server.
    SetLoraAdapters { server_id: ServerId, adapters: Vec<LoraAdapterConfig> },
}

/// The result of applying a `ConfigMutation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MutationOutcome {
    Applied,
    Unchanged,
    TargetMissing,
}

impl ConfigMutation {
    /// Applies the mutation in-place to an AppConfig.
    pub fn apply(&self, config: &mut AppConfig) -> MutationOutcome {
        match self {
            ConfigMutation::SetLoraAdapters { server_id, adapters } => {
                if let Some(server) = config.servers.get_mut(server_id) {
                    if server.lora_adapters != *adapters {
                        server.lora_adapters = adapters.clone();
                        MutationOutcome::Applied
                    } else {
                        MutationOutcome::Unchanged
                    }
                } else {
                    MutationOutcome::TargetMissing
                }
            }
        }
    }
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
    fn test_app_config_full_roundtrip() {
        let mut config = AppConfig {
            global: GlobalConfig {
                bind_address: IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
                grpc_port: 9000,
                http_port: 9001,
                log_level: LogLevel::Debug,
                log_format: LogFormat::Json,
                metrics_db_path: PathBuf::from("test.db"),
                metrics_retention_days: 7,
                queue_max_depth: 50,
                queue_default_timeout_secs: 60,
                restart_stagger_secs: 1,
                api_key: Some("secret".to_string()),
                state_dir: PathBuf::from("/tmp/test"),
            },
            grpc: GrpcConfig {
                tls_cert: Some(PathBuf::from("cert.pem")),
                tls_key: Some(PathBuf::from("key.pem")),
                ..Default::default()
            },
            prometheus: PrometheusConfig { port: 9999, path: "/metrics_test".to_string() },
            web_ui: WebUiConfig { enabled: false },
            cors: CorsConfig::default(),
            servers: HashMap::new(),
            priority_rules: PriorityRulesConfig {
                task_types: HashMap::from([(TaskTypeId::from("chat"), 500)]),
                custom_task_types: HashMap::from([(TaskTypeId::from("ext"), 150)]),
                task_type_classes: HashMap::from([(
                    TaskTypeId::from("chat"),
                    TaskClassName::from("ui"),
                )]),
                requestor_bonuses: HashMap::from([(RequestorId::from("vip"), 100)]),
                requestor_classes: HashMap::new(),
                starvation: StarvationConfig {
                    check_interval_secs: 5,
                    boost_scale: 50.0,
                    boost_start_after_secs: 30,
                    max_boost: 400,
                },
                fair_share: FairShareConfig {
                    weights: HashMap::from([(RequestorId::from("default"), 100)]),
                },
                preemption: PreemptionConfig { enabled: true, min_tokens_before_preempt: 5 },
            },
            routing: Default::default(),
        };

        config.servers.insert(
            ServerId::from("s1"),
            ServerConfig {
                model_path: PathBuf::from("model.gguf"),
                alias: Some("alias".to_string()),
                port: 1234,
                backend: Backend::Custom,
                devices: vec![DeviceSpec { gpu_id: GpuId(0), layer_range: Some(0..10) }],
                gpu_layers: 32,
                parallel_slots: 2,
                ctx_size: 4096,
                cache_type_k: CacheType::Q8_0,
                cache_type_v: CacheType::Q4_0,
                flash_attn: FlashAttnMode::Enabled,
                lora_adapters: vec![LoraAdapterConfig {
                    path: PathBuf::from("lora.gguf"),
                    scale: 0.5,
                }],
                estimated_vram_mb: Some(1024),
                shutdown_drain_timeout_secs: None,
                speculative_method: SpeculativeMethod::Lookup { ngram_size: 3 },
                model_architecture: ModelArchitecture::Recurrent,
                max_restart_attempts: 5,
                restart_on_degradation: true,
                restart_cooldown_secs: 10,
            },
        );

        let toml_str = toml::to_string(&config).expect("failed to serialize full config");
        let parsed: AppConfig = toml::from_str(&toml_str).expect("failed to parse full config");
        assert_eq!(config, parsed, "roundtrip failed for fully populated config");
    }

    #[test]
    fn test_global_config_defaults() {
        let toml = "[global]\n";
        let config: AppConfig = toml::from_str(toml).unwrap();
        let g = config.global;
        assert_eq!(g.bind_address, IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)));
        assert_eq!(g.grpc_port, 50051);
        assert_eq!(g.http_port, 8000);
        assert_eq!(g.log_level, LogLevel::Info);
        assert_eq!(g.log_format, LogFormat::Pretty);
        assert_eq!(g.metrics_db_path, PathBuf::from("llamaforge.db"));
        assert_eq!(g.metrics_retention_days, 30);
        assert_eq!(g.queue_max_depth, 1000);
        assert_eq!(g.queue_default_timeout_secs, 300);
        assert_eq!(g.restart_stagger_secs, 5);
        assert_eq!(g.api_key, None);
        assert_eq!(g.state_dir, PathBuf::from("/var/run/llamaforge/"));
    }

    #[test]
    fn test_unknown_keys_ignored() {
        let toml = "[global]\nbogus_key = 42\n";
        let result: Result<AppConfig, _> = toml::from_str(toml);
        assert!(result.is_ok());
    }
}
