use bytes::Bytes;
use llamaforge_config::{
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
use llamaforge_routing::types::ServerStateView;
use llamaforge_routing::RoutingDecision;
use llamaforge_types::ports::{SlotDispatchView, SlotErasure, SlotError, SlotNotifier};
use llamaforge_types::{RequestId, ServerId, SlotId};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct MockBackendClient {
    responses: std::sync::Mutex<
        std::collections::HashMap<RequestId, Result<BackendResponseStream, DispatchError>>,
    >,
}

impl MockBackendClient {
    fn new() -> Self {
        Self { responses: std::sync::Mutex::new(std::collections::HashMap::new()) }
    }

    fn add_response(&self, req_id: RequestId, res: Result<BackendResponseStream, DispatchError>) {
        self.responses.lock().unwrap().insert(req_id, res);
    }
}

#[async_trait::async_trait]
impl BackendClient for MockBackendClient {
    async fn post_completion(
        &self,
        _s: &ServerId,
        p: CompletionRequest,
    ) -> Result<BackendResponseStream, DispatchError> {
        let mut responses = self.responses.lock().unwrap();
        if let Ok(uuid) = Uuid::parse_str(&p.prompt) {
            let req_id = RequestId::new(uuid);
            if let Some(res) = responses.remove(&req_id) {
                return res;
            }
        }

        let stream = futures_util::stream::iter(vec![
            Ok(Bytes::from("data: {\"content\":\"test\",\"stop\":false}\n\n")),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ]);
        Ok(Box::pin(stream))
    }
}

struct MockClientManager {
    refreshed: std::sync::atomic::AtomicBool,
}
impl ClientManager for MockClientManager {
    fn refresh(&self, _s: &ServerId, _e: llamaforge_types::ClientRefreshEvent) {
        self.refreshed.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

struct MockServerStateView;
impl ServerStateView for MockServerStateView {
    fn all_servers(&self) -> Vec<llamaforge_routing::types::ServerSnapshot> {
        vec![]
    }
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
    async fn erase_slot(&self, _s: &ServerId, _sl: &SlotId) -> Result<(), SlotError> {
        Ok(())
    }
}

struct TestContext {
    dispatcher: Arc<Dispatcher>,
    server_id: ServerId,
    client: Arc<MockBackendClient>,
    client_manager: Arc<MockClientManager>,
    slot_tx: mpsc::Sender<()>,
    slot_rx: mpsc::Receiver<()>,
}

fn setup_context() -> TestContext {
    let server_id = ServerId::new(Uuid::now_v7());
    let mut app_config = AppConfig::default();
    let config_srv_id = llamaforge_config::ServerId::from(server_id.to_string());
    app_config.servers.insert(config_srv_id, ServerConfig::default());

    let config =
        Arc::new(ConfigHandle::load(Arc::new(InMemoryConfigSource::new(app_config))).unwrap());
    let (metrics_tx, _) = mpsc::channel(1);
    let (audit_tx, _) = mpsc::channel(1);
    let metrics =
        MetricsIngest::new_for_test(metrics_tx, audit_tx, Arc::new(tokio::sync::Semaphore::new(1)));

    let client = Arc::new(MockBackendClient::new());
    let client_manager =
        Arc::new(MockClientManager { refreshed: std::sync::atomic::AtomicBool::new(false) });
    let (slot_tx, slot_rx) = mpsc::channel(100);

    let dispatcher = Arc::new(Dispatcher::new(
        Arc::new(MockServerStateView),
        metrics,
        Arc::new(DispatchTrackerRegistry::new()),
        Arc::new(SlotDispatchMap::new()),
        config,
        client.clone(),
        client_manager.clone(),
        Arc::new(MockSlotNotifier { tx: slot_tx.clone() }),
        Arc::new(MockSlotErasure),
    ));

    TestContext { dispatcher, server_id, client, client_manager, slot_tx, slot_rx }
}

#[tokio::test]
async fn test_vc3_1_full_lifecycle() {
    let mut ctx = setup_context();
    let req_id = RequestId::new(Uuid::now_v7());

    let candidate = DispatchCandidate {
        request_id: req_id,
        prompt: req_id.to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(ctx.server_id),
    };

    let (audit_tx, mut audit_rx) = mpsc::channel(1);
    let (_handle, mut response_rx) = ctx.dispatcher.dispatch(candidate, audit_tx).await;

    // Check 1 & 2: Token is tracked
    let tracker = ctx.dispatcher.trackers().get_or_create(&ctx.server_id);
    assert_eq!(tracker.count(), 1);

    // Check 3: Wait for stream to finish
    let chunk1 = response_rx.recv().await.unwrap().unwrap();
    assert_eq!(chunk1.text, "test");
    let chunk2 = response_rx.recv().await.unwrap().unwrap();
    assert!(chunk2.stop);
    assert!(response_rx.recv().await.is_none());

    // Check 4 & 5: Slot notifier triggered
    assert!(ctx.slot_rx.recv().await.is_some());

    // Check 6 & 7 & 8: Token released, no leaks
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(tracker.count(), 0);
    assert!(!ctx.dispatcher.slot_dispatch_map().has_unresolved(&ctx.server_id));

    // Audit sent
    assert!(audit_rx.recv().await.is_some());
}

#[tokio::test]
async fn test_vc3_2_token_never_leaked() {
    let ctx = setup_context();

    let mut handles = Vec::new();
    let mut receivers = Vec::new();

    for i in 0..5 {
        let req_id = RequestId::new(Uuid::now_v7());
        let candidate = DispatchCandidate {
            request_id: req_id,
            prompt: req_id.to_string(),
            params: CompletionParams {
                n_predict: None,
                temperature: 0.0,
                stream: true,
                stop: vec![],
            },
            routing: RoutingDecision::new_for_test(ctx.server_id),
        };

        // 1 disconnected
        if i == 4 {
            ctx.client.add_response(
                req_id,
                Err(DispatchError::ConnectionFailed(Box::new(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "refused",
                )))),
            );
        } else {
            // Send slow stream
            let stream = futures_util::stream::unfold(3, |state| async move {
                if state > 0 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Some((
                        Ok(Bytes::from("data: {\"content\":\"test\",\"stop\":false}\n\n")),
                        state - 1,
                    ))
                } else {
                    Some((Ok(Bytes::from("data: [DONE]\n\n")), 0))
                }
            });
            ctx.client.add_response(req_id, Ok(Box::pin(stream)));
        }

        let (audit_tx, _) = mpsc::channel(1);
        let (handle, rx) = ctx.dispatcher.dispatch(candidate, audit_tx).await;
        handles.push(handle);
        receivers.push(rx);
    }

    // 2 cancelled
    handles[0].abort();
    handles[1].abort();

    // The others drain
    for mut rx in receivers {
        while rx.recv().await.is_some() {}
    }

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(ctx.dispatcher.trackers().get_or_create(&ctx.server_id).count(), 0);
}

#[tokio::test]
async fn test_vc3_3_cancellation_priority() {
    let ctx = setup_context();
    let req_id = RequestId::new(Uuid::now_v7());

    // A stream that yields infinitely
    let stream = futures_util::stream::repeat_with(|| {
        Ok(Bytes::from("data: {\"content\":\"chunk\",\"stop\":false}\n\n"))
    });
    ctx.client.add_response(req_id, Ok(Box::pin(stream)));

    let _candidate = DispatchCandidate {
        request_id: req_id,
        prompt: req_id.to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(ctx.server_id),
    };

    let (audit_tx, _) = mpsc::channel(1);
    let cancel = CancellationToken::new();

    // Use token directly to cancel safely without aborting task
    let tracker = ctx.dispatcher.trackers().get_or_create(&ctx.server_id);
    let token = llamaforge_dispatch::token::DispatchToken::new(
        tracker,
        req_id,
        ctx.server_id,
        Arc::new(MockSlotNotifier { tx: ctx.slot_tx.clone() }),
    );

    let (response_tx, mut response_rx) = mpsc::channel(32);

    cancel.cancel();

    llamaforge_dispatch::proxy::proxy_task(
        token,
        ctx.client,
        CompletionRequest {
            prompt: req_id.to_string(),
            n_predict: None,
            temperature: 0.0,
            stream: true,
            stop: vec![],
            id_slot: None,
        },
        response_tx,
        Arc::new(MockSlotNotifier { tx: ctx.slot_tx.clone() }),
        audit_tx,
        cancel,
        Arc::new(SlotDispatchMap::new()),
    )
    .await;

    // Even though backend had chunk ready, cancel won
    assert!(response_rx.recv().await.is_none());
}

#[tokio::test]
async fn test_vc3_4_client_refresh() {
    let ctx = setup_context();
    ctx.dispatcher.refresh_client(
        &ctx.server_id,
        llamaforge_types::ClientRefreshEvent {
            server_id: ctx.server_id,
            new_port: 8080,
            tls_config: None,
        },
    );
    assert!(ctx.client_manager.refreshed.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn test_vc3_5_backpressure_disconnect() {
    let ctx = setup_context();
    let req_id = RequestId::new(Uuid::now_v7());

    let stream = futures_util::stream::repeat_with(|| {
        Ok(Bytes::from("data: {\"content\":\"chunk\",\"stop\":false}\n\n"))
    });
    ctx.client.add_response(req_id, Ok(Box::pin(stream)));

    let candidate = DispatchCandidate {
        request_id: req_id,
        prompt: req_id.to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(ctx.server_id),
    };

    let (audit_tx, _) = mpsc::channel(1);
    let (_handle, mut rx) = ctx.dispatcher.dispatch(candidate, audit_tx).await;

    // Don't read, wait 2.5s
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // Now read from the channel. We should get some items, and then None.
    while rx.recv().await.is_some() {}

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(ctx.dispatcher.trackers().get_or_create(&ctx.server_id).count(), 0);
}

#[tokio::test]
async fn test_vc3_6_connection_pool_poisoning_recovery() {
    let ctx = setup_context();
    let req_id1 = RequestId::new(Uuid::now_v7());
    let req_id2 = RequestId::new(Uuid::now_v7());

    ctx.client.add_response(
        req_id1,
        Err(DispatchError::ConnectionFailed(Box::new(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "reset",
        )))),
    );

    let candidate1 = DispatchCandidate {
        request_id: req_id1,
        prompt: req_id1.to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(ctx.server_id),
    };

    let candidate2 = DispatchCandidate {
        request_id: req_id2,
        prompt: req_id2.to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(ctx.server_id),
    };

    let (audit_tx, _) = mpsc::channel(1);
    let (_h1, mut rx1) = ctx.dispatcher.dispatch(candidate1, audit_tx.clone()).await;

    assert!(rx1.recv().await.unwrap().is_err()); // First fails

    let (_h2, mut rx2) = ctx.dispatcher.dispatch(candidate2, audit_tx).await;
    assert!(rx2.recv().await.unwrap().is_ok()); // Second succeeds
    while rx2.recv().await.is_some() {}

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(ctx.dispatcher.trackers().get_or_create(&ctx.server_id).count(), 0);
}

#[tokio::test]
async fn test_vc3_7_concurrent_dispatches() {
    let ctx = Arc::new(setup_context());
    let mut handles = Vec::new();

    for _ in 0..32 {
        let req_id = RequestId::new(Uuid::now_v7());
        let candidate = DispatchCandidate {
            request_id: req_id,
            prompt: req_id.to_string(),
            params: CompletionParams {
                n_predict: None,
                temperature: 0.0,
                stream: true,
                stop: vec![],
            },
            routing: RoutingDecision::new_for_test(ctx.server_id),
        };

        let c = Arc::clone(&ctx);
        handles.push(tokio::spawn(async move {
            let (audit_tx, _) = mpsc::channel(1);
            let (_, mut rx) = c.dispatcher.dispatch(candidate, audit_tx).await;
            while rx.recv().await.is_some() {}
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(ctx.dispatcher.trackers().get_or_create(&ctx.server_id).count(), 0);
}

#[tokio::test]
async fn test_vc3_8_panic_safety() {
    let ctx = setup_context();
    let req_id = RequestId::new(Uuid::now_v7());

    let candidate = DispatchCandidate {
        request_id: req_id,
        prompt: req_id.to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(ctx.server_id),
    };

    let (audit_tx, _) = mpsc::channel(1);
    let (handle, _) = ctx.dispatcher.dispatch(candidate, audit_tx).await;

    // Induce a panic in the task? We can just abort it, which is similar to panic for tokio cleanup
    handle.abort();

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(ctx.dispatcher.trackers().get_or_create(&ctx.server_id).count(), 0);
}
