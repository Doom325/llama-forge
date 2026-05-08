# Metrics & Storage — Implementation Plan

**Component:** Metrics & Storage (`02_METRICS_AND_STORAGE.md`)  
**Estimated LOC:** ~2,900  
**Estimated Duration:** 10–11 working days (single engineer) / 6–7 days (two engineers, PR4 code + PR5 code parallelizable after PR3; PR5 tests require PR4 merged)  
**Design Document:** `component-designs/02_METRICS_AND_STORAGE.md`  
**Branch Prefix:** `metrics/`  
**Target:** `main` (via squash-merge PRs)  
**Prerequisite:** Config Manager (PR1–PR7 merged) — depends on `AppConfig`, `ServerId`, `TaskTypeId`, `RequestorId` types and `ConfigReader` for `metrics_db_path`/`retention_days`.  
**Codebase Location:** `~/git/llama-forge` (Cargo workspace root)  
**Existing Crates:** `llamaforge` (binary), `llamaforge-config` (implemented), `llamaforge-proto` (stub)

---

## Ground Rules for CoderAgents

1. **Read the design doc first.** Every PR description must reference the specific design doc sections it implements.
2. **No `unwrap()` or `expect()` anywhere** — not in production code, not inside `interact()` closures. A panic inside `interact()` permanently drops the pool connection.
3. **All SQLite I/O runs off the Tokio reactor.** Writer thread uses `std::thread`. Read path uses `deadpool-sqlite` `interact()` (runs on blocking threadpool).
4. **Single-writer invariant is sacred.** Only the writer thread mutates the database. Read pool connections have `PRAGMA query_only = ON`.
5. **Audit before telemetry.** Whenever both channels have data, drain audit first (billing-critical).
6. **Every public type gets a doc comment.** Reference the design doc section number (e.g., `/// See design doc Section 2.5.`).
7. **Tests use `tempfile::TempDir` for database isolation.** Never use a fixed path.
8. **Commit messages:** `metrics: <imperative summary>` (e.g., `metrics: add batch collector with audit-priority draining`).

### Conventions from Config Manager Implementation (MUST follow)

9. **Module visibility:** All modules declared as `pub(crate) mod` in `lib.rs`, with explicit `pub use` re-exports for the public API. Never use bare `pub mod`.
10. **Sealed traits:** All port/consumer traits use the sealed trait pattern (`trait Sealed {}` in a `private` module). See `llamaforge-config/src/traits.rs` for the exact pattern.
11. **Compile-time dyn assertions:** Add `const _: Option<&dyn TraitName> = None;` for every trait that must be object-safe.
12. **`#[non_exhaustive]`:** All public enums get `#[non_exhaustive]` for forward compatibility.
13. **`metrics` crate (not `prometheus`):** Use the `metrics` facade crate (already in workspace at version `0.22`) with `counter!`, `histogram!`, `describe_counter!`, `describe_histogram!` macros — NOT the `prometheus` crate directly. This matches Config Manager's `InstrumentedConfigHandle` pattern.
14. **`parking_lot::Mutex`** for synchronous locks (already in workspace). NOT `std::sync::Mutex`.
15. **`#[allow(async_fn_in_trait)]`** on async trait definitions. No `#[async_trait]` macro.
16. **Error types:** Use `thiserror`, `#[non_exhaustive]`, and `Box<dyn std::error::Error + Send + Sync>` for opaque inner errors.
17. **Test doubles:** Follow the `InMemoryConfigSource` pattern: `pub(crate)` struct with `#[cfg_attr(not(test), allow(dead_code))]`.
18. **`spawn_blocking` for file I/O** in async context, with parent span propagation: `let parent_span = tracing::Span::current(); tokio::task::spawn_blocking(move || { parent_span.in_scope(|| ...) })`.
19. **Instrumented decorator pattern:** Zero overhead on hot paths (no instrumentation on reads), full tracing + metrics on write/mutation paths. Use `tracing::field::Empty` for late-recorded span fields.

---

## Dependency Graph (PR merge order)

```
PR1 ─── PR2 ─── PR3 ─── PR4 ─── PR5 ─── PR6 ─── PR7 ─── PR8
                          │               │
                          └── can overlap ─┘
                          (PR4 writer vs PR5 read pool)
```

PR1 → PR2 → PR3 → PR4 → PR5 → PR6 → PR7 → PR8 (strict merge ordering; PR4 code and PR5 code can be **developed** concurrently after PR3 merges, but PR5 **integration tests** require PR4 merged because read-path tests need a functioning writer to insert data)

---

## PR1: Crate Setup & Domain Types

**Branch:** `metrics/01-types`  
**Merge Target:** `main`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 3–4 hours

### Purpose

Create the `llamaforge-metrics` library crate, define all domain types from design doc Section 2.5, error types from Section 2.4, and query/filter types. This is the type foundation — every other PR builds on it. No I/O, no SQLite, no channels.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 1.1 | Add `llamaforge-metrics` to workspace | New lib crate `crates/llamaforge-metrics`. Add to workspace members. Depends on `llamaforge-config` (for `ServerId`, `TaskTypeId`, `RequestorId`). | `Cargo.toml`, `crates/llamaforge-metrics/Cargo.toml` |
| 1.2 | Wire workspace dependencies | Inherit from workspace (already available): `serde`, `thiserror`, `tokio`, `tracing`, `parking_lot`, `metrics`, `nix`. Add new workspace deps: `chrono = { version = "0.4", features = ["serde"] }`. Dev-deps (inherit): `tempfile`, `proptest`, `typed-builder`, `tokio` (with `test-util`), `tracing-subscriber`. | `Cargo.toml` (workspace), `crates/llamaforge-metrics/Cargo.toml` |
| 1.3 | Create module structure | `lib.rs` declares all modules as `pub(crate) mod`: `types`, `error`, `filter`, `traits`, `ingest`, `sqlite`, `metrics_recording`, `instrumented`. Stub each module file. Add explicit `pub use` re-exports at bottom of `lib.rs` for the public API (populated as types are implemented in Tasks 1.4–1.11). Follow the exact pattern from `llamaforge-config/src/lib.rs`. **Note:** module is `metrics_recording` not `prometheus_metrics` — the `metrics` facade crate is backend-agnostic. | `src/lib.rs`, `src/*.rs` |
| 1.4 | Implement producer-side types | `TelemetryEvent` (with `#[non_exhaustive] TelemetryEventKind` enum), `AuditRecord`, `#[non_exhaustive] RequestStatus` enum. All with `Debug, Clone`. `RequestStatus` also gets `Serialize, Deserialize, PartialEq, Eq, Copy`. | `src/types.rs` |
| 1.5 | Implement domain storage types | `AuditEvent`, `#[non_exhaustive] EventType` enum, `RequestMetrics`, `ManagedProcess`, `ProcessId` newtype (`u32`, transparent serde, derive `Default` per convention), `ConfigSnapshot`, `#[non_exhaustive] ConfigSnapshotTrigger` enum. All per design doc Section 2.5. | `src/types.rs` |
| 1.6 | Implement `RequestId` newtype | `RequestId(pub String)` with `Debug, Clone, PartialEq, Eq, Hash`. Add `Display`, `From<String>`, `From<&str>`. | `src/types.rs` |
| 1.7 | Implement query/filter types | `EventFilter`, `MetricsFilter` (both `Default`-able), `GroupByField` enum, `AggregatedMetrics` (with `Serialize, Deserialize`). | `src/filter.rs` |
| 1.8 | Implement WAL types | `CheckpointMode` enum (`Passive`, `Truncate`), `CheckpointResult` struct. | `src/types.rs` |
| 1.9 | Implement error types | `#[non_exhaustive] StoreError` (3 variants: `Unavailable`, `Timeout`, `Internal` with `Box<dyn std::error::Error + Send + Sync>`), `#[non_exhaustive] AuditSendError` (3 variants: `ChannelClosed`, `Timeout`, `SemaphoreExhausted`). Both with `thiserror`. Follow the exact pattern from `llamaforge-config/src/error.rs` — `#[derive(Debug, Error)]`, `#[non_exhaustive]`, descriptive `#[error("...")]` messages. | `src/error.rs` |
| 1.10 | Implement `WriterMode` enum | `Healthy`, `Degraded { consecutive_failures: u32, since: Instant }`. Internal to writer, but define now for use in later PRs. | `src/types.rs` |
| 1.11 | Implement `MetricsConfig` struct | All 12 configurable knobs from design doc Section 6: `db_path: PathBuf`, `retention_days: u32`, `telemetry_channel_capacity: usize`, `audit_channel_capacity: usize`, `audit_semaphore_permits: usize`, `read_pool_size: usize`, `read_statement_timeout_secs: u64`, `batch_flush_interval_ms: u64`, `batch_flush_size: usize`, `wal_checkpoint_interval_secs: u64`, `wal_oversize_threshold_mb: u64`, `writer_failure_threshold: u32`. Every field `#[serde(default = "...")]` with a named default matching design doc. Derive `Default, Debug, Clone, Serialize, Deserialize, PartialEq`. **This struct lives in `llamaforge-metrics`, NOT in `llamaforge-config`** — the composition root (`main.rs`) extracts values from `AppConfig` and constructs `MetricsConfig`. | `src/types.rs` |
| 1.12 | Unit tests: type construction | Construct every type, verify `Debug` output is meaningful. Verify `RequestStatus` serde roundtrip. Verify `ProcessId` transparent serde. Verify `EventFilter::default()` has all `None` fields. Verify `MetricsConfig::default()` matches design doc defaults (db_path="./llamaforge.db", retention_days=30, telemetry_channel_capacity=10000, etc.). | `src/types.rs` (tests) |
| 1.13 | Unit tests: type interop with config types | Verify `ServerId`, `TaskTypeId`, `RequestorId` from `llamaforge-config` work in `AuditRecord`, `RequestMetrics`, `ManagedProcess`. Verify no orphan-rule issues. | `tests/type_interop.rs` |

