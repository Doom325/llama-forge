//! Module: router

use crate::error::RoutingConstraint;
use crate::types::{RoutingReason, ServerSnapshot, ServerStateView, SlotTarget};
use crate::{
    CapacityChecker, DispatchCountView, RequestCapabilities, RequestView, RouteValidator,
    RoutingDecision, RoutingError, ServerSelector,
};
use dashmap::DashMap;
use llamaforge_config::{ConfigHandle, ConfigReader};
use llamaforge_types::{ModelAlias, ModelTarget, ServerId};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The main routing engine.
///
/// See design doc Section 3.1.
pub struct Router {
    config: Arc<ConfigHandle>,
    server_state: Arc<dyn ServerStateView>,
    dispatch_counts: Arc<dyn DispatchCountView>,
    round_robin: DashMap<ModelAlias, AtomicU64>,
    affinity_cache: std::sync::Mutex<crate::affinity::SessionAffinityCache>,
    started_at: std::time::Instant,
}

struct ScoredCandidate {
    server: ServerSnapshot,
    score: f64,
}

impl Router {
    pub fn new(
        config: Arc<ConfigHandle>,
        server_state: Arc<dyn ServerStateView>,
        dispatch_counts: Arc<dyn DispatchCountView>,
    ) -> Self {
        let max_entries = config.snapshot().routing.session_affinity_max_entries;
        Self {
            config,
            server_state,
            dispatch_counts,
            round_robin: DashMap::new(),
            affinity_cache: std::sync::Mutex::new(crate::affinity::SessionAffinityCache::new(
                max_entries as usize,
            )),
            started_at: std::time::Instant::now(),
        }
    }

