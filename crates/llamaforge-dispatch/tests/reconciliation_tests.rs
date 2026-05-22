use llamaforge_core::config::{
    source::memory::InMemoryConfigSource, AppConfig, ConfigHandle, ServerConfig,
};
use llamaforge_dispatch::client::{BackendClient, BackendResponseStream, ClientManager};
use llamaforge_dispatch::dispatcher::{CompletionRequest, Dispatcher};
use llamaforge_dispatch::error::DispatchError;
use llamaforge_dispatch::slot_map::SlotDispatchMap;
use llamaforge_dispatch::tracker::DispatchTrackerRegistry;
use llamaforge_metrics::MetricsIngest;
use llamaforge_core::routing::types::ServerStateView;
use llamaforge_core::types::ports::{SlotErasure, SlotError};
use llamaforge_core::types::traits::SlotNotifier;
use llamaforge_core::types::{RequestId, ServerId, SlotId};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

struct MockBackendClient;
#[async_trait::async_trait]
impl BackendClient for MockBackendClient {
    async fn post_completion(
        &self,
        _s: &ServerId,
        _p: CompletionRequest,
    ) -> Result<BackendResponseStream, DispatchError> {
        Ok(Box::pin(tokio_stream::empty()))
    }
}

struct MockClientManager;
impl ClientManager for MockClientManager {
    fn refresh(&self, _s: &ServerId, _e: llamaforge_core::types::ClientRefreshEvent) {}
}

struct MockServerStateView;
impl ServerStateView for MockServerStateView {
    fn all_servers(&self) -> Vec<llamaforge_core::routing::types::ServerSnapshot> {
        vec![]
    }
}

struct MockSlotNotifier;
impl SlotNotifier for MockSlotNotifier {
    fn notify_slot_available(&self) {}
}

struct MockSlotErasure {
    erased: Mutex<Vec<(ServerId, SlotId)>>,
}
impl MockSlotErasure {
    fn new() -> Self {
        Self { erased: Mutex::new(Vec::new()) }
    }
}
#[async_trait::async_trait]
impl SlotErasure for MockSlotErasure {
    async fn erase_slot(&self, server_id: &ServerId, slot_id: &SlotId) -> Result<(), SlotError> {
        self.erased.lock().unwrap().push((*server_id, *slot_id));
        Ok(())
    }
}

fn create_dispatcher(
    ctx_size: u32,
    map: Arc<SlotDispatchMap>,
    erasure: Arc<MockSlotErasure>,
    srv_id: ServerId,
) -> Dispatcher {
    let mut app_config = AppConfig::default();
    let config_srv_id = llamaforge_core::types::ServerId::from_config_id(&srv_id.to_string());
    let srv_cfg = ServerConfig { ctx_size, ..Default::default() };
    app_config.servers.insert(config_srv_id, srv_cfg);

    let config =
        Arc::new(ConfigHandle::load(Arc::new(InMemoryConfigSource::new(app_config))).unwrap());
    let (metrics_tx, _) = mpsc::channel(1);
    let (audit_tx, _) = mpsc::channel(1);
    let metrics =
        MetricsIngest::new_for_test(metrics_tx, audit_tx, Arc::new(tokio::sync::Semaphore::new(1)));

    Dispatcher::new(
        Arc::new(MockServerStateView),
        metrics,
        Arc::new(DispatchTrackerRegistry::new()),
        map,
        config,
        Arc::new(MockBackendClient),
        Arc::new(MockClientManager),
        Arc::new(MockSlotNotifier),
        erasure,
    )
}

#[tokio::test(start_paused = true)]
async fn test_7_8_forward_reconciliation_detects_orphan() {
    let map = Arc::new(SlotDispatchMap::new());
    let erasure = Arc::new(MockSlotErasure::new());
    let s1 = ServerId::new(Uuid::now_v7());
    let d = create_dispatcher(4000, map.clone(), erasure.clone(), s1); // ctx_size=4000 -> grace 30s
    let slot = SlotId::new(5);

    d.reconcile_slots(&s1, &[slot]).await;
    // first seen registered, no erasure yet
    assert!(erasure.erased.lock().unwrap().is_empty());

    // Advance 31 seconds
    tokio::time::advance(Duration::from_secs(31)).await;
    d.reconcile_slots(&s1, &[slot]).await;

    // Must yield tokio runtime to let tokio::spawn for erase_slot run
    tokio::task::yield_now().await;

    let erased = erasure.erased.lock().unwrap();
    assert_eq!(erased.len(), 1);
    assert_eq!(erased[0], (s1, slot));
}

