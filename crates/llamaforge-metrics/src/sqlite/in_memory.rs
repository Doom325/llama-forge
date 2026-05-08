//! In-memory test double for the metrics store.

use chrono::{DateTime, Utc};
use parking_lot::Mutex;

use crate::error::StoreError;
use crate::filter::{AggregatedMetrics, EventFilter, GroupByField, MetricsFilter};
use crate::traits::{private, EventStore, ProcessStore, RequestMetricsStore, StoreMaintenance};
use crate::types::{AuditEvent, ConfigSnapshot, ManagedProcess, ProcessId, RequestMetrics};

/// An in-memory implementation of the storage traits, used for testing decorators and consumers.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Default)]
pub(crate) struct InMemoryStore {
    events: Mutex<Vec<AuditEvent>>,
    metrics: Mutex<Vec<RequestMetrics>>,
    processes: Mutex<Vec<ManagedProcess>>,
    snapshots: Mutex<Vec<ConfigSnapshot>>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl InMemoryStore {
    #[cfg(test)]
    pub fn replace_events(&self, new_events: Vec<AuditEvent>) {
        *self.events.lock() = new_events;
    }
}

impl private::Sealed for InMemoryStore {}

impl EventStore for InMemoryStore {
    async fn insert_events(&self, events: Vec<AuditEvent>) -> Result<(), StoreError> {
        self.events.lock().extend(events);
        Ok(())
    }

    async fn query_events(
        &self,
        _filter: &EventFilter,
        _limit: Option<usize>,
        _offset: Option<usize>,
    ) -> Result<Vec<AuditEvent>, StoreError> {
        // Simplified for testing: just returns all events
        Ok(self.events.lock().clone())
    }
}

impl RequestMetricsStore for InMemoryStore {
    async fn insert_request_metrics(&self, metrics: Vec<RequestMetrics>) -> Result<(), StoreError> {
        self.metrics.lock().extend(metrics);
        Ok(())
    }

    async fn query_request_metrics(
        &self,
        _filter: &MetricsFilter,
        _limit: Option<usize>,
        _offset: Option<usize>,
    ) -> Result<Vec<RequestMetrics>, StoreError> {
        // Simplified for testing: just returns all metrics
        Ok(self.metrics.lock().clone())
    }

    async fn aggregate_request_metrics(
        &self,
        _filter: &MetricsFilter,
        _group_by: &[GroupByField],
    ) -> Result<Vec<AggregatedMetrics>, StoreError> {
        // Stub implementation for testing
        Ok(Vec::new())
    }
}

impl ProcessStore for InMemoryStore {
    async fn upsert_managed_process(&self, process: &ManagedProcess) -> Result<(), StoreError> {
        let mut procs = self.processes.lock();
        if let Some(existing) = procs.iter_mut().find(|p| p.pid == process.pid) {
            *existing = process.clone();
        } else {
            procs.push(process.clone());
        }
        Ok(())
    }

    async fn remove_managed_process(&self, pid: ProcessId) -> Result<(), StoreError> {
        self.processes.lock().retain(|p| p.pid != pid);
        Ok(())
    }

    async fn list_managed_processes(&self) -> Result<Vec<ManagedProcess>, StoreError> {
        Ok(self.processes.lock().clone())
    }
}

impl StoreMaintenance for InMemoryStore {
    async fn insert_config_snapshot(&self, snapshot: &ConfigSnapshot) -> Result<(), StoreError> {
        self.snapshots.lock().push(snapshot.clone());
        Ok(())
    }

    async fn prune_events_older_than(&self, cutoff: DateTime<Utc>) -> Result<u64, StoreError> {
        let mut events = self.events.lock();
        let initial_len = events.len();
        events.retain(|e| e.created_at >= cutoff);
        let deleted = (initial_len - events.len()) as u64;

        let mut metrics = self.metrics.lock();
        metrics.retain(|m| m.received_at >= cutoff);

        Ok(deleted)
    }

    async fn estimated_size_bytes(&self) -> Result<u64, StoreError> {
        // Dummy size based on item count
        let total_items = self.events.lock().len()
            + self.metrics.lock().len()
            + self.processes.lock().len()
            + self.snapshots.lock().len();
        Ok((total_items * 1024) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_store_coverage() {
        let store = InMemoryStore::default();
        store.replace_events(vec![]);
        drop(store.processes.lock());
        drop(store.snapshots.lock());
    }
}
