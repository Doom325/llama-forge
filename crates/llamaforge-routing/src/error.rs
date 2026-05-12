//! Module: error
use llamaforge_types::ServerId;

/// Constraint that caused a routing failure.
///
/// See design doc Section 2.2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingConstraint {
    Model,
    LoraAdapter,
    Multimodal,
    Embedding,
    Reranking,
    Architecture,
}

impl std::fmt::Display for RoutingConstraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Model => "model",
            Self::LoraAdapter => "lora_adapter",
            Self::Multimodal => "multimodal",
            Self::Embedding => "embedding",
            Self::Reranking => "reranking",
            Self::Architecture => "architecture",
        };
        write!(f, "{}", s)
    }
}

/// Errors that can occur during routing decisions.
///
/// See design doc Section 2.2.
#[derive(Debug, thiserror::Error)]
pub enum RoutingError {
    #[error("Model not found: {model}")]
    ModelNotFound { model: String },

    #[error("No capable server found for constraint {constraint} requested: {requested}")]
    NoCapableServer { constraint: RoutingConstraint, requested: String },

    #[error("Model {model} is currently unavailable. Server states: {server_states:?}")]
    ModelUnavailable {
        model: String,
        // TODO(PR3): change String to ServerLifecycle once available or imported
        server_states: Vec<(ServerId, String)>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routing_constraint_display() {
        assert_eq!(RoutingConstraint::Model.to_string(), "model");
        assert_eq!(RoutingConstraint::LoraAdapter.to_string(), "lora_adapter");
        assert_eq!(RoutingConstraint::Multimodal.to_string(), "multimodal");
        assert_eq!(RoutingConstraint::Embedding.to_string(), "embedding");
        assert_eq!(RoutingConstraint::Reranking.to_string(), "reranking");
        assert_eq!(RoutingConstraint::Architecture.to_string(), "architecture");

        // Exhaustive match to ensure all variants are covered
        let constraint = RoutingConstraint::Model;
        match constraint {
            RoutingConstraint::Model => {}
            RoutingConstraint::LoraAdapter => {}
            RoutingConstraint::Multimodal => {}
            RoutingConstraint::Embedding => {}
            RoutingConstraint::Reranking => {}
            RoutingConstraint::Architecture => {}
        }
    }
}
