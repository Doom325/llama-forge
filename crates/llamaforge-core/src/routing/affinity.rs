//! Module: affinity

use llamaforge_types::{ServerId, SessionId};
use lru::LruCache;
use std::num::NonZeroUsize;

/// An LRU cache mapping sessions to their last used server.
///
/// Protected by a Mutex (or similar) in the `Router`.
/// See design doc Section 2.3.
pub(crate) struct SessionAffinityCache {
    cache: LruCache<SessionId, ServerId>,
}

impl SessionAffinityCache {
    /// Creates a new `SessionAffinityCache` with the given capacity.
    ///
    /// The default in the plan is 10,000.
    #[allow(dead_code)]
    pub(crate) fn new(max_entries: usize) -> Self {
        let capacity = NonZeroUsize::new(max_entries.max(1)).unwrap_or(NonZeroUsize::MIN);
        Self { cache: LruCache::new(capacity) }
    }

    /// Looks up a session ID in the cache.
    ///
    /// If the session is found AND the server is present in `available_servers`,
    /// returns `Some(ServerId)` and touches the entry (updates LRU status).
    /// Otherwise returns `None`.
    #[allow(dead_code)]
    pub(crate) fn get(
        &mut self,
        session_id: &SessionId,
        available_servers: &[ServerId],
    ) -> Option<ServerId> {
        if let Some(server_id) = self.cache.get(session_id) {
            if available_servers.contains(server_id) {
                return Some(*server_id);
            }
        }
        None
    }

    #[allow(dead_code)]
    pub(crate) fn set(&mut self, session_id: SessionId, server_id: ServerId) {
        self.cache.put(session_id, server_id);
    }

    /// Removes all entries pointing to the given server.
    /// Called when server enters `Draining` or `Failed` state.
    #[allow(dead_code)]
    pub(crate) fn invalidate_server(&mut self, server_id: &ServerId) {
        let to_remove: Vec<SessionId> =
            self.cache.iter().filter(|(_, &v)| v == *server_id).map(|(k, _)| k.clone()).collect();

        for key in to_remove {
            self.cache.pop(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_session(id: &str) -> SessionId {
        SessionId::new(Arc::from(id)).unwrap()
    }

    fn make_server() -> ServerId {
        ServerId::new(uuid::Uuid::now_v7())
    }

    #[test]
    fn test_set_and_get() {
        let mut cache = SessionAffinityCache::new(10);
        let session = make_session("sess1");
        let server = make_server();

        cache.set(session.clone(), server);

        let available = vec![server];
        assert_eq!(cache.get(&session, &available), Some(server));
    }

    #[test]
    fn test_get_not_available() {
        let mut cache = SessionAffinityCache::new(10);
        let session = make_session("sess1");
        let server = make_server();
        let other_server = make_server();

        cache.set(session.clone(), server);

        let available = vec![other_server];
        assert_eq!(cache.get(&session, &available), None);
    }

    #[test]
    fn test_eviction() {
        let mut cache = SessionAffinityCache::new(2);

        let s1 = make_session("sess1");
        let s2 = make_session("sess2");
        let s3 = make_session("sess3");

        let server = make_server();
        let available = vec![server];

        cache.set(s1.clone(), server);
        cache.set(s2.clone(), server);
        cache.set(s3.clone(), server);

        // s1 should be evicted because capacity is 2
        assert_eq!(cache.get(&s1, &available), None);
        assert_eq!(cache.get(&s2, &available), Some(server));
        assert_eq!(cache.get(&s3, &available), Some(server));
    }

    #[test]
    fn test_invalidate_server() {
        let mut cache = SessionAffinityCache::new(10);

        let s1 = make_session("sess1");
        let s2 = make_session("sess2");
        let s3 = make_session("sess3");

        let server_a = make_server();
        let server_b = make_server();
        let available = vec![server_a, server_b];

        cache.set(s1.clone(), server_a);
        cache.set(s2.clone(), server_a);
        cache.set(s3.clone(), server_b);

        cache.invalidate_server(&server_a);

        assert_eq!(cache.get(&s1, &available), None);
        assert_eq!(cache.get(&s2, &available), None);
        assert_eq!(cache.get(&s3, &available), Some(server_b));
    }
}
