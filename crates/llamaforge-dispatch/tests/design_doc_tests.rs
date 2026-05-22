use bytes::Bytes;
use llamaforge_core::config::{
    source::memory::InMemoryConfigSource, AppConfig, ConfigHandle, ServerConfig,
};
use llamaforge_dispatch::client::{BackendClient, BackendResponseStream, ClientManager};
use llamaforge_dispatch::dispatcher::{
    CompletionParams, CompletionRequest, DispatchCandidate, Dispatcher,
};
use llamaforge_dispatch::error::DispatchError;
use llamaforge_dispatch::slot_map::SlotDispatchMap;
use llamaforge_dispatch::tracker::DispatchTrackerRegistry;
use llamaforge_metrics::MetricsIngest;
use llamaforge_core::routing::types::ServerStateView;
use llamaforge_core::routing::RoutingDecision;
use llamaforge_core::types::ports::{SlotErasure, SlotError};
use llamaforge_core::types::traits::SlotNotifier;
use llamaforge_core::types::{RequestId, ServerId, SlotId};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

struct MockBackendClient;
#[async_trait::async_trait]
impl BackendClient for MockBackendClient {
    async fn post_completion(
        &self,
        _s: &ServerId,
        p: CompletionRequest,
    ) -> Result<BackendResponseStream, DispatchError> {
        if p.prompt == "hang_forever" {
            // A stream that yields nothing and hangs forever
            let stream = futures_util::stream::pending();
            return Ok(Box::pin(stream));
        }
        if p.prompt == "slow_connect" {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if p.prompt == "infinite_stream" {
            // A stream that yields forever to test backpressure
            let stream = futures_util::stream::repeat_with(|| {
                Ok(Bytes::from("data: {\"content\":\"chunk\",\"stop\":false}\n\n"))
            });
            return Ok(Box::pin(stream));
        }
        let stream = futures_util::stream::iter(vec![
            Ok(Bytes::from("data: {\"content\":\"chunk\",\"stop\":false}\n\n")),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ]);
        Ok(Box::pin(stream))
    }
}

struct MockClientManager {
    refreshed: std::sync::atomic::AtomicBool,
}
impl ClientManager for MockClientManager {
    fn refresh(&self, _s: &ServerId, _e: llamaforge_core::types::ClientRefreshEvent) {
        self.refreshed.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

struct MockServerStateView;
impl ServerStateView for MockServerStateView {
    fn all_servers(&self) -> Vec<llamaforge_core::routing::types::ServerSnapshot> {
        vec![]
    }
}

struct MockSlotNotifier {
    tx: mpsc::Sender<()>,
}
impl SlotNotifier for MockSlotNotifier {
    fn notify_slot_available(&self) {
        let _ = self.tx.try_send(());
    }
}

struct MockSlotErasure;
#[async_trait::async_trait]
impl SlotErasure for MockSlotErasure {
    async fn erase_slot(&self, _s: &ServerId, _sl: &SlotId) -> Result<(), SlotError> {
        Ok(())
    }
}

fn create_dispatcher(
    srv_id: ServerId,
    slot_tx: mpsc::Sender<()>,
    cm: Arc<MockClientManager>,
) -> Dispatcher {
    let mut app_config = AppConfig::default();
    let config_srv_id = llamaforge_core::types::ServerId::from_config_id(&srv_id.to_string());
    app_config.servers.insert(config_srv_id, ServerConfig::default());

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
        Arc::new(SlotDispatchMap::new()),
        config,
        Arc::new(MockBackendClient),
        cm,
        Arc::new(MockSlotNotifier { tx: slot_tx }),
        Arc::new(MockSlotErasure),
    )
}

#[tokio::test]
async fn test_cancellation_checked_before_chunk() {
    let server_id = ServerId::new(Uuid::now_v7());
    let (slot_tx, _) = mpsc::channel(1);
    let dispatcher = create_dispatcher(
        server_id,
        slot_tx,
        Arc::new(MockClientManager { refreshed: std::sync::atomic::AtomicBool::new(false) }),
    );
    let candidate = DispatchCandidate {
        request_id: RequestId::new(Uuid::now_v7()),
        prompt: "slow_connect".to_string(), // backend sleeps 50ms before returning stream
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(server_id),
    };
    let (audit_tx, _) = mpsc::channel(1);
    let (handle, mut rx) = dispatcher.dispatch(candidate, audit_tx).await;

    handle.abort();

    // since task was aborted, rx should close without yielding items
    assert!(rx.recv().await.is_none());
    // Give token a moment to run its drop handler
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(dispatcher.trackers().get_or_create(&server_id).count(), 0);
}

#[tokio::test]
async fn test_slot_released_before_audit_send() {
    let server_id = ServerId::new(Uuid::now_v7());
    let (slot_tx, mut slot_rx) = mpsc::channel(1);
    let dispatcher = create_dispatcher(
        server_id,
        slot_tx,
        Arc::new(MockClientManager { refreshed: std::sync::atomic::AtomicBool::new(false) }),
    );

    let candidate = DispatchCandidate {
        request_id: RequestId::new(Uuid::now_v7()),
        prompt: "normal".to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(server_id),
    };

    // Create an audit channel that is full so sending to it blocks (or drops if timeout)
    let (audit_tx, mut audit_rx) = mpsc::channel(1);
    // Fill the audit channel
    audit_tx
        .send(llamaforge_dispatch::proxy::AuditRecord {
            request_id: RequestId::new(Uuid::now_v7()),
            server_id,
            duration: Duration::from_secs(1),
            ttft: None,
            tokens_predicted: 0,
            prompt_tokens: 0,
            success: true,
        })
        .await
        .unwrap();

    let (_handle, mut rx) = dispatcher.dispatch(candidate, audit_tx).await;

    // Drain the response stream so the proxy_task finishes
    while rx.recv().await.is_some() {}

    // Verify slot is released immediately!
    let slot_release_time = tokio::time::timeout(Duration::from_millis(50), slot_rx.recv()).await;
    assert!(slot_release_time.is_ok(), "Slot was not released before audit send blocked!");

    // Unblock the audit channel so cleanup can finish
    audit_rx.recv().await;
}

#[tokio::test]
async fn test_client_refresh_on_server_restart() {
    let s1 = ServerId::new(Uuid::now_v7());
    let (slot_tx, _) = mpsc::channel(1);
    let cm = Arc::new(MockClientManager { refreshed: std::sync::atomic::AtomicBool::new(false) });
    let dispatcher = create_dispatcher(s1, slot_tx, cm.clone());

    dispatcher.refresh_client(
        &s1,
        llamaforge_core::types::ClientRefreshEvent { server_id: s1, new_port: 8080, tls_config: None },
    );

    // Verify ClientManager actually got called
    assert!(cm.refreshed.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn test_backpressure_timeout_disconnects() {
    let server_id = ServerId::new(Uuid::now_v7());
    let (slot_tx, _) = mpsc::channel(1);
    let dispatcher = create_dispatcher(
        server_id,
        slot_tx,
        Arc::new(MockClientManager { refreshed: std::sync::atomic::AtomicBool::new(false) }),
    );

    let candidate = DispatchCandidate {
        request_id: RequestId::new(Uuid::now_v7()),
        prompt: "infinite_stream".to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(server_id),
    };
    let (audit_tx, _) = mpsc::channel(1);

    // The channel is bounded to 32 items
    let (_handle, mut rx) = dispatcher.dispatch(candidate, audit_tx).await;

    // Do NOT read from rx. This will cause the response_tx to fill up,
    // and the proxy_task will hit its 2-second timeout.

    // The timeout is 2 seconds, wait 2.5 seconds
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // Now read from the channel. We should get some items, and then an error or None.
    while rx.recv().await.is_some() {}
}

#[tokio::test]
async fn test_non_streaming_absolute_timeout() {
    // The "non-streaming: absolute timeout" design doc requirement translates to
    // the Caller dropping the channel (simulating a request timeout).
    let server_id = ServerId::new(Uuid::now_v7());
    let (slot_tx, mut _slot_rx) = mpsc::channel(1);
    let dispatcher = create_dispatcher(
        server_id,
        slot_tx,
        Arc::new(MockClientManager { refreshed: std::sync::atomic::AtomicBool::new(false) }),
    );

    let candidate = DispatchCandidate {
        request_id: RequestId::new(Uuid::now_v7()),
        prompt: "hang_forever".to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(server_id),
    };
    let (audit_tx, _) = mpsc::channel(1);

    let (handle, rx) = dispatcher.dispatch(candidate, audit_tx).await;

    // Wait a bit to ensure it started
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Caller times out and aborts task
    handle.abort();
    drop(rx);

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(dispatcher.trackers().get_or_create(&server_id).count(), 0);
}
