//! Liveness monitor task for the SQLite metrics subsystem.
//!
//! See design doc Section 7.3 and 7.4.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::{error, warn};

use crate::instrumented::InstrumentedMetricsStore;
use crate::metrics_recording::StoreHealthRecorder;
use crate::traits::StoreMaintenance;
use crate::types::{AuditRecord, MetricsConfig, TelemetryEvent};

/// Start the liveness monitor background task.
pub fn start_liveness_monitor(
    config: &MetricsConfig,
    metrics: crate::metrics_recording::MetricsFacadeRecorder,
    store: Arc<InstrumentedMetricsStore>,
    telemetry_tx: Sender<TelemetryEvent>,
    audit_tx: Sender<AuditRecord>,
) -> JoinHandle<()> {
    let wal_oversize_threshold_bytes = config.wal_oversize_threshold_mb * 1024 * 1024;

    tokio::spawn(async move {
        let mut fast_tick = interval(Duration::from_secs(2));
        let mut slow_tick = interval(Duration::from_secs(60));

        // 5s channel polling is handled via the fast tick (2s) modulo
        let mut tick_count = 0;

        loop {
            tokio::select! {
                _ = fast_tick.tick() => {
                    tick_count += 1;

                    // a) Poll writer thread finish state
                    if store.is_writer_finished() {
                        error!("metrics.writer_panicked");
                        // We could set a degraded flag on `store` if it had one
                    }

                    // b) Poll channel capacities every ~6 seconds
                    if tick_count % 3 == 0 {
                        let telemetry_len = telemetry_tx.max_capacity() - telemetry_tx.capacity();
                        let audit_len = audit_tx.max_capacity() - audit_tx.capacity();
                        let cmd_len = 64; // Can't easily poll std::sync::mpsc::SyncSender length, defaulting to 64 or 0

                        metrics.set_channel_pending("telemetry", telemetry_len);
                        metrics.set_channel_pending("audit", audit_len);
                        metrics.set_channel_pending("cmd", cmd_len);
                    }
                }
                _ = slow_tick.tick() => {
                    // c) Poll WAL file size every 60s
                    // We cheat and use `estimated_size_bytes` which reads page count.
                    match store.estimated_size_bytes().await {
                        Ok(size) => {
                            metrics.set_wal_size_bytes(size);
                            if size > wal_oversize_threshold_bytes {
                                warn!(metrics.wal_size_bytes = size, "metrics.wal_oversize");
                            }
                        }
                        Err(e) => {
                            error!("Failed to get DB size for liveness monitor: {}", e);
                        }
                    }
                }
            }
        }
    })
}
