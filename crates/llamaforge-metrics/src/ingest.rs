//! Metrics and event ingestion API.
//!
//! See design doc Section 2.2.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{error::TrySendError, Sender};
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::warn;

use crate::error::AuditSendError;
use crate::sqlite::batch_collector::CMD_CHANNEL_EVENTS_DROPPED;
use crate::types::{AuditRecord, TelemetryEvent};

pub static TELEMETRY_EVENTS_DROPPED: AtomicU64 = AtomicU64::new(0);
pub static AUDIT_EVENTS_DROPPED: AtomicU64 = AtomicU64::new(0);

/// Get the number of telemetry events dropped due to backpressure.
pub fn telemetry_drops() -> u64 {
    TELEMETRY_EVENTS_DROPPED.load(Ordering::Relaxed)
}

/// Get the number of audit events dropped due to backpressure.
pub fn audit_drops() -> u64 {
    AUDIT_EVENTS_DROPPED.load(Ordering::Relaxed)
}

/// Get the number of events dropped because the writer command channel was full.
pub fn cmd_channel_drops() -> u64 {
    CMD_CHANNEL_EVENTS_DROPPED.load(Ordering::Relaxed)
}

/// Ingestion interface for telemetry and audit events.
#[derive(Clone)]
pub struct MetricsIngest {
    telemetry_tx: Sender<TelemetryEvent>,
    audit_tx: Sender<AuditRecord>,
    audit_semaphore: Arc<Semaphore>,
}

impl MetricsIngest {
    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn new_for_test(
        telemetry_tx: Sender<TelemetryEvent>,
        audit_tx: Sender<AuditRecord>,
        audit_semaphore: Arc<Semaphore>,
    ) -> Self {
        Self::new(telemetry_tx, audit_tx, audit_semaphore)
    }

    pub(crate) fn new(
        telemetry_tx: Sender<TelemetryEvent>,
        audit_tx: Sender<AuditRecord>,
        audit_semaphore: Arc<Semaphore>,
    ) -> Self {
        Self { telemetry_tx, audit_tx, audit_semaphore }
    }

    /// Send a telemetry event. Returns `true` if accepted, `false` if dropped.
    pub fn send_telemetry(&self, event: TelemetryEvent) -> bool {
        match self.telemetry_tx.try_send(event) {
            Ok(_) => true,
            Err(TrySendError::Full(_)) => {
                let dropped = TELEMETRY_EVENTS_DROPPED.fetch_add(1, Ordering::Relaxed) + 1;
                if dropped % 100 == 1 {
                    warn!("Telemetry channel full. Dropped {} events so far.", dropped);
                }
                false
            }
            Err(TrySendError::Closed(_)) => {
                // If the channel is closed, the system is shutting down or the writer died.
                // We still count it as dropped.
                let dropped = TELEMETRY_EVENTS_DROPPED.fetch_add(1, Ordering::Relaxed) + 1;
                if dropped % 100 == 1 {
                    warn!("Telemetry channel closed. Dropped {} events so far.", dropped);
                }
                false
            }
        }
    }

    /// Send an audit record with backpressure.
    pub async fn send_audit(&self, record: AuditRecord) -> Result<(), AuditSendError> {
        let _permit = self.audit_semaphore.try_acquire().map_err(|_| {
            AUDIT_EVENTS_DROPPED.fetch_add(1, Ordering::Relaxed);
            AuditSendError::SemaphoreExhausted
        })?;

        match timeout(Duration::from_secs(10), self.audit_tx.send(record)).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(_)) => {
                AUDIT_EVENTS_DROPPED.fetch_add(1, Ordering::Relaxed);
                Err(AuditSendError::ChannelClosed)
            }
            Err(_) => {
                AUDIT_EVENTS_DROPPED.fetch_add(1, Ordering::Relaxed);
                Err(AuditSendError::Timeout)
            }
        }
    }

    /// Signal to producers that the pipeline is shutting down.
    /// New audit requests will fail to acquire a permit.
    pub fn close(&self) {
        self.audit_semaphore.close();
    }
}
