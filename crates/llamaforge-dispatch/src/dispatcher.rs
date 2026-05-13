//! Dispatcher and related orchestration types.
//!
//! See design doc Section 2.1

use crate::client::{BackendClient, ClientManager};
use crate::slot_map::SlotDispatchMap;
use crate::token::DispatchToken;
use crate::tracker::DispatchTrackerRegistry;
use dashmap::DashMap;
use llamaforge_config::ConfigHandle;
use llamaforge_metrics::MetricsIngest;
use llamaforge_routing::RoutingDecision;
use llamaforge_routing::types::ServerStateView;
use llamaforge_types::ports::{SlotErasure, SlotNotifier};
use llamaforge_types::{RequestId, ServerId, SlotId};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// A completion request payload sent to the backend.
///
/// See design doc Section 2.5.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub prompt: String,
    pub n_predict: Option<u32>,
    pub temperature: f32,
    pub stream: bool,
    pub stop: Vec<String>,
    pub id_slot: Option<SlotId>,
}

/// A request to be dispatched.
#[derive(Debug, Clone)]
pub struct DispatchCandidate {
    pub request_id: RequestId,
    pub prompt: String,
    pub params: CompletionParams,
    pub routing: RoutingDecision,
}

#[derive(Debug, Clone)]
pub struct CompletionParams {
    pub n_predict: Option<u32>,
    pub temperature: f32,
    pub stream: bool,
    pub stop: Vec<String>,
}

/// Orchestrates the dispatching of requests to backend servers.
///
/// See design doc Section 2.1.
pub struct Dispatcher {
    #[allow(dead_code)]
    pub(crate) server_state: Arc<dyn ServerStateView>,
    #[allow(dead_code)]
    metrics_ingest: MetricsIngest,
    trackers: Arc<DispatchTrackerRegistry>,
    slot_dispatch_map: Arc<SlotDispatchMap>,
    #[allow(dead_code)]
    config: Arc<ConfigHandle>,
    backend_client: Arc<dyn BackendClient>,
    client_manager: Arc<dyn ClientManager>,
    slot_notifier: Arc<dyn SlotNotifier>,
    slot_erasure: Arc<dyn SlotErasure>,
    proxy_tasks: DashMap<ServerId, Mutex<JoinSet<()>>>,
    cancellation_tokens: DashMap<ServerId, CancellationToken>,
    first_seen_processing: DashMap<(ServerId, SlotId), Instant>,
}

