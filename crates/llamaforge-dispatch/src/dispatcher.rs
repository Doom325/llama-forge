//! Dispatcher and related orchestration types.
//!
//! See design doc Section 2.1

use crate::client::{BackendClient, ClientManager};
use crate::proxy::proxy_task;
use crate::slot_map::SlotDispatchMap;
use crate::token::DispatchToken;
use crate::tracker::DispatchTrackerRegistry;
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
use tracing::warn;

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

/// The main orchestrator for dispatching requests to servers.
///
/// See design doc Section 2.1.
pub struct Dispatcher {
    #[allow(dead_code)]
    server_state: Arc<dyn ServerStateView>,
    
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
        }
    }

    /// Dispatches a request to a server.
    pub async fn dispatch(&self, candidate: DispatchCandidate) {
        let server_id = *candidate.routing.server_id();
        
        // Step 1: Get or create tracker for `routing.server_id`.
        let tracker = self.trackers.get_or_create(&server_id);
        
        // Step 2: Create `DispatchToken`
        let token = DispatchToken::new(
            tracker, 
            candidate.request_id, 
            server_id, 
            Arc::clone(&self.slot_notifier)
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
        
        tasks.spawn(async move {
            proxy_task(token, backend_client, request).await;
        });
    }

    /// Accessor for the trackers registry.
    pub fn trackers(&self) -> Arc<DispatchTrackerRegistry> {
        Arc::clone(&self.trackers)
    }

    /// Accessor for the slot dispatch map.
    pub fn slot_dispatch_map(&self) -> Arc<SlotDispatchMap> {
        Arc::clone(&self.slot_dispatch_map)
    }

    /// Refreshes the backend client for a server.
    pub fn refresh_client(&self, server_id: &ServerId, event: llamaforge_types::ClientRefreshEvent) {
        self.client_manager.refresh(server_id, event);
    }

    /// Gracefully shutdown all proxy tasks with a timeout.
    pub async fn shutdown(&self) {
        let mut tasks = self.proxy_tasks.lock().await;
        
        if tasks.is_empty() {
            return;
        }

        let timeout = Duration::from_secs(5);
        
        // Need to use an intermediate variable to avoid moving self inside JoinSet
        // Use a loop to poll next() and timeout
        let _ = tokio::time::timeout(timeout, async {
            while tasks.join_next().await.is_some() {}
        }).await;
        
        if !tasks.is_empty() {
            warn!("Shutdown timeout reached. {} proxy tasks were cancelled.", tasks.len());
            tasks.abort_all();
        }
    }

    /// Drains tasks for a specific server.
    pub async fn drain_server(&self, _server_id: &ServerId) -> usize {
        // We will need a CancellationToken per server or a way to abort specific tasks.
        // For PR5, we'll return 0 as a stub, and PR6 will add the per-server cancellation.
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use crate::error::DispatchError;
    use llamaforge_routing::types::ServerSnapshot;
    use llamaforge_types::ports::SlotDispatchView;

    struct MockServerStateView;
    impl ServerStateView for MockServerStateView {
        fn all_servers(&self) -> Vec<ServerSnapshot> {
            vec![]
        }
    }

    struct MockSlotNotifier;
    impl SlotNotifier for MockSlotNotifier {
        fn notify_slot_available(&self, _server_id: &ServerId) {}
    }

    struct MockClientManager;
    impl ClientManager for MockClientManager {
        fn refresh(&self, _server_id: &ServerId, _event: llamaforge_types::ClientRefreshEvent) {}
    }

    #[derive(Clone)]
    struct MockBackendClient;
    #[async_trait::async_trait]
    impl BackendClient for MockBackendClient {
        async fn post_completion(
            &self,
            _server_id: &ServerId,
            _payload: CompletionRequest,
        ) -> Result<crate::client::BackendResponseStream, DispatchError> {
            Ok(Box::pin(tokio_stream::empty()))
        }
    }

    fn create_dispatcher() -> Dispatcher {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "[global]\n").unwrap();
        let paths = llamaforge_config::ConfigPaths::new(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            tmp.path().parent().unwrap().to_path_buf(),
        );
        let source = Arc::new(llamaforge_config::TomlFileConfigSource::new(paths));
        let config = Arc::new(ConfigHandle::load(source).unwrap());

        let metrics_ingest = llamaforge_metrics::MetricsIngest::new_for_test(
            tokio::sync::mpsc::channel(100).0,
            tokio::sync::mpsc::channel(100).0,
            Arc::new(tokio::sync::Semaphore::new(100)),
        );

        Dispatcher::new(
            Arc::new(MockServerStateView),
            metrics_ingest,
            Arc::new(DispatchTrackerRegistry::new()),
            Arc::new(SlotDispatchMap::new()),
            config,
            Arc::new(MockBackendClient),
            Arc::new(MockClientManager),
            Arc::new(MockSlotNotifier),
        )
    }

    #[tokio::test]
    async fn test_dispatcher_construction_and_accessors() {
        let dispatcher = create_dispatcher();
        assert!(dispatcher.trackers().get(&ServerId::new(Uuid::now_v7())).is_none());
        assert!(!dispatcher.slot_dispatch_map().has_unresolved(&ServerId::new(Uuid::now_v7())));
    }

    #[tokio::test]
    async fn test_dispatch_creates_token() {
        let dispatcher = create_dispatcher();
        let server_id = ServerId::new(Uuid::now_v7());
        
        let candidate = DispatchCandidate {
            request_id: RequestId::new(Uuid::now_v7()),
            prompt: "test".to_string(),
            params: CompletionParams {
                n_predict: None,
                temperature: 0.0,
                stream: false,
                stop: vec![],
            },
            routing: RoutingDecision::new_for_test(server_id),
        };

        dispatcher.dispatch(candidate).await;
        
        // Let the spawn run
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Tracker should have been created
        let tracker = dispatcher.trackers().get(&server_id).unwrap();
        
        // Since proxy_task is a stub that releases the token immediately, count should be 0.
        // During execution it would be 1.
        assert_eq!(tracker.count(), 0);
    }
}
