//! Config diff computation and types.
//!
//! See design doc Section 2.8 for diff type definitions and classification rules.

use crate::config::types::{AppConfig, ServerConfig, ServerId};
use std::time::{Instant, SystemTime};

/// Describes what changed between two config snapshots.
/// See design doc Section 2.8.
#[derive(Debug, Clone)]
pub struct ConfigDiff {
    pub changes: Vec<ConfigChange>,
    pub timestamp: Instant,
    pub wall_clock: SystemTime,
}

/// Individual config change classification.
/// See design doc Section 2.8.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ConfigChange {
    /// Global settings changed.
    GlobalChanged,
    /// Priority or scheduling rules changed.
    PriorityRulesChanged,
    /// Routing configuration changed.
    RoutingConfigChanged,
    /// CORS configuration changed.
    CorsConfigChanged,
    /// Gateway (request body / concurrency) settings changed.
    GatewayConfigChanged,
    /// gRPC TLS settings changed (enabled flag or cert/key paths).
    GrpcTlsChanged,
    /// A new server was added.
    ServerAdded { id: ServerId },
    /// A server was removed.
    ServerRemoved { id: ServerId },
    /// A server's configuration was modified.
    ServerChanged { id: ServerId, diff: ServerConfigDiff },
}

impl ConfigDiff {
    pub fn new(changes: Vec<ConfigChange>) -> Self {
        Self { changes, timestamp: Instant::now(), wall_clock: SystemTime::now() }
    }

    pub fn servers_added(&self) -> impl Iterator<Item = &ServerId> {
        self.changes.iter().filter_map(|c| match c {
            ConfigChange::ServerAdded { id } => Some(id),
            _ => None,
        })
    }

    pub fn servers_removed(&self) -> impl Iterator<Item = &ServerId> {
        self.changes.iter().filter_map(|c| match c {
            ConfigChange::ServerRemoved { id } => Some(id),
            _ => None,
        })
    }

    pub fn servers_changed(&self) -> impl Iterator<Item = (&ServerId, &ServerConfigDiff)> {
        self.changes.iter().filter_map(|c| match c {
            ConfigChange::ServerChanged { id, diff } => Some((id, diff)),
            _ => None,
        })
    }
}

/// Enum of all server config fields.
/// See design doc Section 2.8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerConfigField {
    BinaryPath,
    ModelPath,
    Alias,
    Port,
    Backend,
    Devices,
    GpuLayers,
    ParallelSlots,
    CtxSize,
    CacheTypeK,
    CacheTypeV,
    FlashAttn,
    LoraAdapters,
    EstimatedVramMb,
    SpeculativeMethod,
    ModelArchitecture,
}

impl ServerConfigField {
    /// Returns true if this field requires a server restart.
    /// See design doc Section 2.8.
    pub fn requires_restart(&self) -> bool {
        matches!(
            self,
            Self::BinaryPath
                | Self::ModelPath
                | Self::CtxSize
                | Self::ParallelSlots
                | Self::Devices
                | Self::Backend
                | Self::GpuLayers
                | Self::CacheTypeK
                | Self::CacheTypeV
                | Self::FlashAttn
                | Self::Port
                | Self::SpeculativeMethod
                | Self::ModelArchitecture
        )
    }

    /// Returns true if this field can be hot-reloaded.
    /// See design doc Section 2.8.
    pub fn hot_reloadable(&self) -> bool {
        matches!(self, Self::LoraAdapters | Self::Alias | Self::EstimatedVramMb)
    }
}

/// Summary of changes for a single server.
/// See design doc Section 2.8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfigDiff {
    pub fields_changed: Vec<ServerConfigField>,
}

impl ServerConfigDiff {
    /// True if any changed field requires a restart.
    pub fn requires_restart(&self) -> bool {
        self.fields_changed.iter().any(|f| f.requires_restart())
    }

    /// True if ALL changed fields are hot-reloadable.
    pub fn hot_reloadable(&self) -> bool {
        self.fields_changed.iter().all(|f| f.hot_reloadable())
    }
}

