use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

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

/// Represents capabilities required by a request.
///
/// See design doc Section 2.2, line 198-199.
#[derive(Debug, Clone, Default)]
pub struct RequestCapabilities {
    pub lora_adapter: Option<LoraAdapterId>,
    pub multimodal: bool,
    pub embedding: bool,
    pub reranking: bool,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SlotId(u32);

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
}