#[tokio::test(start_paused = true)]
async fn test_7_9_forward_reconciliation_respects_grace() {
    let map = Arc::new(SlotDispatchMap::new());
    let erasure = Arc::new(MockSlotErasure::new());
    let s1 = ServerId::new(Uuid::now_v7());
    let d = create_dispatcher(4000, map.clone(), erasure.clone(), s1);
    let slot = SlotId::new(5);

    d.reconcile_slots(&s1, &[slot]).await;
    assert!(erasure.erased.lock().unwrap().is_empty());

    tokio::time::advance(Duration::from_secs(29)).await;
    d.reconcile_slots(&s1, &[slot]).await;

    tokio::task::yield_now().await;
    assert!(erasure.erased.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn test_7_10_dynamic_grace_period() {
    let map = Arc::new(SlotDispatchMap::new());
    let erasure = Arc::new(MockSlotErasure::new());
    let s1 = ServerId::new(Uuid::now_v7());
    let d = create_dispatcher(128000, map.clone(), erasure.clone(), s1); // grace max(30, 128000/500) = 256s
    let slot = SlotId::new(5);

    d.reconcile_slots(&s1, &[slot]).await;

    // Advance 255s -> no erase
    tokio::time::advance(Duration::from_secs(255)).await;
    d.reconcile_slots(&s1, &[slot]).await;
    tokio::task::yield_now().await;
    assert!(erasure.erased.lock().unwrap().is_empty());

    // Advance 2s more -> >256s -> erase
    tokio::time::advance(Duration::from_secs(2)).await;
    d.reconcile_slots(&s1, &[slot]).await;
    tokio::task::yield_now().await;
    assert_eq!(erasure.erased.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn test_7_11_reverse_reconciliation_cleans_stale_entry() {
    let map = Arc::new(SlotDispatchMap::new());
    let erasure = Arc::new(MockSlotErasure::new());
    let s1 = ServerId::new(Uuid::now_v7());
    let d = create_dispatcher(4000, map.clone(), erasure.clone(), s1);
    let req1 = RequestId::new(Uuid::now_v7());
    let slot = SlotId::new(5);

    map.insert(req1, s1, slot);

    tokio::time::advance(Duration::from_secs(31)).await;
    // Call with processing_slots empty -> slot not processing
    d.reconcile_slots(&s1, &[]).await;

    assert!(map.get_slot_for_request(&req1).is_none());
}

#[tokio::test(start_paused = true)]
async fn test_7_12_reverse_reconciliation_skips_when_unresolved() {
    let map = Arc::new(SlotDispatchMap::new());
    let erasure = Arc::new(MockSlotErasure::new());
    let s1 = ServerId::new(Uuid::now_v7());
    let d = create_dispatcher(4000, map.clone(), erasure.clone(), s1);
    let req1 = RequestId::new(Uuid::now_v7());
    let req2 = RequestId::new(Uuid::now_v7());
    let slot = SlotId::new(5);

    map.insert(req1, s1, slot);

    // To simulate unresolved, we add a request to the tracker but NOT the map
    let _token = llamaforge_dispatch::token::DispatchToken::new(
        d.trackers().get_or_create(&s1),
        req2,
        s1,
        Arc::new(MockSlotNotifier),
    );

    tokio::time::advance(Duration::from_secs(31)).await;
    d.reconcile_slots(&s1, &[]).await;

    // Due to has_unresolved(), req1 should NOT be removed
    assert!(map.get_slot_for_request(&req1).is_some());
}

#[tokio::test(start_paused = true)]
async fn test_7_13_first_seen_processing_cleanup() {
    let map = Arc::new(SlotDispatchMap::new());
    let erasure = Arc::new(MockSlotErasure::new());
    let s1 = ServerId::new(Uuid::now_v7());
    let d = create_dispatcher(4000, map.clone(), erasure.clone(), s1);
    let slot = SlotId::new(5);

    d.reconcile_slots(&s1, &[slot]).await;
    // It's in the tracker now.

    // If it is now in slot_dispatch_map, it should be removed from first_seen.
    let req1 = RequestId::new(Uuid::now_v7());
    map.insert(req1, s1, slot);
    d.reconcile_slots(&s1, &[slot]).await;

    // Advance 31 seconds. If it was not cleaned up, it would fire an erase now.
    tokio::time::advance(Duration::from_secs(31)).await;
    map.remove(&req1); // remove from map so we can observe if it fires
    d.reconcile_slots(&s1, &[slot]).await; // adds it back as fresh
    tokio::task::yield_now().await;

    // No erasure because the old entry was removed, and the new one hasn't hit grace yet.
    assert!(erasure.erased.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn test_7_14_end_to_end_reconciliation_cycle() {
    let map = Arc::new(SlotDispatchMap::new());
    let erasure = Arc::new(MockSlotErasure::new());
    let s1 = ServerId::new(Uuid::now_v7());
    let d = create_dispatcher(4000, map.clone(), erasure.clone(), s1);

    let req1 = RequestId::new(Uuid::now_v7());
    let slot = SlotId::new(5);

    // Insert dispatch into slot_dispatch_map
    map.insert(req1, s1, slot);

    // Track the dispatch so it's formally active but not "unresolved"
    let _token = llamaforge_dispatch::token::DispatchToken::new(
        d.trackers().get_or_create(&s1),
        req1,
        s1,
        Arc::new(MockSlotNotifier),
    );

    // Advance past grace
    tokio::time::advance(Duration::from_secs(31)).await;

    // Reconcile with empty processing_slots (simulate backend completing it without LlamaForge knowing)
    d.reconcile_slots(&s1, &[]).await;

    // Assert cleanup
    assert!(map.get_slot_for_request(&req1).is_none());
}
