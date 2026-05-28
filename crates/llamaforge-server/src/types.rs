//! Domain types for Server Manager
//! See design doc Section 2.3.

use llamaforge_core::types::ServerId;
pub use llamaforge_core::types::{ClusterState, ClusterStatus, ModelArchitecture, ServerLifecycle};
use serde::{Deserialize, Serialize};

/// The speculative decoding method used by the server.
/// See design doc Section 2.3.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpeculativeMethod {
    #[default]
    None,
    Draft,
    Lookup,
    Mtp,
}

/// The capabilities supported by a running server.
/// See design doc Section 2.3.
#[derive(Debug, Clone, Default)]
pub struct ServerCapabilities {
    pub models: Vec<String>,
    pub lora_adapters: Vec<String>,
    pub multimodal: bool,
    pub speculative: SpeculativeMethod,
    pub architecture: ModelArchitecture,
}

/// The state of a single processing slot on the server.
/// See design doc Section 2.3.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SlotState {
    pub id: u32,
    #[serde(alias = "state", deserialize_with = "deserialize_state_as_bool", default)]
    pub is_processing: bool,
    #[serde(alias = "n_past_tokens", default)]
    pub n_past: Option<u32>,
    #[serde(alias = "n_decoded_tokens", default)]
    pub n_decoded: Option<u32>,
    #[serde(default)]
    pub n_ctx: Option<u32>,
    #[serde(default)]
    pub prompt_hash: Option<u64>,
}

fn deserialize_state_as_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum StateOrBool {
        State(u32),
        Bool(bool),
    }

    match StateOrBool::deserialize(deserializer)? {
        StateOrBool::State(s) => Ok(s != 0),
        StateOrBool::Bool(b) => Ok(b),
    }
}

/// An event broadcasted when a client should refresh its connection to a server.
#[derive(Debug, Clone)]
pub struct ClientRefreshEvent {
    pub server_id: ServerId,
    pub new_port: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_types_debug_clone() {
        let lifecycle = ServerLifecycle::Starting;
        assert_eq!(lifecycle.clone(), ServerLifecycle::Starting);
        assert!(format!("{:?}", lifecycle).contains("Starting"));

        let method = SpeculativeMethod::Draft;
        assert_eq!(method.clone(), SpeculativeMethod::Draft);

        let arch = ModelArchitecture::Llama;
        assert_eq!(arch.clone(), ModelArchitecture::Llama);

        let caps = ServerCapabilities {
            models: vec!["test".into()],
            lora_adapters: vec![],
            multimodal: false,
            speculative: method,
            architecture: arch,
        };
        assert!(format!("{:?}", caps).contains("test"));

        let slot = SlotState {
            id: 0,
            is_processing: false,
            n_past: Some(0),
            n_decoded: Some(0),
            n_ctx: None,
            prompt_hash: Some(12345),
        };
        assert!(format!("{:?}", slot).contains("12345"));

        let event =
            ClientRefreshEvent { server_id: ServerId::from_config_id("srv1"), new_port: 8080 };
        assert!(format!("{:?}", event).contains("8080"));
    }

    #[test]
    #[allow(unreachable_patterns)]
    fn test_enums_non_exhaustive() {
        let lifecycle = ServerLifecycle::Starting;
        match lifecycle {
            ServerLifecycle::Stopped => {}
            ServerLifecycle::Starting => {}
            ServerLifecycle::Running => {}
            ServerLifecycle::Draining => {}
            ServerLifecycle::Restarting => {}
            ServerLifecycle::Failed => {}
            _ => {} // Enforces non_exhaustive at compile time
        }

        let method = SpeculativeMethod::None;
        match method {
            SpeculativeMethod::None => {}
            SpeculativeMethod::Draft => {}
            SpeculativeMethod::Lookup => {}
            SpeculativeMethod::Mtp => {}
            _ => {}
        }

        let arch = ModelArchitecture::Llama;
        match arch {
            ModelArchitecture::Llama => {}
            ModelArchitecture::Mistral => {}
            ModelArchitecture::Mixtral => {}
            ModelArchitecture::Qwen => {}
            ModelArchitecture::Gemma => {}
            ModelArchitecture::Phi => {}
            ModelArchitecture::DeepSeek => {}
            ModelArchitecture::Other(_) => {}
            _ => {}
        }
    }

    #[test]
    fn test_serde_roundtrip() {
        let method = SpeculativeMethod::Mtp;
        let json = serde_json::to_string(&method).unwrap();
        assert_eq!(json, "\"mtp\"");
        let parsed: SpeculativeMethod = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, method);

        let arch = ModelArchitecture::Other("custom".into());
        let json = serde_json::to_string(&arch).unwrap();
        let parsed: ModelArchitecture = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, arch);
    }
}
