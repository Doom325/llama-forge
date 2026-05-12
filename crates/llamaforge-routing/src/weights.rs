//! Module: weights

/// Error returned when a routing weight is out of valid bounds.
#[derive(Debug, thiserror::Error)]
pub enum WeightsValidationError {
    #[error("weight '{field}' out of range 0.0..=1.0: {value}")]
    OutOfRange { field: &'static str, value: f64 },
}

/// The weights used for calculating the routing score.
///
/// See design doc Section 2.2.
#[derive(Debug, Clone)]
pub struct RoutingWeights {
    w_slots: f64,
    w_kv: f64,
    w_queue: f64,
    w_affinity: f64,
}

impl RoutingWeights {
    pub fn builder() -> RoutingWeightsBuilder {
        RoutingWeightsBuilder::default()
    }

    pub fn w_slots(&self) -> f64 {
        self.w_slots
    }

    pub fn w_kv(&self) -> f64 {
        self.w_kv
    }

    pub fn w_queue(&self) -> f64 {
        self.w_queue
    }

    pub fn w_affinity(&self) -> f64 {
        self.w_affinity
    }
}

/// Builder for `RoutingWeights`.
#[derive(Debug, Clone)]
pub struct RoutingWeightsBuilder {
    w_slots: f64,
    w_kv: f64,
    w_queue: f64,
    w_affinity: f64,
}

impl Default for RoutingWeightsBuilder {
    fn default() -> Self {
        Self { w_slots: 0.4, w_kv: 0.2, w_queue: 0.2, w_affinity: 0.2 }
    }
}

impl RoutingWeightsBuilder {
    pub fn w_slots(mut self, val: f64) -> Self {
        self.w_slots = val;
        self
    }

    pub fn w_kv(mut self, val: f64) -> Self {
        self.w_kv = val;
        self
    }

    pub fn w_queue(mut self, val: f64) -> Self {
        self.w_queue = val;
        self
    }

    pub fn w_affinity(mut self, val: f64) -> Self {
        self.w_affinity = val;
        self
    }

    pub fn build(self) -> Result<RoutingWeights, WeightsValidationError> {
        let validate_range = |field: &'static str, value: f64| {
            if !(0.0..=1.0).contains(&value) {
                Err(WeightsValidationError::OutOfRange { field, value })
            } else {
                Ok(())
            }
        };

        validate_range("w_slots", self.w_slots)?;
        validate_range("w_kv", self.w_kv)?;
        validate_range("w_queue", self.w_queue)?;
        validate_range("w_affinity", self.w_affinity)?;

        let sum = self.w_slots + self.w_kv + self.w_queue + self.w_affinity;
        if (sum - 1.0).abs() > f64::EPSILON {
            tracing::warn!("Routing weights sum to {}, expected 1.0", sum);
        }

        Ok(RoutingWeights {
            w_slots: self.w_slots,
            w_kv: self.w_kv,
            w_queue: self.w_queue,
            w_affinity: self.w_affinity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routing_weights_builder_rejects_out_of_range() {
        let res = RoutingWeights::builder().w_slots(1.5).build();
        assert!(matches!(res, Err(WeightsValidationError::OutOfRange { field: "w_slots", .. })));
    }

    #[test]
    #[tracing_test::traced_test]
    fn test_routing_weights_builder_warns_non_unit_sum() {
        let weights = RoutingWeights::builder()
            .w_slots(0.1)
            .w_kv(0.1)
            .w_queue(0.1)
            .w_affinity(0.2)
            .build()
            .unwrap();
        assert_eq!(weights.w_slots(), 0.1);
        assert_eq!(weights.w_kv(), 0.1);
        assert_eq!(weights.w_queue(), 0.1);
        assert_eq!(weights.w_affinity(), 0.2);

        assert!(logs_contain("Routing weights sum to 0.5, expected 1.0"));
    }

    #[test]
    fn test_routing_weights_builder_default_values() {
        let weights = RoutingWeights::builder().build().unwrap();
        assert_eq!(weights.w_slots(), 0.4);
        assert_eq!(weights.w_kv(), 0.2);
        assert_eq!(weights.w_queue(), 0.2);
        assert_eq!(weights.w_affinity(), 0.2);
    }
}
