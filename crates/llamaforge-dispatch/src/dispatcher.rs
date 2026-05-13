//! Dispatcher and related orchestration types.
//!
//! See design doc Section 2.1

use crate::client::{BackendClient, ClientManager};
use crate::proxy::proxy_task;
use crate::slot_map::SlotDispatchMap;
use crate::token::DispatchToken;
use crate::tracker::DispatchTrackerRegistry;
use dashmap::DashMap;
use llamaforge_config::ConfigHandle;
use llamaforge_metrics::MetricsIngest;
use llamaforge_routing::RoutingDecision;
use llamaforge_routing::types::ServerStateView;
use llamaforge_types::ports::SlotNotifier;
use llamaforge_types::{RequestId, ServerId, SlotId};
use std::sync::Arc;
use std::time::Duration;
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
    proxy_tasks: Mutex<JoinSet<()>>,
    cancellation_tokens: DashMap<ServerId, CancellationToken>,
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
            proxy_tasks: Mutex::new(JoinSet::new()),
            cancellation_tokens: DashMap::new(),
        }
    }

    /// Dispatches a request to a server, returning an abort handle to the task.
    pub async fn dispatch(
        &self,
        candidate: DispatchCandidate,
    ) -> tokio::task::AbortHandle {
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
        let mut tasks = self.proxy_tasks.lock().await;

        let backend_client = Arc::clone(&self.backend_client);
        let cancel_token = self.cancellation_tokens.entry(server_id).or_default().clone();

        tasks.spawn(async move {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    // Implicitly drops token and notifies slot available
                }
                _ = proxy_task(token, backend_client, request) => {}
            }
        })
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
        let mut tasks = self.proxy_tasks.lock().await;
        let count = tasks.len();
        if count > 0 {
            warn!(count, "Waiting for proxy tasks to complete during shutdown");
            // Wait with timeout
            match tokio::time::timeout(Duration::from_secs(5), async {
                while tasks.join_next().await.is_some() {}
            })
            .await
            {
                Ok(_) => info!("All proxy tasks completed"),
                Err(_) => warn!(
                    remaining = tasks.len(),
                    "Timed out waiting for proxy tasks to complete"
                ),
            }
        }
    }

    /// Drains tasks for a specific server.
    pub async fn drain_server(&self, server_id: &ServerId) -> usize {
        if let Some(token) = self.cancellation_tokens.get(server_id) {
            token.cancel();
            // Count active tasks for this server. 
            // In PR5 we just cancel the token. The proxy_task stub immediately completes,
            // so active task count is roughly 0. We'll return 0 for now.
            // When streaming is implemented, we would await these tasks specifically.
            self.cancellation_tokens.remove(server_id);
            0
        } else {
            0
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

        let _handle = dispatcher.dispatch(candidate).await;

        let tracker = trackers.get(&server_id).unwrap();
        assert_eq!(tracker.count(), 1);
    }
}