/// Computes the difference between two configurations.
pub fn compute_diff(old: &AppConfig, new: &AppConfig) -> ConfigDiff {
    let mut changes = Vec::new();

    if old.global != new.global {
        changes.push(ConfigChange::GlobalChanged);
    }

    if old.priority_rules != new.priority_rules {
        changes.push(ConfigChange::PriorityRulesChanged);
    }

    if old.routing != new.routing {
        changes.push(ConfigChange::RoutingConfigChanged);
    }

    if old.cors != new.cors {
        changes.push(ConfigChange::CorsConfigChanged);
    }

    if old.gateway != new.gateway {
        changes.push(ConfigChange::GatewayConfigChanged);
    }

    if old.grpc.tls_enabled != new.grpc.tls_enabled
        || old.grpc.tls_cert != new.grpc.tls_cert
        || old.grpc.tls_key != new.grpc.tls_key
    {
        changes.push(ConfigChange::GrpcTlsChanged);
    }

    // Servers added or changed
    for (id, new_server) in &new.servers {
        if let Some(old_server) = old.servers.get(id) {
            if old_server != new_server {
                let diff = compute_server_diff(old_server, new_server);
                changes.push(ConfigChange::ServerChanged { id: *id, diff });
            }
        } else {
            changes.push(ConfigChange::ServerAdded { id: *id });
        }
    }

    // Servers removed
    for id in old.servers.keys() {
        if !new.servers.contains_key(id) {
            changes.push(ConfigChange::ServerRemoved { id: *id });
        }
    }

    ConfigDiff::new(changes)
}

