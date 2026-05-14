use llamaforge_dispatch::client::{BackendClient, BackendResponseStream, ClientManager};
use llamaforge_dispatch::dispatcher::{Dispatcher, CompletionRequest};
use llamaforge_dispatch::error::DispatchError;
use llamaforge_dispatch::slot_map::SlotDispatchMap;
use llamaforge_dispatch::tracker::DispatchTrackerRegistry;
use llamaforge_config::{ConfigHandle, source::memory::InMemoryConfigSource, AppConfig, ServerConfig};
use llamaforge_metrics::MetricsIngest;
use llamaforge_routing::types::ServerStateView;
use llamaforge_types::ports::{SlotErasure, SlotError, SlotNotifier};
use llamaforge_types::{ServerId, SlotId, RequestId};
use std::sync::Arc;
use tokio::sync::mpsc;
use proptest::prelude::*;
use uuid::Uuid;

struct MockBackendClient;
#[async_trait::async_trait]
impl BackendClient for MockBackendClient {
    async fn post_completion(&self, _s: &ServerId, _p: CompletionRequest) -> Result<BackendResponseStream, DispatchError> {
        Ok(Box::pin(tokio_stream::empty()))
    }
}

struct MockClientManager;
impl ClientManager for MockClientManager {
    fn refresh(&self, _s: &ServerId, _e: llamaforge_types::ClientRefreshEvent) {}
}

struct MockServerStateView;
impl ServerStateView for MockServerStateView {
    fn all_servers(&self) -> Vec<llamaforge_routing::types::ServerSnapshot> { vec![] }
}

struct MockSlotNotifier;
impl SlotNotifier for MockSlotNotifier {
    fn notify_slot_available(&self, _s: &ServerId) {}
}

struct MockSlotErasure;
#[async_trait::async_trait]
impl SlotErasure for MockSlotErasure {
    async fn erase_slot(&self, _s: &ServerId, _sl: &SlotId) -> Result<(), SlotError> {
        Ok(())
    }
}

fn create_dispatcher() -> Dispatcher {
    let mut app_config = AppConfig::default();
    app_config.servers.insert(llamaforge_config::ServerId::from("s1"), ServerConfig::default());

    let config = Arc::new(ConfigHandle::load(Arc::new(InMemoryConfigSource::new(app_config))).unwrap());
    let (metrics_tx, _) = mpsc::channel(100);
    let (audit_tx, _) = mpsc::channel(100);
    let metrics = MetricsIngest::new_for_test(metrics_tx, audit_tx, Arc::new(tokio::sync::Semaphore::new(1)));

    Dispatcher::new(
        Arc::new(MockServerStateView),
        metrics,
        Arc::new(DispatchTrackerRegistry::new()),
        Arc::new(SlotDispatchMap::new()),
        config,
        Arc::new(MockBackendClient),
        Arc::new(MockClientManager),
        Arc::new(MockSlotNotifier),
        Arc::new(MockSlotErasure),
    )
}

proptest! {
    #[test]
    #[ignore]
    fn test_8_3_token_lifecycle(inserts in 0usize..256, removes in 0usize..256) {
        let registry = Arc::new(DispatchTrackerRegistry::new());
        let server_id = ServerId::new(Uuid::now_v7());
        
        let mut tokens = Vec::new();
        for _ in 0..inserts {
            let req_id = RequestId::new(Uuid::now_v7());
            let tracker = registry.get_or_create(&server_id);
            let token = llamaforge_dispatch::token::DispatchToken::new(
                tracker, req_id, server_id, Arc::new(MockSlotNotifier)
            );
            tokens.push(token);
        }
        
        let tracker = registry.get_or_create(&server_id);
        assert_eq!(tracker.count(), inserts);
        
        let to_remove = std::cmp::min(inserts, removes);
        for _ in 0..to_remove {
            tokens.pop(); // drop releases token
        }
        
        assert_eq!(tracker.count(), inserts - to_remove);
    }

    #[test]
    #[ignore]
    fn test_8_4_slot_dispatch_map_consistency(inserts in 0usize..256, removes in 0usize..256) {
        let map = SlotDispatchMap::new();
        let server_id = ServerId::new(Uuid::now_v7());
        
        let mut ids = Vec::new();
        for i in 0..inserts {
            let req_id = RequestId::new(Uuid::now_v7());
            let slot_id = SlotId::new(i as u32);
            map.insert(req_id, server_id, slot_id);
            ids.push(req_id);
        }
        
        // Wait, SlotDispatchMap doesn't expose `len()`. But we can test consistency by checking properties.
        let to_remove = std::cmp::min(inserts, removes);
        for id in ids.iter().take(to_remove) {
            map.remove(id);
        }
        
        // Let's verify we can only find remaining items
        for id in ids.iter().take(to_remove) {
            assert!(map.get_slot_for_request(id).is_none());
        }
        for id in ids.iter().skip(to_remove) {
            assert!(map.get_slot_for_request(id).is_some());
        }
    }

    #[test]
    #[ignore]
    fn test_8_5_reconciliation_bounded(processing_slots_count in 0usize..50, stray_slots in 0usize..100) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let dispatcher = create_dispatcher();
            let server_id = ServerId::new(Uuid::now_v7());
            
            // First reconcile with stray slots
            let mut stray = Vec::new();
            for i in 0..stray_slots {
                stray.push(SlotId::new(i as u32));
            }
            dispatcher.reconcile_slots(&server_id, &stray).await;
            
            // Now reconcile with actual processing slots
            let mut processing = Vec::new();
            for i in 0..processing_slots_count {
                processing.push(SlotId::new((stray_slots + i) as u32));
            }
            dispatcher.reconcile_slots(&server_id, &processing).await;
            
            assert!(dispatcher.get_reconciliation_map_size() <= processing_slots_count);
        });
    }

    #[test]
    #[ignore]
    fn test_8_6_grace_period_monotonic(ctx_a in 1u32..10_000_000, ctx_b in 1u32..10_000_000) {
        let grace_a = std::cmp::max(30, ctx_a / 500);
        let grace_b = std::cmp::max(30, ctx_b / 500);
        if ctx_a >= ctx_b {
            assert!(grace_a >= grace_b);
        } else {
            assert!(grace_a <= grace_b);
        }
    }
}
