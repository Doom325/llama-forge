//! Proxy task and streaming implementation.
//!
//! See design doc Section 3

use crate::client::BackendClient;
use crate::dispatcher::CompletionRequest;
use crate::error::{CancellationPhase, StreamError};
use crate::token::DispatchToken;
use futures_util::StreamExt;
use llamaforge_types::ports::SlotNotifier;
use llamaforge_types::{RequestId, ServerId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};

/// Response chunk from the proxy task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedChunk {
    pub text: String,
    pub stop: bool,
    #[serde(default)]
    pub tokens_predicted: u32,
}

/// A proxy task that handles connection, streaming, and cleanup for a single request.
///
/// See design doc Section 3.1.
#[instrument(
    skip_all,
    fields(
        request_id = %token.request_id(),
        server_id = %token.server_id(),
    )
)]
pub async fn proxy_task(
    mut token: DispatchToken,
    client: Arc<dyn BackendClient>,
    request: CompletionRequest,
    response_tx: mpsc::Sender<Result<ParsedChunk, StreamError>>,
    slot_notifier: Arc<dyn SlotNotifier>,
    audit_tx: mpsc::Sender<AuditRecord>,
) {
    let start_time = Instant::now();
    let server_id = *token.server_id();
    let request_id = *token.request_id();
    let mut phase = CancellationPhase::PreConnect;
    let mut ttft = None;
    let mut tokens_predicted = 0;

    // Record pending dispatch metric
    metrics::gauge!("llamaforge_pending_dispatches").increment(1.0);

    let result = async {
        // Phase 1: Connect
        let mut stream = client
            .post_completion(&server_id, request)
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to connect to backend");
                StreamError::from(e)
            })?;

        phase = CancellationPhase::PreStream;
        debug!("Connected to backend, starting stream");

        // Phase 2: Streaming loop
        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(c) => c,
                Err(e) => {
                    error!(error = %e, "Stream error from backend");
                    return Err(e);
                }
            };

            if ttft.is_none() {
                let elapsed = start_time.elapsed();
                ttft = Some(elapsed);
                metrics::histogram!("llamaforge_ttft_seconds").record(elapsed.as_secs_f64());
            }

            phase = CancellationPhase::Streaming;

            let parsed = parse_sse_chunk(&chunk)?;
            tokens_predicted += parsed.tokens_predicted;

            // Phase 2: Send to client with backpressure timeout
            match timeout(Duration::from_secs(2), response_tx.send(Ok(parsed.clone()))).await {
                Ok(Ok(())) => {
                    if parsed.stop {
                        break;
                    }
                }
                Ok(Err(_)) => {
                    warn!("Response channel closed by client");
                    return Err(StreamError::ChannelClosed);
                }
                Err(_) => {
                    warn!("Backpressure timeout: client too slow");
                    metrics::counter!("llamaforge_backpressure_disconnects_total").increment(1);
                    return Err(StreamError::Timeout);
                }
            }
        }

        Ok(())
    }
    .await;

    // Handle final result
    if let Err(e) = result {
        let _ = response_tx.send(Err(e)).await;
    }

    // Phase 3: Cleanup
    phase = CancellationPhase::Cleanup;
    let _ = phase; // satisfy clippy
    let duration = start_time.elapsed();

    // Release slot BEFORE audit
    token.release();
    slot_notifier.notify_slot_available(&server_id);

    // Record final metrics
    metrics::counter!("llamaforge_dispatches_total", "server_id" => server_id.to_string()).increment(1);
    metrics::histogram!("llamaforge_dispatch_duration_seconds").record(duration.as_secs_f64());
    metrics::counter!("llamaforge_tokens_completion_total").increment(tokens_predicted as u64);
    metrics::gauge!("llamaforge_pending_dispatches").decrement(1.0);

    // Audit record (fire and forget with timeout)
    let audit_record = AuditRecord {
        request_id,
        server_id,
        duration,
        ttft,
        tokens_predicted,
        success: true, // simplified for now
    };

    tokio::spawn(async move {
        let _ = timeout(Duration::from_secs(10), audit_tx.send(audit_record)).await;
    });

    info!(
        duration_ms = duration.as_millis(),
        ttft_ms = ttft.map(|t| t.as_millis()),
        tokens = tokens_predicted,
        "Dispatch completed"
    );
}

/// Parses a single SSE chunk from llama-server.
fn parse_sse_chunk(chunk: &[u8]) -> Result<ParsedChunk, StreamError> {
    let s = std::str::from_utf8(chunk).map_err(|e| StreamError::ChunkParse {
        reason: format!("Invalid UTF-8: {}", e),
    })?;

    // llama-server sends "data: {...}"
    if let Some(data) = s.strip_prefix("data: ") {
        if data.trim() == "[DONE]" {
            return Ok(ParsedChunk {
                text: "".to_string(),
                stop: true,
                tokens_predicted: 0,
            });
        }

        let parsed: LlamaResponse = serde_json::from_str(data).map_err(|e| StreamError::ChunkParse {
            reason: format!("JSON parse error: {}", e),
        })?;

        Ok(ParsedChunk {
            text: parsed.content,
            stop: parsed.stop,
            tokens_predicted: if parsed.stop { 1 } else { 0 }, // Simplified token counting for now
        })
    } else {
        Err(StreamError::ChunkParse {
            reason: "Missing 'data: ' prefix".to_string(),
        })
    }
}

#[derive(Deserialize)]
struct LlamaResponse {
    content: String,
    stop: bool,
}

/// Record for the audit log.
#[derive(Debug)]
pub struct AuditRecord {
    pub request_id: RequestId,
    pub server_id: ServerId,
    pub duration: Duration,
    pub ttft: Option<Duration>,
    pub tokens_predicted: u32,
    pub success: bool,
}
