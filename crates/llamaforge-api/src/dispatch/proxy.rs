//! Proxy task and streaming implementation.
//!
//! See design doc Section 3

use super::client::BackendClient;
use super::dispatcher::CompletionRequest;
use super::error::{CancellationPhase, DispatchError, HttpStatus, StreamError};
use super::slot_map::SlotDispatchMap;
use super::token::DispatchToken;
use futures_util::StreamExt;
use llamaforge_core::types::traits::SlotNotifier;
use llamaforge_core::types::{RequestId, ServerId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, instrument, warn};

/// Response chunk from the proxy task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedChunk {
    pub text: String,
    pub stop: bool,
    #[serde(default)]
    pub tokens_predicted: u32,
    /// Upstream `tokens_evaluated` (prompt size). Non-zero only on the
    /// terminal stop chunk; zero on intermediate token chunks.
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct LlamaResponse {
    content: String,
    stop: bool,
    #[serde(default)]
    tokens_evaluated: Option<u32>,
    #[serde(default)]
    prompt_n: Option<u32>,
    #[serde(default)]
    tokens_processed: Option<u32>,
    #[serde(default)]
    tokens_predicted: Option<u32>,
}

/// Record for the audit log.
#[derive(Debug)]
pub struct AuditRecord {
    pub request_id: RequestId,
    pub server_id: ServerId,
    pub duration: Duration,
    pub ttft: Option<Duration>,
    pub tokens_predicted: u32,
    pub prompt_tokens: u32,
    pub success: bool,
}

/// A proxy task that handles connection, streaming, and cleanup for a single request.
///
/// See design doc Section 3.1.
/// RAII guard for pending dispatches metric.
struct PendingDispatchToken;
impl PendingDispatchToken {
    #[allow(clippy::new_without_default)]
    fn new() -> Self {
        metrics::gauge!("llamaforge_pending_dispatches").increment(1.0);
        Self
    }
}
impl Drop for PendingDispatchToken {
    fn drop(&mut self) {
        metrics::gauge!("llamaforge_pending_dispatches").decrement(1.0);
    }
}

