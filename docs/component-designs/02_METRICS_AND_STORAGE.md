# Component Design: Metrics & Storage

**Component:** Metrics & Storage  
**Phase:** 1 (Foundation)  
**Estimated LOC:** ~2,500  
**Risk:** Medium  
**Dependencies:** Config Manager (DB path, retention policy)  
**Consumers:** Dispatch & Proxy (audit channel), Server Manager (event logging, PID tracking), Transport Layer (Prometheus export, read queries)  
**Crate location:** `crates/llamaforge-metrics/` (unchanged across Phase 9).

---

## 1. Purpose & Scope

The Metrics & Storage component owns all durable state in LlamaForge: audit events, request metrics, managed process records, and configuration snapshots. It bridges the async Tokio world to synchronous SQLite via a dedicated writer thread, provides a connection-pooled read path for concurrent HTTP queries, manages WAL lifecycle, and exports Prometheus metrics for external scraping.

**What this component explicitly does NOT do:**

- It does not interpret or act on events (that's the component that produced them).
- It does not serve HTTP endpoints directly (Transport Layer exposes `/metrics`, `/api/metrics/*`).
- It does not store configuration (that's Config Manager + TOML files). The `server_configs` table is a historical audit trail, never read as authoritative config.
- It does not implement the ConfigOverlayStore in v1.0 (deferred to v1.1; schema is created but unused).
- It does not manage log file rotation (that's the tracing subscriber in the application bootstrap).

---

## 2. Public API Surface

### 2.1 MetricsStore Trait

```rust
/// Storage backend for operational metrics, audit events, and process records.
/// All methods are &self — implementations manage their own connection pooling.
#[async_trait::async_trait]
pub trait MetricsStore: Send + Sync + 'static {
    // --- Audit events (batch insert from writer thread) ---
    async fn insert_events(&self, events: &[AuditEvent]) -> Result<(), StoreError>;

    async fn query_events(
        &self,
        filter: &EventFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>, StoreError>;

    // --- Request metrics (billing-critical) ---
    async fn insert_request_metrics(&self, records: &[RequestMetrics]) -> Result<(), StoreError>;

    async fn query_request_metrics(
        &self,
        filter: &MetricsFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<RequestMetrics>, StoreError>;

    async fn aggregate_request_metrics(
        &self,
        filter: &MetricsFilter,
        group_by: &[GroupByField],
    ) -> Result<Vec<AggregatedMetrics>, StoreError>;

    // --- Managed processes (for orphan detection on restart) ---
    async fn upsert_managed_process(&self, proc: &ManagedProcess) -> Result<(), StoreError>;
    async fn remove_managed_process(&self, pid: u32) -> Result<(), StoreError>;
    async fn list_managed_processes(&self) -> Result<Vec<ManagedProcess>, StoreError>;

    // --- Config snapshots (audit trail) ---
    async fn insert_config_snapshot(&self, snapshot: &ConfigSnapshot) -> Result<(), StoreError>;

    // --- Maintenance ---
    async fn prune_events_older_than(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, StoreError>;
    async fn estimated_size_bytes(&self) -> Result<u64, StoreError>;
    async fn wal_checkpoint(&self, mode: CheckpointMode) -> Result<CheckpointResult, StoreError>;
}
```

### 2.2 Channel Producers (for upstream components)

```rust
/// The ingest interface. Components send events through typed channels.
/// This decouples producers from storage implementation.
pub struct MetricsIngest {
    telemetry_tx: tokio::sync::mpsc::Sender<TelemetryEvent>,
    audit_tx: tokio::sync::mpsc::Sender<AuditRecord>,
}

impl MetricsIngest {
    /// Send a telemetry event (lossy — drops if channel full).
    /// Returns true if sent, false if dropped.
    pub fn send_telemetry(&self, event: TelemetryEvent) -> bool {
        match self.telemetry_tx.try_send(event) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                EVENTS_DROPPED.inc();
                false
            }
            Err(TrySendError::Closed(_)) => false, // writer thread dead
        }
    }

    /// Send an audit record (backpressured with timeout).
    /// Must be called from a detached tokio::spawn task (never from proxy task directly).
    pub async fn send_audit(&self, record: AuditRecord) -> Result<(), AuditSendError> {
        match tokio::time::timeout(
            Duration::from_secs(10),
            self.audit_tx.send(record),
        ).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(AuditSendError::ChannelClosed),
            Err(_) => {
                AUDIT_RECORDS_DROPPED.inc();
                Err(AuditSendError::Timeout)
            }
        }
    }

    /// Clone the audit sender for use in detached tasks.
    pub fn audit_sender(&self) -> tokio::sync::mpsc::Sender<AuditRecord> {
        self.audit_tx.clone()
    }
}
```

### 2.3 Prometheus Registry

```rust
/// Owns all Prometheus metric descriptors. Created once at startup.
/// Components receive references to increment/observe specific metrics.
pub struct PrometheusMetrics {
    pub registry: prometheus::Registry,

    // Request lifecycle
    pub requests_total: IntCounterVec,          // labels: model, task_type, status
    pub request_duration_seconds: HistogramVec, // labels: model, task_type
    pub ttft_seconds: HistogramVec,             // labels: model
    pub tokens_per_second: HistogramVec,        // labels: model, phase={prefill,decode}
    pub queue_depth: IntGaugeVec,               // labels: priority_tier
    pub queue_wait_seconds: HistogramVec,       // labels: priority_tier

    // Server health
    pub server_slots_active: IntGaugeVec,       // labels: server_id
    pub server_slots_total: IntGaugeVec,        // labels: server_id
    pub server_restarts_total: IntCounterVec,   // labels: server_id, reason

    // Storage health
    pub sqlite_wal_size_bytes: IntGauge,
    pub audit_channel_pending: IntGauge,
    pub telemetry_channel_pending: IntGauge,
    pub events_dropped_total: IntCounter,
    pub audit_records_dropped_total: IntCounter,

    // Config
    pub config_reloads_total: IntCounterVec,    // labels: result
    pub unknown_requestor_ratio: Gauge,
}
```

### 2.4 Error Types

```rust
/// Structured error for storage operations.
/// Replaces Box<dyn Error> from the trait for internal use.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("storage unavailable: {message}")]
    Unavailable { message: String },

    #[error("operation timed out after {elapsed:?}")]
    Timeout { elapsed: Duration },

    #[error("internal storage error: {source}")]
    Internal { source: anyhow::Error },
}

#[derive(Debug, thiserror::Error)]
pub enum AuditSendError {
    #[error("audit channel closed (writer thread dead)")]
    ChannelClosed,

    #[error("audit send timed out (10s) — record dropped")]
    Timeout,
}
```

---

## 3. Internal Architecture

### 3.1 Overall Structure

```
┌─────────────────────────────────────────────────────────────────────┐
│                     Async World (Tokio Runtime)                       │
│                                                                       │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                  │
│  │ Proxy Task  │  │ Server Mgr  │  │ Queue       │  (producers)      │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘                  │
│         │ try_send        │ try_send        │ try_send               │
│         ▼                 ▼                 ▼                         │
│  ┌──────────────────────────────────────────────────┐                │
│  │        Telemetry Channel (mpsc, cap: 10,000)     │  ← lossy       │
│  └────────────────────────┬─────────────────────────┘                │
│                           │                                           │
│  ┌──────────────┐         │                                           │
│  │ Detached     │         │                                           │
│  │ Audit Tasks  │         │                                           │
│  └──────┬───────┘         │                                           │
│         │ send().await    │                                           │
│         │ (timeout 10s)   │                                           │
│         ▼                 │                                           │
│  ┌──────────────────────────────────────────────────┐                │
│  │        Audit Channel (mpsc, cap: 1,000)          │  ← backpressure│
│  └────────────────────────┬─────────────────────────┘                │
│                           │                                           │
├───────────────────────────┼───────────────────────────────────────────┤
│                           │  std::sync::mpsc (unbounded)              │
│                           ▼                                           │
│  ┌──────────────────────────────────────────────────┐                │
│  │           WRITER THREAD (std::thread)             │                │
│  │                                                    │                │
│  │  ┌─────────────────────────────────────────────┐  │                │
│  │  │ Batch Collector                              │  │                │
│  │  │ - Drains audit channel (priority)            │  │                │
│  │  │ - Drains telemetry channel (lower priority)  │  │                │
│  │  │ - Flushes every 100ms or batch size 100      │  │                │
│  │  └──────────────────┬──────────────────────────┘  │                │
│  │                     │                              │                │
│  │  ┌──────────────────▼──────────────────────────┐  │                │
│  │  │ SQLite Connection (single, owned)            │  │                │
│  │  │ - WAL mode, synchronous=NORMAL               │  │                │
│  │  │ - Prepared statements cached                  │  │                │
│  │  │ - PASSIVE checkpoint every 60s               │  │                │
│  │  └─────────────────────────────────────────────┘  │                │
│  └──────────────────────────────────────────────────┘                │
│                                                                       │
│  ┌──────────────────────────────────────────────────┐                │
│  │       READ POOL (deadpool-sqlite, max_size: 4)    │                │
│  │  - PRAGMA query_only = ON                         │                │
│  │  - Statement timeout: 30s                         │                │
│  │  - pre_recycle: ROLLBACK + re-assert WAL          │                │
│  │  - Used by: HTTP API queries, Web UI              │                │
│  └──────────────────────────────────────────────────┘                │
└─────────────────────────────────────────────────────────────────────┘
```

### 3.2 Writer Thread State Machine

```
                    ┌──────────┐
                    │  IDLE    │
                    └────┬─────┘
                         │ channel has data OR 100ms timer fires
                         ▼
                    ┌──────────┐
                    │ DRAINING │ ← drain audit first (priority), then telemetry
                    └────┬─────┘
                         │ batch ready (100 events or channels empty)
                         ▼
                    ┌──────────┐
                    │ WRITING  │ ← BEGIN; INSERT batch; COMMIT
                    └────┬─────┘
                         │ success
                         ▼
                    ┌─────────────────┐
                    │ CHECKPOINT_CHECK │ ← every 60s: PRAGMA wal_checkpoint(PASSIVE)
                    └────┬────────────┘
                         │
                         ▼
                    ┌──────────┐
                    │  IDLE    │
                    └──────────┘
```

**Error in WRITING state:** If `COMMIT` fails (disk full, corruption), the writer thread:
1. Logs the error via `tracing::error!` (direct stderr, not through the audit pipeline).
2. Increments a thread-local failure counter.
3. If 10 consecutive failures: set `degraded` flag, stop accepting new writes, surface in `/health`.
4. Attempts recovery: `PRAGMA integrity_check`, reopen connection if needed.

### 3.3 Writer Command Protocol

```rust
/// Commands sent from the async world to the writer thread.
/// Used by the MetricsStore trait impl to bridge async → sync.
enum WriterCommand {
    /// Batch insert events (from batch collector).
    InsertEvents {
        events: Vec<AuditEvent>,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    /// Batch insert request metrics.
    InsertMetrics {
        records: Vec<RequestMetrics>,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    /// Upsert managed process record.
    UpsertProcess {
        proc: ManagedProcess,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    /// Remove managed process record.
    RemoveProcess {
        pid: u32,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    /// Insert config snapshot.
    InsertConfigSnapshot {
        snapshot: ConfigSnapshot,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    /// Prune old events.
    Prune {
        cutoff: chrono::DateTime<chrono::Utc>,
        reply: oneshot::Sender<Result<u64, StoreError>>,
    },
    /// WAL checkpoint (mode: passive or truncate).
    Checkpoint {
        mode: CheckpointMode,
        reply: oneshot::Sender<Result<CheckpointResult, StoreError>>,
    },
    /// Graceful shutdown — flush remaining, close connection.
    Shutdown,
}
```

### 3.4 Channel Architecture Detail

```rust
/// Created at startup. Returns the ingest handle and spawns the writer thread.
pub fn create_metrics_pipeline(
    config: &AppConfig,
) -> Result<(MetricsIngest, SqliteMetricsStore, JoinHandle<()>), StoreError> {
    // 1. Open SQLite, run migrations, configure WAL
    let db_path = &config.global.metrics_db_path;
    let conn = open_and_migrate(db_path)?;

    // 2. Create channels
    let (telemetry_tx, telemetry_rx) = tokio::sync::mpsc::channel(10_000);
    let (audit_tx, audit_rx) = tokio::sync::mpsc::channel(1_000);
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel(); // for WriterCommands

    // 3. Spawn writer thread
    let join_handle = std::thread::Builder::new()
        .name("llamaforge-writer".into())
        .spawn(move || {
            writer_thread_main(conn, telemetry_rx, audit_rx, cmd_rx);
        })?;

    // 4. Create read pool
    let read_pool = create_read_pool(db_path, 4)?;

    // 5. Assemble
    let ingest = MetricsIngest { telemetry_tx, audit_tx };
    let store = SqliteMetricsStore { cmd_tx, read_pool, join_handle };

    Ok((ingest, store, join_handle))
}
```

### 3.5 Key Invariants

1. **Single writer:** Only the writer thread mutates SQLite. No other thread/task holds a write connection.
2. **Audit priority:** When both channels have data, the writer drains the audit channel first (billing-critical data).
3. **GPU independence:** Audit writes never block GPU slot release. The detached `tokio::spawn` + timeout pattern ensures this.
4. **Read isolation:** Read pool connections have `query_only = ON`. Cannot accidentally write.
5. **WAL bounded:** Passive checkpoint every 60s + pre_recycle ROLLBACK on read connections prevents unbounded WAL growth.
6. **Graceful degradation:** Writer thread stall does NOT crash the process. It surfaces as degraded health and dropped records.

---

## 4. Boundary Contracts

### 4.1 What This Component REQUIRES

| Dependency | Contract |
|-----------|----------|
| Config Manager | `metrics_db_path` and `metrics_retention_days` available at startup. Change on reload triggers new prune schedule. |
| Filesystem | DB path writable. Enough disk for WAL growth (recommend ≥1GB free). Local filesystem (not NFS — SQLite WAL is unreliable over network FS). |
| Tokio runtime | `tokio::sync::mpsc` channels functional. `spawn` available for detached audit tasks and channel gauge poller. |

### 4.2 What This Component GUARANTEES to Consumers

| Consumer | Guarantee |
|----------|-----------|
| Dispatch & Proxy | `send_telemetry()` never blocks, never panics. Returns immediately (O(1)). |
| Dispatch & Proxy | `audit_sender()` clone is cheap (Arc increment). |
| Transport Layer | `query_events` / `query_request_metrics` return within 30s (statement timeout). Never hang. |
| Transport Layer | `/metrics` endpoint returns Prometheus text format with all registered metrics. Never errors (returns empty metrics on writer stall). |
| Server Manager | `upsert_managed_process` persists PID before process is launched (crash recovery). |
| All consumers | Writer thread stall surfaces via `audit_channel_pending` gauge > 800 and `/health` degraded flag within 30s. |

### 4.3 Failure Propagation

| Failure | Propagation |
|---------|-------------|
| Disk full | Writer thread INSERT fails → logs error → increments failure counter → after 10 consecutive: sets degraded flag. Audit records dropped with counter. Read path unaffected (no new data written). |
| Writer thread panic | `JoinHandle::is_finished()` detected within 10s → `tracing::error!` via stderr → degraded flag set. Channels disconnected → `send_telemetry` returns false, `send_audit` gets `ChannelClosed`. Read path unaffected. |
| Corrupt database | Writer thread: `PRAGMA integrity_check` on 10th failure → if corrupt, log critical error + set degraded. Read pool: connections may error → return 500 to API. No auto-recovery — operator must restore from backup. |
| Read pool exhausted | `pool.get().await` times out (default: 5s) → HTTP 503 to caller. Writer unaffected. |
| Channel full (telemetry) | Event silently dropped. Counter incremented. No propagation. |
| Channel full (audit) | Detached task awaits up to 10s → drops record + counter. GPU slot already released (contract). |

---

## 5. Failure Modes

### FM-1: Writer Thread Panic

- **Scenario:** Unrecoverable rusqlite error (e.g., schema mismatch after downgrade).
- **Detection:** Liveness monitor (polls `is_finished()` every 10s).
- **State lost:** In-flight batch in writer thread memory. Channel contents (up to 11,000 events) orphaned — receivers are dropped, senders get closed error.
- **State preserved:** All previously committed data. Read pool continues serving.
- **Recovery:** Manual restart. Consider: auto-restart writer thread with fresh connection (v1.1 enhancement).

### FM-2: WAL Grows to 100MB+

- **Scenario:** Long-running read query (Web UI Request Explorer with complex filter) holds a WAL snapshot reference.
- **Detection:** `llamaforge_sqlite_wal_size_bytes` gauge + alerting threshold.
- **Impact:** Disk usage grows; no functional impact until disk full.
- **Recovery:** Statement timeout (30s) eventually releases the read transaction. Operator can trigger `POST /api/maintenance/wal-checkpoint {"mode": "truncate"}`. The endpoint cancels in-flight read queries first.

### FM-3: Sustained High Throughput Exceeds Writer Capacity

- **Scenario:** 10,000+ events/sec sustained overwhelms batch INSERT throughput (~50,000 inserts/sec typical for SQLite WAL on SSD).
- **Detection:** `telemetry_channel_pending` gauge rising; `events_dropped_total` increasing.
- **Impact:** Telemetry data loss (acceptable). Audit channel backpressures detached tasks (bounded by timeout).
- **Recovery:** Self-recovering when load decreases. If persistent: increase batch size, reduce flush interval, or move to Postgres backend.

### FM-4: Pre-Recycle Hook Failure

- **Scenario:** `ROLLBACK` in pre_recycle hook fails (connection in corrupt state).
- **Detection:** `deadpool` reports connection as unhealthy → drops and creates fresh connection.
- **Impact:** One failed read query (retryable). Fresh connection replaces the bad one.
- **State preserved:** All data intact. WAL checkpoint can proceed once bad connection is gone.

### FM-5: Database File Deleted While Running

- **Scenario:** Operator accidentally deletes `llamaforge.db` or the WAL/SHM files.
- **Detection:** Next write fails with IO error. Next read pool connection fails.
- **Impact:** Writer thread enters failure loop. Reads return 500.
- **Recovery:** Restart LlamaForge (recreates DB with fresh schema). Historical data lost.

---

## 6. Configuration

| Knob | Location | Default | Valid Range | Hot-Reload |
|------|----------|---------|-------------|------------|
| `metrics_db_path` | `[global]` | `./llamaforge.db` | Any writable path | No (startup only) |
| `metrics_retention_days` | `[global]` | 30 | 1-365 | Yes (prune schedule updates) |
| Telemetry channel capacity | Hardcoded | 10,000 | — | No |
| Audit channel capacity | Hardcoded | 1,000 | — | No |
| Read pool max_size | Hardcoded | 4 | — | No |
| Read statement timeout | Hardcoded | 30 seconds | — | No |
| Batch flush interval | Hardcoded | 100ms | — | No |
| Batch flush size | Hardcoded | 100 events | — | No |
| WAL checkpoint interval | Hardcoded | 60 seconds | — | No |
| WAL oversize threshold | Hardcoded | 100 MB | — | No |
| Writer failure threshold | Hardcoded | 10 consecutive | — | No |

**What changes without restart:**
- `metrics_retention_days`: The periodic prune task reads this from the live config snapshot on each tick.

---

## 7. Observability

### 7.1 Metrics (self-reported)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `llamaforge_events_dropped_total` | Counter | — | Telemetry events dropped due to full channel |
| `llamaforge_audit_records_dropped_total` | Counter | — | Audit records dropped due to timeout |
| `llamaforge_audit_channel_pending` | Gauge | — | Current audit channel occupancy (polled every 5s) |
| `llamaforge_telemetry_channel_pending` | Gauge | — | Current telemetry channel occupancy |
| `llamaforge_sqlite_wal_size_bytes` | Gauge | — | WAL file size (polled every 60s) |
| `llamaforge_sqlite_writes_total` | Counter | `result={success,error}` | Batch write attempts |
| `llamaforge_sqlite_write_duration_seconds` | Histogram | — | Time per batch INSERT+COMMIT |
| `llamaforge_sqlite_checkpoint_duration_seconds` | Histogram | `mode={passive,truncate}` | Checkpoint duration |
| `llamaforge_sqlite_read_duration_seconds` | Histogram | `query_type` | Read query latency |
| `llamaforge_sqlite_read_pool_active` | Gauge | — | Active connections in read pool |
| `llamaforge_prune_rows_deleted` | Counter | — | Rows deleted by retention prune |

**Label cardinality:** `query_type` is bounded enum: {events, metrics, aggregated, processes}. Max 4 values.

### 7.2 Log Events

| Event | Level | Frequency | Content |
|-------|-------|-----------|---------|
| `metrics.store_initialized` | INFO | Once | DB path, schema version, WAL mode confirmed |
| `metrics.writer_started` | INFO | Once | Thread ID |
| `metrics.writer_stalled` | WARN | When 10+ consecutive failures | Error detail, failure count |
| `metrics.writer_panicked` | ERROR | Rare | Panic payload |
| `metrics.wal_oversize` | WARN | When WAL > 100MB | Current size, last checkpoint result |
| `metrics.prune_completed` | INFO | Daily | Rows deleted, duration |
| `metrics.channel_full` | WARN | Sampled (1 per 100) | Which channel, current depth |

### 7.3 Health Signals

- **Ready:** DB opened, schema migrated, writer thread alive, read pool has ≥1 connection.
- **Live:** Writer thread alive (`is_finished() == false`).
- **Degraded:** Writer thread in failure loop (10+ consecutive errors) OR WAL > 100MB OR audit channel > 80% capacity for > 30s.

---

## 8. Testing Strategy

### 8.1 Unit Test Boundaries

**Pure-logic tests:**
- Batch collector: flush timing (100ms), batch size (100), priority (audit before telemetry)
- Event filter → SQL WHERE clause generation
- Aggregation grouping logic
- Prune cutoff computation from retention config
- Channel gauge computation (`capacity - available = pending`)

**Mockable:**
- The `MetricsStore` trait itself is the mock boundary for all consumers. Consumers test with an in-memory `MockMetricsStore`.

### 8.2 Integration Test Requirements

- **Real SQLite:** All writer thread tests use a tempdir database. No mocking rusqlite.
- **Real channels:** Test channel full behavior (telemetry drop, audit backpressure + timeout).
- **Real read pool:** Test concurrent reads during writes, statement timeout enforcement.
- **Tokio runtime:** Multi-threaded for testing detached audit task behavior.

### 8.3 Property-Based Tests

- **Roundtrip:** `∀ event: insert(event) then query(matching_filter) returns event`
- **Retention:** `∀ events older than cutoff: prune(cutoff) → query returns empty`
- **Ordering:** `∀ events with created_at ordering: query returns them in created_at order (not UUID order)`
- **Concurrent reads don't block writes:** Under sustained write load, read latency stays < 30s

### 8.4 Specific Test Cases

```rust
#[tokio::test]
async fn test_telemetry_channel_full_drops_event() {
    // Fill telemetry channel to capacity → next send returns false.
    // Assert: dropped counter incremented. No panic, no block.
}

#[tokio::test]
async fn test_audit_send_timeout() {
    // Block writer thread (don't drain audit channel).
    // Send audit record → expect timeout after 10s.
    // Assert: AUDIT_RECORDS_DROPPED incremented.
}

#[tokio::test]
async fn test_writer_thread_panic_detected() {
    // Inject panic in writer thread → assert liveness monitor detects within 10s.
    // Assert: degraded flag set, tracing::error emitted.
}

#[tokio::test]
async fn test_wal_checkpoint_passive() {
    // Insert 1000 events → trigger PASSIVE checkpoint.
    // Assert: pages_checkpointed > 0, no blocking.
}

#[tokio::test]
async fn test_pre_recycle_rollback() {
    // Start a read transaction, return connection to pool without committing.
    // Get same connection back from pool → assert: no open transaction (pre_recycle ran).
}

#[tokio::test]
async fn test_concurrent_read_during_write() {
    // Spawn writer inserting continuously.
    // Spawn reader querying continuously.
    // Assert: no SQLITE_BUSY errors, reads complete within 30s.
}

#[tokio::test]
async fn test_retention_prune() {
    // Insert events with old timestamps → prune → query returns empty.
    // Insert events with recent timestamps → prune → query still returns them.
}

#[tokio::test]
async fn test_read_pool_statement_timeout() {
    // Execute a deliberately slow query (sleep via custom function).
    // Assert: times out within 30s, returns error, doesn't hold WAL.
}

#[test]
fn test_schema_migration_idempotent() {
    // Run migrations twice on same DB → no error, same schema.
}
```

---

## 9. Open Questions

### Deferred to Implementation

1. **Batch collector location:** Should the batch collector run as a Tokio task that bridges to the writer thread, or should the writer thread itself poll the async channels via `tokio::runtime::Handle::block_on`? Current design: writer thread uses `std::sync::mpsc` with a bridge task that drains async channels and forwards. Alternative: writer thread owns a `tokio::runtime::Handle` and uses `handle.block_on(rx.recv())` directly. Decision: bridge task is cleaner (writer thread stays pure synchronous).

2. **Prometheus histogram buckets:** What bucket boundaries for `request_duration_seconds`? LLM inference ranges from 100ms (short completions) to 300s (max generation). Recommended: exponential from 0.1 to 300 with factor 2 (0.1, 0.2, 0.4, 0.8, 1.6, 3.2, 6.4, 12.8, 25.6, 51.2, 102.4, 204.8, 300).

3. **Schema migration strategy:** Use `rusqlite_migration` crate or hand-rolled version table? `rusqlite_migration` is lightweight and well-tested. Decision: use it.

4. **Read pool connection recycling interval:** Should idle connections be recycled periodically (to prevent stale WAL references from idle connections)? Decision: yes, `max_lifetime: 300s` on read pool connections.

5. **Metrics export format:** Prometheus text exposition format only, or also OpenMetrics? Start with Prometheus text (standard), add OpenMetrics later if needed.

### Alternatives Considered

- **Postgres from day 1:** Rejected. Single-binary constraint. SQLite is zero-config, embedded, and sufficient for single-machine scale (~50k writes/sec).
- **DuckDB for analytics:** Considered for aggregate queries. Rejected — adds a second engine, complicates deployment. SQLite aggregate performance is adequate for the data volumes (~1M rows/day at peak).
- **ClickHouse/TimescaleDB:** Overkill for single-machine. Reserved for multi-node v2.
- **In-memory only (no persistence):** Rejected. Crash recovery requires knowing which processes were managed (orphan PID detection). Audit trail is a hard requirement.

---

## Cross-Cutting Concerns

### Cancellation Safety

- `MetricsIngest::send_telemetry()`: No `.await` — always cancellation-safe.
- `MetricsIngest::send_audit()`: Single `.await` (channel send with timeout). If cancelled, the record is lost (acceptable — GPU slot already released). No partial state.
- `MetricsStore::insert_events()`: Sends `WriterCommand` + awaits oneshot reply. If cancelled, the command still executes on writer thread (fire-and-forget from writer's perspective). Result is dropped. No harm.
- `MetricsStore::query_events()`: Runs via `pool.get().await` then `interact()`. If cancelled during `pool.get()`, no connection acquired. If cancelled during `interact()`, the query runs to completion on the blocking thread but result is dropped. Connection returned to pool cleanly.

### Graceful Shutdown

On SIGTERM:
1. Stop accepting new telemetry/audit events (drop senders — producers get `Closed`).
2. Send `WriterCommand::Shutdown` to writer thread.
3. Writer thread: drain remaining channel contents, flush final batch, run `PRAGMA wal_checkpoint(TRUNCATE)`, close connection.
4. Join writer thread (blocking — must happen AFTER Tokio runtime shutdown or on a dedicated std::thread).
5. Drop read pool (connections closed).

**Ordering constraint:** Writer thread shutdown MUST happen after all detached audit `tokio::spawn` tasks have completed or timed out. Use a `tokio::sync::Semaphore` or `WaitGroup` to track in-flight audit tasks during shutdown.

### Panic Safety

- **Writer thread panic:** Detected by liveness monitor. Channels go dead. Read path unaffected. System degrades but doesn't crash.
- **Read pool interact() panic:** `InteractError::Panic` caught, logged, returns HTTP 500. Connection dropped from pool, fresh one created. No data corruption (read-only connection).
- **Batch collector task panic:** Writer thread starves (no commands arrive). Same effect as writer panic from consumers' perspective.

### Clock Assumptions

- **`created_at` in events:** Uses `Instant` (monotonic) for internal ordering. `DateTime<Utc>` for display/serialization only.
- **Retention prune:** Uses `DateTime<Utc>` for cutoff (acceptable — prune is coarse-grained, ±1 hour doesn't matter).
- **WAL checkpoint interval:** Uses `Instant`-based timer. Not affected by clock skew.

### Memory Bounds

| Buffer | Capacity | Item Size (est.) | Max Memory |
|--------|----------|-----------------|------------|
| Telemetry channel | 10,000 | ~500 bytes | 5 MB |
| Audit channel | 1,000 | ~2 KB | 2 MB |
| Writer batch buffer | 100 | ~500 bytes | 50 KB |
| Read pool connections | 4 | ~100 KB each | 400 KB |
| In-flight audit tasks | Unbounded* | ~2 KB each | See below |

*In-flight audit tasks are bounded by the 10s timeout: at most `throughput × 10s` tasks alive. At 1000 req/s: 10,000 tasks × 2KB = 20MB. Under writer stall, this is the memory ceiling before timeouts start dropping records.

**Total worst case:** ~28 MB. Acceptable.

### Backpressure

- **Telemetry producers → channel:** No backpressure. Drop on full. Producers never block.
- **Audit producers → channel:** Backpressure via bounded channel, bounded by 10s timeout. Then drops.
- **Writer thread → disk:** If disk is slow, writer falls behind. Channels fill. Telemetry drops, audit backpressures then drops. System degrades gracefully.
- **Read queries → pool:** Bounded by pool size (4). Additional queries wait for `pool.get()` (bounded by caller's timeout, typically 5s). No unbounded queueing.
