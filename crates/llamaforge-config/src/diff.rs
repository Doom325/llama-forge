//! Config diff computation and types.

use crate::types::{AppConfig, ServerConfig, ServerId};
use std::time::{Instant, SystemTime};

/// Describes what changed between two config snapshots.
#[derive(Debug, Clone)]
pub struct ConfigDiff {
    pub changes: Vec<ConfigChange>,
    pub timestamp: Instant,
    pub wall_clock: SystemTime,
}

#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ConfigChange {
    GlobalChanged,
    PriorityRulesChanged,
    ServerAdded { id: ServerId },
    ServerRemoved { id: ServerId },
    ServerChanged { id: ServerId, diff: ServerConfigDiff },
}

impl ConfigDiff {
    pub fn new(changes: Vec<ConfigChange>) -> Self {
        Self {
            changes,
            timestamp: Instant::now(),
            wall_clock: SystemTime::now(),
        }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerConfigField {
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
    pub fn requires_restart(&self) -> bool {
        matches!(
            self,
            Self::ModelPath
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

    pub fn hot_reloadable(&self) -> bool {
        matches!(
            self,
            Self::LoraAdapters | Self::Alias | Self::EstimatedVramMb
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfigDiff {
    pub fields_changed: Vec<ServerConfigField>,
}

impl ServerConfigDiff {
    pub fn requires_restart(&self) -> bool {
        self.fields_changed.iter().any(|f| f.requires_restart())
    }

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

    // Servers added or changed
    for (id, new_server) in &new.servers {
        if let Some(old_server) = old.servers.get(id) {
            if old_server != new_server {
                let diff = compute_server_diff(old_server, new_server);
                changes.push(ConfigChange::ServerChanged {
                    id: id.clone(),
                    diff,
                });
            }
        } else {
            changes.push(ConfigChange::ServerAdded { id: id.clone() });
        }
    }

    // Servers removed
    for id in old.servers.keys() {
        if !new.servers.contains_key(id) {
            changes.push(ConfigChange::ServerRemoved { id: id.clone() });
        }
    }

    ConfigDiff::new(changes)
}

fn compute_server_diff(old: &ServerConfig, new: &ServerConfig) -> ServerConfigDiff {
    let mut fields_changed = Vec::new();

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
    use crate::types::{AppConfig, ServerConfig};
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
            ServerId::from("s1"),
            ServerConfig {
                model_path: PathBuf::from("m1.gguf"),
                port: 8080,
                ..Default::default()
            },
        );

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ConfigChange::ServerAdded { id } => assert_eq!(id.0, "s1"),
            _ => panic!("expected ServerAdded"),
        }
    }

    #[test]
    fn test_compute_diff_server_removed() {
        let mut old = AppConfig::default();
        old.servers.insert(
            ServerId::from("s1"),
            ServerConfig {
                model_path: PathBuf::from("m1.gguf"),
                port: 8080,
                ..Default::default()
            },
        );
        let new = AppConfig::default();

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ConfigChange::ServerRemoved { id } => assert_eq!(id.0, "s1"),
            _ => panic!("expected ServerRemoved"),
        }
    }

    #[test]
    fn test_compute_diff_server_changed_restart() {
        let mut old = AppConfig::default();
        old.servers.insert(
            ServerId::from("s1"),
            ServerConfig {
                model_path: PathBuf::from("m1.gguf"),
                port: 8080,
                ..Default::default()
            },
        );
        let mut new = old.clone();
        new.servers.get_mut(&ServerId::from("s1")).unwrap().port = 8081;

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ConfigChange::ServerChanged { id, diff } => {
                assert_eq!(id.0, "s1");
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
            ServerId::from("s1"),
            ServerConfig {
                model_path: PathBuf::from("m1.gguf"),
                alias: Some("old".to_string()),
                ..Default::default()
            },
        );
        let mut new = old.clone();
        new.servers.get_mut(&ServerId::from("s1")).unwrap().alias = Some("new".to_string());

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.changes.len(), 1);
        match &diff.changes[0] {
            ConfigChange::ServerChanged { id, diff } => {
                assert_eq!(id.0, "s1");
                assert!(!diff.requires_restart());
                assert!(diff.hot_reloadable());
                assert!(diff.fields_changed.contains(&ServerConfigField::Alias));
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
}
