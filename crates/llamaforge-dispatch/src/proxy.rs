//! Proxy task and streaming implementation.
//!
//! See design doc Section 3

use crate::client::BackendClient;
use crate::dispatcher::CompletionRequest;
use crate::token::DispatchToken;
use std::sync::Arc;

/// A stub for the proxy task. This will be replaced with the real streaming implementation in PR6.
pub async fn proxy_task(
    mut token: DispatchToken,
    _client: Arc<dyn BackendClient>,
    _request: CompletionRequest,
) {
    // Release the token immediately in the stub.
    token.release();
}
