use bytes::Bytes;
use llamaforge_dispatch::client::{BackendClient, BackendResponseStream};
use llamaforge_dispatch::dispatcher::CompletionRequest;
use llamaforge_dispatch::error::{DispatchError, StreamError};
use llamaforge_dispatch::proxy::{proxy_task, AuditRecord};
use llamaforge_dispatch::slot_map::SlotDispatchMap;
use llamaforge_dispatch::token::DispatchToken;
use llamaforge_dispatch::tracker::DispatchTrackerRegistry;
use llamaforge_types::ports::SlotNotifier;
use llamaforge_types::{RequestId, ServerId};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

struct MockBackendClient {
    chunks: Mutex<Option<Vec<Result<Bytes, StreamError>>>>,
    error_on_connect: Mutex<Option<DispatchError>>,
    delay: Option<Duration>,
    chunk_delay: Option<Duration>,
}

#[async_trait::async_trait]
impl BackendClient for MockBackendClient {
    async fn post_completion(
        &self,
        _server_id: &ServerId,
        _payload: CompletionRequest,
    ) -> Result<BackendResponseStream, DispatchError> {
        if let Some(err) = self.error_on_connect.lock().unwrap().take() {
            return Err(err);
        }
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        let chunks = self.chunks.lock().unwrap().take().unwrap_or_default();
        let chunk_delay = self.chunk_delay;

        let stream = futures_util::stream::unfold(chunks.into_iter(), move |mut iter| async move {
            if let Some(chunk) = iter.next() {
                if let Some(d) = chunk_delay {
                    tokio::time::sleep(d).await;
                }
                Some((chunk, iter))
            } else {
                None
            }
        });
        Ok(Box::pin(stream))
    }
}

struct MockSlotNotifier;
impl SlotNotifier for MockSlotNotifier {
    fn notify_slot_available(&self, _server_id: &ServerId) {}
}

#[allow(clippy::type_complexity)]
fn setup() -> (
    DispatchToken,
    mpsc::Sender<Result<llamaforge_dispatch::proxy::ParsedChunk, StreamError>>,
    mpsc::Receiver<Result<llamaforge_dispatch::proxy::ParsedChunk, StreamError>>,
    mpsc::Sender<AuditRecord>,
    mpsc::Receiver<AuditRecord>,
    CancellationToken,
    Arc<SlotDispatchMap>,
    CompletionRequest,
    Arc<DispatchTrackerRegistry>,
) {
    let registry = Arc::new(DispatchTrackerRegistry::new());
    let server_id = ServerId::new(Uuid::now_v7());
    let request_id = RequestId::new(Uuid::now_v7());
    let tracker = registry.get_or_create(&server_id);
    let token = DispatchToken::new(tracker, request_id, server_id, Arc::new(MockSlotNotifier));

    let (tx, rx) = mpsc::channel(32);
    let (audit_tx, audit_rx) = mpsc::channel(32);
    let cancel = CancellationToken::new();
    let slot_map = Arc::new(SlotDispatchMap::new());
    let req = CompletionRequest {
        prompt: "".to_string(),
        n_predict: None,
        temperature: 0.0,
        stream: true,
        stop: vec![],
        id_slot: None,
    };

    (token, tx, rx, audit_tx, audit_rx, cancel, slot_map, req, registry)
}

#[tokio::test]
async fn test_6_10_successful_streaming() {
    let (token, tx, mut rx, audit_tx, mut audit_rx, cancel, map, req, _) = setup();
    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![
            Ok(Bytes::from("data: {\"content\":\"chunk1\",\"stop\":false}\n\n")),
            Ok(Bytes::from("data: {\"content\":\"chunk2\",\"stop\":false}\n\n")),
            Ok(Bytes::from("data: [DONE]\n\n")),
        ])),
        error_on_connect: Mutex::new(None),
        delay: None,
        chunk_delay: None,
    });

    proxy_task(token, client, req, tx, Arc::new(MockSlotNotifier), audit_tx, cancel, map).await;

    let res1 = rx.recv().await.unwrap().unwrap();
    assert_eq!(res1.text, "chunk1");
    let res2 = rx.recv().await.unwrap().unwrap();
    assert_eq!(res2.text, "chunk2");
    let res3 = rx.recv().await.unwrap().unwrap();
    assert!(res3.stop);

    let audit = audit_rx.recv().await.unwrap();
    assert_eq!(audit.tokens_predicted, 2);
    assert!(audit.success);
}

