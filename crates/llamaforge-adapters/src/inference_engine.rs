//! `InferenceEngineAdapter` — the keystone adapter that wires queue, routing,
//! dispatch and metrics into the `InferenceEngine` port consumed by transport.
//!
//! See design doc §4.2 (Hexagonal layering) and §5.4 (Request lifecycle).
//!
//! ## SmartRouter integration
//!
//! Routing is performed by the queue scheduler (see
//! `llamaforge-queue::pipeline::run_scheduler`), which calls
//! `ServerSelector::select_server` and emits the picked
//! `(server_id, slot_id)` on the `DispatchEvent::Dispatched` channel
//! carried by `QueueTicket`. This adapter therefore does **not** call
//! `Router::select` directly; it forwards the scheduler's pick into
//! `Dispatcher::dispatch` via `RoutingDecision::for_scheduler_pick`.

use async_stream::stream;
use futures_core::Stream;
use std::pin::Pin;
use std::sync::Arc;

use llamaforge_dispatch::dispatcher::{CompletionParams, DispatchCandidate, Dispatcher};
use llamaforge_metrics::MetricsIngest;
use llamaforge_routing::{Router, RoutingDecision};
use llamaforge_transport::InferenceEngine;
use llamaforge_types::traits::{DispatchError, DispatchEvent, IncomingRequest, QueueSubmitter};
use llamaforge_types::{
    ChatCompletionChunk, ChatCompletionRequest, GenerationParams, ModelAlias, ModelTarget,
    RequestId, RequestorClass, RequestorId, Role, StreamError,
};

/// Adapter implementing [`InferenceEngine`] by composing the queue, router,
/// dispatcher and metrics layers. See module-level docs.
pub struct InferenceEngineAdapter {
    queue: Arc<dyn QueueSubmitter>,
    /// Held for future RAG/affinity decisions made outside the scheduler
    /// hot path; not consulted by `chat_completion` today (see module docs).
    #[allow(dead_code)]
    router: Arc<Router>,
    dispatcher: Arc<Dispatcher>,
    #[allow(dead_code)]
    metrics: Arc<MetricsIngest>,
}

impl InferenceEngineAdapter {
    pub fn new(
        queue: Arc<dyn QueueSubmitter>,
        router: Arc<Router>,
        dispatcher: Arc<Dispatcher>,
        metrics: Arc<MetricsIngest>,
    ) -> Self {
        Self { queue, router, dispatcher, metrics }
    }
}

fn map_stream_error(e: llamaforge_dispatch::error::StreamError) -> StreamError {
    use llamaforge_dispatch::error::StreamError as De;
    match e {
        De::BackendDisconnect => StreamError::BackendDisconnect,
        De::Timeout => StreamError::Timeout,
        De::Cancelled => StreamError::Cancelled,
        De::ChunkParse { reason } => StreamError::ChunkParse { reason },
        De::ChannelClosed => StreamError::Upstream(Box::new(De::ChannelClosed)),
        De::Internal(inner) => StreamError::Upstream(inner),
    }
}

fn render_prompt(messages: &[llamaforge_types::Message]) -> String {
    messages
        .iter()
        .map(|m| format!("{}: {}", role_str(&m.role), m.content))
        .collect::<Vec<_>>()
        .join("\n")
}

fn role_str(r: &Role) -> &'static str {
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