fn compute_server_diff(old: &ServerConfig, new: &ServerConfig) -> ServerConfigDiff {
    let mut fields_changed = Vec::new();

    if old.binary_path != new.binary_path {
        fields_changed.push(ServerConfigField::BinaryPath);
    }
    if old.model_path != new.model_path {
        fields_changed.push(ServerConfigField::ModelPath);
    }
    if old.alias != new.alias {
        fields_changed.push(ServerConfigField::Alias);
    }
    if old.port != new.port {
        fields_changed.push(ServerConfigField::Port);
    }
    if old.backend != new.backend {
        fields_changed.push(ServerConfigField::Backend);
    }
    if old.devices != new.devices {
        fields_changed.push(ServerConfigField::Devices);
    }
    if old.gpu_layers != new.gpu_layers {
        fields_changed.push(ServerConfigField::GpuLayers);
    }
    if old.parallel_slots != new.parallel_slots {
        fields_changed.push(ServerConfigField::ParallelSlots);
    }
    if old.ctx_size != new.ctx_size {
        fields_changed.push(ServerConfigField::CtxSize);
    }
    if old.cache_type_k != new.cache_type_k {
        fields_changed.push(ServerConfigField::CacheTypeK);
    }
    if old.cache_type_v != new.cache_type_v {
        fields_changed.push(ServerConfigField::CacheTypeV);
    }
    if old.flash_attn != new.flash_attn {
        fields_changed.push(ServerConfigField::FlashAttn);
    }
    if old.lora_adapters != new.lora_adapters {
        fields_changed.push(ServerConfigField::LoraAdapters);
    }
    if old.estimated_vram_mb != new.estimated_vram_mb {
        fields_changed.push(ServerConfigField::EstimatedVramMb);
    }
    if old.speculative_method != new.speculative_method {
        fields_changed.push(ServerConfigField::SpeculativeMethod);
    }
    if old.model_architecture != new.model_architecture {
        fields_changed.push(ServerConfigField::ModelArchitecture);
    }

    ServerConfigDiff { fields_changed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{AppConfig, LoraAdapterConfig, ServerConfig};
    use std::path::PathBuf;

    #[test]
    fn test_compute_diff_identical() {
        let config = AppConfig::default();
        let diff = compute_diff(&config, &config);
        assert!(diff.changes.is_empty());
    }

    #[test]
    fn test_compute_diff_server_added() {
        let old = AppConfig::default();
        let mut new = AppConfig::default();
        new.servers.insert(
            ServerId::from_config_id("s1"),
            ServerConfig { model_path: PathBuf::from("m1.gguf"), port: 8080, ..Default::default() },
        );

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ConfigChange::ServerAdded { id } => assert_eq!(*id, ServerId::from_config_id("s1")),
            _ => panic!("expected ServerAdded"),
        }
    }

    #[test]
    fn test_compute_diff_server_removed() {
        let mut old = AppConfig::default();
        old.servers.insert(
            ServerId::from_config_id("s1"),
            ServerConfig { model_path: PathBuf::from("m1.gguf"), port: 8080, ..Default::default() },
        );
        let new = AppConfig::default();

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ConfigChange::ServerRemoved { id } => assert_eq!(*id, ServerId::from_config_id("s1")),
            _ => panic!("expected ServerRemoved"),
        }
    }

    #[test]
    fn test_compute_diff_server_changed_restart() {
        let mut old = AppConfig::default();
        old.servers.insert(
            ServerId::from_config_id("s1"),
            ServerConfig { model_path: PathBuf::from("m1.gguf"), port: 8080, ..Default::default() },
        );
        let mut new = old.clone();
        new.servers.get_mut(&ServerId::from_config_id("s1")).unwrap().port = 8081;

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ConfigChange::ServerChanged { id, diff } => {
                assert_eq!(*id, ServerId::from_config_id("s1"));
                assert!(diff.requires_restart());
                assert!(diff.fields_changed.contains(&ServerConfigField::Port));
            }
            _ => panic!("expected ServerChanged"),
        }
    }

    #[test]
    fn test_compute_diff_server_changed_hot_reload() {
        let mut old = AppConfig::default();
        old.servers.insert(
            ServerId::from_config_id("s1"),
            ServerConfig {
                model_path: PathBuf::from("m1.gguf"),
                lora_adapters: vec![LoraAdapterConfig {
                    path: PathBuf::from("l1.gguf"),
                    scale: 0.5,
                }],
                ..Default::default()
            },
        );
        let mut new = old.clone();
        new.servers.get_mut(&ServerId::from_config_id("s1")).unwrap().lora_adapters =
            vec![LoraAdapterConfig { path: PathBuf::from("l1.gguf"), scale: 0.8 }];

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ConfigChange::ServerChanged { id, diff } => {
                assert_eq!(*id, ServerId::from_config_id("s1"));
                assert!(!diff.requires_restart());
                assert!(diff.hot_reloadable());
                assert!(diff.fields_changed.contains(&ServerConfigField::LoraAdapters));
            }
            _ => panic!("expected ServerChanged"),
        }
    }

    #[test]
    fn test_compute_diff_global_changed() {
        let old = AppConfig::default();
        let mut new = AppConfig::default();
        new.global.http_port = 9000;

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ConfigChange::GlobalChanged => {}
            _ => panic!("expected GlobalChanged"),
        }
    }

    #[test]
    fn test_compute_diff_priority_rules_changed() {
        let old = AppConfig::default();
        let mut new = AppConfig::default();
        new.priority_rules.starvation.check_interval_secs = 60;

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ConfigChange::PriorityRulesChanged => {}
            _ => panic!("expected PriorityRulesChanged"),
        }
    }

    #[test]
    fn test_compute_diff_gateway_changed() {
        let old = AppConfig::default();
        let mut new = AppConfig::default();
        new.gateway.max_concurrent_large_requests = 12;

        let diff = compute_diff(&old, &new);
        assert!(diff.changes.iter().any(|c| matches!(c, ConfigChange::GatewayConfigChanged)));
    }

    #[test]
    fn test_compute_diff_grpc_tls_changed() {
        let old = AppConfig::default();
        let mut new = AppConfig::default();
        new.grpc.tls_enabled = true;
        new.grpc.tls_cert = Some(PathBuf::from("/etc/llamaforge/cert.pem"));
        new.grpc.tls_key = Some(PathBuf::from("/etc/llamaforge/key.pem"));

        let diff = compute_diff(&old, &new);
        assert!(diff.changes.iter().any(|c| matches!(c, ConfigChange::GrpcTlsChanged)));
    }

    #[test]
    fn test_compute_diff_multi_change() {
        let mut old = AppConfig::default();
        old.servers.insert(
            ServerId::from_config_id("s1"),
            ServerConfig { model_path: PathBuf::from("m1.gguf"), port: 8080, ..Default::default() },
        );

        let mut new = old.clone();
        new.global.http_port = 9000;
        new.servers.remove(&ServerId::from_config_id("s1"));
        new.servers.insert(
            ServerId::from_config_id("s2"),
            ServerConfig { model_path: PathBuf::from("m2.gguf"), port: 8081, ..Default::default() },
        );

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 3);
        let has_global = diff.changes.iter().any(|c| matches!(c, ConfigChange::GlobalChanged));
        let has_s1_removed = diff
            .changes
            .iter()
            .any(|c| matches!(c, ConfigChange::ServerRemoved { id } if *id == ServerId::from_config_id("s1")));
        let has_s2_added = diff
            .changes
            .iter()
            .any(|c| matches!(c, ConfigChange::ServerAdded { id } if *id == ServerId::from_config_id("s2")));

        assert!(has_global);
        assert!(has_s1_removed);
        assert!(has_s2_added);
    }
}
