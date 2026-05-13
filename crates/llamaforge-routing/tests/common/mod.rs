#![allow(dead_code)]
#![allow(unused_imports)]
pub use llamaforge_config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
pub use llamaforge_routing::{
    DispatchCountView, RequestCapabilities, RequestView, Router, RoutingError,
};
pub use llamaforge_types::{ModelAlias, ModelTarget, ServerId, ServerLifecycle};
pub use std::collections::HashSet;
pub use std::sync::Arc;
pub use std::time::Duration;

#[derive(Clone, Default)]
pub struct MockDispatchCountView {
    pub counts: Arc<dashmap::DashMap<ServerId, usize>>,
}

impl MockDispatchCountView {
    pub fn new() -> Self {
        Self { counts: Arc::new(dashmap::DashMap::new()) }
    }

    pub fn set_count(&self, server_id: ServerId, count: usize) {
        self.counts.insert(server_id, count);
    }
}

impl llamaforge_routing::DispatchCountView for MockDispatchCountView {
    fn in_flight_count(&self, server_id: &ServerId) -> usize {
        self.counts.get(server_id).map(|r| *r).unwrap_or(0)
    }
}

pub struct MockServerStateView {
    pub servers: std::sync::RwLock<Vec<llamaforge_routing::types::ServerSnapshot>>,
}

impl MockServerStateView {
    pub fn new(servers: Vec<llamaforge_routing::types::ServerSnapshot>) -> Self {
        Self { servers: std::sync::RwLock::new(servers) }
    }

    pub fn set_servers(&self, servers: Vec<llamaforge_routing::types::ServerSnapshot>) {
        *self.servers.write().unwrap() = servers;
    }
}

impl llamaforge_routing::types::ServerStateView for MockServerStateView {
    fn all_servers(&self) -> Vec<llamaforge_routing::types::ServerSnapshot> {
        self.servers.read().unwrap().clone()
    }
}

pub struct MockRequest {
    pub model: ModelTarget,
    pub caps: RequestCapabilities,
    pub session: Option<llamaforge_types::SessionId>,
}

impl RequestView for MockRequest {
    fn model_target(&self) -> &ModelTarget {
        &self.model
    }
    fn capabilities(&self) -> &RequestCapabilities {
        &self.caps
    }
    fn session_id(&self) -> Option<&llamaforge_types::SessionId> {
        self.session.as_ref()
    }
}

pub fn create_test_config() -> Arc<ConfigHandle> {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "[global]\n").unwrap();
    let paths = ConfigPaths::new(
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
        tmp.path().parent().unwrap().to_path_buf(),
    );
    let source = Arc::new(TomlFileConfigSource::new(paths));
    Arc::new(ConfigHandle::load(source).unwrap())
}

pub fn default_server(alias: &str) -> llamaforge_routing::types::ServerSnapshot {
    let mut models = HashSet::new();
    models.insert(ModelAlias::new(Arc::from(alias)));
    llamaforge_routing::types::ServerSnapshot {
        id: ServerId::new(uuid::Uuid::now_v7()),
        lifecycle: ServerLifecycle::Running,
        models,
        capabilities: HashSet::new(),
        lora_adapters: HashSet::new(),
        multimodal: false,
        embedding: false,
        reranking: false,
        architecture: llamaforge_types::ModelArchitecture::Llama,
        total_slots: 4,
        idle_slots: 4,
        kv_usage: 0.0,
        deferred_ratio: 0.0,
    }
}
