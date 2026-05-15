//! Storage traits for the metrics subsystem.

use chrono::{DateTime, Utc};
use llamaforge_config::ServerId;

use crate::error::StoreError;
use crate::filter::{AggregatedMetrics, EventFilter, GroupByField, MetricsFilter};
use crate::types::{AuditEvent, ConfigSnapshot, ManagedProcess, ProcessId, RequestMetrics};

/// Storage operations for telemetry and audit events.
#[allow(async_fn_in_trait)]
pub trait EventStore: Send + Sync + private::Sealed {
    /// Insert a batch of events.
    async fn insert_events(&self, events: Vec<AuditEvent>) -> Result<(), StoreError>;

    /// Query events matching the given filter with optional pagination.
    async fn query_events(
        &self,
        filter: &EventFilter,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<AuditEvent>, StoreError>;
}

/// Storage operations for request-level metrics.
#[async_trait::async_trait]
pub trait RequestMetricsStore: Send + Sync + private::Sealed {
    /// Insert a batch of request metrics.
    async fn insert_request_metrics(&self, metrics: Vec<RequestMetrics>) -> Result<(), StoreError>;

    /// Query request metrics matching the given filter with optional pagination.
    async fn query_request_metrics(
        &self,
        filter: &MetricsFilter,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<RequestMetrics>, StoreError>;

    /// Aggregate request metrics grouped by the specified fields.
    async fn aggregate_request_metrics(
        &self,
        filter: &MetricsFilter,
        group_by: &[GroupByField],
    ) -> Result<Vec<AggregatedMetrics>, StoreError>;
}

/// Storage operations for managing the lifecycle of llama-server processes.
#[allow(async_fn_in_trait)]
pub trait ProcessStore: Send + Sync + private::Sealed {
    /// Upsert a managed process record.
    async fn upsert_managed_process(&self, process: &ManagedProcess) -> Result<(), StoreError>;

    /// Remove a managed process record.
    async fn remove_managed_process(&self, pid: ProcessId) -> Result<(), StoreError>;

    /// List all managed processes.
    async fn list_managed_processes(&self) -> Result<Vec<ManagedProcess>, StoreError>;
}

/// Administrative storage operations.
#[async_trait::async_trait]
pub trait StoreMaintenance: Send + Sync + private::Sealed {
    /// Insert a snapshot of the application configuration.
    async fn insert_config_snapshot(&self, snapshot: &ConfigSnapshot) -> Result<(), StoreError>;

    /// Prune old events and request metrics.
    ///
    /// Returns the number of rows deleted.
    async fn prune_events_older_than(&self, cutoff: DateTime<Utc>) -> Result<u64, StoreError>;

    /// Return the estimated size of the database in bytes.
    async fn estimated_size_bytes(&self) -> Result<u64, StoreError>;

    /// Perform a WAL checkpoint.
    async fn wal_checkpoint(
        &self,
        mode: crate::types::CheckpointMode,
    ) -> Result<crate::types::CheckpointResult, StoreError>;
}

/// An override for a specific configuration value on a specific server.
#[derive(Debug, Clone)]
pub struct ConfigOverlay {
    pub server_id: ServerId,
    pub overlay_key: String,
    pub value: String,
    pub applied_at: DateTime<Utc>,
}

/// Storage operations for server configuration overrides.
///
/// Trait defined; production implementation deferred per design doc §2.8.
#[allow(async_fn_in_trait)]
pub trait ConfigOverlayStore: Send + Sync + private::Sealed {
    /// Get a specific overlay.
    async fn get_overlay(
        &self,
        server_id: ServerId,
        overlay_key: &str,
    ) -> Result<Option<ConfigOverlay>, StoreError>;

    /// Set an overlay value.
    async fn set_overlay(&self, overlay: ConfigOverlay) -> Result<(), StoreError>;

    /// Remove a specific overlay.
    async fn remove_overlay(
        &self,
        server_id: ServerId,
        overlay_key: &str,
    ) -> Result<(), StoreError>;

    /// List all overlays for a specific server.
    async fn list_overlays(&self, server_id: ServerId) -> Result<Vec<ConfigOverlay>, StoreError>;
}

pub(crate) mod private {
    pub trait Sealed {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::in_memory::InMemoryStore;

    async fn test_event_store(store: &impl EventStore) {
        let events = store.query_events(&EventFilter::default(), None, None).await.unwrap();
        assert!(events.is_empty());
    }

    async fn test_request_metrics_store(store: &impl RequestMetricsStore) {
        let metrics =
            store.query_request_metrics(&MetricsFilter::default(), None, None).await.unwrap();
        assert!(metrics.is_empty());
    }

    async fn test_process_store(store: &impl ProcessStore) {
        let procs = store.list_managed_processes().await.unwrap();
        assert!(procs.is_empty());
    }

    async fn test_store_maintenance(store: &impl StoreMaintenance) {
        let size = store.estimated_size_bytes().await.unwrap();
        assert_eq!(size, 0);
    }

    #[tokio::test]
    async fn test_traits_static_dispatch() {
        let store = InMemoryStore::default();
        test_event_store(&store).await;
        test_request_metrics_store(&store).await;
        test_process_store(&store).await;
        test_store_maintenance(&store).await;
    }
}