### PR1 Acceptance Criteria

- [x] `cargo build --workspace` succeeds with zero warnings
- [x] All domain types compile with required derives
- [x] `llamaforge-metrics` depends on `llamaforge-config` and uses its newtypes
- [x] `StoreError` uses `Box<dyn Error>` (not `anyhow::Error`) per design doc
- [x] All public enums have `#[non_exhaustive]`
- [x] `lib.rs` uses `pub(crate) mod` + explicit `pub use` pattern (matching `llamaforge-config`)
- [x] All unit tests pass
- [x] `EventFilter::default()` and `MetricsFilter::default()` produce empty filters

### PR1 Creation Steps

```bash
git checkout -b metrics/01-types main
# ... implement tasks 1.1–1.12 ...
git add -A && git commit -m "metrics: add llamaforge-metrics crate with domain types, errors, and filters"
git push -u origin metrics/01-types
# Open PR: "Metrics & Storage PR1: Crate Setup & Domain Types"
# Description: "Creates llamaforge-metrics lib crate. Defines all domain types (Section 2.5),
# error types (Section 2.4), and query/filter types. Producer-side types (TelemetryEvent,
# AuditRecord) and storage types (AuditEvent, RequestMetrics, ManagedProcess, ConfigSnapshot).
# Newtypes: ProcessId, RequestId. No I/O — pure type definitions."
```

---

## PR2: Storage Traits & Observability Port Traits

**Branch:** `metrics/02-traits`  
**Base:** `metrics/01-types`  
**Size:** Small–Medium (~250 lines)  
**Estimated Time:** 3–4 hours

### Purpose

