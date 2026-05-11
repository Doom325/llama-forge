//! Domain types for Server Manager
//! See design doc Section 2.3.

use llamaforge_config::ServerId;
use serde::{Deserialize, Serialize};

/// The lifecycle state of a managed server process.
/// See design doc Section 2.3.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerLifecycle {
    Stopped,
    Starting,
    Running,
    Draining,
    Restarting,
    Failed,
}

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

/// The architecture family of the underlying model.
/// See design doc Section 2.3.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelArchitecture {
    Llama,
    Mistral,
    Mixtral,
    Qwen,
    Gemma,
    Phi,
    DeepSeek,
    Other(String),
}

impl Default for ModelArchitecture {
    fn default() -> Self {
        Self::Other("unknown".into())
    }
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
#[derive(Debug, Clone)]
pub struct SlotState {
    pub id: u32,
    pub is_processing: bool,
    pub n_past: u32,
    pub n_decoded: u32,
    pub prompt_hash: Option<u64>,
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
            n_past: 0,
            n_decoded: 0,
            prompt_hash: Some(12345),
        };
        assert!(format!("{:?}", slot).contains("12345"));

        let event = ClientRefreshEvent { server_id: ServerId("srv1".into()), new_port: 8080 };
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
