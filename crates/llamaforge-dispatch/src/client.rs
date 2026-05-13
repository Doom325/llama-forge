//! Backend client adapter and port traits.
//!
//! See design doc Section 2.5

use arc_swap::ArcSwap;
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use futures_core::Stream;
use llamaforge_config::ServerConfig;
use llamaforge_types::{ClientRefreshEvent, ServerId};
use reqwest::Client;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::dispatcher::CompletionRequest;
use crate::error::{DispatchError, HttpStatus, StreamError};

/// A pinned, boxed, thread-safe stream of chunks from the backend.
///
/// See design doc Section 2.5.
pub type BackendResponseStream = Pin<Box<dyn Stream<Item = Result<Bytes, StreamError>> + Send>>;

/// Interface for making requests to a backend server.
///
/// See design doc Section 2.5.
#[async_trait]
pub trait BackendClient: Send + Sync {
    /// Post a completion request and return the response stream.
    async fn post_completion(
        &self,
        server_id: &ServerId,
        payload: CompletionRequest,
    ) -> Result<BackendResponseStream, DispatchError>;
}

/// Interface for managing backend client lifecycles (e.g. rotating clients on restart).
///
/// See design doc Section 2.5.
pub trait ClientManager: Send + Sync {
    /// Update the underlying client configuration when a server restarts.
    fn refresh(&self, server_id: &ServerId, event: ClientRefreshEvent);
}

/// Holds a reqwest client and the base URL for a specific server.
#[derive(Debug, Clone)]
pub struct ServerConnectionInfo {
    pub client: Client,
    pub base_url: String,
}

/// A concrete implementation of `BackendClient` and `ClientManager` using `reqwest`.
#[derive(Debug, Default)]
pub struct ReqwestClientStore {
    clients: DashMap<ServerId, ArcSwap<ServerConnectionInfo>>,
}

impl ReqwestClientStore {
    /// Creates a new empty `ReqwestClientStore`.
    pub fn new() -> Self {
        Self {
            clients: DashMap::new(),
        }
    }

    /// Register a new server with its configuration.
    pub fn register_server(&self, server_id: ServerId, config: &ServerConfig) {
        let client = Self::build_client(config.parallel_slots as usize);
        let info = ServerConnectionInfo {
            client,
            base_url: format!("http://127.0.0.1:{}", config.port),
        };
        self.clients.insert(server_id, ArcSwap::from_pointee(info));
    }

    /// Helper to build a `reqwest::Client` with the correct pool settings.
    fn build_client(parallel_slots: usize) -> Client {
        let max_idle = parallel_slots.max(16);
        
        Client::builder()
            .pool_idle_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(max_idle)
            .tcp_keepalive(Duration::from_secs(10)) // idle
            .build()
            .unwrap_or_else(|e| {
                tracing::error!("Failed to build reqwest client: {}", e);
                Client::new()
            })
    }
}

#[async_trait]
impl BackendClient for ReqwestClientStore {
    async fn post_completion(
        &self,
        server_id: &ServerId,
        payload: CompletionRequest,
    ) -> Result<BackendResponseStream, DispatchError> {
        let client_ref = self.clients.get(server_id).ok_or_else(|| {
            DispatchError::ConnectionFailed(Box::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("No client registered for server {}", server_id),
            )))
        })?;

        let info = client_ref.load();
        let url = format!("{}/completion", info.base_url);

        // Serialize payload manually to avoid deriving Serialize on CompletionRequest (as per PR2 requirements)
        let body = serde_json::json!({
            "prompt": payload.prompt,
            "n_predict": payload.n_predict,
            "temperature": payload.temperature,
            "stream": payload.stream,
            "stop": payload.stop,
            "id_slot": payload.id_slot.map(|id| id.into_inner()),
        });

        let request = info.client.post(&url).json(&body).build().map_err(|e| {
            DispatchError::ConnectionFailed(Box::new(e))
        })?;

        let response = info.client.execute(request).await.map_err(|e| {
            if e.is_timeout() {
                DispatchError::Timeout
            } else {
                DispatchError::ConnectionFailed(Box::new(e))
            }
        })?;

        let status = response.status();
        if !status.is_success() {
            return Err(DispatchError::BackendError {
                status: HttpStatus(status.as_u16()),
            });
        }

        // Return a dummy stream for now. The real streaming logic will be implemented in PR6.
        // For PR5, we just return an empty stream to satisfy the signature.
        let stream = tokio_stream::empty();
        Ok(Box::pin(stream))
    }
}

impl ClientManager for ReqwestClientStore {
    fn refresh(&self, server_id: &ServerId, event: ClientRefreshEvent) {
        if let Some(entry) = self.clients.get(server_id) {
            let old_info = entry.load();
            let new_client = Self::build_client(16); // Fallback to 16 slots, we don't have the config here
            
            // Extract the host from the old base_url
            let host = old_info.base_url.split(':').nth(1).unwrap_or("//127.0.0.1").trim_start_matches("//");
            
            let new_info = ServerConnectionInfo {
                client: new_client,
                base_url: format!("http://{}:{}", host, event.new_port),
            };
            
            entry.store(Arc::new(new_info));
            info!("Refreshed client for server {}, new port: {}", server_id, event.new_port);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_arcswap_refresh() {
        let store = ReqwestClientStore::new();
        let server_id = ServerId::new(Uuid::now_v7());
        
        let config = ServerConfig {
            port: 8080,
            ..Default::default()
        };
        store.register_server(server_id, &config);

        let initial_url = {
            let info = store.clients.get(&server_id).unwrap().load().clone();
            info.base_url.clone()
        };
        assert_eq!(initial_url, "http://127.0.0.1:8080");

        let event = ClientRefreshEvent {
            server_id,
            new_port: 8081,
            tls_config: None,
        };
        store.refresh(&server_id, event);

        let new_url = {
            let info = store.clients.get(&server_id).unwrap().load().clone();
            info.base_url.clone()
        };
        assert_eq!(new_url, "http://127.0.0.1:8081");
    }
}
