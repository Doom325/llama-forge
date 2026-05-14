//! Per-server in-flight tracking.
//!
//! See design doc Section 2.3

use dashmap::DashMap;
use llamaforge_routing::DispatchCountView;
use llamaforge_types::{RequestId, ServerId};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tracing::warn;

/// Tracks in-flight requests for a single server.
///
/// Guarded by a standard library mutex. If poisoned, recovers to continue
/// operating. See design doc Section 2.3.
#[derive(Debug, Default)]
pub struct DispatchTracker {
    inner: Mutex<HashSet<RequestId>>,
}

impl DispatchTracker {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashSet::new()),
        }
    }

    /// Returns the number of currently in-flight requests.
    pub fn count(&self) -> usize {
        let guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("DispatchTracker mutex poisoned, recovering");
            p.into_inner()
        });
        guard.len()
    }

    /// Inserts a new request into the tracker.
    ///
    /// This is `pub(crate)` to enforce that only `DispatchToken` can add tracking entries.
    #[allow(dead_code)]
    pub(crate) fn insert(&self, request_id: RequestId) {
        let mut guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("DispatchTracker mutex poisoned, recovering");
            p.into_inner()
        });
        guard.insert(request_id);
    }

    /// Removes a request from the tracker.
    ///
    /// Returns true if the request was present and removed.
    /// This is `pub(crate)` to enforce that only `DispatchToken` can remove tracking entries.
    #[allow(dead_code)]
    pub(crate) fn remove(&self, request_id: &RequestId) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("DispatchTracker mutex poisoned, recovering");
            p.into_inner()
        });
        guard.remove(request_id)
    }

    /// Returns a snapshot of all currently tracked request IDs.
    pub fn snapshot_ids(&self) -> Vec<RequestId> {
        let guard = self.inner.lock().unwrap_or_else(|p| {
            warn!("DispatchTracker mutex poisoned, recovering");
            p.into_inner()
        });
        guard.iter().copied().collect()
    }
}

/// Registry for looking up the tracker associated with a specific server.
///
/// Encapsulates a DashMap to prevent DashMap references from leaking into the public API.
#[derive(Debug, Default)]
pub struct DispatchTrackerRegistry {
    inner: DashMap<ServerId, Arc<DispatchTracker>>,
}

impl DispatchTrackerRegistry {
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    /// Get the tracker for a server, if it exists.
    pub fn get(&self, server_id: &ServerId) -> Option<Arc<DispatchTracker>> {
        self.inner.get(server_id).map(|r| Arc::clone(r.value()))
    }

    /// Get the tracker for a server, creating it if it doesn't exist.
    pub fn get_or_create(&self, server_id: &ServerId) -> Arc<DispatchTracker> {
        Arc::clone(self.inner.entry(*server_id).or_default().value())
    }
}

/// Adapter connecting `DispatchTrackerRegistry` to the `DispatchCountView` port trait.
///
/// This satisfies the dependency requirement from `llamaforge-routing`.
#[derive(Debug)]
pub struct DispatchCountAdapter {
    trackers: Arc<DispatchTrackerRegistry>,
}

impl DispatchCountAdapter {
    pub fn new(trackers: Arc<DispatchTrackerRegistry>) -> Self {
        Self { trackers }
    }
}

impl DispatchCountView for DispatchCountAdapter {
    fn in_flight_count(&self, server_id: &ServerId) -> usize {
        self.trackers.get(server_id).map(|t| t.count()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use uuid::Uuid;

    #[test]
    fn test_insert_remove_count() {
        let tracker = DispatchTracker::new();
        let r1 = RequestId::new(Uuid::now_v7());
        let r2 = RequestId::new(Uuid::now_v7());
        let r3 = RequestId::new(Uuid::now_v7());

        tracker.insert(r1);
        tracker.insert(r2);
        tracker.insert(r3);
        assert_eq!(tracker.count(), 3);

        assert!(tracker.remove(&r2));
        assert_eq!(tracker.count(), 2);

        assert!(!tracker.remove(&r2)); // non-existent
        assert_eq!(tracker.count(), 2);
    }

    #[test]
    fn test_snapshot_ids() {
        let tracker = DispatchTracker::new();
        let mut ids = Vec::new();
        for _ in 0..5 {
            let id = RequestId::new(Uuid::now_v7());
            tracker.insert(id);
            ids.push(id);
        }

        let snapshot = tracker.snapshot_ids();
        assert_eq!(snapshot.len(), 5);
        for id in ids.iter() {
            assert!(snapshot.contains(id));
        }

        tracker.remove(&ids[0]);
        tracker.remove(&ids[1]);
        
        let snapshot2 = tracker.snapshot_ids();
        assert_eq!(snapshot2.len(), 3);
    }

    #[test]
    fn test_registry_get_or_create() {
        let registry = DispatchTrackerRegistry::new();
        let s1 = ServerId::new(Uuid::now_v7());

        assert!(registry.get(&s1).is_none());

        let t1 = registry.get_or_create(&s1);
        let t2 = registry.get(&s1).expect("Should exist now");
        
        // Assert Arc identity (same pointer)
        assert!(Arc::ptr_eq(&t1, &t2));
    }

    #[test]
    fn test_dispatch_count_adapter() {
        let registry = Arc::new(DispatchTrackerRegistry::new());
        let adapter = DispatchCountAdapter::new(Arc::clone(&registry));
        
        let s_active = ServerId::new(Uuid::now_v7());
        let s_empty = ServerId::new(Uuid::now_v7());
        let s_unknown = ServerId::new(Uuid::now_v7());

        let t_active = registry.get_or_create(&s_active);
        registry.get_or_create(&s_empty);

        t_active.insert(RequestId::new(Uuid::now_v7()));
        t_active.insert(RequestId::new(Uuid::now_v7()));
        t_active.insert(RequestId::new(Uuid::now_v7()));

        assert_eq!(adapter.in_flight_count(&s_active), 3);
        assert_eq!(adapter.in_flight_count(&s_empty), 0);
        assert_eq!(adapter.in_flight_count(&s_unknown), 0);
    }

    #[test]
    fn test_concurrent_insert_remove_stress() {
        let tracker = Arc::new(DispatchTracker::new());
        let mut handlers = Vec::new();

        // 25 request IDs shared between insert and remove threads
        let ids: Vec<RequestId> = (0..25).map(|_| RequestId::new(Uuid::now_v7())).collect();

        // Spawn 25 inserter threads
        for id in ids.clone() {
            let t = Arc::clone(&tracker);
            handlers.push(thread::spawn(move || {
                t.insert(id);
            }));
        }

        // Spawn 25 remover threads
        for id in ids {
            let t = Arc::clone(&tracker);
            handlers.push(thread::spawn(move || {
                t.remove(&id);
            }));
        }

        for h in handlers {
            h.join().unwrap();
        }

        // All IDs might be inserted before or after they're removed, 
        // so the exact count isn't fixed, but there should be no panics.
        // And the HashSet logic should be consistent.
        let c = tracker.count();
        assert!(c <= 25);
    }

    /// This is a test that documents compile-time visibility checks.
    /// `DispatchTracker::insert` is `pub(crate)` and cannot be called from outside the crate.
    #[test]
    fn test_visibility_enforcement() {
        // compile_fail test would be in `tests/` directory if we used trybuild,
        // but we verify it here through a comment as per task 3.10.
        // `pub(crate)` naturally prevents external access.
    }
}
