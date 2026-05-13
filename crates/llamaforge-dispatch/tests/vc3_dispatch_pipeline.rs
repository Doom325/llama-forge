use llamaforge_dispatch::client::{BackendClient, BackendResponseStream, ClientManager};
use llamaforge_dispatch::dispatcher::{Dispatcher, CompletionRequest, DispatchCandidate, CompletionParams};
use llamaforge_dispatch::error::DispatchError;
use llamaforge_dispatch::slot_map::SlotDispatchMap;
use llamaforge_dispatch::tracker::DispatchTrackerRegistry;
use llamaforge_config::{ConfigHandle, source::memory::InMemoryConfigSource, AppConfig, ServerConfig};
use llamaforge_metrics::MetricsIngest;
use llamaforge_routing::RoutingDecision;
use llamaforge_routing::types::ServerStateView;
use llamaforge_types::ports::{SlotErasure, SlotError, SlotNotifier, SlotDispatchView};
use llamaforge_types::{ServerId, SlotId, RequestId};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;
use bytes::Bytes;

struct MockBackendClient;
#[async_trait::async_trait]
impl BackendClient for MockBackendClient {
    async fn post_completion(&self, _s: &ServerId, _p: CompletionRequest) -> Result<BackendResponseStream, DispatchError> {
        let stream = futures_util::stream::iter(vec![
            Ok(Bytes::from("data: {\"content\":\"test\",\"stop\":false}\n\n")),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ]);
        Ok(Box::pin(stream))
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

struct MockSlotNotifier {
    tx: mpsc::Sender<()>,
}
impl SlotNotifier for MockSlotNotifier {
    fn notify_slot_available(&self, _s: &ServerId) {
        let _ = self.tx.try_send(());
    }
}

struct MockSlotErasure;
#[async_trait::async_trait]
impl SlotErasure for MockSlotErasure {
    async fn erase_slot(&self, _s: &ServerId, _sl: &SlotId) -> Result<(), SlotError> { Ok(()) }
}

fn create_dispatcher(tx: mpsc::Sender<()>, srv_id: ServerId) -> Dispatcher {
    let mut app_config = AppConfig::default();
    let config_srv_id = llamaforge_config::ServerId::from(srv_id.to_string());
    app_config.servers.insert(config_srv_id, ServerConfig::default());

    let config = Arc::new(ConfigHandle::load(Arc::new(InMemoryConfigSource::new(app_config))).unwrap());
    let (metrics_tx, _) = mpsc::channel(1);
    let (audit_tx, _) = mpsc::channel(1);
    let metrics = MetricsIngest::new_for_test(metrics_tx, audit_tx, Arc::new(tokio::sync::Semaphore::new(1)));

    Dispatcher::new(
        Arc::new(MockServerStateView),
        metrics,
        Arc::new(DispatchTrackerRegistry::new()),
        Arc::new(SlotDispatchMap::new()),
        config,
        Arc::new(MockBackendClient),
        Arc::new(MockClientManager),
        Arc::new(MockSlotNotifier { tx }),
        Arc::new(MockSlotErasure),
    )
}

#[tokio::test]
async fn test_vc3_integration_checks() {
    let server_id = ServerId::new(Uuid::now_v7());
    let (tx, mut rx) = mpsc::channel(1);
    let dispatcher = create_dispatcher(tx, server_id);
    
    let candidate = DispatchCandidate {
        request_id: RequestId::new(Uuid::now_v7()),
        prompt: "test".to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(server_id),
    };
    
    let (audit_tx, _) = mpsc::channel(1);
    let (_handle, mut response_rx) = dispatcher.dispatch(candidate, audit_tx).await;
    
    // Check 1 & 2: Token is tracked
    let tracker = dispatcher.trackers().get_or_create(&server_id);
    assert_eq!(tracker.count(), 1);
    
    // Check 3: Wait for stream to finish
    let chunk1 = response_rx.recv().await.unwrap().unwrap();
    assert_eq!(chunk1.text, "test");
    
    let chunk2 = response_rx.recv().await.unwrap().unwrap();
    assert!(chunk2.stop);
    
    assert!(response_rx.recv().await.is_none());
    
    // Check 4 & 5: Slot notifier triggered
    assert!(rx.recv().await.is_some());
    
    // Check 6 & 7 & 8: Token released, no leaks
    tokio::time::sleep(Duration::from_millis(10)).await; // give cleanup task time
    assert_eq!(tracker.count(), 0);
    assert!(!dispatcher.slot_dispatch_map().has_unresolved(&server_id));
}