impl Dispatcher {
    /// Creates a new `Dispatcher`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        server_state: Arc<dyn ServerStateView>,
        metrics_ingest: MetricsIngest,
        trackers: Arc<DispatchTrackerRegistry>,
        slot_dispatch_map: Arc<SlotDispatchMap>,
        config: Arc<ConfigHandle>,
        backend_client: Arc<dyn BackendClient>,
        client_manager: Arc<dyn ClientManager>,
        slot_notifier: Arc<dyn SlotNotifier>,
        slot_erasure: Arc<dyn SlotErasure>,
    ) -> Self {
        Self {
            server_state,
            metrics_ingest,
            trackers,
            slot_dispatch_map,
            config,
            backend_client,
            client_manager,
            slot_notifier,
            slot_erasure,
            proxy_tasks: DashMap::new(),
            cancellation_tokens: DashMap::new(),
            first_seen_processing: DashMap::new(),
        }
    }

    /// Dispatches a request to a server, returning a receiver for the response stream.
    pub async fn dispatch(
        &self,
        candidate: DispatchCandidate,
        audit_tx: tokio::sync::mpsc::Sender<crate::proxy::AuditRecord>,
    ) -> (
        tokio::task::AbortHandle,
        tokio::sync::mpsc::Receiver<Result<crate::proxy::ParsedChunk, crate::error::StreamError>>,
    ) {
        let server_id = *candidate.routing.server_id();

        // Step 1: Get or create tracker for `routing.server_id`.
        let tracker = self.trackers.get_or_create(&server_id);

        // Step 2: Create `DispatchToken`
        let token = DispatchToken::new(
            tracker,
            candidate.request_id,
            server_id,
            Arc::clone(&self.slot_notifier),
        );

        // Step 3: Build `CompletionRequest`
        let request = CompletionRequest {
            prompt: candidate.prompt,
            n_predict: candidate.params.n_predict,
            temperature: candidate.params.temperature,
            stream: candidate.params.stream,
            stop: candidate.params.stop,
            id_slot: candidate.routing.slot_target(),
        };

        // Step 4: Add task to `proxy_tasks` JoinSet
        let tasks_mutex = self.proxy_tasks.entry(server_id).or_insert_with(|| Mutex::new(JoinSet::new()));
        let mut tasks = tasks_mutex.lock().await;

        let backend_client = Arc::clone(&self.backend_client);
        let cancel_token = self.cancellation_tokens.entry(server_id).or_default().clone();
        let slot_notifier = Arc::clone(&self.slot_notifier);
        let slot_dispatch_map = Arc::clone(&self.slot_dispatch_map);
        let (response_tx, response_rx) = tokio::sync::mpsc::channel(32);

        let handle = tasks.spawn(async move {
            crate::proxy::proxy_task(
                token,
                backend_client,
                request,
                response_tx,
                slot_notifier,
                audit_tx,
                cancel_token,
                slot_dispatch_map,
            )
            .await;
        });

        (handle, response_rx)
    }

    /// Accessor for the trackers registry.
    pub fn trackers(&self) -> Arc<DispatchTrackerRegistry> {
        Arc::clone(&self.trackers)
    }

    /// Accessor for the slot dispatch map.
    pub fn slot_dispatch_map(&self) -> Arc<SlotDispatchMap> {
        Arc::clone(&self.slot_dispatch_map)
    }

    /// Refreshes the client for a specific server.
    pub fn refresh_client(&self, server_id: &ServerId, event: llamaforge_types::ClientRefreshEvent) {
        self.client_manager.refresh(server_id, event);
    }

    /// Gracefully shuts down the dispatcher, waiting for all proxy tasks to complete.
    pub async fn shutdown(&self) {
        let mut all_tasks = Vec::new();
        let mut total_count = 0;
        
        let server_ids: Vec<_> = self.proxy_tasks.iter().map(|e| *e.key()).collect();
        for id in server_ids {
            if let Some((_, tasks_mutex)) = self.proxy_tasks.remove(&id) {
                let tasks = tasks_mutex.into_inner();
                if !tasks.is_empty() {
                    total_count += tasks.len();
                    all_tasks.push(tasks);
                }
            }
        }

        if total_count > 0 {
            warn!(count = total_count, "Waiting for proxy tasks to complete during shutdown");
            // Wait with timeout
            match tokio::time::timeout(Duration::from_secs(5), async {
                for mut tasks in all_tasks {
                    while tasks.join_next().await.is_some() {}
                }
            })
            .await
            {
                Ok(_) => info!("All proxy tasks completed"),
                Err(_) => warn!("Timed out waiting for proxy tasks to complete"),
            }
        }
    }

    /// Drains tasks for a specific server.
    pub async fn drain_server(&self, server_id: &ServerId) -> usize {
        if let Some((_, token)) = self.cancellation_tokens.remove(server_id) {
            token.cancel();
        }

        if let Some((_, tasks_mutex)) = self.proxy_tasks.remove(server_id) {
            let mut tasks = tasks_mutex.into_inner();
            let mut count = 0;
            while tasks.join_next().await.is_some() {
                count += 1;
            }
            count
        } else {
            0
        }
    }

    /// Reconciles slot states based on the periodic server status update.
    pub async fn reconcile_slots(&self, server_id: &ServerId, processing_slots: &[SlotId]) {
        use llamaforge_config::ConfigReader;
        let grace_period = {
            let config = self.config.snapshot();
            let config_server_id = llamaforge_config::ServerId::from(server_id.to_string());
            if let Some(server_config) = config.servers.get(&config_server_id) {
                Duration::from_secs(30.max(server_config.ctx_size as u64 / 500))
            } else {
                Duration::from_secs(30)
            }
        };

        // Forward reconciliation: detects orphaned slots
        use llamaforge_types::ports::SlotDispatchView;
        for &slot_id in processing_slots {
            if self.slot_dispatch_map.get_request_for_slot(server_id, &slot_id).is_none() {
                let key = (*server_id, slot_id);
                let first_seen = *self.first_seen_processing.entry(key).or_insert_with(Instant::now);
                if first_seen.elapsed() > grace_period {
                    metrics::counter!("llamaforge_reconciliation_erasures_total", "direction" => "forward").increment(1);
                    tracing::info!(server_id = %server_id, slot_id = %slot_id.into_inner(), "reconciliation.erasure.forward");
                    
                    let erasure = Arc::clone(&self.slot_erasure);
                    let s_id = *server_id;
                    tokio::spawn(async move {
                        let _ = tokio::time::timeout(Duration::from_secs(5), erasure.erase_slot(&s_id, &slot_id)).await;
                    });
                }
            }
        }
        
        // Cleanup first_seen_processing
        let mut keys_to_remove = Vec::new();
        for entry in self.first_seen_processing.iter() {
            let (srv, slot) = entry.key();
            if srv == server_id {
                let in_map = self.slot_dispatch_map.get_request_for_slot(srv, slot).is_some();
                let is_processing = processing_slots.contains(slot);
                if in_map || !is_processing {
                    keys_to_remove.push((*srv, *slot));
                }
            }
        }
        for k in keys_to_remove {
            self.first_seen_processing.remove(&k);
        }

        let has_unresolved = {
            let tracked_ids = self.trackers.get_or_create(server_id).snapshot_ids();
            let mut unresolved = false;
            for id in tracked_ids {
                if self.slot_dispatch_map.get_slot_for_request(&id).is_none() {
                    unresolved = true;
                    break;
                }
            }
            unresolved
        };

        // Reverse reconciliation: cleans stale map entries
        if has_unresolved {
            tracing::debug!(server_id = %server_id, "reconciliation.skipped.reverse");
        } else {
            let active_slots = self.slot_dispatch_map.get_dispatched_slots_for_server(server_id);
            for (req_id, slot_id, dispatched_at) in active_slots {
                if !processing_slots.contains(&slot_id) && dispatched_at.elapsed() > grace_period {
                    metrics::counter!("llamaforge_reconciliation_erasures_total", "direction" => "reverse").increment(1);
                    tracing::info!(server_id = %server_id, request_id = %req_id, slot_id = %slot_id.into_inner(), "reconciliation.erasure.reverse");
                    self.slot_dispatch_map.remove(&req_id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::BackendResponseStream;
    use crate::error::DispatchError;
    use llamaforge_config::TomlFileConfigSource;
    use llamaforge_types::RequestId;
    use uuid::Uuid;

    struct MockBackendClient;
    #[async_trait::async_trait]
    impl BackendClient for MockBackendClient {
        async fn post_completion(
            &self,
            _server_id: &ServerId,
            _payload: CompletionRequest,
        ) -> Result<BackendResponseStream, DispatchError> {
            Ok(Box::pin(tokio_stream::empty()))
        }
    }

    struct MockClientManager;
    impl ClientManager for MockClientManager {
        fn refresh(&self, _server_id: &ServerId, _event: llamaforge_types::ClientRefreshEvent) {}
    }

    struct MockServerStateView;
    impl ServerStateView for MockServerStateView {
        fn all_servers(&self) -> Vec<llamaforge_routing::types::ServerSnapshot> {
            vec![]
        }
    }

    struct MockSlotNotifier;
    impl SlotNotifier for MockSlotNotifier {
        fn notify_slot_available(&self, _server_id: &ServerId) {}
    }

    struct MockSlotErasure;
    #[async_trait::async_trait]
    impl SlotErasure for MockSlotErasure {
        async fn erase_slot(&self, _server_id: &ServerId, _slot_id: &SlotId) -> Result<(), llamaforge_types::ports::SlotError> {
            Ok(())
        }
    }

    fn create_test_config() -> Arc<ConfigHandle> {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let path = temp.path().to_path_buf();
        std::fs::write(&path, "[global]\n").unwrap();
        let paths = llamaforge_config::ConfigPaths::new(path, Default::default(), Default::default());
        Arc::new(ConfigHandle::load(Arc::new(TomlFileConfigSource::new(paths))).unwrap())
    }

    #[tokio::test]
    async fn test_dispatcher_construction_and_accessors() {
        let trackers = Arc::new(DispatchTrackerRegistry::new());
        let slot_map = Arc::new(SlotDispatchMap::new());
        let config = create_test_config();
        let (metrics_tx, _) = tokio::sync::mpsc::channel(1);
        let (audit_tx, _) = tokio::sync::mpsc::channel(1);
        let metrics = MetricsIngest::new_for_test(metrics_tx, audit_tx, Arc::new(tokio::sync::Semaphore::new(1)));

        let dispatcher = Dispatcher::new(
            Arc::new(MockServerStateView),
            metrics,
            Arc::clone(&trackers),
            Arc::clone(&slot_map),
            config,
            Arc::new(MockBackendClient),
            Arc::new(MockClientManager),
            Arc::new(MockSlotNotifier),
            Arc::new(MockSlotErasure),
        );

        assert!(Arc::ptr_eq(&dispatcher.trackers(), &trackers));
        assert!(Arc::ptr_eq(&dispatcher.slot_dispatch_map(), &slot_map));
    }

    #[tokio::test]
    async fn test_dispatch_creates_token() {
        let trackers = Arc::new(DispatchTrackerRegistry::new());
        let slot_map = Arc::new(SlotDispatchMap::new());
        let config = create_test_config();
        let (metrics_tx, _) = tokio::sync::mpsc::channel(1);
        let (audit_tx, _) = tokio::sync::mpsc::channel(1);
        let metrics = MetricsIngest::new_for_test(metrics_tx, audit_tx, Arc::new(tokio::sync::Semaphore::new(1)));

        let dispatcher = Dispatcher::new(
            Arc::new(MockServerStateView),
            metrics,
            Arc::clone(&trackers),
            Arc::clone(&slot_map),
            config,
            Arc::new(MockBackendClient),
            Arc::new(MockClientManager),
            Arc::new(MockSlotNotifier),
            Arc::new(MockSlotErasure),
        );

        let server_id = ServerId::new(Uuid::now_v7());
        let request_id = RequestId::new(Uuid::now_v7());

        let candidate = DispatchCandidate {
            request_id,
            prompt: "hello".to_string(),
            params: CompletionParams {
                n_predict: Some(10),
                temperature: 0.7,
                stream: true,
                stop: vec![],
            },
            routing: RoutingDecision::new_for_test(server_id),
        };

        let (audit_tx_dispatch, _) = tokio::sync::mpsc::channel(1);
        let _handle = dispatcher.dispatch(candidate, audit_tx_dispatch).await;

        let tracker = trackers.get(&server_id).unwrap();
        assert_eq!(tracker.count(), 1);
    }
}
