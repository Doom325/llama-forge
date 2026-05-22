mod common;
use common::*;
use llamaforge_core::routing::Router;
use llamaforge_core::types::LoraAdapterId;
use llamaforge_core::types::SessionId;
use std::sync::Arc;

// VC-2.1
#[test]
fn test_vc2_1_scoring_correctness() {
    let mut s1 = default_server("model1");
    // Server A: 3/4 free, 0.1 KV, 0.0 deferred
    s1.idle_slots = 3;
    s1.total_slots = 4;
    s1.kv_usage = 0.1;
    s1.deferred_ratio = 0.0;

    let mut s2 = default_server("model1");
    // Server B: 1/4 free, 0.5 KV, 0.3 deferred
    s2.idle_slots = 1;
    s2.total_slots = 4;
    s2.kv_usage = 0.5;
    s2.deferred_ratio = 0.3;

    let s1_id = s1.id;

    let state = Arc::new(MockServerStateView::new(vec![s1, s2]));
    let dispatch = Arc::new(MockDispatchCountView::new());
    let config = create_test_config();
    let router = Router::new(config, state, dispatch);

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: None,
    };

    let dec = router.select_server(&req).unwrap();
    // A should score higher and be selected
    assert_eq!(*dec.server_id(), s1_id);
}

// VC-2.2
#[test]
fn test_vc2_2_dispatch_compensation() {
    let mut s1 = default_server("model1");
    // 4 slots, 3 idle
    s1.idle_slots = 3;
    s1.total_slots = 4;
    let s1_id = s1.id;

    let state = Arc::new(MockServerStateView::new(vec![s1]));
    let dispatch = Arc::new(MockDispatchCountView::new());

    // in_flight_count = 3
    dispatch.set_count(s1_id, 3);

    let config = create_test_config();
    let router = Router::new(config, state, dispatch);

    let model_target = ModelTarget::Exact(ModelAlias::new(Arc::from("model1")));

    // effective_free = 0, has_idle_slot = false
    assert!(!router.has_idle_slot(&model_target, &Default::default()));
}

// VC-2.3
#[test]
fn test_vc2_3_session_affinity_lifecycle() {
    let mut s1 = default_server("model1");
    s1.idle_slots = 4;
    let mut s2 = default_server("model1");
    s2.idle_slots = 1;

    let s1_id = s1.id;
    let _s2_id = s2.id;

    let state = Arc::new(MockServerStateView::new(vec![s1, s2]));
    let dispatch = Arc::new(MockDispatchCountView::new());
    let config = create_test_config();
    let router = Router::new(config, state, dispatch);

    let session_id = SessionId::new("sess_vc2_3").unwrap();

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: Some(session_id.clone()),
    };

    // 1. Initial request -> no affinity, scores and picks s1 (4 slots vs 1)
    let dec1 = router.select_server(&req).unwrap();
    assert_eq!(*dec1.server_id(), s1_id);
    assert!(matches!(
        dec1.reason(),
        llamaforge_core::routing::types::RoutingReason::HighestScore { .. }
    ));

    // 2. Next request -> affinity hit for s1
    let dec2 = router.select_server(&req).unwrap();
    assert_eq!(*dec2.server_id(), s1_id);
    assert!(matches!(
        dec2.reason(),
        llamaforge_core::routing::types::RoutingReason::SessionAffinity { .. }
    ));

    // 3. Invalidate server
    router.invalidate_affinity(&s1_id);

    // 4. Request again -> affinity miss, falls back to scoring (s1 still best)
    let dec3 = router.select_server(&req).unwrap();
    assert_eq!(*dec3.server_id(), s1_id);
    assert!(matches!(
        dec3.reason(),
        llamaforge_core::routing::types::RoutingReason::HighestScore { .. }
    ));
}

// VC-2.4
#[test]
fn test_vc2_4_constraint_filtering() {
    let mut s1 = default_server("model1");
    let mut s2 = default_server("model1");
    let mut s3 = default_server("model1");

    let adapter_id = LoraAdapterId::new("adapter1").unwrap();

    // Server 1 has only multimodal
    s1.multimodal = true;

    // Server 2 has only lora
    s2.lora_adapters.insert(adapter_id.clone());

    // Server 3 has both
    s3.multimodal = true;
    s3.lora_adapters.insert(adapter_id.clone());
    let s3_id = s3.id;

    let state = Arc::new(MockServerStateView::new(vec![s1, s2, s3]));
    let dispatch = Arc::new(MockDispatchCountView::new());
    let config = create_test_config();
    let router = Router::new(config, state, dispatch);

    let caps = RequestCapabilities {
        multimodal: true,
        lora_adapter: Some(adapter_id),
        ..Default::default()
    };

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps,
        session: None,
    };

    let dec = router.select_server(&req).unwrap();
    assert_eq!(*dec.server_id(), s3_id);
}

// VC-2.5
#[test]
#[ignore] // Slow stress test
fn test_vc2_5_round_robin_fairness() {
    let s1 = default_server("model1");
    let s2 = default_server("model1");
    let s1_id = s1.id;
    let s2_id = s2.id;

    let state = Arc::new(MockServerStateView::new(vec![s1, s2]));
    let dispatch = Arc::new(MockDispatchCountView::new());
    let config = create_test_config();
    let router = Router::new(config, state, dispatch);

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: None,
    };

    let mut s1_count: i32 = 0;
    let mut s2_count: i32 = 0;

    for _ in 0..100 {
        let dec = router.select_server(&req).unwrap();
        if *dec.server_id() == s1_id {
            s1_count += 1;
        } else if *dec.server_id() == s2_id {
            s2_count += 1;
        } else {
            panic!("Unknown server selected");
        }
    }

    assert!((50 - s1_count).abs() <= 1, "s1 count: {}", s1_count);
    assert!((50 - s2_count).abs() <= 1, "s2 count: {}", s2_count);
}

// VC-2.6
#[test]
fn test_vc2_6_mutex_poison_recovery() {
    let s1 = default_server("model1");
    let s2 = default_server("model1");

    let state = Arc::new(MockServerStateView::new(vec![s1, s2]));
    let dispatch = Arc::new(MockDispatchCountView::new());
    let config = create_test_config();
    let router = Arc::new(Router::new(config, state, dispatch));

    // Poison the cache
    let router_clone = router.clone();
    let _ = std::thread::spawn(move || {
        router_clone.poison_affinity_cache_for_test();
    })
    .join();

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: Some(SessionId::new("sess1").unwrap()),
    };

    for _ in 0..10 {
        let dec = router.select_server(&req);
        assert!(dec.is_ok());
    }
}
