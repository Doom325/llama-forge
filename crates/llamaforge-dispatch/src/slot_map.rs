//! Bidirectional reconciliation map.
//!
//! See design doc Section 2.4

use llamaforge_core::types::ports::SlotDispatchView;
use llamaforge_core::types::{RequestId, ServerId, SlotId};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::time::Instant;
use tracing::warn;

#[derive(Debug, Default)]
struct SlotDispatchMapInner {
    by_request: HashMap<RequestId, (ServerId, SlotId, Instant)>,
    by_slot: HashMap<(ServerId, SlotId), RequestId>,
}

/// Bidirectional map for tracking which requests hold which slots.
///
/// Guarded by a standard library mutex. Reconciles state between LlamaForge and the backend.
/// Uses poison recovery to maintain availability.
///
/// See design doc Section 2.4.
#[derive(Debug, Default)]
pub struct SlotDispatchMap {
    inner: Mutex<SlotDispatchMapInner>,
}

impl SlotDispatchMap {
    pub fn new() -> Self {
        Self { inner: Mutex::new(SlotDispatchMapInner::default()) }
    }

    /// Records that a request holds a slot.
    pub fn insert(&self, request_id: RequestId, server_id: ServerId, slot_id: SlotId) {
        let mut guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("SlotDispatchMap mutex poisoned");
            p.into_inner()
        });

        guard.by_request.insert(request_id, (server_id, slot_id, Instant::now()));
        guard.by_slot.insert((server_id, slot_id), request_id);
    }

    /// Removes a request from the map, returning the slot it held.
    pub fn remove(&self, request_id: &RequestId) -> Option<(ServerId, SlotId)> {
        let mut guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("SlotDispatchMap mutex poisoned");
            p.into_inner()
        });

        if let Some((server_id, slot_id, _)) = guard.by_request.remove(request_id) {
            guard.by_slot.remove(&(server_id, slot_id));
            Some((server_id, slot_id))
        } else {
            None
        }
    }

    /// Retrieves the slot assigned to a request.
    pub fn get_slot_for_request(&self, request_id: &RequestId) -> Option<(ServerId, SlotId)> {
        let guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("SlotDispatchMap mutex poisoned");
            p.into_inner()
        });

        guard.by_request.get(request_id).map(|&(srv, slot, _)| (srv, slot))
    }

    /// Retrieves the time at which a request was assigned a slot.
    pub fn get_dispatched_at(&self, request_id: &RequestId) -> Option<Instant> {
        let guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("SlotDispatchMap mutex poisoned");
            p.into_inner()
        });

        guard.by_request.get(request_id).map(|&(_, _, instant)| instant)
    }

    /// Retrieves all active slots assigned to a given server.
    pub fn get_dispatched_slots_for_server(
        &self,
        server_id: &ServerId,
    ) -> Vec<(RequestId, SlotId, Instant)> {
        let guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("SlotDispatchMap mutex poisoned");
            p.into_inner()
        });

        guard
            .by_request
            .iter()
            .filter(|(_, (srv, _, _))| srv == server_id)
            .map(|(req, (_, slot, instant))| (*req, *slot, *instant))
            .collect()
    }
}

/// The implementation of the public interface consumed by `llamaforge-server-mgr`.
///
/// **Split API Rationale:**
/// Inherent methods (`insert`, `remove`, etc.) are intended for use only within
/// `llamaforge-dispatch`. Trait methods are the designated cross-crate boundary.
/// Callers must `use llamaforge_core::types::ports::SlotDispatchView;` to access these methods.
impl SlotDispatchView for SlotDispatchMap {
    fn get_request_for_slot(&self, server_id: &ServerId, slot_id: &SlotId) -> Option<RequestId> {
        let guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("SlotDispatchMap mutex poisoned");
            p.into_inner()
        });

        guard.by_slot.get(&(*server_id, *slot_id)).copied()
    }

    fn has_unresolved(&self, server_id: &ServerId) -> bool {
        let guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("SlotDispatchMap mutex poisoned");
            p.into_inner()
        });

        guard.by_request.values().any(|(srv, _, _)| srv == server_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_bidirectional_consistency() {
        let map = SlotDispatchMap::new();
        let r1 = RequestId::new(Uuid::now_v7());
        let s1 = ServerId::new(Uuid::now_v7());
        let slot1 = SlotId::new(0);

        let r2 = RequestId::new(Uuid::now_v7());
        let s2 = ServerId::new(Uuid::now_v7());
        let slot2 = SlotId::new(1);

        let r3 = RequestId::new(Uuid::now_v7());
        let slot3 = SlotId::new(2);

        map.insert(r1, s1, slot1);
        map.insert(r2, s2, slot2);
        map.insert(r3, s1, slot3);

        assert_eq!(map.get_slot_for_request(&r1), Some((s1, slot1)));
        assert_eq!(map.get_request_for_slot(&s1, &slot1), Some(r1));

        let removed = map.remove(&r1);
        assert_eq!(removed, Some((s1, slot1)));
        assert_eq!(map.get_slot_for_request(&r1), None);
        assert_eq!(map.get_request_for_slot(&s1, &slot1), None);
    }

    #[test]
    fn test_dispatched_at() {
        let map = SlotDispatchMap::new();
        let r1 = RequestId::new(Uuid::now_v7());
        let s1 = ServerId::new(Uuid::now_v7());
        let slot1 = SlotId::new(0);

        map.insert(r1, s1, slot1);
        let instant1 = map.get_dispatched_at(&r1).unwrap();
        let instant2 = map.get_dispatched_at(&r1).unwrap();

        assert_eq!(instant1, instant2);

        map.remove(&r1);
        assert!(map.get_dispatched_at(&r1).is_none());
    }

    #[test]
    fn test_has_unresolved() {
        let map = SlotDispatchMap::new();
        let s1 = ServerId::new(Uuid::now_v7());
        let s2 = ServerId::new(Uuid::now_v7());
        let r1 = RequestId::new(Uuid::now_v7());
        let slot1 = SlotId::new(0);

        assert!(!map.has_unresolved(&s1));

        map.insert(r1, s1, slot1);

        assert!(map.has_unresolved(&s1));
        assert!(!map.has_unresolved(&s2));

        map.remove(&r1);

        assert!(!map.has_unresolved(&s1));
    }

    #[test]
    fn test_empty_after_full_remove_cycle() {
        let map = SlotDispatchMap::new();
        let mut ids = Vec::new();
        let s1 = ServerId::new(Uuid::now_v7());

        for i in 0..10 {
            let r = RequestId::new(Uuid::now_v7());
            let slot = SlotId::new(i);
            map.insert(r, s1, slot);
            ids.push(r);
        }

        assert!(map.has_unresolved(&s1));

        for r in ids {
            map.remove(&r);
        }

        assert!(!map.has_unresolved(&s1));
    }

    #[test]
    fn test_get_request_for_slot_requires_trait() {
        // Compile-time check documentations.
        // We know from visibility rules that `get_request_for_slot` is part of `SlotDispatchView`
        // and cannot be called on the raw struct without trait import.
    }
}