#[tokio::test]
async fn test_6_11_cancellation_before_first_chunk() {
    let (token, tx, mut rx, audit_tx, mut audit_rx, cancel, map, req, _) = setup();
    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![Ok(Bytes::from(
            "data: {\"content\":\"slow\",\"stop\":false}\n\n",
        ))])),
        error_on_connect: Mutex::new(None),
        delay: Some(Duration::from_millis(50)),
        chunk_delay: None,
    });

    let cancel_clone = cancel.clone();
    tokio::spawn(async move {
        cancel_clone.cancel();
    });

    proxy_task(token, client, req, tx, Arc::new(MockSlotNotifier), audit_tx, cancel, map).await;

    // Task exits without sending anything since it was cancelled before streaming phase
    assert!(rx.recv().await.is_none());

    let audit = audit_rx.recv().await.unwrap();
    assert!(!audit.success);
}

#[tokio::test]
async fn test_6_12_cancellation_mid_stream() {
    let (token, tx, mut rx, audit_tx, mut audit_rx, cancel, map, req, _) = setup();
    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![
            Ok(Bytes::from("data: {\"content\":\"1\",\"stop\":false}\n\n")),
            Ok(Bytes::from("data: {\"content\":\"2\",\"stop\":false}\n\n")),
        ])),
        error_on_connect: Mutex::new(None),
        delay: None,
        chunk_delay: Some(Duration::from_millis(50)),
    });

    let cancel_clone = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(75)).await;
        cancel_clone.cancel();
    });

    proxy_task(token, client, req, tx, Arc::new(MockSlotNotifier), audit_tx, cancel, map).await;

    // May receive failure
    let mut got_failure = false;
    while let Some(res) = rx.recv().await {
        if let Ok(chunk) = res {
            if chunk.finish_reason.as_deref() == Some("FINISH_REASON_UPSTREAM_FAILURE") {
                got_failure = true;
            }
        }
    }
    assert!(got_failure);
    let audit = audit_rx.recv().await.unwrap();
    assert!(!audit.success);
}

#[tokio::test]
async fn test_6_13_backend_disconnect() {
    let (token, tx, mut rx, audit_tx, mut audit_rx, cancel, map, req, _) = setup();
    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![
            Ok(Bytes::from("data: {\"content\":\"1\",\"stop\":false}\n\n")),
            Err(StreamError::Cancelled), // Simulate disconnect
        ])),
        error_on_connect: Mutex::new(None),
        delay: None,
        chunk_delay: None,
    });

    proxy_task(token, client, req, tx, Arc::new(MockSlotNotifier), audit_tx, cancel, map).await;

    let _ = rx.recv().await.unwrap(); // first chunk
    let err = rx.recv().await.unwrap(); // error
    assert!(err.is_err());

    let audit = audit_rx.recv().await.unwrap();
    assert!(!audit.success);
}

#[tokio::test]
async fn test_6_14_backpressure_timeout() {
    let (token, tx, mut rx, audit_tx, mut audit_rx, cancel, map, req, _) = setup();
    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![Ok(Bytes::from(
            "data: {\"content\":\"1\",\"stop\":false}\n\n",
        ))])),
        error_on_connect: Mutex::new(None),
        delay: None,
        chunk_delay: None,
    });

    // Fill the channel so send blocks
    for _ in 0..32 {
        tx.send(Ok(llamaforge_dispatch::proxy::ParsedChunk {
            text: "".to_string(),
            stop: false,
            tokens_predicted: 0,
            finish_reason: None,
        }))
        .await
        .unwrap();
    }

    // The task should timeout and disconnect
    proxy_task(token, client, req, tx, Arc::new(MockSlotNotifier), audit_tx, cancel, map).await;

    let audit = audit_rx.recv().await.unwrap();
    assert!(!audit.success);

    // flush the channel so the test passes
    while rx.try_recv().is_ok() {}
}

