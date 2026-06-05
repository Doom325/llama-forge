//! Assembles an [`AppConfig`] from wizard inputs and renders it to TOML.
//!
//! The wizard submits a flat list of [`ServerConfigDraft`]s plus optional
//! [`GlobalOverrides`]; this module maps each draft to a core [`ServerConfig`],
//! merges them into any existing config (Open Question #4 — merge by design),
//! and renders the result. Pure logic with no I/O, so it is unit-testable.

use std::path::PathBuf;

use llamaforge_core::config::{AppConfig, CacheType, FlashAttnMode, ServerConfig, ServerId};
use serde::{Deserialize, Serialize};

/// A single server as configured in the wizard's configure step
/// (08_SETUP_WIZARD.md Appendix: `ServerConfigDraft`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfigDraft {
    pub model_path: String,
    pub alias: String,
    pub binary_path: String,
    pub port: u16,
    pub gpu_layers: i32,
    pub parallel_slots: u32,
    pub ctx_size: u32,
    pub cache_type_k: String,
    pub cache_type_v: String,
    pub flash_attn: String,
    pub estimated_vram_mb: u64,
}

/// Optional global config overrides (08_SETUP_WIZARD.md `SaveConfigRequest`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalOverrides {
    #[serde(default)]
    pub bind_address: Option<String>,
    #[serde(default)]
    pub http_port: Option<u16>,
}

/// Body of `POST /api/setup/save-config` and `/preview-config`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveConfigRequest {
    pub path: String,
    #[serde(default)]
    pub force: bool,
    pub servers: Vec<ServerConfigDraft>,
    #[serde(default)]
    pub global_overrides: Option<GlobalOverrides>,
}

/// Map one wizard draft to a core [`ServerConfig`], parsing the enum-valued
/// string fields. Returns a human-readable error on an invalid value.
pub fn draft_to_server_config(draft: &ServerConfigDraft) -> Result<ServerConfig, String> {
    Ok(ServerConfig {
        binary_path: PathBuf::from(&draft.binary_path),
        model_path: PathBuf::from(&draft.model_path),
        alias: Some(draft.alias.clone()).filter(|a| !a.is_empty()),
        port: draft.port,
        gpu_layers: draft.gpu_layers,
        parallel_slots: draft.parallel_slots,
        ctx_size: draft.ctx_size,
        cache_type_k: parse_enum::<CacheType>("cache_type_k", &draft.cache_type_k)?,
        cache_type_v: parse_enum::<CacheType>("cache_type_v", &draft.cache_type_v)?,
        flash_attn: parse_enum::<FlashAttnMode>("flash_attn", &draft.flash_attn)?,
        estimated_vram_mb: Some(draft.estimated_vram_mb).filter(|v| *v > 0),
        ..ServerConfig::default()
    })
}

/// Assemble the final [`AppConfig`]: start from `existing` (or a default),
/// apply global overrides, and merge in the drafted servers.
pub fn assemble_config(
    existing: Option<AppConfig>,
    request: &SaveConfigRequest,
) -> Result<AppConfig, String> {
    let mut config = existing.unwrap_or_default();

    if let Some(overrides) = &request.global_overrides {
        if let Some(addr) = &overrides.bind_address {
            config.global.bind_address =
                addr.parse().map_err(|_| format!("invalid bind_address: {addr}"))?;
        }
        if let Some(port) = overrides.http_port {
            config.global.http_port = port;
        }
    }

    for draft in &request.servers {
        let server = draft_to_server_config(draft)?;
        // Stable id derived from the alias (falling back to the model file
        // stem) so re-running the wizard updates rather than duplicates.
        let id = ServerId::from_config_id(&server_slug(draft));
        config.servers.insert(id, server);
    }

    Ok(config)
}

/// Render an [`AppConfig`] to a TOML string.
pub fn render_toml(config: &AppConfig) -> Result<String, String> {
    toml::to_string_pretty(config).map_err(|e| format!("failed to render TOML: {e}"))
}

/// Derive a stable slug for a server from its alias or model filename.
fn server_slug(draft: &ServerConfigDraft) -> String {
    if !draft.alias.is_empty() {
        return draft.alias.clone();
    }
    PathBuf::from(&draft.model_path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "server".to_string())
}

/// Parse an enum-valued string field via serde, returning a field-tagged error.
fn parse_enum<T: serde::de::DeserializeOwned>(field: &str, value: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .map_err(|_| format!("invalid {field}: {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> ServerConfigDraft {
        ServerConfigDraft {
            model_path: "/models/llama-3-8b.gguf".to_string(),
            alias: "llama3".to_string(),
            binary_path: "/usr/local/bin/llama-server".to_string(),
            port: 8080,
            gpu_layers: -1,
            parallel_slots: 2,
            ctx_size: 8192,
            cache_type_k: "q8_0".to_string(),
            cache_type_v: "q8_0".to_string(),
            flash_attn: "auto".to_string(),
            estimated_vram_mb: 6000,
        }
    }

    #[test]
    fn draft_maps_fields_and_parses_enums() {
        let server = draft_to_server_config(&draft()).expect("valid draft");
        assert_eq!(server.port, 8080);
        assert_eq!(server.gpu_layers, -1);
        assert_eq!(server.alias.as_deref(), Some("llama3"));
        assert_eq!(server.cache_type_k, CacheType::Q8_0);
        assert_eq!(server.flash_attn, FlashAttnMode::Auto);
        assert_eq!(server.estimated_vram_mb, Some(6000));
    }

    #[test]
    fn invalid_cache_type_is_rejected() {
        let mut bad = draft();
        bad.cache_type_k = "not_a_cache".to_string();
        let err = draft_to_server_config(&bad).unwrap_err();
        assert!(err.contains("cache_type_k"), "{err}");
    }

    #[test]
    fn assemble_applies_overrides_and_inserts_servers() {
        let request = SaveConfigRequest {
            path: "/tmp/llamaforge.toml".to_string(),
            force: false,
            servers: vec![draft()],
            global_overrides: Some(GlobalOverrides {
                bind_address: Some("0.0.0.0".to_string()),
                http_port: Some(9000),
            }),
        };
        let config = assemble_config(None, &request).expect("assembled");
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.global.http_port, 9000);
        assert_eq!(config.global.bind_address.to_string(), "0.0.0.0");
        // Round-trips through TOML.
        let toml = render_toml(&config).expect("render");
        assert!(toml.contains("9000"));
    }

    #[test]
    fn invalid_bind_address_is_rejected() {
        let request = SaveConfigRequest {
            path: "/tmp/x.toml".to_string(),
            force: false,
            servers: vec![],
            global_overrides: Some(GlobalOverrides {
                bind_address: Some("not-an-ip".to_string()),
                http_port: None,
            }),
        };
        assert!(assemble_config(None, &request).is_err());
    }
}
