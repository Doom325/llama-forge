//! Backend client adapter and port traits.
//!
//! See design doc Section 2.5

use arc_swap::ArcSwap;
use async_trait::async_trait;
use bytes::Bytes;
use dashmap::DashMap;
use futures_core::Stream;
use futures_util::StreamExt;
use llamaforge_config::ServerConfig;
use llamaforge_types::ports::{SlotErasure, SlotError};
use llamaforge_types::{ClientRefreshEvent, ServerId, SlotId};
use reqwest::Client;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

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

/// Holds a reqwest client and the connection info for a specific server.
#[derive(Debug, Clone)]
struct ServerConnectionInfo {
    client: Client,
    base_url: String,
    host: String,
    parallel_slots: usize,
}

/// A concrete implementation of `BackendClient` and `ClientManager` using `reqwest`.
#[derive(Debug, Default)]
pub struct ReqwestClientStore {
    clients: DashMap<ServerId, ArcSwap<ServerConnectionInfo>>,
}

impl ReqwestClientStore {
    /// Creates a new empty `ReqwestClientStore`.
    pub fn new() -> Self {
        Self { clients: DashMap::new() }
    }

    /// Register a new server with its configuration.
    pub fn add_server(
        &self,
        server_id: ServerId,
        config: &ServerConfig,
    ) -> Result<(), reqwest::Error> {
        let max_idle = config.parallel_slots as usize;
        let client = Self::build_client(max_idle)?;
        let info = ServerConnectionInfo {
            client,
            base_url: format!("http://127.0.0.1:{}", config.port),
            host: "127.0.0.1".to_string(),
            parallel_slots: max_idle,
        };
        self.clients.insert(server_id, ArcSwap::from_pointee(info));
        Ok(())
    }

    /// Helper to build a `reqwest::Client` with the correct pool settings.
    //
    // Note on timeouts: only `connect_timeout` is set here. A Client-level
    // `.timeout()` would cap the entire request including body read, which
    // for SSE streaming responses would terminate legitimate long completions.
    // Stuck-after-connect protection lives at the dispatch layer (proxy_task
    // wraps the connect future in tokio::time::timeout for defense in depth).
    fn build_client(parallel_slots: usize) -> Result<Client, reqwest::Error> {
        let max_idle = parallel_slots.max(16);

        Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(max_idle)
            .tcp_keepalive(Duration::from_secs(10))
            .build()
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

        let request = info
            .client
            .post(&url)
            .json(&body)
            .build()
            .map_err(|e| DispatchError::ConnectionFailed(Box::new(e)))?;

        let response = info.client.execute(request).await.map_err(|e| {
            if e.is_timeout() {
                DispatchError::Timeout
            } else {
                DispatchError::ConnectionFailed(Box::new(e))
            }
        })?;

        let status = response.status();
        if !status.is_success() {
            return Err(DispatchError::BackendError { status: HttpStatus(status.as_u16()) });
        }

        let stream =
            response.bytes_stream().map(|res| res.map_err(|e| StreamError::Internal(Box::new(e))));
        Ok(Box::pin(stream))
    }
}

/// Reconciliation hook: forwards `erase_slot` calls to the backend's
/// `POST {base_url}/slots/{slot}?action=erase` endpoint with a 5s
/// timeout. Documented in design doc §6.629.
///
/// Fire-and-forget semantics: errors (timeout, server unreachable,
/// non-200 response) translate to `SlotError` but do not block the
/// next reconciliation cycle. If the server isn't registered in the
/// store, we surface `ServerUnreachable` — the caller will retry on
/// the next sweep.
#[async_trait]
impl SlotErasure for ReqwestClientStore {
    async fn erase_slot(&self, server_id: &ServerId, slot_id: &SlotId) -> Result<(), SlotError> {
        let client_ref = self
            .clients
            .get(server_id)
            .ok_or_else(|| SlotError::ServerUnreachable { server_id: *server_id })?;
        let info = client_ref.load();
        let url = format!("{}/slots/{}?action=erase", info.base_url, slot_id.into_inner());

        let res = tokio::time::timeout(Duration::from_secs(5), info.client.post(&url).send())
            .await
            .map_err(|_| SlotError::ErasureFailed {
                server_id: *server_id,
                slot_id: *slot_id,
                reason: "timeout".to_string(),
            })?
            .map_err(|e| SlotError::ErasureFailed {
                server_id: *server_id,
                slot_id: *slot_id,
                reason: e.to_string(),
            })?;

        if !res.status().is_success() {
            return Err(SlotError::ErasureFailed {
                server_id: *server_id,
                slot_id: *slot_id,
                reason: format!("status {}", res.status()),
            });
        }
        Ok(())
    }
}

impl ClientManager for ReqwestClientStore {
    fn refresh(&self, server_id: &ServerId, event: ClientRefreshEvent) {
        if let Some(entry) = self.clients.get(server_id) {
            let old_info = entry.load();
            match Self::build_client(old_info.parallel_slots) {
                Ok(new_client) => {
                    let new_info = ServerConnectionInfo {
                        client: new_client,
                        base_url: format!("http://{}:{}", old_info.host, event.new_port),
                        host: old_info.host.clone(),
                        parallel_slots: old_info.parallel_slots,
                    };

                    entry.store(Arc::new(new_info));
                    tracing::info!(
                        "Refreshed client for server {}, new port: {}",
                        server_id,
                        event.new_port
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to build reqwest client during refresh. Keeping old client.");
                }
            }
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

        let config = ServerConfig { port: 8080, ..Default::default() };
        store.add_server(server_id, &config).unwrap();

        let initial_url = {
            let info = store.clients.get(&server_id).unwrap().load().clone();
            info.base_url.clone()
        };
        assert_eq!(initial_url, "http://127.0.0.1:8080");

        let event = ClientRefreshEvent { server_id, new_port: 8081, tls_config: None };
        store.refresh(&server_id, event);

        let new_url = {
            let info = store.clients.get(&server_id).unwrap().load().clone();
            info.base_url.clone()
        };
        assert_eq!(new_url, "http://127.0.0.1:8081");
    }
}
