//! Core domain types — request/response shapes, IDs, port traits.
//!
//! Implements design.md §6.1 (Core Types). Used by every component
//! crate; the only crate that depends on no other LlamaForge crate.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

pub mod ports;
pub mod traits;

/// Error returned when an ID is invalid.
#[derive(Debug, thiserror::Error)]
pub enum IdValidationError {
    #[error("ID cannot be empty or just whitespace")]
    Empty,
    #[error("ID exceeds maximum length of 128 characters")]
    TooLong,
}

/// Validates an ID string.
pub fn validate_id(id: &str) -> Result<(), IdValidationError> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(IdValidationError::Empty);
    }
    if trimmed.len() > 128 {
        return Err(IdValidationError::TooLong);
    }
    Ok(())
}

/// A unique identifier for a session, used for session affinity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(Arc<str>);

impl SessionId {
    pub fn new(id: impl AsRef<str>) -> Result<Self, IdValidationError> {
        let s = id.as_ref();
        validate_id(s)?;
        Ok(Self(Arc::from(s.trim())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A unique identifier for a LoRA adapter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct LoraAdapterId(Arc<str>);

impl LoraAdapterId {
    pub fn new(id: impl AsRef<str>) -> Result<Self, IdValidationError> {
        let s = id.as_ref();
        validate_id(s)?;
        Ok(Self(Arc::from(s.trim())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for LoraAdapterId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        LoraAdapterId::new(s).map_err(serde::de::Error::custom)
    }
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
    Recurrent,
    Other(String),
}

impl Default for ModelArchitecture {
    fn default() -> Self {
        Self::Other("unknown".into())
    }
}

/// Represents capabilities required by a request.
///
/// See design doc Section 2.2, line 198-199.
#[derive(Debug, Clone, Default)]
pub struct RequestCapabilities {
    pub lora_adapter: Option<LoraAdapterId>,
    pub multimodal: bool,
    pub embedding: bool,
    pub reranking: bool,
    pub architecture: Option<ModelArchitecture>,
}

/// A unique identifier for a single request.
///
/// See design doc Section 2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId(Uuid);

impl RequestId {
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
    pub fn into_inner(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identifies a specific slot on a specific server.
///
/// See design doc Section 2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SlotId(pub(crate) u32);

impl SlotId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }
    pub fn into_inner(self) -> u32 {
        self.0
    }
}

impl fmt::Display for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A unique identifier for a `llama-server` process.
///
/// See design doc Section 2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerId(Uuid);

/// The lifecycle state of a managed server process.
/// See design doc Section 2.3.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerLifecycle {
    Stopped,
    Starting,
    Running,
    Draining,
    Restarting,
    Failed,
}

impl ServerId {
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
    pub fn into_inner(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Identifies the caller or user making a request.
///
/// See design doc Section 2.3.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestorId(Arc<str>);

impl RequestorId {
    pub fn new(id: Arc<str>) -> Self {
        Self(id)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_inner(self) -> Arc<str> {
        self.0
    }
}

impl fmt::Display for RequestorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The class of a requestor, used to lookup policy (e.g. rate limits).
///
/// See design doc Section 2.3.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestorClass(Arc<str>);

impl RequestorClass {
    pub fn new(class: Arc<str>) -> Self {
        Self(class)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_inner(self) -> Arc<str> {
        self.0
    }
}

impl fmt::Display for RequestorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The short name or alias of a model (e.g., "llama-3-8b-instruct").
///
/// See design doc Section 2.3.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelAlias(Arc<str>);

impl ModelAlias {
    pub fn new(alias: Arc<str>) -> Self {
        Self(alias)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_inner(self) -> Arc<str> {
        self.0
    }
}

impl fmt::Display for ModelAlias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Target model requirements for a request.
///
/// See design doc Section 2.3.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelTarget {
    Exact(ModelAlias),
    AnyOf(Vec<ModelAlias>),
    AnyWithCapability(Capability),
}

/// A capability requirement for a model.
///
/// See design doc Section 2.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    CodeGeneration,
    Chat,
    Embedding,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_id_validation() {
        assert!(SessionId::new("valid-id").is_ok());
        assert!(matches!(SessionId::new(""), Err(IdValidationError::Empty)));
        assert!(matches!(SessionId::new("   "), Err(IdValidationError::Empty)));

        let too_long = "a".repeat(129);
        assert!(matches!(SessionId::new(&too_long), Err(IdValidationError::TooLong)));
    }

    #[test]
    fn test_lora_adapter_id_serde_roundtrip() {
        let id = LoraAdapterId::new("adapter-1").unwrap();
        let serialized = serde_json::to_string(&id).unwrap();
        assert_eq!(serialized, "\"adapter-1\"");

        let deserialized: LoraAdapterId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(id, deserialized);

        // Invalid input (empty string)
        let res: Result<LoraAdapterId, _> = serde_json::from_str("\"   \"");
        assert!(res.is_err());
    }

    use crate::traits::{QueueSnapshot, ServerSummary};
    use std::collections::HashMap;

    #[test]
    fn test_server_summary_serde_roundtrip() {
        let summary = ServerSummary {
            id: ServerId::new(uuid::Uuid::now_v7()),
            alias: ModelAlias::new("test-alias".into()),
            state: ServerLifecycle::Running,
            port: 8080,
            slots_total: 4,
            slots_busy: 2,
            started_at: Some(chrono::Utc::now()),
            last_restart_at: None,
        };
        let serialized = serde_json::to_string(&summary).unwrap();
        let deserialized: ServerSummary = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.id, summary.id);
        assert_eq!(deserialized.state, summary.state);
        assert_eq!(deserialized.port, summary.port);
    }

    #[test]
    fn test_queue_snapshot_serde_roundtrip() {
        let snapshot = QueueSnapshot { depth: 42, estimated_throughput_req_sec: 1.5 };
        let serialized = serde_json::to_string(&snapshot).unwrap();
        let deserialized: QueueSnapshot = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.depth, snapshot.depth);
        assert_eq!(
            deserialized.estimated_throughput_req_sec,
            snapshot.estimated_throughput_req_sec
        );
    }

    #[test]
    fn test_generation_params_serde_roundtrip() {
        let mut logit_bias = HashMap::new();
        logit_bias.insert(42, 10.5);
        let params = GenerationParams {
            max_tokens: Some(100),
            temperature: Some(0.8),
            top_p: Some(0.9),
            top_k: Some(40),
            logit_bias: Some(logit_bias),
            stop: vec!["stop".to_string()],
            seed: Some(12345),
        };
        let serialized = serde_json::to_string(&params).unwrap();
        let deserialized: GenerationParams = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.max_tokens, params.max_tokens);
        assert_eq!(deserialized.stop, params.stop);
        assert_eq!(deserialized.seed, params.seed);
    }
}

/// TLS configuration for backend connection.
#[derive(Debug, Clone)]
pub struct TlsConfig;

/// Event emitted when a server restarts, signaling that clients should reconnect.
#[derive(Debug, Clone)]
pub struct ClientRefreshEvent {
    pub server_id: ServerId,
    pub new_port: u16,
    pub tls_config: Option<TlsConfig>,
}

// --- Added for Transport Layer ---

/// Domain type representing a message role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Domain type representing a single chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// Domain type representing an incoming chat completion request.
#[derive(Debug, Clone)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
    pub logit_bias: Option<std::collections::HashMap<i32, f64>>,
    pub tools: Option<serde_json::Value>,
    pub tool_choice: Option<serde_json::Value>,
}

/// Domain type representing a chunk of a chat completion response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ChatCompletionChunk {
    /// A queue update without generating tokens.
    QueueUpdate { position: u32 },
    /// A generated token.
    Token(String),
    /// The final chunk with a finish reason and the cumulative
    /// generated-token count for the request. The token count is what
    /// the Anthropic shim reports as `usage.output_tokens` and what
    /// OpenAI clients receive on the terminal chunk.
    Stop { finish_reason: String, output_tokens: u32 },
}

/// Error type for stream failures.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("Backend disconnected")]
    BackendDisconnect,
    #[error("Timeout")]
    Timeout,
    #[error("Cancelled")]
    Cancelled,
    #[error("Chunk parse error: {reason}")]
    ChunkParse { reason: String },
    #[error("Upstream error: {0}")]
    Upstream(Box<dyn std::error::Error + Send + Sync>),
}

/// Generation parameters for a request.
/// See design doc §5.4.2.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GenerationParams {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub logit_bias: Option<std::collections::HashMap<i32, f64>>,
    #[serde(default)]
    pub stop: Vec<String>,
    pub seed: Option<u64>,
}
