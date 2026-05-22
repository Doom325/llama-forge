mod common;
use common::*;
use llamaforge_core::routing::error::RoutingConstraint;
use llamaforge_core::routing::weights::RoutingWeights;
use llamaforge_core::routing::Router;

#[test]
fn test_routing_with_mock_dispatch_count_view() {
    let mut s1 = default_server("model1");
    s1.idle_slots = 4;
    s1.total_slots = 4;
    s1.kv_usage = 0.0;

    let mut s2 = default_server("model1");
    s2.idle_slots = 4;
    s2.total_slots = 4;
    s2.kv_usage = 0.0;

    let s1_id = s1.id;
    let s2_id = s2.id;

    let state = Arc::new(MockServerStateView::new(vec![s1, s2]));
    let dispatch = Arc::new(MockDispatchCountView::new());

    // s1 has 3 in flight, effective free = 1.
    dispatch.set_count(s1_id, 3);
    // s2 has 1 in flight, effective free = 3.
    dispatch.set_count(s2_id, 1);

    let config = create_test_config();
    let router = Router::new(config, state, dispatch);

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: None,
    };

    let dec = router.select_server(&req).unwrap();
    // s2 has more effective free slots, so it should be selected.
    assert_eq!(*dec.server_id(), s2_id);
}

#[test]
fn test_routing_constraint_exhaustive_match() {
    // This test ensures at compile time that we handle all RoutingConstraint variants.
    let constraint = RoutingConstraint::Architecture;

    let display = match constraint {
        RoutingConstraint::Model => "model",
        RoutingConstraint::LoraAdapter => "lora_adapter",
        RoutingConstraint::Multimodal => "multimodal",
        RoutingConstraint::Embedding => "embedding",
        RoutingConstraint::Reranking => "reranking",
        RoutingConstraint::Architecture => "architecture",
    };

    assert_eq!(display, "architecture");
}

#[test]
fn test_routing_decision_field_encapsulation() {
    let s1 = default_server("model1");
    let state = Arc::new(MockServerStateView::new(vec![s1.clone()]));
    let dispatch = Arc::new(MockDispatchCountView::new());
    let config = create_test_config();
    let router = Router::new(config, state, dispatch);

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: None,
    };

    let dec = router.select_server(&req).unwrap();

    // Test getters
    let _id = dec.server_id();
    let _reason = dec.reason();

    // Attempting to access private fields would cause a compile error:
    // let score = dec.score;
    // let slot_target = dec.slot_target;

    let server_id = dec.into_server_id();
    assert_eq!(server_id, s1.id);
}

#[test]
fn test_slot_target_auto_in_v1_decisions() {
    let s1 = default_server("model1");
    let state = Arc::new(MockServerStateView::new(vec![s1]));
    let dispatch = Arc::new(MockDispatchCountView::new());
    let config = create_test_config();
    let router = Router::new(config, state, dispatch);

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: None,
    };

    let dec = router.select_server(&req).unwrap();
    let debug_str = format!("{:?}", dec);
    assert!(debug_str.contains("Auto"));
}

#[test]
fn test_weights_builder_rejects_negative() {
    let res = RoutingWeights::builder().w_kv(-0.5).build();
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.to_string(), "weight 'w_kv' out of range 0.0..=1.0: -0.5");
}

#[test]
fn test_stale_slot_compensation_floor() {
    let mut s1 = default_server("model1");
    s1.idle_slots = 2;
    s1.total_slots = 4;
    s1.kv_usage = 0.0;
    let s1_id = s1.id;

    let state = Arc::new(MockServerStateView::new(vec![s1]));
    let dispatch = Arc::new(MockDispatchCountView::new());

    // in_flight_count (5) > idle_slots (2) -> effective_free should floor to 0
    dispatch.set_count(s1_id, 5);

    let config = create_test_config();
    let router = Router::new(config, state, dispatch);

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: None,
    };

    // The score formula should use effective_free = 0
    let dec = router.select_server(&req).unwrap();

    // We can't read score directly, but we can verify it doesn't underflow
    // and returns a valid decision. Default weight w_slots=0.4, w_kv=0.2, w_queue=0.2
    // effective_free=0 -> free_ratio=0.0 -> score = 0.2*1.0 + 0.2*1.0 = 0.4
    assert!(matches!(
        dec.reason(),
        llamaforge_core::routing::types::RoutingReason::HighestScore { .. }
    ));
}
