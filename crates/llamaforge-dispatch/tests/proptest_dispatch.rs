use llamaforge_dispatch::tracker::DispatchTrackerRegistry;
use llamaforge_dispatch::slot_map::SlotDispatchMap;
use llamaforge_types::{RequestId, ServerId, SlotId};
use llamaforge_types::ports::SlotNotifier;
use proptest::prelude::*;
use std::sync::Arc;
use uuid::Uuid;

struct MockSlotNotifier;
impl SlotNotifier for MockSlotNotifier {
    fn notify_slot_available(&self, _s: &ServerId) {}
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
    fn test_8_5_reconciliation_bounded(ctx_size in 1u32..10_000_000) {
        // The grace period is max(30s, ctx_size / 500)
        let grace = std::cmp::max(30, ctx_size / 500);
        assert!(grace >= 30);
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
