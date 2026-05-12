//! Module: config
use crate::weights::{RoutingWeights, WeightsValidationError};
use serde::Deserialize;

/// Serde DTO for routing weights configuration.
///
/// See design doc Section 2.2.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RoutingWeightsDto {
    pub w_slots: f64,
    pub w_kv: f64,
    pub w_queue: f64,
    pub w_affinity: f64,
}

impl Default for RoutingWeightsDto {
    fn default() -> Self {
        Self { w_slots: 0.4, w_kv: 0.2, w_queue: 0.2, w_affinity: 0.2 }
    }
}

impl TryFrom<RoutingWeightsDto> for RoutingWeights {
    type Error = WeightsValidationError;

    fn try_from(dto: RoutingWeightsDto) -> Result<Self, Self::Error> {
        RoutingWeights::builder()
            .w_slots(dto.w_slots)
            .w_kv(dto.w_kv)
            .w_queue(dto.w_queue)
            .w_affinity(dto.w_affinity)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routing_weights_dto_conversion_valid() {
        let dto = RoutingWeightsDto { w_slots: 0.5, w_kv: 0.1, w_queue: 0.2, w_affinity: 0.2 };
        let weights: RoutingWeights = dto.try_into().unwrap();
        assert_eq!(weights.w_slots(), 0.5);
    }

    #[test]
    fn test_routing_weights_dto_conversion_invalid() {
        let dto = RoutingWeightsDto { w_slots: 1.5, ..Default::default() };
        let res: Result<RoutingWeights, _> = dto.try_into();
        assert!(matches!(res, Err(WeightsValidationError::OutOfRange { field: "w_slots", .. })));
    }
}
