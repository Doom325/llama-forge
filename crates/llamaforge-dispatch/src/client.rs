//! Backend client adapter and port traits.
//!
//! See design doc Section 2.5

use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;
use llamaforge_types::{ClientRefreshEvent, ServerId};
use std::pin::Pin;

use crate::dispatcher::CompletionRequest;
use crate::error::{DispatchError, StreamError};

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
