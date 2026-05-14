//! RAII slot accounting guards.
//!
//! See design doc Section 2.2

use crate::tracker::DispatchTracker;
use llamaforge_types::ports::SlotNotifier;
use llamaforge_types::{RequestId, ServerId};
use std::sync::Arc;

/// RAII guard representing an actively dispatched request to a server.
///
/// Increments the server's tracking count on creation, and decrements it
/// on explicit release or drop.
///
/// See design doc Section 2.2.
#[must_use = "DispatchToken will release the slot immediately if dropped"]
pub struct DispatchToken {
    tracker: Arc<DispatchTracker>,
    request_id: RequestId,
    server_id: ServerId,
    slot_notifier: Arc<dyn SlotNotifier>,
    released: bool,
}

impl DispatchToken {
    /// Create a new token, immediately incrementing the in-flight count for the server.
    pub fn new(
        tracker: Arc<DispatchTracker>,
        request_id: RequestId,
        server_id: ServerId,
        slot_notifier: Arc<dyn SlotNotifier>,
    ) -> Self {
        tracker.insert(request_id);
        Self { tracker, request_id, server_id, slot_notifier, released: false }
    }

    /// Explicitly release the slot.
    ///
    /// This is typically called by the proxy task when it finishes streaming.
    /// Double-releases are safe and have no effect.
    pub fn release(&mut self) {
        if !self.released {
            self.tracker.remove(&self.request_id);
            self.released = true;
        }
    }

    /// The ID of the request associated with this token.
    pub fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    /// The ID of the server this token is dispatched to.
    pub fn server_id(&self) -> &ServerId {
        &self.server_id
    }
}

impl Drop for DispatchToken {
    fn drop(&mut self) {
        if !self.released {
            self.tracker.remove(&self.request_id);
            // Safety net for panics. The primary path calls `notify_slot_available` explicitly
            // after `release()`.
            self.slot_notifier.notify_slot_available(&self.server_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct MockSlotNotifier;
    impl SlotNotifier for MockSlotNotifier {
        fn notify_slot_available(&self, _server_id: &ServerId) {}
    }

    #[test]
    fn test_raii_drop_releases() {
        let tracker = Arc::new(DispatchTracker::new());
        let notifier: Arc<dyn SlotNotifier> = Arc::new(MockSlotNotifier);
        let req_id = RequestId::new(Uuid::now_v7());
        let srv_id = ServerId::new(Uuid::now_v7());

        assert_eq!(tracker.count(), 0);

        {
            let _token = DispatchToken::new(Arc::clone(&tracker), req_id, srv_id, notifier);
            assert_eq!(tracker.count(), 1);
        }

        assert_eq!(tracker.count(), 0);
    }

    #[test]
    fn test_explicit_release_prevents_double_decrement() {
        let tracker = Arc::new(DispatchTracker::new());
        let notifier: Arc<dyn SlotNotifier> = Arc::new(MockSlotNotifier);
        let req_id = RequestId::new(Uuid::now_v7());
        let srv_id = ServerId::new(Uuid::now_v7());

        let mut token = DispatchToken::new(Arc::clone(&tracker), req_id, srv_id, notifier);
        assert_eq!(tracker.count(), 1);

        token.release();
        assert_eq!(tracker.count(), 0);

        drop(token);
        assert_eq!(tracker.count(), 0); // Still 0
    }

    #[test]
    fn test_server_id_accessor() {
        let tracker = Arc::new(DispatchTracker::new());
        let notifier: Arc<dyn SlotNotifier> = Arc::new(MockSlotNotifier);
        let req_id = RequestId::new(Uuid::now_v7());
        let srv_id = ServerId::new(Uuid::now_v7());

        let token = DispatchToken::new(Arc::clone(&tracker), req_id, srv_id, notifier);
        assert_eq!(*token.server_id(), srv_id);
    }
}
