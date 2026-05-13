//! Module: config
use crate::weights::{RoutingWeights, WeightsValidationError};

impl TryFrom<llamaforge_config::RoutingWeightsConfig> for RoutingWeights {
    type Error = WeightsValidationError;

    fn try_from(config: llamaforge_config::RoutingWeightsConfig) -> Result<Self, Self::Error> {
        RoutingWeights::builder()
            .w_slots(config.w_slots)
            .w_kv(config.w_kv)
            .w_queue(config.w_queue)
            .w_affinity(config.w_affinity)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routing_weights_dto_conversion_valid() {
        let config = llamaforge_config::RoutingWeightsConfig {
            w_slots: 0.5,
            w_kv: 0.1,
            w_queue: 0.2,
            w_affinity: 0.2,
        };
        let weights: RoutingWeights = config.try_into().unwrap();
        assert_eq!(weights.w_slots(), 0.5);
    }

    #[test]
    fn test_routing_weights_dto_conversion_invalid() {
        let config = llamaforge_config::RoutingWeightsConfig { w_slots: 1.5, ..Default::default() };
        let res: Result<RoutingWeights, _> = config.try_into();
        assert!(matches!(res, Err(WeightsValidationError::OutOfRange { field: "w_slots", .. })));
    }
}
