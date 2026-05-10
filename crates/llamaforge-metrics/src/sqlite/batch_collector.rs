//! Async batch collector task for metrics and events.
//!
//! Bridges the non-blocking async world to the synchronous writer thread.
//! See design doc Section 3.2.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::Duration;

use tokio::sync::mpsc::Receiver;
use tokio::sync::oneshot;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, warn, Span};

use crate::sqlite::commands::WriterCommand;
use crate::types::{
    AuditEvent, AuditRecord, EventType, MetricsConfig, RequestMetrics, RequestStatus,
    TelemetryEvent, TelemetryEventKind,
};

// Tracks telemetry dropped during SQLite writer saturation, allowing observability
// of backpressure without blocking the application's hot path.
pub(crate) static CMD_CHANNEL_EVENTS_DROPPED: AtomicU64 = AtomicU64::new(0);

/// Get the number of events dropped due to full command channel.
#[allow(dead_code)]
pub fn cmd_channel_drops() -> u64 {
    CMD_CHANNEL_EVENTS_DROPPED.load(Ordering::Relaxed)
}

impl From<TelemetryEvent> for AuditEvent {
    fn from(t: TelemetryEvent) -> Self {
        let event_kind = match t.kind {
            TelemetryEventKind::ServerStarted => "server_started".to_string(),
            TelemetryEventKind::ServerStopped => "server_stopped".to_string(),
            TelemetryEventKind::ServerCrashed => "server_crashed".to_string(),
            TelemetryEventKind::ConfigReloaded => "config_reloaded".to_string(),
        };

        Self {
            event_id: uuid::Uuid::now_v7().to_string(),
            event_type: EventType::Telemetry,
            event_kind,
            server_id: t.server_id,
            request_id: None,
            created_at: t.timestamp,
            payload: t.payload,
        }
    }
}

impl From<AuditRecord> for AuditEvent {
    fn from(r: AuditRecord) -> Self {
        let event_kind = match r.request_metrics.status {
            RequestStatus::Queued => "request_queued".to_string(),
            RequestStatus::Processing => "request_processing".to_string(),
            RequestStatus::Completed => "request_completed".to_string(),
            RequestStatus::Failed => "request_failed".to_string(),
            RequestStatus::Evicted => "request_evicted".to_string(),
            RequestStatus::Cancelled => "request_cancelled".to_string(),
            RequestStatus::Timeout => "request_timeout".to_string(),
        };

        let payload = serde_json::json!({
            "model_name": r.request_metrics.model_name,
            "slot_id": r.request_metrics.slot_id,
            "task_type": r.request_metrics.task_type.0,
            "requestor": r.request_metrics.requestor.0,
            "effective_priority": r.request_metrics.effective_priority,
            "queue_wait_ms": r.request_metrics.queue_wait_ms,
            "ttft_ms": r.request_metrics.ttft_ms,
            "generation_duration_ms": r.request_metrics.generation_duration_ms,
            "total_latency_ms": r.request_metrics.total_latency_ms,
            "prompt_tokens": r.request_metrics.prompt_tokens,
            "completion_tokens": r.request_metrics.completion_tokens,
            "prompt_tps": r.request_metrics.prompt_tps,
            "generation_tps": r.request_metrics.generation_tps,
            "status": match r.request_metrics.status {
                RequestStatus::Queued => "queued",
                RequestStatus::Processing => "processing",
                RequestStatus::Completed => "completed",
                RequestStatus::Failed => "failed",
                RequestStatus::Evicted => "evicted",
                RequestStatus::Cancelled => "cancelled",
                RequestStatus::Timeout => "timeout",
            },
            "finish_reason": r.request_metrics.finish_reason,
            "was_evicted": r.request_metrics.was_evicted,
            "eviction_count": r.request_metrics.eviction_count,
            "error_message": r.request_metrics.error_message,
        });

        Self {
            event_id: uuid::Uuid::now_v7().to_string(),
            event_type: EventType::Audit,
            event_kind,
            server_id: r.request_metrics.server_id.clone(),
            request_id: Some(r.request_metrics.request_id.clone()),
            created_at: r.request_metrics.completed_at.unwrap_or(r.request_metrics.received_at),
            payload,
        }
    }
}

impl From<AuditRecord> for RequestMetrics {
    fn from(r: AuditRecord) -> Self {
        r.request_metrics
    }
}

/// The main loop for the async batch collector.
pub async fn batch_collector_main(
    mut audit_rx: Receiver<AuditRecord>,
    mut telemetry_rx: Receiver<TelemetryEvent>,
    cmd_tx: SyncSender<WriterCommand>,
    config: MetricsConfig,
) {
    let mut tick = interval(Duration::from_millis(config.batch_flush_interval_ms));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let batch_cap = config.batch_flush_size;

    loop {
        tick.tick().await;

        let mut events_batch = Vec::new();
        let mut metrics_batch = Vec::new();

        while events_batch.len() < batch_cap {
            match audit_rx.try_recv() {
                Ok(record) => {
                    events_batch.push(AuditEvent::from(record.clone()));
                    metrics_batch.push(RequestMetrics::from(record));
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    // One of the channels is dead, but we should keep going until both are dead.
                    // For simplicity, we just break this draining loop.
                    break;
                }
            }
        }

        while events_batch.len() < batch_cap {
            match telemetry_rx.try_recv() {
                Ok(event) => events_batch.push(AuditEvent::from(event)),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        if events_batch.is_empty() && metrics_batch.is_empty() {
            // Check if both senders dropped, meaning we should exit.
            if audit_rx.is_closed() && telemetry_rx.is_closed() {
                debug!("Batch collector channels closed. Exiting.");
                break;
            }
            continue;
        }

        if !events_batch.is_empty() {
            let (reply_tx, _reply_rx) = oneshot::channel();
            let batch_len = events_batch.len();
            let cmd = WriterCommand::InsertEvents {
                events: events_batch,
                reply: reply_tx,
                span: Span::current(),
            };

            match cmd_tx.try_send(cmd) {
                Ok(_) => {}
                Err(TrySendError::Full(_)) => {
                    warn!("Command channel full! Dropped {} events.", batch_len);
                    CMD_CHANNEL_EVENTS_DROPPED.fetch_add(batch_len as u64, Ordering::Relaxed);
                }
                Err(TrySendError::Disconnected(_)) => {
                    debug!("Writer command channel disconnected. Exiting collector.");
                    break;
                }
            }
        }

        // Phase 4: Send InsertMetrics
        if !metrics_batch.is_empty() {
            let (reply_tx, _reply_rx) = oneshot::channel();
            let batch_len = metrics_batch.len();
            let cmd = WriterCommand::InsertMetrics {
                records: metrics_batch,
                reply: reply_tx,
                span: Span::current(),
            };

            match cmd_tx.try_send(cmd) {
                Ok(_) => {}
                Err(TrySendError::Full(_)) => {
                    warn!("Command channel full! Dropped {} request metrics.", batch_len);
                    CMD_CHANNEL_EVENTS_DROPPED.fetch_add(batch_len as u64, Ordering::Relaxed);
                }
                Err(TrySendError::Disconnected(_)) => {
                    debug!("Writer command channel disconnected. Exiting collector.");
                    break;
                }
            }
        }
    }
}