    /// Safely gets an affinity entry, handling Mutex poison.
    #[allow(dead_code)]
    fn get_affinity(
        &self,
        session_id: &llamaforge_types::SessionId,
        available: &[ServerId],
    ) -> Option<ServerId> {
        let mut guard = match self.affinity_cache.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!("session affinity cache poisoned, degrading to score-only routing");
                poisoned.into_inner()
            }
        };
        guard.get(session_id, available)
    }

    /// Records a routing decision into the affinity cache.
    #[allow(dead_code)]
    fn set_affinity(&self, session_id: llamaforge_types::SessionId, server_id: ServerId) {
        let mut guard = match self.affinity_cache.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!("session affinity cache poisoned, degrading to score-only routing");
                poisoned.into_inner()
            }
        };
        guard.set(session_id, server_id);
    }

    /// Invalidates all affinity entries pointing to a server.
    pub fn invalidate_affinity(&self, server_id: &ServerId) {
        let mut guard = match self.affinity_cache.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                tracing::warn!("session affinity cache poisoned, degrading to score-only routing");
                poisoned.into_inner()
            }
        };
        guard.invalidate_server(server_id);
    }

    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn poison_affinity_cache_for_test(&self) {
        let _guard = self.affinity_cache.lock().unwrap();
        panic!("poisoning mutex");
    }

    /// Hard constraint filter.
    fn eligible_servers(
        &self,
        model: &ModelTarget,
        capabilities: &RequestCapabilities,
    ) -> Result<Vec<ServerSnapshot>, RoutingError> {
        let all_servers = self.server_state.all_servers();

        let mut model_matches = Vec::new();
        for server in all_servers {
            let serves_model = match model {
                ModelTarget::Exact(alias) => server.models.contains(alias),
                ModelTarget::AnyOf(aliases) => {
                    aliases.iter().any(|alias| server.models.contains(alias))
                }
                ModelTarget::AnyWithCapability(cap) => server.capabilities.contains(cap),
            };
            if serves_model {
                model_matches.push(server);
            }
        }

        if model_matches.is_empty() {
            let model_str = match model {
                ModelTarget::Exact(a) => a.as_str().to_string(),
                ModelTarget::AnyOf(a) => format!("{:?}", a),
                ModelTarget::AnyWithCapability(c) => format!("{:?}", c),
            };
            return Err(RoutingError::ModelNotFound { model: model_str });
        }

        let mut running = Vec::new();
        let mut server_states = Vec::new();
        for server in &model_matches {
            server_states.push((server.id, server.lifecycle));
            if server.lifecycle == llamaforge_types::ServerLifecycle::Running {
                running.push(server.clone());
            }
        }

        if running.is_empty() {
            let model_str = match model {
                ModelTarget::Exact(a) => a.as_str().to_string(),
                ModelTarget::AnyOf(a) => format!("{:?}", a),
                ModelTarget::AnyWithCapability(c) => format!("{:?}", c),
            };
            return Err(RoutingError::ModelUnavailable { model: model_str, server_states });
        }

        let mut eligible = Vec::new();
        let mut failed_constraint = None;
        let mut requested_value = String::new();

        for server in running {
            let mut server_capable = true;

            if let Some(lora) = &capabilities.lora_adapter {
                if !server.lora_adapters.contains(lora) {
                    server_capable = false;
                    failed_constraint = Some(RoutingConstraint::LoraAdapter);
                    requested_value = lora.as_str().to_string();
                }
            }
            if server_capable && capabilities.multimodal && !server.multimodal {
                server_capable = false;
                failed_constraint = Some(RoutingConstraint::Multimodal);
                requested_value = "true".to_string();
            }
            if server_capable && capabilities.embedding && !server.embedding {
                server_capable = false;
                failed_constraint = Some(RoutingConstraint::Embedding);
                requested_value = "true".to_string();
            }
            if server_capable && capabilities.reranking && !server.reranking {
                server_capable = false;
                failed_constraint = Some(RoutingConstraint::Reranking);
                requested_value = "true".to_string();
            }

            if server_capable {
                if let Some(arch) = &capabilities.architecture {
                    if server.architecture != *arch {
                        server_capable = false;
                        failed_constraint = Some(RoutingConstraint::Architecture);
                        requested_value = format!("{:?}", arch);
                    }
                }
            }

            if server_capable {
                eligible.push(server);
            }
        }

        if eligible.is_empty() {
            if let Some(constraint) = failed_constraint {
                return Err(RoutingError::NoCapableServer {
                    constraint,
                    requested: requested_value,
                });
            }
        }

        Ok(eligible)
    }

    /// Scoring formula.
    fn score_server(
        &self,
        server: &ServerSnapshot,
        weights: &crate::weights::RoutingWeights,
    ) -> f64 {
        let total_slots = server.total_slots;
        let free_ratio = if total_slots == 0 {
            0.0
        } else {
            let in_flight = self.dispatch_counts.in_flight_count(&server.id);
            let effective_free = server.idle_slots.saturating_sub(in_flight as u32);
            effective_free as f64 / total_slots as f64
        };

        let is_recurrent = server.architecture == llamaforge_types::ModelArchitecture::Recurrent;

        let (kv_component, affinity_component) = if is_recurrent {
            (0.0, 0.0)
        } else {
            (
                weights.w_kv() * (1.0 - server.kv_usage),
                weights.w_affinity() * 0.0, // v1 placeholder
            )
        };

        let score = weights.w_slots() * free_ratio
            + kv_component
            + weights.w_queue() * (1.0 - server.deferred_ratio)
            + affinity_component;

        debug_assert!(score.is_finite());
        score
    }

    /// Round-robin tie-breaking.
    fn break_tie(&self, alias: &ModelAlias, candidates: &[ScoredCandidate]) -> (usize, u64) {
        let counter = self.round_robin.entry(alias.clone()).or_default();
        let val = counter.fetch_add(1, Ordering::Relaxed);
        ((val % candidates.len() as u64) as usize, val)
    }

    /// Check if we have any capacity for the model.
    pub fn has_capacity(&self, target: &ModelTarget) -> bool {
        // Fast path check without constraint checking (since capabilities might not be known yet).
        let all_servers = self.server_state.all_servers();
        let mut model_matches = Vec::new();
        for server in all_servers {
            let serves_model = match target {
                ModelTarget::Exact(alias) => server.models.contains(alias),
                ModelTarget::AnyOf(aliases) => {
                    aliases.iter().any(|alias| server.models.contains(alias))
                }
                ModelTarget::AnyWithCapability(cap) => server.capabilities.contains(cap),
            };
            if serves_model && server.lifecycle == llamaforge_types::ServerLifecycle::Running {
                model_matches.push(server);
            }
        }

        for server in model_matches {
            let in_flight = self.dispatch_counts.in_flight_count(&server.id);
            let effective_free = server.idle_slots.saturating_sub(in_flight as u32);
            if effective_free > 0 {
                return true;
            }
        }
        false
    }

    /// Validate if it's even possible to route this request.
    pub fn validate_routable(
        &self,
        model: &ModelTarget,
        capabilities: &RequestCapabilities,
    ) -> Result<(), RoutingError> {
        let res = self.eligible_servers(model, capabilities);
        match res {
            Ok(_) => Ok(()),
            Err(e) => {
                if let RoutingError::ModelUnavailable { ref server_states, .. } = e {
                    let all_starting = !server_states.is_empty()
                        && server_states.iter().all(|(_, state)| {
                            *state == llamaforge_types::ServerLifecycle::Starting
                        });
                    if all_starting {
                        let wait_secs = self.config.snapshot().routing.startup_wait_secs;
                        if self.started_at.elapsed().as_secs() < wait_secs {
                            return Ok(());
                        }
                    }
                }

                metrics::counter!("llamaforge_routing_rejections_total", "reason" => match &e {
                    RoutingError::ModelNotFound { .. } => "not_found",
                    RoutingError::NoCapableServer { .. } => "no_capable",
                    RoutingError::ModelUnavailable { .. } => "unavailable",
                })
                .increment(1);

                if let RoutingError::NoCapableServer { constraint, requested } = &e {
                    tracing::warn!(
                        model = ?model,
                        constraint = %constraint,
                        requested = %requested,
                        "routing.rejected"
                    );
                } else {
                    tracing::warn!(
                        model = ?model,
                        error = %e,
                        "routing.rejected"
                    );
                }

                Err(e)
            }
        }
    }

    /// Selects the best server for a given request.
    pub fn select_server(
        &self,
        request: &dyn RequestView,
    ) -> Result<RoutingDecision, RoutingError> {
        let config = self.config.snapshot();
        let model = request.model_target();
        let capabilities = request.capabilities();

        let eligible = self.eligible_servers(model, capabilities)?;

        let weights = match config.routing.weights.clone().try_into() {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "routing.weights_invalid");
                crate::weights::RoutingWeights::default()
            }
        };

        // Score servers
        let candidates: Vec<ScoredCandidate> = eligible
            .into_iter()
            .map(|server| {
                let score = self.score_server(&server, &weights);
                ScoredCandidate { server, score }
            })
            .collect();

        // Find max score
        let max_score = candidates.iter().map(|c| c.score).fold(f64::NEG_INFINITY, f64::max);

        // Filter to those tying for max score (allowing for floating point inaccuracy)
        let mut top_candidates = Vec::new();
        for cand in candidates {
            if (cand.score - max_score).abs() < f64::EPSILON {
                top_candidates.push(cand);
            }
        }

        let is_tie = top_candidates.len() > 1;

        let (best_candidate, reason) = if is_tie {
            metrics::counter!("llamaforge_routing_decisions_total", "reason" => "round_robin")
                .increment(1);
            let alias = match model {
                ModelTarget::Exact(a) => a.clone(),
                ModelTarget::AnyOf(aliases) => aliases.first().cloned().unwrap_or_else(|| {
                    top_candidates[0]
                        .server
                        .models
                        .iter()
                        .next()
                        .cloned()
                        .unwrap_or_else(|| ModelAlias::new(std::sync::Arc::from("unknown")))
                }),
                ModelTarget::AnyWithCapability(_) => top_candidates[0]
                    .server
                    .models
                    .iter()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| ModelAlias::new(std::sync::Arc::from("unknown"))),
            };
            let (idx, counter) = self.break_tie(&alias, &top_candidates);
            (&top_candidates[idx], RoutingReason::RoundRobin { counter })
        } else {
            metrics::counter!("llamaforge_routing_decisions_total", "reason" => "score")
                .increment(1);
            (&top_candidates[0], RoutingReason::HighestScore { score: top_candidates[0].score })
        };

        let server_id = best_candidate.server.id;
        let score = best_candidate.score;

        metrics::histogram!("llamaforge_routing_score", "server_id" => server_id.to_string())
            .record(score);

        tracing::debug!(
            server_id = %server_id,
            score = %score,
            reason = ?reason,
            "routing.selected"
        );

        if let Some(session_id) = request.session_id() {
            self.set_affinity(session_id.clone(), server_id);
        }

        Ok(RoutingDecision::new(server_id, reason, score, SlotTarget::Auto))
    }
}