struct OrderSlotNotifier {
    tx: tokio::sync::mpsc::Sender<()>,
}

impl SlotNotifier for OrderSlotNotifier {
    fn notify_slot_available(&self, _server_id: &ServerId) {
        let _ = self.tx.try_send(());
    }
}

#[tokio::test]
async fn test_6_15_slot_released_before_audit() {
    let registry = Arc::new(DispatchTrackerRegistry::new());
    let server_id = ServerId::new(Uuid::now_v7());
    let request_id = RequestId::new(Uuid::now_v7());
    let tracker = registry.get_or_create(&server_id);

    let (slot_tx, mut slot_rx) = tokio::sync::mpsc::channel(1);
    let notifier = Arc::new(OrderSlotNotifier { tx: slot_tx });
    let token = DispatchToken::new(tracker, request_id, server_id, notifier.clone());

    let (tx, _rx) = mpsc::channel(32);
    let (audit_tx, mut audit_rx) = mpsc::channel(32);
    let cancel = CancellationToken::new();
    let slot_map = Arc::new(SlotDispatchMap::new());
    let req = CompletionRequest {
        prompt: "".to_string(),
        n_predict: None,
        temperature: 0.0,
        stream: true,
        stop: vec![],
        id_slot: None,
    };

    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![Ok(Bytes::from("data: [DONE]\n\n"))])),
        error_on_connect: Mutex::new(None),
        delay: None,
        chunk_delay: None,
    });

    proxy_task(token, client, req, tx, notifier, audit_tx, cancel, slot_map).await;

    // The slot must be released immediately before the audit task even sends.
    assert!(slot_rx.try_recv().is_ok());
    let audit = audit_rx.recv().await.unwrap();
    assert!(audit.success);
}

#[tokio::test]
async fn test_6_16_connection_refused() {
    let (token, tx, mut rx, audit_tx, mut audit_rx, cancel, map, req, _) = setup();
    let err = DispatchError::ConnectionFailed(Box::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "refused",
    )));
    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![])),
        error_on_connect: Mutex::new(Some(err)),
        delay: None,
        chunk_delay: None,
    });

    proxy_task(token, client, req, tx, Arc::new(MockSlotNotifier), audit_tx, cancel, map).await;

    let err = rx.recv().await.unwrap();
    assert!(err.is_err());

    let audit = audit_rx.recv().await.unwrap();
    assert!(!audit.success);
}

#[tokio::test]
async fn test_6_17_backend_http_500() {
    let (token, tx, mut rx, audit_tx, _audit_rx, cancel, map, req, _) = setup();
    let err = DispatchError::BackendError { status: llamaforge_dispatch::error::HttpStatus(500) };
    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![])),
        error_on_connect: Mutex::new(Some(err)),
        delay: None,
        chunk_delay: None,
    });

    proxy_task(token, client, req, tx, Arc::new(MockSlotNotifier), audit_tx, cancel, map).await;

    let err = rx.recv().await.unwrap();
    assert!(err.is_err());
}

#[tokio::test]
async fn test_6_18_ttft_metric() {
    let (token, tx, _rx, audit_tx, mut audit_rx, cancel, map, req, _) = setup();
    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![Ok(Bytes::from("data: [DONE]\n\n"))])),
        error_on_connect: Mutex::new(None),
        delay: Some(Duration::from_millis(50)),
        chunk_delay: None,
    });

    proxy_task(token, client, req, tx, Arc::new(MockSlotNotifier), audit_tx, cancel, map).await;

    let audit = audit_rx.recv().await.unwrap();
    assert!(audit.ttft.is_some());
}

use llamaforge_config::ConfigHandle;
use llamaforge_dispatch::client::ClientManager;
use llamaforge_dispatch::dispatcher::{CompletionParams, DispatchCandidate, Dispatcher};
use llamaforge_metrics::MetricsIngest;
use llamaforge_routing::types::ServerStateView;
use llamaforge_routing::RoutingDecision;