#[async_trait::async_trait]
impl InferenceEngine for InferenceEngineAdapter {
    async fn chat_completion(
        &self,
        req: ChatCompletionRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>,
        DispatchError,
    > {
        let request_id = RequestId::new(uuid::Uuid::now_v7());
        let requestor_id = RequestorId::new(Arc::from("anonymous"));
        let requestor_class = RequestorClass::new(Arc::from("default"));
        let model_target = ModelTarget::Exact(ModelAlias::new(Arc::from(req.model.clone())));

        let params = GenerationParams {
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            top_p: None,
            top_k: None,
            logit_bias: req.logit_bias.clone(),
            stop: vec![],
            seed: None,
        };

        let incoming = IncomingRequest {
            request_id,
            requestor_id,
            requestor_class,
            model_target,
            capabilities: Default::default(),
            messages: req.messages.clone(),
            params,
            stream: req.stream,
            session_id: None,
            submitted_at: chrono::Utc::now(),
        };

        let mut ticket = self.queue.submit(incoming).map_err(|_| DispatchError::Failed)?;

        let prompt = render_prompt(&req.messages);
        let completion_params = CompletionParams {
            n_predict: req.max_tokens,
            temperature: req.temperature.unwrap_or(1.0),
            stream: req.stream,
            stop: vec![],
        };

        let dispatcher = Arc::clone(&self.dispatcher);

        let s = stream! {
            let start_time = tokio::time::Instant::now();
            let mut first_token = false;
            let mut tokens_emitted: u32 = 0;
            let cancel_token = ticket.cancel_token.clone();

            // Phase 1: wait for the scheduler to dispatch, fail, or be cancelled.
            // While waiting, forward `Queued` events as `QueueUpdate` chunks.
            let (server_id, slot_id) = loop {
                tokio::select! {
                    biased;
                    _ = cancel_token.cancelled() => {
                        yield Err(StreamError::Cancelled);
                        return;
                    }
                    ev = ticket.events.recv() => {
                        match ev {
                            Some(DispatchEvent::Queued { position }) => {
                                yield Ok(ChatCompletionChunk::QueueUpdate { position });
                            }
                            Some(DispatchEvent::Dispatched { server_id, slot_id }) => {
                                let wait = start_time.elapsed().as_secs_f64();
                                metrics::histogram!("llamaforge_queue_wait_seconds").record(wait);
                                break (server_id, slot_id);
                            }
                            Some(DispatchEvent::Failed { error }) => {
                                yield Err(StreamError::Upstream(Box::new(
                                    std::io::Error::other(error),
                                )));
                                return;
                            }
                            Some(_) => { /* FirstToken/Completed: ignored before dispatch */ }
                            None => {
                                yield Err(StreamError::BackendDisconnect);
                                return;
                            }
                        }
                    }
                }
            };

            // Phase 2: dispatch to the picked server/slot and stream chunks.
            let routing = RoutingDecision::for_scheduler_pick(server_id, Some(slot_id));
            let candidate = DispatchCandidate {
                request_id,
                prompt,
                params: completion_params,
                routing,
            };

            // Audit channel: dispatch emits its own `AuditRecord` shape
            // (different from `metrics::AuditRecord`), so a translator is
            // needed to forward records into the metrics ingest. The
            // translator lives in PR9b alongside the e2e harness work that
            // exercises the audit path end-to-end. For now records are
            // received and dropped — explicitly logged so the drop is not
            // silent.
            let (audit_tx, mut audit_rx) = tokio::sync::mpsc::channel(32);
            tokio::spawn(async move {
                let mut dropped: u64 = 0;
                while audit_rx.recv().await.is_some() {
                    dropped = dropped.saturating_add(1);
                }
                if dropped > 0 {
                    tracing::debug!(
                        dropped,
                        "inference_engine.audit_records_unwired"
                    );
                }
            });

            let (_abort, mut chunk_rx) = dispatcher.dispatch(candidate, audit_tx).await;

            loop {
                tokio::select! {
                    biased;
                    _ = cancel_token.cancelled() => {
                        yield Err(StreamError::Cancelled);
                        return;
                    }
                    chunk = chunk_rx.recv() => {
                        match chunk {
                            Some(Ok(parsed)) => {
                                if !first_token {
                                    first_token = true;
                                    metrics::histogram!("llamaforge_ttft_seconds")
                                        .record(start_time.elapsed().as_secs_f64());
                                }
                                if parsed.stop {
                                    metrics::counter!("llamaforge_tokens_total")
                                        .increment(parsed.tokens_predicted as u64);
                                    let reason = parsed
                                        .finish_reason
                                        .filter(|s| !s.is_empty())
                                        .unwrap_or_else(|| "stop".to_string());
                                    yield Ok(ChatCompletionChunk::Stop(reason));
                                    return;
                                } else if !parsed.text.is_empty() {
                                    tokens_emitted = tokens_emitted.saturating_add(1);
                                    yield Ok(ChatCompletionChunk::Token(parsed.text));
                                }
                            }
                            Some(Err(e)) => {
                                yield Err(map_stream_error(e));
                                return;
                            }
                            None => {
                                // Backend stream closed without an explicit
                                // stop terminator. The dispatch proxy task
                                // (`llamaforge-dispatch::proxy::proxy_task`,
                                // backend-EOF arm at proxy.rs:142-144)
                                // falls through to its cleanup without
                                // emitting a `stop=true` chunk on this path,
                                // so this *is* reachable, not dead code. If
                                // we got any tokens we synthesize a clean
                                // Stop; otherwise we surface the loss as
                                // BackendDisconnect.
                                if tokens_emitted > 0 {
                                    yield Ok(ChatCompletionChunk::Stop("stop".to_string()));
                                } else {
                                    yield Err(StreamError::BackendDisconnect);
                                }
                                return;
                            }
                        }
                    }
                }
            }
        };

        Ok(Box::pin(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use llamaforge_dispatch::client::{BackendClient, BackendResponseStream, ClientManager};
    use llamaforge_dispatch::error::DispatchError as DispatchCrateError;
    use llamaforge_dispatch::slot_map::SlotDispatchMap;
    use llamaforge_dispatch::tracker::DispatchTrackerRegistry;
    use llamaforge_routing::types::ServerStateView;
    use llamaforge_types::ports::{SlotErasure, SlotError, SlotNotifier};
    use llamaforge_types::traits::{
        AdmissionError, QueueError, QueueSnapshot, QueueSubmitter, QueueTicket,
    };
    use llamaforge_types::{ClientRefreshEvent, Message, RequestId, ServerId, SlotId};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tokio::sync::{mpsc, Mutex as AsyncMutex};
    use tokio_util::sync::CancellationToken;

    // ---- Mock QueueSubmitter ------------------------------------------------

    struct MockQueue {
        captured: Mutex<Option<IncomingRequest>>,
        handle_tx: Mutex<Option<mpsc::Sender<DispatchEvent>>>,
        cancel_token: Mutex<Option<CancellationToken>>,
    }

    impl MockQueue {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                captured: Mutex::new(None),
                handle_tx: Mutex::new(None),
                cancel_token: Mutex::new(None),
            })
        }

        fn captured(&self) -> Option<IncomingRequest> {
            self.captured.lock().unwrap().clone()
        }

        async fn push(&self, ev: DispatchEvent) {
            let tx = self.handle_tx.lock().unwrap().clone().expect("submit not called yet");
            tx.send(ev).await.expect("event channel closed");
        }

        fn cancel_ticket(&self) {
            self.cancel_token.lock().unwrap().clone().expect("submit not called yet").cancel();
        }
    }

    impl QueueSubmitter for MockQueue {
        fn submit(&self, req: IncomingRequest) -> Result<QueueTicket, AdmissionError> {
            *self.captured.lock().unwrap() = Some(req.clone());
            let (tx, rx) = mpsc::channel(16);
            let cancel = CancellationToken::new();
            *self.handle_tx.lock().unwrap() = Some(tx);
            *self.cancel_token.lock().unwrap() = Some(cancel.clone());
            Ok(QueueTicket {
                request_id: req.request_id,
                admitted_at: chrono::Utc::now(),
                cancel_token: cancel,
                events: rx,
            })
        }

        fn cancel(&self, _id: &RequestId) -> Result<(), QueueError> {
            Ok(())
        }

        fn status(&self) -> Result<QueueSnapshot, QueueError> {
            Ok(QueueSnapshot { depth: 0, estimated_throughput_req_sec: 0.0 })
        }
    }

    // ---- Mock dispatch plumbing --------------------------------------------

    struct ScriptedBackend {
        script:
            AsyncMutex<Option<Vec<Result<bytes::Bytes, llamaforge_dispatch::error::StreamError>>>>,
    }

    impl ScriptedBackend {
        fn new(
            script: Vec<Result<bytes::Bytes, llamaforge_dispatch::error::StreamError>>,
        ) -> Arc<Self> {
            Arc::new(Self { script: AsyncMutex::new(Some(script)) })
        }
    }

    #[async_trait::async_trait]
    impl BackendClient for ScriptedBackend {
        async fn post_completion(
            &self,
            _server_id: &ServerId,
            _payload: llamaforge_dispatch::dispatcher::CompletionRequest,
        ) -> Result<BackendResponseStream, DispatchCrateError> {
            let chunks = self.script.lock().await.take().unwrap_or_default();
            Ok(Box::pin(tokio_stream::iter(chunks)))
        }
    }

    struct NoopClientManager;
    impl ClientManager for NoopClientManager {
        fn refresh(&self, _: &ServerId, _: ClientRefreshEvent) {}
    }
    struct EmptyServerState;
    impl ServerStateView for EmptyServerState {
        fn all_servers(&self) -> Vec<llamaforge_routing::types::ServerSnapshot> {
            vec![]
        }
    }
    struct ZeroDispatchCounts;
    impl llamaforge_routing::DispatchCountView for ZeroDispatchCounts {
        fn in_flight_count(&self, _: &ServerId) -> usize {
            0
        }
    }
    struct NoopSlotNotifier;
    impl SlotNotifier for NoopSlotNotifier {
        fn notify_slot_available(&self, _: &ServerId) {}
    }
    struct NoopSlotErasure;
    #[async_trait::async_trait]
    impl SlotErasure for NoopSlotErasure {
        async fn erase_slot(&self, _: &ServerId, _: &SlotId) -> Result<(), SlotError> {
            Ok(())
        }
    }

    fn test_config() -> Arc<llamaforge_config::ConfigHandle> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "[global]\n").unwrap();
        let paths = llamaforge_config::ConfigPaths::new(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
        );
        let source = Arc::new(llamaforge_config::TomlFileConfigSource::new(paths));
        Arc::new(llamaforge_config::ConfigHandle::load(source).unwrap())
    }

    fn test_router() -> Arc<Router> {
        Arc::new(Router::new(
            test_config(),
            Arc::new(EmptyServerState),
            Arc::new(ZeroDispatchCounts),
        ))
    }

    fn build_dispatcher(backend: Arc<dyn BackendClient>) -> Arc<Dispatcher> {
        let (m_tx, _) = mpsc::channel(8);
        let (a_tx, _) = mpsc::channel(8);
        let metrics =
            MetricsIngest::new_for_test(m_tx, a_tx, Arc::new(tokio::sync::Semaphore::new(1)));
        Arc::new(Dispatcher::new(
            Arc::new(EmptyServerState),
            metrics,
            Arc::new(DispatchTrackerRegistry::new()),
            Arc::new(SlotDispatchMap::new()),
            test_config(),
            backend,
            Arc::new(NoopClientManager),
            Arc::new(NoopSlotNotifier),
            Arc::new(NoopSlotErasure),
        ))
    }

    fn test_metrics() -> Arc<MetricsIngest> {
        let (m_tx, _) = mpsc::channel(8);
        let (a_tx, _) = mpsc::channel(8);
        Arc::new(MetricsIngest::new_for_test(m_tx, a_tx, Arc::new(tokio::sync::Semaphore::new(1))))
    }

    fn test_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "test-model".into(),
            messages: vec![Message { role: Role::User, content: "hi".into() }],
            max_tokens: Some(8),
            temperature: Some(0.7),
            stream: true,
            logit_bias: Some(HashMap::new()),
            tools: None,
            tool_choice: None,
        }
    }

    fn llama_chunk(content: &str, stop: bool) -> bytes::Bytes {
        let json = serde_json::json!({ "content": content, "stop": stop }).to_string();
        bytes::Bytes::from(format!("data: {}\n", json))
    }

    // ---- Tests --------------------------------------------------------------

    #[tokio::test]
    async fn translates_request_into_incoming_fields() {
        let queue = MockQueue::new();
        let adapter = InferenceEngineAdapter::new(
            queue.clone() as Arc<dyn QueueSubmitter>,
            test_router(),
            build_dispatcher(ScriptedBackend::new(vec![])),
            test_metrics(),
        );

        let _ = adapter.chat_completion(test_request()).await.expect("submit ok");

        let captured = queue.captured().expect("queue saw a submit");
        assert_eq!(captured.params.max_tokens, Some(8));
        assert_eq!(captured.params.temperature, Some(0.7));
        assert!(captured.stream);
        match &captured.model_target {
            ModelTarget::Exact(alias) => assert_eq!(alias.as_str(), "test-model"),
            other => panic!("expected Exact, got {:?}", other),
        }
        assert_eq!(captured.messages.len(), 1);
        assert_eq!(captured.requestor_id.as_str(), "anonymous");
    }

    #[tokio::test]
    async fn streams_queue_update_then_tokens_then_stop() {
        let queue = MockQueue::new();
        let backend = ScriptedBackend::new(vec![
            Ok(llama_chunk("Hel", false)),
            Ok(llama_chunk("lo", false)),
            Ok(llama_chunk("", true)),
        ]);
        let adapter = InferenceEngineAdapter::new(
            queue.clone() as Arc<dyn QueueSubmitter>,
            test_router(),
            build_dispatcher(backend),
            test_metrics(),
        );

        let mut stream = adapter.chat_completion(test_request()).await.expect("submit ok");

        queue.push(DispatchEvent::Queued { position: 1 }).await;
        queue
            .push(DispatchEvent::Dispatched {
                server_id: ServerId::new(uuid::Uuid::now_v7()),
                slot_id: SlotId::new(0),
            })
            .await;

        let mut got: Vec<ChatCompletionChunk> = Vec::new();
        while let Some(item) = stream.next().await {
            got.push(item.expect("chunk error"));
            if matches!(got.last(), Some(ChatCompletionChunk::Stop(_))) {
                break;
            }
        }

        assert!(matches!(got[0], ChatCompletionChunk::QueueUpdate { position: 1 }));
        assert!(got.iter().any(|c| matches!(c, ChatCompletionChunk::Token(s) if s == "Hel")));
        assert!(got.iter().any(|c| matches!(c, ChatCompletionChunk::Token(s) if s == "lo")));
        assert!(matches!(got.last(), Some(ChatCompletionChunk::Stop(_))));
    }

    #[tokio::test]
    async fn maps_mid_stream_dispatch_error() {
        let queue = MockQueue::new();
        let backend = ScriptedBackend::new(vec![
            Ok(llama_chunk("partial", false)),
            Err(llamaforge_dispatch::error::StreamError::BackendDisconnect),
        ]);
        let adapter = InferenceEngineAdapter::new(
            queue.clone() as Arc<dyn QueueSubmitter>,
            test_router(),
            build_dispatcher(backend),
            test_metrics(),
        );

        let mut stream = adapter.chat_completion(test_request()).await.expect("submit ok");

        queue
            .push(DispatchEvent::Dispatched {
                server_id: ServerId::new(uuid::Uuid::now_v7()),
                slot_id: SlotId::new(0),
            })
            .await;

        let mut saw_error = false;
        let mut saw_token = false;
        while let Some(item) = stream.next().await {
            match item {
                Ok(ChatCompletionChunk::Token(_)) => saw_token = true,
                Err(_) => {
                    saw_error = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_token, "should have seen the partial token before the error");
        assert!(saw_error, "should have surfaced the dispatch error");
    }

    #[tokio::test]
    async fn cancellation_ends_stream() {
        let queue = MockQueue::new();
        let adapter = InferenceEngineAdapter::new(
            queue.clone() as Arc<dyn QueueSubmitter>,
            test_router(),
            build_dispatcher(ScriptedBackend::new(vec![])),
            test_metrics(),
        );

        let mut stream = adapter.chat_completion(test_request()).await.expect("submit ok");
        queue.cancel_ticket();

        let first = stream.next().await.expect("stream yielded nothing");
        assert!(matches!(first, Err(StreamError::Cancelled)));
        assert!(stream.next().await.is_none());
    }
}
