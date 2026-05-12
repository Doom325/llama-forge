use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

pub mod traits;

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
