//! Periodic prune task for metrics maintenance.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::{error, info};

use llamaforge_core::config::ConfigReader;

use crate::metrics_recording::StoreHealthRecorder;
use crate::traits::StoreMaintenance;

/// Start the periodic maintenance task.
pub fn start_prune_task<
    M: StoreHealthRecorder + Send + Sync + 'static,
    S: StoreMaintenance + Send + Sync + 'static,
>(
    store: Arc<S>,
    config_reader: Arc<dyn ConfigReader>,
    metrics: M,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Run daily
        let mut tick = interval(Duration::from_secs(86400));

        loop {
            tick.tick().await;

            let retention_days = config_reader.snapshot().global.metrics_retention_days as i64;
            let cutoff = Utc::now() - chrono::Duration::days(retention_days);

            match store.prune_events_older_than(cutoff).await {
                Ok(deleted) => {
                    info!(
                        metrics.prune_completed = true,
                        rows_deleted = deleted,
                        "Periodic prune completed"
                    );
                    metrics.record_prune_rows(deleted);
                }
                Err(e) => {
                    error!("Periodic prune failed: {}", e);
                }
            }
        }
    })
}