Define the interface-segregated storage traits (design doc Section 2.1), the observability port traits (Section 2.7), and the `ConfigOverlayStore` trait (Section 2.8, v1.1 deferred). These are the contract boundaries — every consumer and implementation codes against these traits.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 2.1 | Define `EventStore` trait | `insert_events(Vec<AuditEvent>)`, `query_events(&EventFilter, limit, offset)`. `Send + Sync + private::Sealed`. `#[allow(async_fn_in_trait)]` on the trait. Add sealed trait module `pub(crate) mod private { pub trait Sealed {} }` at bottom of `traits.rs`. Add compile-time dyn assertion: `const _: Option<&dyn EventStore> = None;`. | `src/traits.rs` |
| 2.2 | Define `RequestMetricsStore` trait | `insert_request_metrics(Vec<RequestMetrics>)`, `query_request_metrics(&MetricsFilter, limit, offset)`, `aggregate_request_metrics(&MetricsFilter, &[GroupByField])`. `Send + Sync + private::Sealed`. `#[allow(async_fn_in_trait)]`. Add dyn assertion. | `src/traits.rs` |
| 2.3 | Define `ProcessStore` trait | `upsert_managed_process(&ManagedProcess)`, `remove_managed_process(ProcessId)`, `list_managed_processes()`. `Send + Sync + private::Sealed`. `#[allow(async_fn_in_trait)]`. Add dyn assertion. | `src/traits.rs` |
| 2.4 | Define `StoreMaintenance` trait | `insert_config_snapshot(&ConfigSnapshot)`, `prune_events_older_than(DateTime<Utc>)` → returns row count, `estimated_size_bytes()`. `Send + Sync + private::Sealed`. `#[allow(async_fn_in_trait)]`. Add dyn assertion. | `src/traits.rs` |
| 2.5 | Define `ConfigOverlayStore` trait | `get_overlay`, `set_overlay`, `remove_overlay`, `list_overlays`. All return `Result<_, StoreError>`. `Send + Sync + private::Sealed`. `#[allow(async_fn_in_trait)]`. Mark with `/// v1.1: trait defined, impl deferred. See design doc Section 2.8.` doc comment. Define `ConfigOverlay` struct with fields: `server_id: ServerId`, `overlay_key: String`, `value: String`, `applied_at: chrono::DateTime<chrono::Utc>`. Derive `Debug, Clone`. | `src/traits.rs` |
| 2.6 | Define `RequestMetricsRecorder` trait | 9 methods per design doc Section 2.7: `record_request`, `observe_request_duration`, `observe_ttft`, `observe_tokens_per_second`, `set_queue_depth`, `observe_queue_wait`, `set_server_slots`, `record_server_restart`, `record_config_reload`. All `&self`, `Send + Sync`. These are **port traits** — backend-agnostic. The `metrics` facade crate (used by Config Manager) is the implementation target, not the trait definition. | `src/metrics_recording.rs` |
| 2.7 | Define `StoreHealthRecorder` trait | 10 methods per design doc Section 2.7: `set_wal_size_bytes`, `set_channel_pending`, `record_events_dropped`, `record_write_result`, `observe_write_duration`, `observe_checkpoint_duration`, `observe_read_duration`, `set_read_pool_active`, `record_prune_rows`, `record_cmd_channel_dropped`. | `src/metrics_recording.rs` |
| 2.8 | Define `AppMetrics` super-trait | `pub trait AppMetrics: RequestMetricsRecorder + StoreHealthRecorder {}`. | `src/metrics_recording.rs` |
| 2.9 | Implement `NoopMetrics` | Test double implementing both recorder traits with empty bodies. `pub(crate)` with `#[cfg_attr(not(test), allow(dead_code))]` — follows `InMemoryConfigSource` pattern from Config Manager. Used in unit tests to avoid metrics dependency. | `src/metrics_recording.rs` |
| 2.10 | Define `WriterCommand` enum | `#[non_exhaustive]`. All 8 variants per design doc Section 3.4: `InsertEvents`, `InsertMetrics`, `UpsertProcess`, `RemoveProcess`, `InsertConfigSnapshot`, `Prune`, `Checkpoint`, `Shutdown`. Each carries `oneshot::Sender` reply and `tracing::Span`. | `src/sqlite/commands.rs` |
| 2.11 | Implement `InMemoryStore` test double | Implements all 4 storage traits (`EventStore`, `RequestMetricsStore`, `ProcessStore`, `StoreMaintenance`) + `private::Sealed`. Uses `parking_lot::Mutex<Vec<T>>` for each table (matches Config Manager's `InMemoryConfigSource` pattern which uses `parking_lot::Mutex`). `pub(crate)` with `#[cfg_attr(not(test), allow(dead_code))]`. Used for unit testing `InstrumentedMetricsStore` (PR7) and by future component tests. Add `#[cfg(test)] pub fn replace_events(&self, ...)` helper method for test manipulation. | `src/sqlite/in_memory.rs` |
| 2.12 | Unit tests: trait static dispatch | Add runtime tests: verify InMemoryStore works for basic insert+query on all 4 traits using static dispatch (`impl EventStore`). | `src/traits.rs` (tests) |

### PR2 Acceptance Criteria

- [x] All 5 storage traits compile
- [x] All storage traits use sealed trait pattern (`private::Sealed`)
- [x] Both recorder traits compile and are object-safe
- [x] `NoopMetrics` implements all recorder trait methods (`pub(crate)`, test-only)
- [x] `WriterCommand` enum has all 8 variants with correct payloads, `#[non_exhaustive]`
- [x] `ConfigOverlayStore` trait is defined (no impl yet)
- [x] All async trait methods use `#[allow(async_fn_in_trait)]` (no `#[async_trait]` macro)
- [x] No `metrics` crate or `prometheus` crate dependency yet — traits are backend-agnostic

### PR2 Creation Steps

```bash
git checkout -b metrics/02-traits main
# ... implement tasks 2.1–2.11 ...
git add -A && git commit -m "metrics: define storage traits, observability port traits, and WriterCommand protocol"
git push -u origin metrics/02-traits
# Open PR: "Metrics & Storage PR2: Storage Traits & Observability Ports"
# Description: "Defines interface-segregated storage traits (Section 2.1): EventStore,
# RequestMetricsStore, ProcessStore, StoreMaintenance. Observability port traits
# (Section 2.7): RequestMetricsRecorder, StoreHealthRecorder, AppMetrics.
# WriterCommand protocol (Section 3.4). ConfigOverlayStore (Section 2.8, deferred).
# NoopMetrics test double. No implementations yet — contracts only."
```

---

## VC-1: Contracts Verification Checkpoint

**Gate:** PR1 + PR2 merged. All types and trait contracts defined.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-1.1 | Trait static dispatch | Tests compile and verify `InMemoryStore` works via static dispatch (`&impl EventStore`). (Relaxed from dyn safety). | Compiles |
| VC-1.2 | Cross-crate types | `llamaforge-metrics` uses `ServerId` from `llamaforge-config` without issues | Compiles and tests pass |
| VC-1.3 | WriterCommand completeness | Every storage trait method has a corresponding `WriterCommand` variant | Manual audit |
| VC-1.4 | Error type hygiene | `StoreError` does not reference `rusqlite`, `anyhow`, or any backend type | Grep for banned deps in `error.rs` |
| VC-1.5 | Convention compliance | All modules `pub(crate)` with explicit `pub use`. All public enums `#[non_exhaustive]`. All storage traits sealed. Async traits use `#[allow(async_fn_in_trait)]`. | Manual audit + `rg "pub mod" src/lib.rs` returns 0 hits |

---

## PR3: SQLite Schema, Migrations & Database Setup

**Branch:** `metrics/03-sqlite-setup`  
**Base:** `metrics/02-traits`  
**Size:** Medium (~300 lines)  
**Estimated Time:** 4–5 hours

### Purpose

Set up SQLite: opening the database, running migrations, configuring WAL mode and PRAGMAs, creating the read pool with `deadpool-sqlite`, and advisory locking to prevent dual-instance corruption. This is the infrastructure foundation — no business logic yet, just database plumbing.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 3.1 | Add SQLite dependencies | Add to workspace deps: `rusqlite = { version = "0.31", features = ["bundled"] }` (Note: 0.31 instead of 0.32 to be compatible with deadpool-sqlite 0.8), `rusqlite_migration = "1"`, `deadpool-sqlite = "0.8"`. Wire into `llamaforge-metrics/Cargo.toml`. `nix` is already a workspace dep (version `0.29`, features `["fs"]`) — used by Config Manager for `flock` in writeback. Reuse for advisory flock in Task 3.4. | `Cargo.toml`, `crates/llamaforge-metrics/Cargo.toml` |
| 3.2 | Implement `open_and_migrate()` | Open SQLite connection at `db_path`. Run PRAGMAs: `journal_mode = WAL`, `synchronous = NORMAL`, `busy_timeout = 5000`, `foreign_keys = ON`. Run schema migrations via `rusqlite_migration`. Return the `Connection`. | `src/sqlite/mod.rs` |
| 3.3 | Define schema migrations | Migration V1: all 5 tables + indexes from design doc Section 2.6 (`audit_events`, `request_metrics`, `managed_processes`, `config_snapshots`, `config_overlays`). Use `rusqlite_migration::Migrations::new()`. | `src/sqlite/migrations.rs` |
| 3.4 | Implement advisory flock on DB | At startup, open `{db_path}.lock` and `flock(LOCK_EX | LOCK_NB)`. If fails: return `StoreError::Unavailable` with message about another instance. Hold the lock file fd for process lifetime (leaked into a `ManuallyDrop`). | `src/sqlite/mod.rs` |
| 3.5 | Implement `create_read_pool()` | `deadpool_sqlite::Config` with `max_size` from `MetricsConfig::read_pool_size` (validate range 1–16), `runtime = tokio`. `post_create` hook: execute `PRAGMA journal_mode;` and **assert** result is `"wal"` (do NOT set — the writer connection already set WAL mode; if it's not WAL, return an error from the hook). Then set `PRAGMA query_only = ON`, `PRAGMA busy_timeout = 5000`, `PRAGMA synchronous = NORMAL`. `pre_recycle` hook: `ROLLBACK` (ignore error if no transaction) + re-assert WAL mode. `max_lifetime = 300s`. | `src/sqlite/pool.rs` |
| 3.6 | Implement row types (DTOs) | `AuditEventRow`, `RequestMetricsRow`, `ManagedProcessRow`, `ConfigSnapshotRow`, `ConfigOverlayRow` — all with bare `String`/`i64`/`f64` fields for SQL compatibility. `impl From<AuditEvent> for AuditEventRow` and vice versa. Same for all 4 domain↔row pairs. | `src/sqlite/rows.rs` |
| 3.7 | Integration tests: migration | (a) Fresh DB → migrate → verify all 5 tables exist via `sqlite_master`. (b) Run migration twice → no error (idempotent). (c) Verify WAL mode set (`PRAGMA journal_mode` returns `wal`). (d) Verify `query_only` is ON for read pool connections. | `tests/sqlite_setup.rs` |
| 3.8 | Integration tests: advisory lock | (a) Open DB with lock. Open second connection to same DB with lock → fails with `Unavailable`. (b) Drop first lock → second succeeds. | `tests/sqlite_setup.rs` |
| 3.9 | Integration tests: row roundtrip | Create `AuditEvent` → convert to `AuditEventRow` → convert back → assert equality. Same for `RequestMetrics`, `ManagedProcess`, `ConfigSnapshot`. | `tests/row_conversion.rs` |

### PR3 Acceptance Criteria

- [x] `open_and_migrate()` creates a WAL-mode SQLite DB with all 5 tables
- [x] Migrations are idempotent (running twice is harmless)
- [x] Read pool connections have `query_only = ON`
- [x] Advisory flock prevents dual-instance
- [x] All row DTO↔domain conversions are lossless
- [x] All integration tests pass using `tempfile::TempDir`
- [x] No `unwrap()` in any `interact()` closure

### PR3 Creation Steps

```bash
git checkout -b metrics/03-sqlite-setup main
# ... implement tasks 3.1–3.9 ...
git add -A && git commit -m "metrics: add SQLite setup, migrations, read pool, and advisory locking"
git push -u origin metrics/03-sqlite-setup
# Open PR: "Metrics & Storage PR3: SQLite Schema, Migrations & Read Pool"
# Description: "Implements database infrastructure (design doc Section 2.6, 3.5).
# Schema with 5 tables + indexes, rusqlite_migration, WAL mode PRAGMAs,
# deadpool-sqlite read pool with query_only + pre_recycle, advisory flock.
# Row DTOs with domain↔row conversions. 9 integration tests."
```

---

## PR4: Writer Thread & Batch Collector

**Branch:** `metrics/04-writer`  
**Base:** `metrics/03-sqlite-setup`  
**Size:** Large (~500 lines)  
**Estimated Time:** 10–14 hours (1.5–2 days — the async/sync boundary, batch collector timer semantics, and degradation state machine are each individually complex)

### Purpose

Implement the core write path: the synchronous writer thread that owns the single write connection (design doc Section 3.3), the async batch collector task that bridges Tokio channels to the writer (Section 3.2), and the `WriterCommand` dispatch loop. This is the most complex PR in the component — it crosses the async/sync boundary.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 4.1 | Implement `writer_thread_main()` | `std::thread` entry point. Receives `Connection` + `std::sync::mpsc::Receiver<WriterCommand>` + relevant `MetricsConfig` values (`writer_failure_threshold`, `wal_checkpoint_interval_secs`). Match on commands: `InsertEvents` → batch INSERT in transaction, `InsertMetrics` → batch INSERT, `UpsertProcess` → INSERT OR REPLACE, `RemoveProcess` → DELETE, `InsertConfigSnapshot` → INSERT, `Prune` → DELETE WHERE created_at < cutoff (from BOTH `audit_events` AND `request_metrics` tables), `Checkpoint` → `PRAGMA wal_checkpoint(mode)`, `Shutdown` → **(a) drain ALL remaining commands from `cmd_rx` and process each, (b) run `PRAGMA wal_checkpoint(TRUNCATE)` for clean WAL state, (c) close connection, (d) break loop.** Each command: enter carried `tracing::Span`, execute, send reply via oneshot. | `src/sqlite/writer.rs` |
| 4.2 | Implement `WriterMode` state tracking | Start `Healthy`. On INSERT/COMMIT failure: increment `consecutive_failures`. On success: reset `consecutive_failures` to 0 — but **do NOT exit Degraded mode** if already degraded. On reaching `writer_failure_threshold` (from `MetricsConfig`, default 10) consecutive failures: transition to `Degraded { since: Instant::now() }`. **This transition is irreversible within a single process lifetime — recovery requires restart** (per design doc Section 3.3). In Degraded mode: reject new write commands with `StoreError::Unavailable`, attempt `PRAGMA integrity_check` every 60s, log results. Track mode with `parking_lot::Mutex<WriterMode>` if shared, or direct field if writer thread is the sole accessor. | `src/sqlite/writer.rs` |
| 4.3 | Implement prepared statement caching | Cache prepared statements for: `INSERT INTO audit_events`, `INSERT INTO request_metrics`, `INSERT OR REPLACE INTO managed_processes`, `DELETE FROM managed_processes WHERE pid = ?`, `INSERT INTO config_snapshots`, `DELETE FROM audit_events WHERE created_at < ?`, `DELETE FROM request_metrics WHERE created_at < ?`. Use `rusqlite::CachedStatement` via `conn.prepare_cached()`. | `src/sqlite/writer.rs` |
| 4.4 | Implement periodic WAL checkpoint | Track `last_checkpoint: Instant` in writer thread. After each command, check if 60s elapsed → `PRAGMA wal_checkpoint(PASSIVE)`. Log result. | `src/sqlite/writer.rs` |
| 4.5 | Implement batch collector task | `async fn batch_collector_main()`. Owns `tokio::sync::mpsc::Receiver` for both channels + `std::sync::mpsc::SyncSender<WriterCommand>`. Interval tick configured via `MetricsConfig::batch_flush_interval_ms` (default 100ms). Batch cap via `MetricsConfig::batch_flush_size` (default 100). **Phase 1:** drain audit channel fully (priority). For each `AuditRecord`, convert to BOTH an `AuditEvent` (via Task 4.7) AND a `RequestMetrics` (via Task 4.8). Collect events into `events_batch: Vec<AuditEvent>` and metrics into `metrics_batch: Vec<RequestMetrics>`. **Phase 2:** drain telemetry up to remaining batch capacity (batch cap minus events already collected). Convert each `TelemetryEvent` to `AuditEvent` (via Task 4.6), append to `events_batch`. **Phase 3:** if `events_batch` non-empty, send `WriterCommand::InsertEvents` via `try_send()`. **Phase 4:** if `metrics_batch` non-empty, send `WriterCommand::InsertMetrics` via `try_send()`. On `try_send` full for either: drop that batch, increment `CMD_CHANNEL_EVENTS_DROPPED` counter by batch size, log sampled warning. On sender disconnected: break loop. | `src/sqlite/batch_collector.rs` |
| 4.6 | Implement `TelemetryEvent` → `AuditEvent` conversion | Map `TelemetryEventKind` to `event_kind` string and JSON payload. `event_type = EventType::Telemetry`. | `src/sqlite/batch_collector.rs` |
| 4.7 | Implement `AuditRecord` → `AuditEvent` conversion | Map fields to `event_kind = "request_completed"` (or status-based), JSON payload with all billing fields. `event_type = EventType::Audit`. | `src/sqlite/batch_collector.rs` |
| 4.8 | Implement `AuditRecord` → `RequestMetrics` conversion | Separate conversion: `AuditRecord` produces both an `AuditEvent` (for the event log) and a `RequestMetrics` (for the metrics table). Batch collector sends `InsertEvents` for the AuditEvent batch and `InsertMetrics` for the RequestMetrics batch. | `src/sqlite/batch_collector.rs` |
| 4.9 | Implement test helper: `WriterTestHandle` | A test-only struct that owns `SyncSender<WriterCommand>` + `JoinHandle<()>`. `impl Drop`: sends `WriterCommand::Shutdown` and `join()`s the thread. This prevents zombie writer threads in tests. All PR4 tests use this handle. | `src/sqlite/writer.rs` (tests) |
| 4.10 | Unit tests: writer command dispatch | Using a real tempdir SQLite DB + `WriterTestHandle`: (a) Send `InsertEvents` with 10 events → verify 10 rows in `audit_events`. (b) Send `InsertMetrics` with 5 records → verify 5 rows in `request_metrics`. (c) Send `UpsertProcess` → verify row in `managed_processes`. (d) Send `RemoveProcess` → verify row deleted. (e) Send `InsertConfigSnapshot` → verify row in `config_snapshots`. (f) Send `Prune` with old cutoff → verify old rows deleted, new rows kept. (g) Send `Checkpoint(Passive)` → verify no error. | `tests/writer_thread.rs` |
| 4.11 | Unit tests: writer degradation | (a) Simulate `writer_failure_threshold` consecutive failures (use a corrupt DB or drop tables mid-flight) → verify `WriterMode::Degraded`. (b) In degraded mode: send a command → verify `StoreError::Unavailable` reply. (c) Verify degradation is irreversible: insert succeeds on a fresh table but mode remains `Degraded`. | `tests/writer_thread.rs` |
| 4.12 | Unit tests: batch collector draining | Using `tokio::sync::mpsc::channel` in test: (a) Send 50 audit + 80 telemetry events. Verify batch collector drains audit first (all 50), then telemetry up to batch limit (50 more). (b) Verify batch size capped at `batch_flush_size`. (c) Verify empty channels produce no command. (d) Verify timer fires at configured interval. (e) **Verify AuditRecords produce BOTH InsertEvents AND InsertMetrics commands** — assert both commands are sent per tick when audit records are present. | `tests/batch_collector.rs` |
| 4.13 | Unit tests: command channel full | Fill the `sync_channel(64)` with pending commands → batch collector `try_send` fails → verify batch dropped and counter incremented. | `tests/batch_collector.rs` |

### PR4 Acceptance Criteria

- [x] Writer thread processes all 8 `WriterCommand` variants correctly
- [x] Batch INSERT uses a transaction (BEGIN/INSERT.../COMMIT) for atomicity
- [x] Prepared statements are cached (not re-prepared per batch)
- [x] WAL checkpoint fires every 60s during active writing
- [x] Writer enters `Degraded` mode after 10 consecutive failures
- [x] Batch collector drains audit before telemetry (priority ordering)
- [x] Batch size capped at 100 events
- [x] Command channel backpressure: batch dropped (not queued unboundedly)
- [x] Tracing spans are entered from carried `Span` on each command
- [x] `WriterTestHandle` sends Shutdown and joins thread on Drop (no zombie threads in tests)
- [x] All 13 tests pass

### PR4 Creation Steps

```bash
git checkout -b metrics/04-writer main
# ... implement tasks 4.1–4.12 ...
git add -A && git commit -m "metrics: implement writer thread, batch collector, and command dispatch"
git push -u origin metrics/04-writer
# Open PR: "Metrics & Storage PR4: Writer Thread & Batch Collector"
# Description: "Implements the write path (design doc Sections 3.2, 3.3, 3.4).
# Synchronous writer thread with WriterCommand dispatch, prepared statement caching,
# periodic WAL checkpoint, and degradation tracking. Async batch collector task with
# audit-priority draining, 100ms tick, 100-event batch cap, and bounded command channel.
# TelemetryEvent/AuditRecord → AuditEvent/RequestMetrics conversions. 12 tests."
```

---

## PR5: Read Path (SqliteMetricsStore Query Implementation)

**Branch:** `metrics/05-read-path`  
**Base:** `metrics/04-writer`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 5–6 hours  
**Concurrency Note:** Query builder code and read pool logic can be developed in parallel with PR4, but integration tests (Tasks 5.11–5.14) require PR4 merged because they need a functioning writer thread to INSERT data before querying. If developing concurrently, branch from `metrics/03-sqlite-setup`, add unit tests for query builders, and defer integration tests until PR4 lands.

### Purpose

Implement the read side of `SqliteMetricsStore`: all query methods from the storage traits using the `deadpool-sqlite` read pool. This includes filter-to-SQL translation, aggregate queries with GROUP BY, and statement timeout enforcement.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 5.1 | Implement `SqliteMetricsStore` struct | Fields: `cmd_tx: std::sync::mpsc::SyncSender<WriterCommand>`, `read_pool: deadpool_sqlite::Pool`, `writer_join_handle: Option<std::thread::JoinHandle<()>>`. Constructor is private — created only via `create_metrics_pipeline()`. | `src/sqlite/store.rs` |
| 5.2 | Implement `EventStore` for `SqliteMetricsStore` | `insert_events`: send `WriterCommand::InsertEvents` via `cmd_tx`, await oneshot reply. `query_events`: `pool.get().await` → `interact()` → build SQL WHERE from `EventFilter`, execute, map rows to `AuditEvent` via row DTOs. | `src/sqlite/store.rs` |
| 5.3 | Implement `RequestMetricsStore` for `SqliteMetricsStore` | `insert_request_metrics`: send `WriterCommand::InsertMetrics`. `query_request_metrics`: filter → SQL. `aggregate_request_metrics`: build GROUP BY from `GroupByField` enum, compute COUNT, SUM, AVG. For p50/p99: use `NTILE` window function or collect and compute in Rust (SQLite lacks native percentiles). | `src/sqlite/store.rs` |
| 5.4 | Implement `ProcessStore` for `SqliteMetricsStore` | `upsert_managed_process`: send `WriterCommand::UpsertProcess`. `remove_managed_process`: send `WriterCommand::RemoveProcess`. `list_managed_processes`: read pool query. | `src/sqlite/store.rs` |
| 5.5 | Implement `StoreMaintenance` for `SqliteMetricsStore` | `insert_config_snapshot`: send `WriterCommand::InsertConfigSnapshot`. `prune_events_older_than`: send `WriterCommand::Prune`. `estimated_size_bytes`: read pool → `PRAGMA page_count * PRAGMA page_size`. | `src/sqlite/store.rs` |
| 5.6 | Implement `wal_checkpoint()` inherent method | NOT a trait method — SQLite-specific. Sends `WriterCommand::Checkpoint`, returns `CheckpointResult`. | `src/sqlite/store.rs` |
| 5.7 | Implement filter → SQL builder | `fn build_event_where(filter: &EventFilter) -> (String, Vec<Box<dyn ToSql>>)`. Dynamic WHERE clause construction. Parameterized (no SQL injection). Same for `build_metrics_where(filter: &MetricsFilter)`. | `src/sqlite/query_builder.rs` |
| 5.8 | Implement GROUP BY SQL builder | `fn build_group_by(fields: &[GroupByField]) -> String`. Maps `GroupByField::Hour` → `strftime('%Y-%m-%d %H:00:00', created_at)`, `GroupByField::ServerId` → `server_id`, etc. | `src/sqlite/query_builder.rs` |
| 5.9 | Implement percentile computation | Since SQLite lacks native percentile functions: fetch sorted `total_duration_ms` values in aggregate queries, compute p50/p99 in Rust. Limit to 10,000 rows for percentile computation (return `None` if more). | `src/sqlite/store.rs` |
| 5.10 | Implement statement timeout | Wrap read pool `interact()` calls with `tokio::time::timeout(Duration::from_secs(30), ...)`. On timeout: return `StoreError::Timeout`. The underlying SQLite query continues on the blocking thread but result is dropped. | `src/sqlite/store.rs` |
| 5.11 | Integration tests: event queries | Insert 100 events via writer → query with various filters: (a) by `server_id`, (b) by `event_type`, (c) by time range `since/until`, (d) by `request_id`, (e) with `limit` and `offset` pagination, (f) empty filter returns all. | `tests/event_queries.rs` |
| 5.12 | Integration tests: metrics queries | Insert 50 request metrics → query: (a) by `requestor`, (b) by `model_name`, (c) by `status`, (d) aggregate by `ServerId` → verify COUNT and AVG, (e) aggregate by `Hour` → verify time bucketing. | `tests/metrics_queries.rs` |
| 5.13 | Integration tests: process store | (a) Upsert 3 processes → list → verify 3 returned. (b) Remove 1 → list → verify 2. (c) Upsert same PID → verify update (not duplicate). | `tests/process_store.rs` |
| 5.14 | Integration tests: estimated size | Insert data → `estimated_size_bytes()` → verify > 0 and reasonable (not wildly off from actual file size). | `tests/maintenance.rs` |
| 5.15 | Unit tests: query builder | (a) Empty filter → no WHERE clause. (b) `server_id` filter → `WHERE server_id = ?`. (c) Time range → `WHERE created_at >= ? AND created_at <= ?`. (d) Multiple filters → AND combination. (e) GROUP BY `[ServerId, Hour]` → `GROUP BY server_id, strftime(...)`. | `tests/query_builder.rs` |
| 5.16 | Implement VC-2 integration tests | All 8 checks from the VC-2 checkpoint below. Each as a separate `#[tokio::test]`. These test the composition of the write path (PR4) and read path (PR5) together using real SQLite. | `tests/vc2_write_read_pipeline.rs` |

### PR5 Acceptance Criteria

- [ ] `SqliteMetricsStore` implements all 4 storage traits
- [ ] Write methods send commands to writer thread, await reply
- [ ] Read methods use read pool with `query_only = ON`
- [ ] Filter-to-SQL produces parameterized queries (no SQL injection)
- [ ] Aggregate queries produce correct COUNT, SUM, AVG, p50, p99
- [ ] Statement timeout of 30s is enforced on read queries
- [ ] `wal_checkpoint()` is an inherent method (not on any trait)
- [ ] All 16 integration + unit tests pass (including VC-2 tests)
- [ ] No `unwrap()` in any `interact()` closure

### PR5 Creation Steps

```bash
git checkout -b metrics/05-read-path main
# ... implement tasks 5.1–5.15 ...
git add -A && git commit -m "metrics: implement SqliteMetricsStore with query builders and read pool"
git push -u origin metrics/05-read-path
# Open PR: "Metrics & Storage PR5: Read Path & SqliteMetricsStore"
# Description: "Implements SqliteMetricsStore (design doc Sections 2.1, 3.5).
# All 4 storage trait implementations: write via WriterCommand, read via deadpool
# read pool. Dynamic filter→SQL query builder, GROUP BY builder, percentile
# computation in Rust. 30s statement timeout on reads. 15 tests."
```

---

## VC-2: Write + Read Path Verification Checkpoint

**Gate:** PR3 + PR4 + PR5 merged. Full write and read pipeline works.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-2.1 | Write → read roundtrip | Insert 100 events via writer → query all → verify 100 returned with correct fields | All assertions pass |
| VC-2.2 | Batch collector → writer → read | Send events through telemetry/audit channels → batch collector batches → writer inserts → read queries return them | Events visible in DB |
| VC-2.3 | Audit priority | Send 50 audit + 200 telemetry events simultaneously → verify first batch contains all 50 audit events | Audit events processed first |
| VC-2.4 | Filter accuracy | Insert events for 3 servers → filter by `server_id` → only matching events returned | Correct filtering |
| VC-2.5 | Aggregate accuracy | Insert 100 metrics → aggregate by `ServerId` → verify SUM(tokens) matches manual sum | Math checks out |
| VC-2.6 | Prune correctness | Insert old + new events → prune with cutoff → old gone, new preserved | Retention works |
| VC-2.7 | WAL checkpoint | Insert 1000 events → trigger PASSIVE checkpoint → verify `pages_checkpointed > 0` | Checkpoint runs |
| VC-2.8 | Concurrent read + write | Continuous writes + continuous reads for 5s → no `SQLITE_BUSY`, reads < 30s | WAL isolation works |

**Integration test file:** `tests/vc2_write_read_pipeline.rs`

---

## PR6: MetricsIngest & Pipeline Assembly

**Branch:** `metrics/06-ingest-pipeline`  
**Base:** `metrics/05-read-path`  
**Size:** Medium (~250 lines)  
**Estimated Time:** 4–5 hours

### Purpose

Implement `MetricsIngest` (the producer-facing API, design doc Section 2.2) and `create_metrics_pipeline()` (the composition root, Section 3.5). After this PR, the entire pipeline can be constructed and used end-to-end.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 6.1 | Implement `MetricsIngest` | `Clone`-able struct with `telemetry_tx`, `audit_tx`, `audit_semaphore`. `send_telemetry()`: `try_send`, on `Full` → increment `TELEMETRY_EVENTS_DROPPED` static `AtomicU64` via `fetch_add(1, Ordering::Relaxed)`, use return value for sampled warn log (`if count % 100 == 1 { tracing::warn!(...) }`). `send_audit()`: `try_acquire` semaphore → `timeout(10s, audit_tx.send())` → on timeout/semaphore exhaustion → increment `AUDIT_EVENTS_DROPPED` AtomicU64. | `src/ingest.rs` |
| 6.2 | Define static drop counters | `static TELEMETRY_EVENTS_DROPPED: AtomicU64 = AtomicU64::new(0)` and `static AUDIT_EVENTS_DROPPED: AtomicU64 = AtomicU64::new(0)`. Also `static CMD_CHANNEL_EVENTS_DROPPED: AtomicU64 = AtomicU64::new(0)` (used by batch collector in PR4). **Use `AtomicU64` (not Prometheus counters)** to avoid leaking `prometheus` dependency into `MetricsIngest`. Expose via `pub fn telemetry_drops() -> u64`, `pub fn audit_drops() -> u64`, `pub fn cmd_channel_drops() -> u64` for the Prometheus adapter (PR7) to read and export. | `src/ingest.rs` |
| 6.3 | Implement `create_metrics_pipeline()` | Per design doc Section 3.5. **Parameters: `config: &MetricsConfig`** (NOT individual args — thread the full config struct). Returns `Result<MetricsPipeline, StoreError>`. Steps: (1) open_and_migrate at `config.db_path`, (2) create channels with capacities from config: `tokio::sync::mpsc::channel(config.telemetry_channel_capacity)`, `tokio::sync::mpsc::channel(config.audit_channel_capacity)`, `std::sync::mpsc::sync_channel(64)` for cmd, (3) spawn writer thread with `config.writer_failure_threshold` and `config.wal_checkpoint_interval_secs`, (4) spawn batch collector with `config.batch_flush_interval_ms` and `config.batch_flush_size`, (5) create read pool with `config.read_pool_size` and `config.read_statement_timeout_secs`, (6) create `audit_semaphore` with `config.audit_semaphore_permits`, (7) assemble `MetricsPipeline { ingest, store, batch_collector_handle }`. **No magic numbers — all from MetricsConfig.** | `src/sqlite/pipeline.rs` |
| 6.4 | Implement `MetricsPipeline` struct | `pub ingest: MetricsIngest`, `pub store: SqliteMetricsStore`, `pub batch_collector_handle: JoinHandle<()>`. | `src/sqlite/pipeline.rs` |
| 6.5 | Implement graceful shutdown on Drop | `impl Drop for SqliteMetricsStore`: (1) send `WriterCommand::Shutdown`, (2) `join_handle.take().map(|h| h.join())`. Log if join fails. The writer thread's Shutdown handler (PR4 Task 4.1) must: (a) drain ALL remaining commands from `cmd_rx` and process each, (b) run `PRAGMA wal_checkpoint(TRUNCATE)` for clean WAL state, (c) close connection, (d) break loop. **Shutdown ordering constraint** (per design doc Cross-Cutting Concerns): writer thread shutdown MUST happen after all detached audit `tokio::spawn` tasks have completed or timed out. The caller (composition root) must `audit_semaphore.close()` and wait for all permits to be returned before dropping `SqliteMetricsStore`. Document this in a `/// # Shutdown Ordering` doc comment on `MetricsPipeline`. | `src/sqlite/store.rs` |
| 6.6 | Integration tests: full pipeline | (a) `create_metrics_pipeline()` → send telemetry event via `ingest.send_telemetry()` → wait 200ms (batch collector tick) → query events → verify event stored. (b) Same with audit record. (c) Send audit when channel is empty → returns Ok immediately. | `tests/pipeline_integration.rs` |
| 6.7 | Integration tests: telemetry drop | Fill telemetry channel to 10,000 → next `send_telemetry()` returns `false` → verify `telemetry_drops()` incremented. | `tests/pipeline_integration.rs` |
| 6.8 | Integration tests: audit timeout | Block writer thread (send a slow command) → fill audit channel → `send_audit()` times out after 10s → verify `AuditSendError::Timeout`. | `tests/pipeline_integration.rs` |
| 6.9 | Integration tests: audit semaphore exhaustion | Acquire all 5,000 semaphore permits → `send_audit()` returns `SemaphoreExhausted` immediately. | `tests/pipeline_integration.rs` |
| 6.10 | Integration tests: graceful shutdown | Create pipeline → send some events → drop `SqliteMetricsStore` → verify writer thread joins cleanly, data is committed. | `tests/pipeline_integration.rs` |

### PR6 Acceptance Criteria

- [ ] `MetricsIngest` is `Clone` (cheap — Arc-backed)
- [ ] `send_telemetry()` never blocks, never panics
- [ ] `send_audit()` respects 10s timeout and semaphore bound
- [ ] `create_metrics_pipeline()` assembles the full pipeline in one call
- [ ] Drop on `SqliteMetricsStore` sends `Shutdown` and joins writer thread
- [ ] All 5 integration test scenarios pass
- [ ] Static drop counters are AtomicU64 (no Prometheus leak into ingest)

### PR6 Creation Steps

```bash
git checkout -b metrics/06-ingest-pipeline main
# ... implement tasks 6.1–6.10 ...
git add -A && git commit -m "metrics: implement MetricsIngest and create_metrics_pipeline assembly"
git push -u origin metrics/06-ingest-pipeline
# Open PR: "Metrics & Storage PR6: MetricsIngest & Pipeline Assembly"
# Description: "Implements producer API (design doc Section 2.2) and pipeline factory
# (Section 3.5). MetricsIngest with lossy telemetry send and backpressured audit send.
# create_metrics_pipeline() wires channels, writer thread, batch collector, and read pool.
# Graceful shutdown via Drop. 5 integration test scenarios."
```

---

## VC-3: End-to-End Pipeline Verification Checkpoint

**Gate:** PR1–PR6 merged. Complete pipeline from producer to storage to query.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-3.1 | Producer → storage → query | `send_telemetry()` → wait 200ms → `query_events(matching filter)` → event found | Event persisted |
| VC-3.2 | Audit → both tables | `send_audit()` → wait 200ms → event in `audit_events` AND record in `request_metrics` | Both tables populated |
| VC-3.3 | Backpressure under load | Send 20,000 telemetry events in 1s → verify: (a) channel full events dropped, (b) batch collector batches correctly, (c) no OOM, (d) some events make it to DB | Graceful degradation |
| VC-3.4 | Shutdown data safety | Send 100 events → immediately shutdown → restart with same DB → query returns all 100 | No data loss on graceful shutdown |
| VC-3.5 | Dual-instance prevention | Start pipeline on DB → start second pipeline on same DB → second fails with `Unavailable` | Advisory lock works |
| VC-3.6 | Process tracking roundtrip | `upsert_managed_process` → `list_managed_processes` → `remove_managed_process` → `list` returns empty | Full CRUD works |

**Integration test file:** `tests/vc3_e2e_pipeline.rs`

---

## PR7: MetricsFacadeRecorder & InstrumentedMetricsStore

**Branch:** `metrics/07-observability`  
**Base:** `metrics/06-ingest-pipeline`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 4–6 hours

### Purpose

Implement the `MetricsFacadeRecorder` adapter (design doc Section 2.7) using the `metrics` facade crate (matching Config Manager's `InstrumentedConfigHandle` approach), and the `InstrumentedMetricsStore` decorator that adds tracing and metrics recording to storage operations. After this PR, all observability is wired up.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 7.1 | Verify `metrics` crate wired | `metrics = "0.22"` is already a workspace dependency (used by Config Manager's `InstrumentedConfigHandle`). Verify it's wired into `llamaforge-metrics/Cargo.toml`. Add `metrics-exporter-prometheus = "0.15"` to workspace deps for the exposition endpoint (used by Transport Layer, not by this crate directly — but needed for VC-4 tests). | `Cargo.toml`, `crates/llamaforge-metrics/Cargo.toml` |
| 7.2 | Implement `MetricsFacadeRecorder` | Uses `metrics` crate macros (`counter!`, `histogram!`, `gauge!`, `describe_counter!`, `describe_histogram!`, `describe_gauge!`) — the same pattern as Config Manager's `InstrumentedConfigHandle`. On construction, call `describe_*!` for all metrics: `llamaforge_requests_total`, `llamaforge_request_duration_seconds`, `llamaforge_ttft_seconds`, `llamaforge_tokens_per_second`, `llamaforge_queue_depth`, `llamaforge_queue_wait_seconds`, `llamaforge_server_slots_active`, `llamaforge_server_slots_total`, `llamaforge_server_restarts_total`, `llamaforge_sqlite_wal_size_bytes`, `llamaforge_channel_pending`, `llamaforge_events_dropped_total`, `llamaforge_cmd_channel_events_dropped_total`, `llamaforge_sqlite_writes_total`, `llamaforge_sqlite_write_duration_seconds`, `llamaforge_sqlite_checkpoint_duration_seconds`, `llamaforge_sqlite_read_duration_seconds`, `llamaforge_sqlite_read_pool_active`, `llamaforge_prune_rows_deleted`, `llamaforge_config_reloads_total`. The `events_dropped_total` and `cmd_channel_events_dropped_total` metrics are exported by reading the static `AtomicU64` counters from `ingest.rs` (via `telemetry_drops()`, `audit_drops()`, `cmd_channel_drops()`). **Note:** the `metrics` facade is backend-agnostic — the actual Prometheus exporter is wired in `main.rs` by the Transport Layer component. | `src/metrics_recording.rs` |
| 7.3 | No histogram bucket configuration needed | The `metrics` facade crate delegates bucket configuration to the exporter (e.g., `metrics-exporter-prometheus`). Document recommended buckets as constants or a `pub fn install_metrics_recorder()` helper that configures `PrometheusBuilder` with LLM-tuned buckets: `request_duration_seconds` exponential 0.1–300 (factor 2), `ttft_seconds` exponential 0.01–60 (factor 2), `tokens_per_second` linear [1, 5, 10, 20, 50, 100, 200], `write_duration_seconds` exponential 0.001–5, `checkpoint_duration_seconds` exponential 0.01–30, `read_duration_seconds` exponential 0.001–30. This helper will be called by `main.rs`. | `src/metrics_recording.rs` |
| 7.4 | Implement `RequestMetricsRecorder` for `MetricsFacadeRecorder` | Each method: call `counter!("llamaforge_...", "label" => value).increment(1)` or `histogram!("llamaforge_...").record(duration)` or `gauge!("llamaforge_...").set(value)`. Matches the exact style of `InstrumentedConfigHandle` in Config Manager. | `src/metrics_recording.rs` |
| 7.5 | Implement `StoreHealthRecorder` for `MetricsFacadeRecorder` | Each method: call appropriate `metrics` macro. | `src/metrics_recording.rs` |
| 7.6 | Implement `AppMetrics` blanket | `impl AppMetrics for MetricsFacadeRecorder {}`. | `src/metrics_recording.rs` |
| 7.7 | Expose `install_metrics_recorder()` helper | `pub fn install_metrics_recorder() -> Result<metrics_exporter_prometheus::PrometheusHandle, ...>`. Installs the global `metrics` recorder with LLM-tuned histogram buckets. Returns the `PrometheusHandle` which the Transport Layer uses for `/metrics` text exposition. This function is called once in `main.rs`. | `src/metrics_recording.rs` |
| 7.8 | Implement `InstrumentedMetricsStore` | Wraps `SqliteMetricsStore` directly (not generic — follows Config Manager's `InstrumentedConfigHandle` which wraps `ConfigHandle` directly, not a generic `T: ConfigReader`). Implements all 4 storage traits + `private::Sealed`. Hot path methods (reads): minimal overhead — just delegate. Write/mutation methods: `#[instrument]` with `tracing::field::Empty` for late-recorded fields, record `histogram!` duration and `counter!` result labels. Match the exact style of `InstrumentedConfigHandle::reload()` and `::writeback()`. | `src/instrumented.rs` |
| 7.9 | Implement liveness monitor task | `tokio::spawn` task that: (a) polls `writer_join_handle.is_finished()` every 2s — if finished: `tracing::error!("metrics.writer_panicked")`, set degraded flag; (b) polls channel capacities every 5s → `set_channel_pending()`; (c) polls WAL file size every 60s → `set_wal_size_bytes()` — **if WAL size exceeds `MetricsConfig::wal_oversize_threshold_mb`, emit `tracing::warn!("metrics.wal_oversize", metrics.wal_size_bytes = size)` and set degraded health flag** (per design doc Section 7.3/7.4). | `src/sqlite/liveness.rs` |
| 7.10 | Implement periodic prune task | `tokio::spawn` task that runs once daily (or on a configurable interval). Each tick: reads `retention_days` from `ConfigReader::snapshot()` (hot-reloadable per design doc Section 6), computes `cutoff = Utc::now() - Duration::days(retention_days)`, sends `WriterCommand::Prune { cutoff }` via the store. Logs `metrics.prune_completed` with rows deleted. Also prunes `request_metrics` table with the same cutoff. **This is the integration point with Config Manager's hot-reloadable `retention_days`.** | `src/sqlite/prune.rs` |
| 7.11 | Unit tests: MetricsFacadeRecorder | (a) Install a `metrics-util::debugging::DebuggingRecorder` in tests. (b) Create `MetricsFacadeRecorder` → call `record_request()` → snapshot → verify counter key exists and value incremented. (c) `observe_ttft()` → snapshot → verify histogram key has observation. (d) `set_queue_depth()` → snapshot → verify gauge value. **Note:** Use `metrics-util` dev-dep for test snapshots, NOT the global recorder (which can only be set once per process). | `tests/metrics_recording_tests.rs` |
| 7.12 | Unit tests: InstrumentedMetricsStore | Using `InMemoryStore` (from PR2 Task 2.11) as the inner store: (a) `insert_events()` → verify inner called and events stored. (b) `query_events()` → verify inner called and results returned. Use `tracing_subscriber` with `TestWriter` or a capture layer to verify spans emitted (follow the same tracing test pattern from Config Manager's `InstrumentedConfigHandle` tests). | `tests/instrumented_store.rs` |
| 7.13 | Unit tests: liveness monitor | Mock a pipeline where writer thread panics → verify monitor detects within 5s (2 poll intervals + margin). | `tests/liveness_monitor.rs` |

### PR7 Acceptance Criteria

- [ ] `MetricsFacadeRecorder` uses `metrics` crate macros (`counter!`, `histogram!`, `gauge!`) — NOT `prometheus` crate directly
- [ ] All metrics prefixed with `llamaforge_` (matching Config Manager's `llamaforge_config_*` prefix)
- [ ] All label cardinality is bounded (no dynamic labels)
- [ ] `install_metrics_recorder()` configures LLM-tuned histogram buckets via `metrics-exporter-prometheus`
- [ ] `InstrumentedMetricsStore` wraps `SqliteMetricsStore` directly (not generic), follows `InstrumentedConfigHandle` pattern
- [ ] Liveness monitor detects writer thread death within 5s
- [ ] Channel depth and WAL size gauges are polled periodically
- [ ] All unit tests pass

### PR7 Creation Steps

```bash
git checkout -b metrics/07-observability main
# ... implement tasks 7.1–7.13 ...
git add -A && git commit -m "metrics: add MetricsFacadeRecorder, InstrumentedMetricsStore, and liveness monitor"
git push -u origin metrics/07-observability
# Open PR: "Metrics & Storage PR7: Observability — metrics crate facade, Instrumented Decorator & Liveness"
# Description: "Implements MetricsFacadeRecorder (design doc Section 2.7) using the `metrics`
# facade crate (same pattern as Config Manager's InstrumentedConfigHandle). All metric
# descriptors with `describe_*!` macros. LLM-tuned histogram bucket configuration via
# install_metrics_recorder() helper for metrics-exporter-prometheus. InstrumentedMetricsStore
# decorator wraps SqliteMetricsStore with tracing + metrics on write paths (zero overhead
# on reads). Liveness monitor polls writer thread health, channel depths, and WAL size. 13 tests."
```

---

## PR8: Property-Based Tests, VC Integration Tests & Public API

**Branch:** `metrics/08-verification`  
**Base:** `metrics/07-observability`  
**Size:** Medium (~350 lines of tests + lib.rs re-exports)  
**Estimated Time:** 5–7 hours

### Purpose

Final PR: property-based tests (design doc Section 8.3), all VC-3 and VC-4 integration tests, design doc Section 8.4 specific test cases, and clean public API re-exports. Pure test code + API polish.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 8.1 | Property test: insert → query roundtrip | `proptest! { fn roundtrip(event in arb_audit_event()) { insert → query(matching_filter) → assert event returned } }`. Implement `arb_audit_event()` strategy. | `tests/proptest_metrics.rs` |
| 8.2 | Property test: retention correctness | `proptest! { fn retention(events in vec(arb_audit_event(), 1..50), cutoff in arb_datetime()) { insert all → prune(cutoff) → query → only events newer than cutoff remain } }`. | `tests/proptest_metrics.rs` |
| 8.3 | Property test: ordering | `proptest! { fn ordering(events in vec(arb_audit_event(), 1..50)) { insert (shuffled) → query → returned in created_at order } }`. | `tests/proptest_metrics.rs` |
| 8.4 | Property test: concurrent reads don't block writes | Under sustained write load (100 inserts/sec), concurrent reads complete within 30s. | `tests/proptest_metrics.rs` |
| 8.5 | Implement VC-3 integration tests | All 6 checks from VC-3 above, each as `#[tokio::test]`. | `tests/vc3_e2e_pipeline.rs` |
| 8.6 | Implement design doc Section 8.4 tests | All 9 specific test cases: `test_telemetry_channel_full_drops_event`, `test_audit_send_timeout`, `test_writer_thread_panic_detected`, `test_wal_checkpoint_passive`, `test_pre_recycle_rollback`, `test_concurrent_read_during_write`, `test_retention_prune`, `test_read_pool_statement_timeout`, `test_schema_migration_idempotent`. | `tests/design_doc_tests.rs` |
| 8.7 | Finalize public API re-exports in lib.rs | All modules remain `pub(crate) mod`. Add explicit `pub use` re-exports following the exact pattern from `llamaforge-config/src/lib.rs` — list every public type individually (NO glob `pub use types::*`). Key exports: `pub use types::{TelemetryEvent, AuditRecord, AuditEvent, RequestMetrics, ManagedProcess, ProcessId, RequestId, ...};`, `pub use error::{StoreError, AuditSendError};`, `pub use filter::{EventFilter, MetricsFilter, GroupByField, AggregatedMetrics};`, `pub use traits::{EventStore, RequestMetricsStore, ProcessStore, StoreMaintenance};`, `pub use ingest::MetricsIngest;`, `pub use sqlite::pipeline::{create_metrics_pipeline, MetricsPipeline};`, `pub use sqlite::store::SqliteMetricsStore;`, `pub use metrics_recording::{MetricsFacadeRecorder, RequestMetricsRecorder, StoreHealthRecorder, AppMetrics, install_metrics_recorder};`, `pub use instrumented::InstrumentedMetricsStore;`. | `src/lib.rs` |
| 8.8 | Add `#[ignore]` slow tests | Tag property-based tests and concurrency stress tests with `#[ignore]`. | — |
| 8.9 | Final CI check | `cargo test --workspace`, `cargo test --workspace -- --ignored`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`, `cargo doc --no-deps -p llamaforge-metrics`. | — |

### PR8 Acceptance Criteria

- [ ] All 4 property-based tests pass (256 cases each)
- [ ] All 6 VC-3 integration tests pass
- [ ] All 9 design doc Section 8.4 test cases pass
- [ ] Public API is cleanly re-exported from `lib.rs`
- [ ] `cargo doc --no-deps -p llamaforge-metrics` builds without warnings
- [ ] No production code changes — test code and re-exports only
- [ ] `cargo test --workspace -- --ignored` passes

### PR8 Creation Steps

```bash
git checkout -b metrics/08-verification main
# ... implement tasks 8.1–8.9 ...
git add -A && git commit -m "metrics: add property tests, VC integration tests, design doc tests, and public API"
git push -u origin metrics/08-verification
# Open PR: "Metrics & Storage PR8: Property Tests, Full Verification & Public API"
# Description: "Pure test + API PR. 4 proptest property-based tests (Section 8.3),
# 6 VC-3 e2e integration tests, 9 design doc Section 8.4 test cases. Clean public
# API re-exports from lib.rs. Total: 19 new test functions. No production code changes."
```

---

## VC-4: Full Component Verification Checkpoint

**Gate:** All 8 PRs merged. Metrics & Storage is feature-complete.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-4.1 | Full lifecycle | `create_metrics_pipeline()` → send telemetry+audit → batch collector batches → writer inserts → query returns events → prune old → checkpoint WAL → shutdown cleanly | All steps succeed |
| VC-4.2 | Startup failure: unwritable DB path | `/nonexistent/path/db.sqlite` → `StoreError::Internal` with clear message | Error is descriptive |
| VC-4.3 | Startup failure: locked DB | Start pipeline → start second pipeline on same DB → `StoreError::Unavailable` | Advisory lock blocks |
| VC-4.4 | Writer degradation | Trigger 10 consecutive write failures → verify `WriterMode::Degraded` → verify `/health` reports degraded | Degradation surfaces |
| VC-4.5 | Telemetry backpressure | 50,000 events in 1 second → verify: channel drops events, counter increments, system stays alive, some events persist | Graceful degradation |
| VC-4.6 | Audit backpressure | Block writer → fill audit channel → `send_audit` times out at 10s → semaphore cap at 5,000 | Bounded memory usage |
| VC-4.7 | Reader isolation | Long-running read (30s query) → concurrent writes succeed → query eventually times out | WAL isolation holds |
| VC-4.8 | Retention enforcement | Insert 100 events with timestamps 60 days old → prune with 30-day retention → all 100 deleted | Prune works |
| VC-4.9 | Metrics endpoint | Call `install_metrics_recorder()` → create `MetricsFacadeRecorder` → record some operations → render from `PrometheusHandle` → verify text format output contains expected metric names | Valid Prometheus text |
| VC-4.10 | Memory bounds | Under sustained load (10K events/sec for 10s) → measure RSS → verify < 50MB above baseline | No memory leak |
| VC-4.11 | Observability decorator | Query via `InstrumentedMetricsStore` → capture tracing output → verify spans present | Spans emitted |
| VC-4.12 | Config integration | Read `metrics_db_path` and `retention_days` from `AppConfig` (via `ConfigReader`) → pipeline uses them | Config drives behavior |

**Integration test file:** `tests/vc4_full_component.rs`

---

## VC-5: Final Sign-Off Checkpoint

**Gate:** All VC-4 checks pass. Component ready for integration.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-5.1 | All tests pass | `cargo test --workspace && cargo test --workspace -- --ignored` | Zero failures |
| VC-5.2 | Zero clippy warnings | `cargo clippy --workspace -- -D warnings` | Exit code 0 |
| VC-5.3 | Code coverage | `cargo llvm-cov --workspace` → `llamaforge-metrics` > 80% line coverage | Coverage report |
| VC-5.4 | Public API audit | Review `lib.rs`. No internal types leak. `SqliteMetricsStore` only exposed for composition root. | Manual review |
| VC-5.5 | Design doc compliance | Cross-check every Section in `02_METRICS_AND_STORAGE.md` against implementation | Checklist complete |
| VC-5.6 | Documentation | `cargo doc --no-deps -p llamaforge-metrics` builds clean | No doc warnings |
| VC-5.7 | Dependency audit | `cargo deny check` passes. No duplicate versions. | Clean |
| VC-5.8 | No `unwrap()` audit | `rg 'unwrap\(\)' crates/llamaforge-metrics/src/` returns zero hits in production code | Zero matches |
| VC-5.9 | No `expect()` audit | `rg 'expect\(' crates/llamaforge-metrics/src/` returns zero hits in production code | Zero matches |

---

## Summary: PR → File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `src/types.rs` (incl. MetricsConfig), `src/error.rs`, `src/filter.rs`, `src/lib.rs` (pub(crate) mods + pub use) | ~400 |
| PR2 | `src/traits.rs` (sealed traits + dyn assertions), `src/metrics_recording.rs` (trait defs), `src/sqlite/commands.rs`, `src/sqlite/in_memory.rs` | ~300 |
| PR3 | `src/sqlite/mod.rs`, `src/sqlite/migrations.rs`, `src/sqlite/pool.rs`, `src/sqlite/rows.rs` | ~300 |
| PR4 | `src/sqlite/writer.rs`, `src/sqlite/batch_collector.rs` | ~500 |
| PR5 | `src/sqlite/store.rs`, `src/sqlite/query_builder.rs`, `tests/vc2_write_read_pipeline.rs` | ~400 |
| PR6 | `src/ingest.rs`, `src/sqlite/pipeline.rs` | ~250 |
| PR7 | `src/metrics_recording.rs` (impl via `metrics` crate), `src/instrumented.rs`, `src/sqlite/liveness.rs`, `src/sqlite/prune.rs` | ~400 |
| PR8 | `tests/proptest_metrics.rs`, `tests/vc3_*.rs`, `tests/design_doc_tests.rs`, `src/lib.rs` (finalize re-exports) | ~350 |
| **Total** | | **~2,900** |

---

## Timeline (Single Engineer)

| Day | Work | PRs |
|-----|------|-----|
| Day 1 | Domain types + MetricsConfig + traits + InMemoryStore | PR1, PR2 |
| Day 2 | SQLite setup, migrations, read pool, row DTOs | PR3 |
| Day 3 | Writer thread + command dispatch + degradation state machine | PR4 (part 1) |
| Day 4 | Batch collector (dual-command flow) + type conversions | PR4 (part 2) |
| Day 5 | Writer + batch collector tests | PR4 (part 3) |
| Day 6 | Read path: queries, filters, aggregates, VC-2 tests | PR5 |
| Day 7 | MetricsIngest + pipeline assembly + integration | PR6 |
| Day 8 | MetricsFacadeRecorder + instrumented decorator + liveness + prune task | PR7 |
| Day 9 | Property tests + VC tests + public API | PR8 |
| Day 10–11 | VC-4, VC-5 verification, code review fixes, docs | — |

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `deadpool-sqlite` API changes | Low | Medium | Pin version. Wrap in internal pool abstraction if needed. |
| SQLite WAL grows unbounded under load | Medium | Medium | 60s passive checkpoint + oversize alert. Statement timeout prevents long reads. |
| `std::sync::mpsc` contention from async context | Low | Low | `try_send` is brief. Profile. Fall back to `crossbeam::channel` if contention shows. |
| Percentile computation in Rust is slow for large datasets | Medium | Low | Limit to 10,000 rows for percentile calc. Return `None` for larger sets. |
| `rusqlite_migration` schema versioning conflicts | Low | Medium | Test idempotency. Pin migration version. Document schema evolution process. |
| Writer thread panic leaves orphaned data | Low | High | Liveness monitor detects within 2s. Committed data survives. Channel data lost (bounded). |
| Cross-platform `flock` differences | Low | Medium | `nix` crate abstracts. Test on Linux (CI) and macOS (dev). |
| Batch collector timer drift under load | Low | Low | `tokio::time::interval` is monotonic. Acceptable — batches flush on size OR timer. |

---

## Integration Points (for next components)

After Metrics & Storage is complete, the following integration seams are ready:

1. **Dispatch & Proxy** will use `MetricsIngest::send_telemetry()` (lossy, hot path) and spawn detached tasks calling `send_audit()` (backpressured, per-request).
2. **Server Manager** will call `ProcessStore::upsert_managed_process()` before spawning and `remove_managed_process()` after reaping. Uses `list_managed_processes()` at startup for orphan detection.
3. **Transport Layer** will:
   - Expose `GET /metrics` using the `PrometheusHandle` from `install_metrics_recorder()` for Prometheus text exposition (the `metrics` facade crate + `metrics-exporter-prometheus` backend).
   - Expose `GET /api/events`, `GET /api/metrics` using `EventStore::query_events()` and `RequestMetricsStore::query_request_metrics()`.
   - Expose `POST /api/maintenance/wal-checkpoint` using `SqliteMetricsStore::wal_checkpoint()`.
4. **Config Manager** integration: `StoreMaintenance::insert_config_snapshot()` called on every reload/writeback. `retention_days` read from `ConfigReader::snapshot().global.metrics_retention_days` (field already exists on `GlobalConfig` in the implemented Config Manager — see `types.rs`).
5. **`main.rs`** will construct: `install_metrics_recorder()` → `create_metrics_pipeline(&metrics_config)` → wrap store in `InstrumentedMetricsStore::new(store)` → inject `MetricsIngest` to producers, `dyn EventStore` / `dyn RequestMetricsStore` to query consumers. The `MetricsConfig` struct (defined in `llamaforge-metrics`) is populated from `AppConfig::global.metrics_db_path` and `AppConfig::global.metrics_retention_days` (already present in `llamaforge-config`).

---

## Cross-Component Dependency Diagram

```
llamaforge-config (PR1–PR7 complete)
        │
        │ provides: ServerId, TaskTypeId, RequestorId,
        │           AppConfig, ConfigReader
        ▼
llamaforge-metrics (this plan)
        │
        │ provides: MetricsIngest, EventStore, RequestMetricsStore,
        │           ProcessStore, MetricsFacadeRecorder, RequestMetricsRecorder
        ▼
Server Manager, Dispatch & Proxy, Transport Layer (future components)
```

---

## Appendix: Adversarial Review Findings (Resolved)

This plan was subjected to an adversarial review. All findings have been addressed. Summary of changes applied:

| ID | Severity | Finding | Resolution |
|----|----------|---------|------------|
| F-1 | CRITICAL | Task 7.10 modified `llamaforge-config` from metrics PR — ownership violation | Removed. `MetricsConfig` defined in `llamaforge-metrics` (PR1 Task 1.11). Composition root extracts values. |
| F-2 | CRITICAL | `create_metrics_pipeline()` accepted only 2 params, hardcoding 12 config knobs | Pipeline factory now takes `&MetricsConfig`. All values from config struct, no magic numbers. |
| F-3 | CRITICAL | `WriterMode::Degraded` was recoverable — contradicts design doc's irreversible transition | Fixed. Degraded is now irreversible within process lifetime. `consecutive_failures` resets on success but mode stays Degraded. |
| F-4 | CRITICAL | Batch collector only sent `InsertEvents` — missed `InsertMetrics` for AuditRecords | Fixed. Batch collector now sends BOTH `InsertEvents` and `InsertMetrics` when AuditRecords present. 4-phase algorithm. |
| F-5 | HIGH | No periodic prune task anywhere in the plan | Added PR7 Task 7.10: periodic prune task reading `retention_days` from ConfigReader. |
| F-6 | HIGH | `MetricsIngest` mixed Prometheus `.inc()` and AtomicU64 descriptions | Resolved. Consistently AtomicU64 everywhere. `fetch_add(1, Relaxed)` for counters, exposed via getter functions. |
| F-7 | HIGH | PR4 tests had no way to cleanly shut down writer threads | Added Task 4.9: `WriterTestHandle` with Drop impl that sends Shutdown and joins. |
| F-8 | HIGH | PR4/PR5 "can overlap" claim misleading — PR5 tests need PR4's writer | Clarified: code can be developed concurrently, integration tests require PR4 merged. |
| F-9 | HIGH | `MetricsConfig` not defined in PR1 despite being needed by PR3/4/5/6 | Added PR1 Task 1.11 with all 12 fields, defaults, and serde annotations. |
| F-10 | HIGH | `ConfigOverlay` struct had no field definitions | Added explicit fields to PR2 Task 2.5. |
| F-11 | HIGH | Shutdown ordering incomplete — writer didn't drain/checkpoint on Shutdown | Fixed in PR4 Task 4.1 (drain + TRUNCATE checkpoint) and PR6 Task 6.5 (semaphore close ordering). |
| F-12 | MEDIUM | `nix` crate missing from PR3 dependencies | Added to Task 3.1. |
| F-13 | MEDIUM | Read pool PRAGMA assertion vs SET was ambiguous | Clarified in Task 3.5: assert mode is "wal", don't SET it. |
| F-14 | MEDIUM | PR5 base branch inconsistent between header and creation steps | Added concurrency note explaining the nuance. |
| F-15 | MEDIUM | `cmd_channel_events_dropped_total` missing from Prometheus metrics | Added to Task 7.2 field list. |
| F-16 | MEDIUM | `InMemoryStore` mock referenced but never defined | Added PR2 Task 2.11. |
| F-17 | MEDIUM | VC-2 test file had no implementation task | Added PR5 Task 5.16. |
| F-18 | MEDIUM | PR4 estimate too low for complexity | Revised from 6–8 hours to 10–14 hours (1.5–2 days). |
| F-19 | MEDIUM | WAL oversize alerting missing from liveness monitor | Added to Task 7.9: emit warn log + set degraded flag when WAL exceeds threshold. |

---

## Appendix B: Config Manager Implementation Alignment (Post-Implementation Revision)

This plan was updated after reading the implemented Config Manager codebase at `~/git/llama-forge`. The following changes were made to align with established conventions:

| Area | Before | After | Rationale |
|------|--------|-------|-----------|
| Module visibility | `pub mod types;` etc. | `pub(crate) mod types;` + explicit `pub use` re-exports | Matches `llamaforge-config/src/lib.rs` pattern exactly |
| Trait sealing | No sealed traits | All storage traits use `private::Sealed` pattern | Matches `ConfigSource` and `ConfigReader`/`ConfigWriter` |
| Dyn assertions | Runtime `Arc<dyn T>` tests only | Static dispatch test coverage for `impl Trait` (relaxed from `const _` dyn safety assertions) | Match `llamaforge-config` implementation conventions where possible |
| Enum extensibility | Some `#[non_exhaustive]` | ALL public enums `#[non_exhaustive]` | Matches `ConfigChange`, `ConfigLoadError`, etc. |
| Metrics crate | `prometheus = "0.13"` crate | `metrics = "0.22"` facade crate with `counter!`/`histogram!` macros | Matches `InstrumentedConfigHandle` which uses `metrics` crate |
| Prometheus struct | `PrometheusAppMetrics` | `MetricsFacadeRecorder` + `install_metrics_recorder()` helper | `metrics` crate separates recording from exporting |
| Instrumented decorator | Generic `InstrumentedMetricsStore<S, M>` | Concrete `InstrumentedMetricsStore` wrapping `SqliteMetricsStore` | Matches `InstrumentedConfigHandle` wrapping `ConfigHandle` directly |
| Sync mutex | `std::sync::Mutex` | `parking_lot::Mutex` (already in workspace) | Matches `InMemoryConfigSource` |
| Test doubles | `pub struct InMemoryStore` | `pub(crate)` + `#[cfg_attr(not(test), allow(dead_code))]` | Matches `InMemoryConfigSource` visibility |
| Async traits | Native async fn | `#[allow(async_fn_in_trait)]` annotation | Matches `ConfigWriter` trait |
| File naming | `prometheus_metrics.rs` | `metrics_recording.rs` | Backend-agnostic naming since using `metrics` facade |
| Codebase location | Not specified | `~/git/llama-forge` (documented in header) | Actual implementation location |
| Workspace deps | Assumed from scratch | Documents already-available deps: `parking_lot`, `metrics`, `nix`, `anyhow`, `serde_json`, `typed-builder` | Avoids redundant workspace dep additions |