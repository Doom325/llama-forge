use llamaforge_dispatch::client::{BackendClient, BackendResponseStream, ClientManager};
use llamaforge_dispatch::dispatcher::{Dispatcher, CompletionRequest, DispatchCandidate, CompletionParams};
use llamaforge_dispatch::error::DispatchError;
use llamaforge_dispatch::slot_map::SlotDispatchMap;
use llamaforge_dispatch::tracker::DispatchTrackerRegistry;
use llamaforge_config::{ConfigHandle, source::memory::InMemoryConfigSource, AppConfig, ServerConfig};
use llamaforge_metrics::MetricsIngest;
use llamaforge_routing::RoutingDecision;
use llamaforge_routing::types::ServerStateView;
use llamaforge_types::ports::{SlotErasure, SlotError, SlotNotifier};
use llamaforge_types::{ServerId, SlotId, RequestId};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

struct MockBackendClient;
#[async_trait::async_trait]
impl BackendClient for MockBackendClient {
    async fn post_completion(&self, _s: &ServerId, _p: CompletionRequest) -> Result<BackendResponseStream, DispatchError> {
        // Simulate a small delay and successful completion
        tokio::time::sleep(Duration::from_millis(5)).await;
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

fn create_dispatcher(srv_id: ServerId) -> Dispatcher {
    let mut app_config = AppConfig::default();
    let config_srv_id = llamaforge_config::ServerId::from(srv_id.to_string());
    app_config.servers.insert(config_srv_id, ServerConfig::default());

    let config = Arc::new(ConfigHandle::load(Arc::new(InMemoryConfigSource::new(app_config))).unwrap());
    let (metrics_tx, _) = mpsc::channel(1000);
    let (audit_tx, _) = mpsc::channel(1000);
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

#[tokio::test]
#[ignore]
async fn test_8_7_stress_100_concurrent_dispatches() {
    let server_id = ServerId::new(Uuid::now_v7());
    let dispatcher = Arc::new(create_dispatcher(server_id));
    let mut handles = Vec::new();

    for _ in 0..100 {
        let d = Arc::clone(&dispatcher);
        let srv = server_id;
        
        let candidate = DispatchCandidate {
            request_id: RequestId::new(Uuid::now_v7()),
            prompt: "stress".to_string(),
            params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
            routing: RoutingDecision::new_for_test(srv),
        };
        
        let (audit_tx, _) = mpsc::channel(1);
        handles.push(tokio::spawn(async move {
            let (_, mut rx) = d.dispatch(candidate, audit_tx).await;
            while rx.recv().await.is_some() {}
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // Ensure all tasks are drained and joined
    dispatcher.shutdown().await;

    let tracker = dispatcher.trackers().get_or_create(&server_id);
    assert_eq!(tracker.count(), 0);
}

#[tokio::test]
#[ignore]
async fn test_8_8_stress_rapid_client_refresh() {
    let server_id = ServerId::new(Uuid::now_v7());
    let dispatcher = Arc::new(create_dispatcher(server_id));
    let mut handles = Vec::new();

    for i in 0..50 {
        let d = Arc::clone(&dispatcher);
        let srv = server_id;
        
        if i % 5 == 0 {
            let d_clone = Arc::clone(&dispatcher);
            let s_clone = srv;
            tokio::spawn(async move {
                d_clone.refresh_client(&s_clone, llamaforge_types::ClientRefreshEvent {
                    server_id: s_clone, new_port: 8080, tls_config: None
                });
            });
        }
        
        let candidate = DispatchCandidate {
            request_id: RequestId::new(Uuid::now_v7()),
            prompt: "stress".to_string(),
            params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
            routing: RoutingDecision::new_for_test(srv),
        };
        
        let (audit_tx, _) = mpsc::channel(1);
        handles.push(tokio::spawn(async move {
            let (_, mut rx) = d.dispatch(candidate, audit_tx).await;
            while rx.recv().await.is_some() {}
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    dispatcher.shutdown().await;

    let tracker = dispatcher.trackers().get_or_create(&server_id);
    assert_eq!(tracker.count(), 0);
}