struct MockServerStateView;
impl ServerStateView for MockServerStateView {
    fn all_servers(&self) -> Vec<llamaforge_routing::types::ServerSnapshot> {
        vec![]
    }
}

struct MockClientManager;
impl ClientManager for MockClientManager {
    fn refresh(&self, _server_id: &ServerId, _event: llamaforge_types::ClientRefreshEvent) {}
}

struct MockSlotErasure;
#[async_trait::async_trait]
impl llamaforge_types::ports::SlotErasure for MockSlotErasure {
    async fn erase_slot(
        &self,
        _server_id: &ServerId,
        _slot_id: &llamaforge_types::SlotId,
    ) -> Result<(), llamaforge_types::ports::SlotError> {
        Ok(())
    }
}

fn create_dispatcher(client: Arc<dyn BackendClient>) -> Dispatcher {
    let trackers = Arc::new(DispatchTrackerRegistry::new());
    let slot_map = Arc::new(SlotDispatchMap::new());
    let config = Arc::new(
        ConfigHandle::load(Arc::new(llamaforge_config::source::memory::InMemoryConfigSource::new(
            llamaforge_config::AppConfig::default(),
        )))
        .unwrap(),
    );
    let (metrics_tx, _) = tokio::sync::mpsc::channel(1);
    let (audit_tx, _) = tokio::sync::mpsc::channel(1);
    let metrics =
        MetricsIngest::new_for_test(metrics_tx, audit_tx, Arc::new(tokio::sync::Semaphore::new(1)));

    Dispatcher::new(
        Arc::new(MockServerStateView),
        metrics,
        trackers,
        slot_map,
        config,
        client,
        Arc::new(MockClientManager),
        Arc::new(MockSlotNotifier),
        Arc::new(MockSlotErasure),
    )
}

#[tokio::test]
async fn test_6_19_graceful_shutdown() {
    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![Ok(Bytes::from("data: [DONE]\n\n"))])),
        error_on_connect: Mutex::new(None),
        delay: Some(Duration::from_millis(50)),
        chunk_delay: None,
    });

    let dispatcher = create_dispatcher(client);
    let server_id = ServerId::new(Uuid::now_v7());

    let candidate = DispatchCandidate {
        request_id: RequestId::new(Uuid::now_v7()),
        prompt: "".to_string(),
        params: CompletionParams { n_predict: None, temperature: 0.0, stream: true, stop: vec![] },
        routing: RoutingDecision::new_for_test(server_id),
    };

    let (audit_tx, _) = tokio::sync::mpsc::channel(1);
    let _ = dispatcher.dispatch(candidate, audit_tx).await;

    // Shutdown waits for tasks to complete.
    let start = std::time::Instant::now();
    dispatcher.shutdown().await;
    let elapsed = start.elapsed();

    assert!(elapsed >= Duration::from_millis(50));
}

#[tokio::test]
async fn test_6_20_drain_server() {
    let client = Arc::new(MockBackendClient {
        chunks: Mutex::new(Some(vec![Ok(Bytes::from("data: [DONE]\n\n"))])),
        error_on_connect: Mutex::new(None),
        delay: Some(Duration::from_millis(100)),
        chunk_delay: None,
    });

    let dispatcher = create_dispatcher(client);
    let server1 = ServerId::new(Uuid::now_v7());
    let server2 = ServerId::new(Uuid::now_v7());
    let (audit_tx, _) = tokio::sync::mpsc::channel(1);

    for server_id in [server1, server2] {
        let candidate = DispatchCandidate {
            request_id: RequestId::new(Uuid::now_v7()),
            prompt: "".to_string(),
            params: CompletionParams {
                n_predict: None,
                temperature: 0.0,
                stream: true,
                stop: vec![],
            },
            routing: RoutingDecision::new_for_test(server_id),
        };
        let _ = dispatcher.dispatch(candidate, audit_tx.clone()).await;
    }

    // Allow tasks to be spawned
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Drain server 1
    let count = dispatcher.drain_server(&server1).await;
    assert_eq!(count, 1, "Should drain exactly 1 task for server 1");

    // Server 2 tasks should still be active, but let's just assert drain_server returned 1
}
