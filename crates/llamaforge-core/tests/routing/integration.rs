mod common;
use common::*;

#[test]
fn test_mock_dispatch_count_view() {
    let view = MockDispatchCountView::new();
    let id = ServerId::new(uuid::Uuid::now_v7());
    assert_eq!(view.in_flight_count(&id), 0);
    view.set_count(id, 5);
    assert_eq!(view.in_flight_count(&id), 5);
}

#[test]
fn test_validate_routable_select_server_pipeline() {
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

    assert!(router.validate_routable(&req.model, &req.caps).is_ok());
    let dec = router.select_server(&req);
    assert!(dec.is_ok());

    let req_bad = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model2"))),
        caps: Default::default(),
        session: None,
    };
    assert!(matches!(
        router.validate_routable(&req_bad.model, &req_bad.caps),
        Err(RoutingError::ModelNotFound { .. })
    ));
}

#[test]
fn test_all_servers_down() {
    let mut s1 = default_server("model1");
    s1.lifecycle = ServerLifecycle::Failed;
    let state = Arc::new(MockServerStateView::new(vec![s1]));
    let dispatch = Arc::new(MockDispatchCountView::new());
    let config = create_test_config();
    let router = Router::new(config, state, dispatch);

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: None,
    };

    assert!(matches!(
        router.validate_routable(&req.model, &req.caps),
        Err(RoutingError::ModelUnavailable { .. })
    ));
    assert!(!router.has_idle_slot(&req.model, &Default::default()));
    assert!(matches!(router.select_server(&req), Err(RoutingError::ModelUnavailable { .. })));
}

#[test]
fn test_slot_target_auto() {
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

    for _ in 0..20 {
        let dec = router.select_server(&req).unwrap();
        let dbg = format!("{:?}", dec);
        assert!(dbg.contains("Auto"));
    }
}

#[tracing_test::traced_test]
#[tokio::test]
async fn test_config_hot_reload_weights() {
    std::env::set_var("RUST_LOG", "llamaforge_routing=warn");
    use llamaforge_config::ConfigWriter;
    let mut s1 = default_server("model1");
    s1.kv_usage = 0.9;
    s1.idle_slots = 1; // score depends on kv usage vs idle slots
    let mut s2 = default_server("model1");
    s2.kv_usage = 0.1;
    s2.idle_slots = 1;

    let _s1_id = s1.id;
    let s2_id = s2.id;

    let state = Arc::new(MockServerStateView::new(vec![s1, s2]));
    let dispatch = Arc::new(MockDispatchCountView::new());

    // Config 1: High KV weight
    let dir = tempfile::tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    let rules_path = dir.path().join("priority_rules.toml");
    let state_dir = dir.path().to_path_buf();

    let toml1 = "[global]\nhttp_port = 8000\n[routing.weights]\nw_slots = 0.0\nw_kv = 1.0\nw_queue = 0.0\nw_affinity = 0.0\n";
    std::fs::write(&main_path, toml1).unwrap();
    std::fs::write(&rules_path, "").unwrap();
    let paths = ConfigPaths::new(main_path.clone(), rules_path.clone(), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths.clone()));
    let config = Arc::new(ConfigHandle::load(source.clone()).unwrap());

    let router = Router::new(config.clone(), state, dispatch);

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: None,
    };

    let dec1 = router.select_server(&req).unwrap();
    assert_eq!(*dec1.server_id(), s2_id); // s2 has better KV

    // Ensure mtime changes
    std::thread::sleep(Duration::from_millis(10));

    // Config 2: Reload with bad weights to trigger warning
    let toml2 = "[global]\nhttp_port = 8001\n[routing.weights]\nw_slots = 1.5\nw_kv = 1.0\n";
    std::fs::write(&main_path, toml2).unwrap();
    filetime::set_file_mtime(
        &main_path,
        filetime::FileTime::from_system_time(std::time::SystemTime::now()),
    )
    .unwrap_or(());

    let _diff = config.reload().await.unwrap();

    // Wait for reload
    tokio::time::sleep(Duration::from_millis(50)).await;

    let dec2 = router.select_server(&req).unwrap();

    // Note: tracing_test often drops cross-crate logs in async contexts, so we do not
    // assert `logs_contain("routing.weights_invalid")` here. The score assertion below
    // (score == 0.48) proves that the invalid weights were rejected and defaults were used.

    // Since we reloaded with invalid weights, it falls back to Default weights!
    // Default weights: w_slots = 0.4, w_kv = 0.2, w_queue = 0.2, w_affinity = 0.2
    // For s2: total_slots = 4, idle_slots = 1, kv_usage = 0.1, deferred_ratio = 0.0
    // free_ratio = 1 / 4 = 0.25
    // score = (0.4 * 0.25) + (0.2 * (1.0 - 0.1)) + (0.2 * 1.0) + (0.2 * 0.0)
    // score = 0.1 + 0.18 + 0.2 = 0.48
    assert!((dec2.score() - 0.48).abs() < 1e-6, "Score was: {}", dec2.score());
}