impl CapacityChecker for Router {
    fn has_capacity(&self, target: &ModelTarget) -> bool {
        self.has_capacity(target)
    }
}

impl ServerSelector for Router {
    fn select_server(&self, request: &dyn RequestView) -> Result<RoutingDecision, RoutingError> {
        self.select_server(request)
    }
}

impl RouteValidator for Router {
    fn validate_routable(
        &self,
        model: &ModelTarget,
        capabilities: &RequestCapabilities,
    ) -> Result<(), RoutingError> {
        self.validate_routable(model, capabilities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llamaforge_types::LoraAdapterId;
    use std::collections::HashSet;

    struct MockServerStateView {
        servers: Vec<ServerSnapshot>,
    }

    impl ServerStateView for MockServerStateView {
        fn all_servers(&self) -> Vec<ServerSnapshot> {
            self.servers.clone()
        }
    }

    struct MockDispatchCountView {
        counts: std::collections::HashMap<ServerId, usize>,
    }

    impl MockDispatchCountView {
        fn new() -> Self {
            Self { counts: std::collections::HashMap::new() }
        }
    }

    impl DispatchCountView for MockDispatchCountView {
        fn in_flight_count(&self, server_id: &ServerId) -> usize {
            self.counts.get(server_id).cloned().unwrap_or(0)
        }
    }

    fn create_mock_router(servers: Vec<ServerSnapshot>, dispatch: MockDispatchCountView) -> Router {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "[global]\n").unwrap();
        let paths = llamaforge_config::ConfigPaths::new(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            tmp.path().parent().unwrap().to_path_buf(),
        );
        let source = Arc::new(llamaforge_config::TomlFileConfigSource::new(paths));
        let config = Arc::new(ConfigHandle::load(source).unwrap());

        Router::new(config, Arc::new(MockServerStateView { servers }), Arc::new(dispatch))
    }

    fn default_server(_id: u64, alias: &str) -> ServerSnapshot {
        let mut models = HashSet::new();
        models.insert(ModelAlias::new(Arc::from(alias)));
        ServerSnapshot {
            id: ServerId::new(uuid::Uuid::now_v7()),
            lifecycle: llamaforge_types::ServerLifecycle::Running,
            models,
            capabilities: HashSet::new(),
            lora_adapters: HashSet::new(),
            multimodal: false,
            embedding: false,
            reranking: false,
            architecture: llamaforge_types::ModelArchitecture::Llama,
            total_slots: 4,
            idle_slots: 4,
            kv_usage: 0.0,
            deferred_ratio: 0.0,
        }
    }

    struct MockRequest {
        model: ModelTarget,
        caps: RequestCapabilities,
        session: Option<llamaforge_types::SessionId>,
    }
    impl RequestView for MockRequest {
        fn model_target(&self) -> &ModelTarget {
            &self.model
        }
        fn capabilities(&self) -> &RequestCapabilities {
            &self.caps
        }
        fn session_id(&self) -> Option<&llamaforge_types::SessionId> {
            self.session.as_ref()
        }
    }

    #[test]
    fn test_prefers_server_with_more_free_slots() {
        let mut s1 = default_server(1, "model1");
        s1.idle_slots = 3;
        let mut s2 = default_server(2, "model1");
        s2.idle_slots = 1;
        let s1_id = s1.id;

        let router = create_mock_router(vec![s1, s2], MockDispatchCountView::new());
        let req = MockRequest {
            model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
            caps: Default::default(),
            session: None,
        };
        let decision = router.select_server(&req).unwrap();
        assert_eq!(*decision.server_id(), s1_id);
    }

    #[test]
    fn test_dispatch_tracker_compensation() {
        let mut s1 = default_server(1, "model1");
        s1.idle_slots = 3;

        let mut dispatch = MockDispatchCountView::new();
        dispatch.counts.insert(s1.id, 2);

        let router = create_mock_router(vec![s1.clone()], dispatch);
        let score = router.score_server(&s1, &crate::weights::RoutingWeights::default());

        // Effective free = max(0, 3 - 2) = 1.
        // free_ratio = 1 / 4 = 0.25.
        // weight defaults: w_slots 0.4, w_kv 0.2, w_queue 0.2
        // score = 0.4 * 0.25 + 0.2 * (1.0) + 0.2 * (1.0) = 0.1 + 0.2 + 0.2 = 0.5
        assert!((score - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_model_not_found_at_admission() {
        let router = create_mock_router(vec![], MockDispatchCountView::new());
        let model = ModelTarget::Exact(ModelAlias::new(Arc::from("nonexistent-model")));
        let res = router.validate_routable(&model, &Default::default());
        assert!(matches!(res, Err(RoutingError::ModelNotFound { .. })));
    }

    #[test]
    fn test_round_robin_on_tie() {
        let s1 = default_server(1, "model1");
        let s2 = default_server(2, "model1");
        let s1_id = s1.id;
        let s2_id = s2.id;
        let router = create_mock_router(vec![s1, s2], MockDispatchCountView::new());

        let req = MockRequest {
            model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
            caps: Default::default(),
            session: None,
        };

        let mut counts = std::collections::HashMap::new();
        for _ in 0..4 {
            let decision = router.select_server(&req).unwrap();
            *counts.entry(*decision.server_id()).or_insert(0) += 1;
        }

        assert_eq!(counts.get(&s1_id).cloned().unwrap_or(0), 2);
        assert_eq!(counts.get(&s2_id).cloned().unwrap_or(0), 2);
    }

    #[test]
    fn test_zero_total_slots_returns_zero_score() {
        let mut s1 = default_server(1, "model1");
        s1.total_slots = 0;

        let router = create_mock_router(vec![s1.clone()], MockDispatchCountView::new());
        let score = router.score_server(&s1, &crate::weights::RoutingWeights::default());

        assert!(score.is_finite());
        // w_slots * 0.0 + 0.2 + 0.2 = 0.4
        assert!((score - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn test_recurrent_model_zeroes_kv_affinity() {
        let mut s1 = default_server(1, "model1");
        s1.architecture = llamaforge_types::ModelArchitecture::Recurrent;

        let router = create_mock_router(vec![s1.clone()], MockDispatchCountView::new());
        let score = router.score_server(&s1, &crate::weights::RoutingWeights::default());

        // score = 0.4 * (4/4) + 0.0 (KV) + 0.2 * (1.0) + 0.0 = 0.6
        assert!((score - 0.6).abs() < f64::EPSILON, "score was {}", score);
    }

    #[test]
    fn test_has_capacity_with_tracker_compensation() {
        let mut s1 = default_server(1, "model1");
        s1.idle_slots = 4;
        let mut dispatch = MockDispatchCountView::new();
        dispatch.counts.insert(s1.id, 4);

        let router = create_mock_router(vec![s1.clone()], dispatch);
        let model = ModelTarget::Exact(ModelAlias::new(Arc::from("model1")));

        assert!(!router.has_capacity(&model));

        let mut dispatch2 = MockDispatchCountView::new();
        dispatch2.counts.insert(s1.id, 3);
        let router2 = create_mock_router(vec![s1], dispatch2);
        assert!(router2.has_capacity(&model));
    }

    #[test]
    fn test_validate_routable_lora_constraint() {
        let s1 = default_server(1, "model1");
        let router = create_mock_router(vec![s1], MockDispatchCountView::new());

        let model = ModelTarget::Exact(ModelAlias::new(Arc::from("model1")));
        let caps = RequestCapabilities {
            lora_adapter: Some(LoraAdapterId::new("my-adapter").unwrap()),
            ..Default::default()
        };

        let res = router.validate_routable(&model, &caps);
        assert!(matches!(
            res,
            Err(RoutingError::NoCapableServer { constraint: RoutingConstraint::LoraAdapter, .. })
        ));
    }
}