#[instrument(
    skip_all,
    fields(
        request_id = %token.request_id(),
        server_id = %token.server_id(),
    )
)]
#[allow(clippy::too_many_arguments)]
pub async fn proxy_task(
    token: DispatchToken,
    client: Arc<dyn BackendClient>,
    request: CompletionRequest,
    response_tx: mpsc::Sender<Result<ParsedChunk, StreamError>>,
    slot_notifier: Arc<dyn SlotNotifier>,
    audit_tx: mpsc::Sender<AuditRecord>,
    cancel_token: CancellationToken,
    slot_dispatch_map: Arc<SlotDispatchMap>,
) {
    let start_time = Instant::now();
    let server_id = *token.server_id();
    let request_id = *token.request_id();
    #[allow(unused_assignments)]
    let mut phase = CancellationPhase::PreConnect;
    let mut ttft = None;
    let mut tokens_predicted = 0;
    let mut prompt_tokens: u32 = 0;
    let id_slot = request.id_slot; // Copy before move

    let _guard = PendingDispatchToken::new();
    tracing::debug!("dispatch.started");

    // PreConnect phase. Wrap the connect future in a tokio timeout so a
    // backend that accepts the TCP connection but never writes a response
    // cannot pin this dispatch slot indefinitely (the reqwest connect_timeout
    // only covers TCP handshake, not waiting for response headers).
    let connect_future = async {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let res = client.post_completion(&server_id, request.clone()).await;
            match res {
                Ok(s) => return Ok(s),
                Err(e) if attempts < 2 => {
                    let should_retry = matches!(
                        e,
                        DispatchError::ConnectionFailed(_)
                            | DispatchError::BackendError { status: HttpStatus(503) }
                            | DispatchError::Timeout
                    );
                    if should_retry {
                        tracing::debug!(error = %e, attempt = attempts, "dispatch.retry_scheduled");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                    return Err(e);
                }
                Err(e) => return Err(e),
            }
        }
    };
    let connect_future = timeout(Duration::from_secs(30), connect_future);

    let mut stream = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => {
            tracing::debug!("dispatch.cancelled");
            cleanup(token, server_id, request_id, id_slot, start_time, ttft, tokens_predicted, prompt_tokens, false, slot_notifier, audit_tx, slot_dispatch_map);
            return;
        }
        res = connect_future => {
            match res {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "dispatch.failed");
                    let _ = response_tx.send(Err(StreamError::from(e))).await;
                    cleanup(token, server_id, request_id, id_slot, start_time, ttft, tokens_predicted, prompt_tokens, false, slot_notifier, audit_tx, slot_dispatch_map);
                    return;
                }
                Err(_) => {
                    tracing::warn!("dispatch.connect_timeout");
                    let _ = response_tx.send(Err(StreamError::Timeout)).await;
                    cleanup(token, server_id, request_id, id_slot, start_time, ttft, tokens_predicted, prompt_tokens, false, slot_notifier, audit_tx, slot_dispatch_map);
                    return;
                }
            }
        }
    };

    phase = CancellationPhase::PreStream;
    let mut buffer = Vec::new();
    let mut success = true;

    // Slot assigned, add to map
    if let Some(slot) = id_slot {
        slot_dispatch_map.insert(request_id, server_id, slot);
    }

    'outer: loop {
        tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                tracing::debug!("dispatch.cancelled");
                if phase == CancellationPhase::Streaming {
                    // Send upstream failure notification
                    let _ = response_tx.send(Ok(ParsedChunk {
                        text: "".to_string(),
                        stop: true,
                        tokens_predicted: 0,
                        prompt_tokens: 0,
                        finish_reason: Some("FINISH_REASON_UPSTREAM_FAILURE".to_string()),
                    })).await;
                }
                success = false;
                break 'outer;
            }
            chunk_res = stream.next() => {
                let chunk = match chunk_res {
                    Some(Ok(c)) => c,
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "dispatch.failed");
                        let _ = response_tx.send(Err(e)).await;
                        success = false;
                        break 'outer;
                    }
                    None => {
                        // Stream ended
                        break 'outer;
                    }
                };

                if ttft.is_none() {
                    let elapsed = start_time.elapsed();
                    ttft = Some(elapsed);
                    metrics::histogram!("llamaforge_ttft_seconds").record(elapsed.as_secs_f64());
                }

                phase = CancellationPhase::Streaming;
                buffer.extend_from_slice(&chunk);

                let mut chunks_to_send = Vec::new();
                while let Some(idx) = buffer.iter().position(|&b| b == b'\n') {
                    let line_bytes = buffer.drain(..=idx).collect::<Vec<_>>();
                    if let Ok(line) = std::str::from_utf8(&line_bytes) {
                        let line = line.trim();
                        if line.is_empty() { continue; }

                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                                chunks_to_send.push(ParsedChunk {
                                    text: "".to_string(),
                                    stop: true,
                                    tokens_predicted: 0,
                                    prompt_tokens,
                                    finish_reason: None,
                                });
                                break;
                            } else if let Ok(parsed) = serde_json::from_str::<LlamaResponse>(data) {
                                // llama-server only emits a non-null
                                // `tokens_evaluated` (and its aliases) on
                                // the terminal chunk. Latch the largest
                                // value we observe across aliases so the
                                // stop chunk carries the prompt-token
                                // count regardless of the upstream's
                                // field-name convention.
                                let observed = [
                                    parsed.tokens_evaluated,
                                    parsed.prompt_n,
                                    parsed.tokens_processed,
                                ]
                                .into_iter()
                                .flatten()
                                .max()
                                .unwrap_or(0);
                                if observed > prompt_tokens {
                                    prompt_tokens = observed;
                                }

                                if let Some(pred) = parsed.tokens_predicted {
                                    if pred > tokens_predicted {
                                        tokens_predicted = pred;
                                    }
                                } else {
                                    tokens_predicted += 1;
                                }

                                chunks_to_send.push(ParsedChunk {
                                    text: parsed.content,
                                    stop: parsed.stop,
                                    tokens_predicted: if parsed.stop { tokens_predicted } else { 1 },
                                    prompt_tokens: if parsed.stop { prompt_tokens } else { 0 },
                                    finish_reason: None,
                                });
                            }
                        }
                    }
                }

                for parsed in chunks_to_send {
                    let is_stop = parsed.stop;
                    match timeout(Duration::from_secs(2), response_tx.send(Ok(parsed))).await {
                        Ok(Ok(())) => {
                            if is_stop {
                                // Break the loop, stream finished.
                                return cleanup(token, server_id, request_id, id_slot, start_time, ttft, tokens_predicted, prompt_tokens, success, slot_notifier, audit_tx, slot_dispatch_map);
                            }
                        }
                        Ok(Err(_)) => {
                            warn!("Response channel closed by client");
                            success = false;
                            break 'outer;
                        }
                        Err(_) => {
                            warn!("dispatch.backpressure");
                            metrics::counter!("llamaforge_backpressure_disconnects_total").increment(1);
                            let _ = response_tx.try_send(Err(StreamError::Timeout));
                            success = false;
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    tracing::debug!("dispatch.completed");
    cleanup(
        token,
        server_id,
        request_id,
        id_slot,
        start_time,
        ttft,
        tokens_predicted,
        prompt_tokens,
        success,
        slot_notifier,
        audit_tx,
        slot_dispatch_map,
    );
}

#[allow(clippy::too_many_arguments)]
fn cleanup(
    mut token: DispatchToken,
    server_id: ServerId,
    request_id: RequestId,
    id_slot: Option<llamaforge_core::types::SlotId>,
    start_time: Instant,
    ttft: Option<Duration>,
    tokens_predicted: u32,
    prompt_tokens: u32,
    success: bool,
    slot_notifier: Arc<dyn SlotNotifier>,
    audit_tx: mpsc::Sender<AuditRecord>,
    slot_dispatch_map: Arc<SlotDispatchMap>,
) {
    let duration = start_time.elapsed();

    // Remove from slot map
    if id_slot.is_some() {
        slot_dispatch_map.remove(&request_id);
    }

    // Release slot BEFORE audit
    token.release();
    slot_notifier.notify_slot_available();

    // Record final metrics
    metrics::counter!("llamaforge_dispatches_total", "server_id" => server_id.to_string())
        .increment(1);
    metrics::histogram!("llamaforge_dispatch_duration_seconds").record(duration.as_secs_f64());
    metrics::counter!("llamaforge_tokens_completion_total").increment(tokens_predicted as u64);
    metrics::counter!("llamaforge_tokens_prompt_total").increment(prompt_tokens as u64);

    // Audit record (fire and forget with timeout)
    let audit_record = AuditRecord {
        request_id,
        server_id,
        duration,
        ttft,
        tokens_predicted,
        prompt_tokens,
        success,
    };

    tokio::spawn(async move {
        let _ = timeout(Duration::from_secs(10), audit_tx.send(audit_record)).await;
    });

    info!(
        duration_ms = duration.as_millis(),
        ttft_ms = ttft.map(|t| t.as_millis()),
        tokens = tokens_predicted,
        prompt_tokens = prompt_tokens,
        "Dispatch completed"
    );
}