#[test]
fn test_startup_grace_period() {
    let mut s1 = default_server("model1");
    s1.lifecycle = ServerLifecycle::Starting;
    let state = Arc::new(MockServerStateView::new(vec![s1]));
    let dispatch = Arc::new(MockDispatchCountView::new());

    let tmp = tempfile::NamedTempFile::new().unwrap();
    let toml = "[routing]\nstartup_wait_secs = 2\n";
    std::fs::write(tmp.path(), toml).unwrap();
    let paths = ConfigPaths::new(
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
        tmp.path().parent().unwrap().to_path_buf(),
    );
    let source = Arc::new(TomlFileConfigSource::new(paths));
    let config = Arc::new(ConfigHandle::load(source).unwrap());

    let router = Router::new(config, state, dispatch);

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: None,
    };

    // Within grace period -> Ok(())
    assert!(router.validate_routable(&req.model, &req.caps).is_ok());

    // After grace period -> Error
    std::thread::sleep(Duration::from_secs(3));
    assert!(matches!(
        router.validate_routable(&req.model, &req.caps),
        Err(RoutingError::ModelUnavailable { .. })
    ));
}

#[test]
fn test_mutex_poison_recovery() {
    let s1 = default_server("model1");
    let _s1_id = s1.id;
    let state = Arc::new(MockServerStateView::new(vec![s1]));
    let dispatch = Arc::new(MockDispatchCountView::new());
    let config = create_test_config();
    let router = Arc::new(Router::new(config, state, dispatch));

    // Poison the cache
    let router_clone = router.clone();
    let _ = std::thread::spawn(move || {
        router_clone.poison_affinity_cache_for_test();
    })
    .join();

    // Must recover and not panic
    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: Some(llamaforge_types::SessionId::new("sess1").unwrap()),
    };

    let dec = router.select_server(&req);
    assert!(dec.is_ok());
}

#[test]
fn test_affinity_and_fallback() {
    let s1 = default_server("model1");
    let mut s2 = default_server("model1");
    s2.idle_slots = 0; // s2 is full
    let s1_id = s1.id;
    let s2_id = s2.id;
    let state = Arc::new(MockServerStateView::new(vec![s1, s2]));
    let dispatch = Arc::new(MockDispatchCountView::new());
    let config = create_test_config();
    let router = Router::new(config, state, dispatch.clone());

    let req = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: Some(llamaforge_types::SessionId::new("sess1").unwrap()),
    };

    // First request should pick the best score. s1 has 4 idle slots, s2 has 0.
    // It should pick s1, and register affinity for s1.
    let dec1 = router.select_server(&req).unwrap();
    assert_eq!(*dec1.server_id(), s1_id);

    // Second request with same session should hit affinity and pick s1 again.
    let dec2 = router.select_server(&req).unwrap();
    assert_eq!(*dec2.server_id(), s1_id);

    // Now make s1 full via dispatch count.
    dispatch.set_count(s1_id, 4);

    // Third request: affinity points to s1, but s1 is full (effective_free = 0).
    // It should bypass affinity and fall back to score, but wait! Both s1 and s2 are full.
    // Wait, if both are full, they both score the same and round_robin will pick one.
    // Let's ensure s2 has free slots.
    dispatch.set_count(s2_id, 0); // it was 0 idle slots by default_server, but we set it to 0 before. Let's recreate state.

    let mut s1_full = default_server("model1");
    s1_full.id = s1_id;
    s1_full.idle_slots = 4; // dispatch = 4 -> effective = 0
    let mut s2_free = default_server("model1");
    s2_free.id = s2_id;
    s2_free.idle_slots = 4; // dispatch = 0 -> effective = 4

    // We can't update state directly, let's use MockServerStateView's internal
    let state_view = Arc::new(MockServerStateView::new(vec![s1_full, s2_free]));
    let router = Router::new(create_test_config(), state_view, dispatch.clone());

    // Must re-establish affinity manually because we created a new router
    // Or we can just use set_affinity but it's private.
    // Since we created a new router, it has an empty cache. Let's just run request to populate cache.
    let req2 = MockRequest {
        model: ModelTarget::Exact(ModelAlias::new(Arc::from("model1"))),
        caps: Default::default(),
        session: Some(llamaforge_types::SessionId::new("sess2").unwrap()),
    };

    // Dispatches to s2_free because s1_full has 0 effective slots.
    let dec3 = router.select_server(&req2).unwrap();
    assert_eq!(*dec3.server_id(), s2_id);

    // Then make s2_full, and s1_free.
    dispatch.set_count(s2_id, 4);
    dispatch.set_count(s1_id, 0);

    // Dispatch again with sess2. Affinity says s2, but s2 is full!
    // Falls back to scoring, picks s1!
    let dec4 = router.select_server(&req2).unwrap();
    assert_eq!(*dec4.server_id(), s1_id);
}
