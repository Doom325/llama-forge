# Rust Architecture Review: Config Manager

**Project:** LlamaForge — Config Manager Component  
**Date:** 2026-05-07  
**Scope:** Pre-implementation design review of `component-designs/01_CONFIG_MANAGER.md` (593 lines)  
**Overall Verdict:** FAIL (4 HIGH findings)

---

## Executive Summary

The Config Manager design is architecturally sound at its core — the ArcSwap + immutable snapshot + broadcast diff pattern is excellent for a read-heavy configuration system, the `spawn_blocking` usage for filesystem I/O is correct, and the failure mode analysis is thorough. However, the design has gaps in type safety (bare `String` identifiers, missing type definitions), architectural boundary separation (no port/adapter abstraction for filesystem I/O), and observability architecture (no tracing spans, no domain/observability boundary). Four HIGH findings must be resolved before implementation.

---

## Critical Findings (HIGH)

### H-1: Bare `String` Keys in `PriorityRulesConfig` HashMaps

**Domain:** Trait Design (Rule 2 — Newtype Wrappers)  
**Location:** Section 2.3, lines 160-166  
**Severity:** HIGH

Four HashMaps use raw `String` as keys for domain identities:

```rust
pub task_types: HashMap<String, u32>,
pub custom_task_types: HashMap<String, u32>,
pub task_type_classes: HashMap<String, String>,
pub requestor_bonuses: HashMap<String, i32>,
```

A `TaskTypeId` can be accidentally passed where a `RequestorId` is expected, and the compiler won't catch it. The double-String in `task_type_classes` is particularly dangerous — both key and value are bare strings representing different domains.

**Fix:** Introduce `TaskTypeId(String)`, `TaskClassName(String)`, `RequestorId(String)` newtypes with `#[serde(transparent)]` for TOML compatibility.

---

### H-2: `ServerConfigDiff.fields_changed` Is Stringly Typed

**Domain:** Trait Design (Rule 2 + Rule 7)  
**Location:** Section 2.5, line 239  
**Severity:** HIGH

`fields_changed: Vec<String>` forces consumers to match on magic string literals. A typo in `"model_path"` compiles silently. The `requires_restart` and `hot_reloadable` booleans can drift out of sync with the actual field list.

**Fix:** Define a `ServerConfigField` enum with one variant per field. Derive `requires_restart` and `hot_reloadable` from the set of changed fields via a method:

```rust
impl ServerConfigField {
    pub fn requires_restart(&self) -> bool {
        matches!(self, Self::ModelPath | Self::CtxSize | Self::ParallelSlots | Self::Devices)
    }
}
```

---

### H-3: No Port Abstraction for Config Persistence

**Domain:** Boundaries (Rule 13 — Ports Are Traits, Adapters Are Structs)  
**Location:** Section 2.1, lines 30-66  
**Severity:** HIGH

`ConfigHandle` is a concrete struct that directly performs filesystem I/O, flock acquisition, and TOML parsing. There is no trait abstracting config persistence, which means:

1. Unit testing reload serialization or diff broadcasting requires a real filesystem.
2. The v1.1 `ConfigOverlayStore` migration requires modifying the core struct rather than providing a new adapter.
3. Domain logic (diff computation, snapshot management, broadcast) is inseparable from infrastructure (filesystem, flock, TOML).

**Fix:** Extract a `ConfigSource` port trait:

```rust
pub trait ConfigSource: Send + Sync {
    fn load(&self) -> Result<AppConfig, ConfigLoadError>;
    fn persist(&self, config: &AppConfig, mutation: &ConfigMutation) -> Result<(), WritebackError>;
}
```

`ConfigHandle` delegates I/O through `Arc<dyn ConfigSource>`. A `TomlFileConfigSource` adapter implements the flock/read/parse/write/rename logic. Domain logic (ArcSwap, diff, broadcast, mutex serialization) stays in `ConfigHandle`.

**Reviewer note:** The Boundaries sub-agent also flagged serde annotations on domain types (Rule 14) as HIGH. I'm **downgrading that to MEDIUM** in this unified report because Config Manager is fundamentally an infrastructure component — its types ARE serialization targets. Unlike a business domain entity carrying ORM annotations, `AppConfig` exists to be parsed from TOML. A DTO separation layer adds complexity that may not be justified here. However, if `AppConfig` is shared as a dependency to domain-layer components like the Router or Queue, the concern becomes more valid.

---

### H-4: No Domain/Observability Boundary Enforcement

**Domain:** Observability (Rule 26)  
**Location:** Section 3.1-3.2, Section 7.1-7.2  
**Severity:** HIGH

The design defines 6 metrics and 8 log events but does not specify where they're emitted. Without explicit guidance, implementers will place `tracing::info!()` and `counter.inc()` directly inside `reload()` and `writeback()`, embedding telemetry in core logic.

**Fix:** Specify an instrumentation boundary. Recommended: an `InstrumentedConfigHandle` decorator:

```
InstrumentedConfigHandle::reload()
  → span("config.reload")
  → config_reloads_total.inc()
  → inner.reload()            // telemetry-free
  → record duration to histogram
  → emit config.reloaded event
```

This aligns naturally with the `ConfigSource` port abstraction (H-3) — the decorator wraps the trait interface.

**Reviewer note:** The Observability sub-agent also flagged distributed trace context (Rule 27) as HIGH. I'm **downgrading to MEDIUM** because trace context extraction/injection is a Transport Layer responsibility (HTTP middleware), not Config Manager's. Config Manager should expect to receive a tracing span context from its caller, not implement W3C header parsing.

---

## Warnings (MEDIUM)

### M-1: `ConfigHandle` Has No Trait Abstraction for Testing

**Domain:** Trait Design (Rule 6 — Interface Segregation)  
**Location:** Section 2.1, lines 30-66

Read-only consumers (Router, Queue, Dispatcher) are coupled to the full write API. No trait exists for testing.

**Fix:** Extract `ConfigReader` and `ConfigWriter` traits. Read-only consumers depend on `impl ConfigReader`.

*Cross-reference: overlaps with H-3 (port abstraction). Resolving H-3 naturally provides the trait separation needed here.*

---

### M-2: No Typestate for Config Lifecycle

**Domain:** Boundaries (Rule 15 — Typestate)  
**Location:** Section 2.1

The config has two distinct lifecycle phases: startup (`load()`, fatal errors) and running (`reload()`/`writeback()`, non-fatal errors). A single struct allows calling `snapshot()` before `load()`.

**Fix:** `ConfigHandle<Unloaded>` with `load(self) -> Result<ConfigHandle<Running>, _>`. `snapshot()`, `reload()`, and `writeback()` are only available on `ConfigHandle<Running>`.

---

### M-3: Serde Annotations on Domain-Facing Config Types

**Domain:** Boundaries (Rule 14 — Domain/DTO Segregation)  
**Location:** Section 2.3, lines 89-171

`AppConfig` carries `#[derive(Serialize, Deserialize)]` and `#[serde(default)]`. All consumers take a transitive dependency on `serde` and `toml`. If these types are in a shared crate that domain-layer components depend on, this leaks infrastructure into the domain.

**Fix:** If `AppConfig` is shared across crate boundaries, separate `RawAppConfig` (DTO with serde) from `AppConfig` (clean domain type) with `TryFrom` conversion. If it stays within the config crate, the current approach is acceptable with a documented note.

*Downgraded from HIGH (Boundaries sub-agent) to MEDIUM because Config Manager is itself infrastructure.*

---

### M-4: `reload_mutex` Held Across `spawn_blocking().await` — Cancellation Concern

**Domain:** Safety (Rule 22)  
**Location:** Section 3.1, lines 250-274

The mutex guard is held while awaiting `spawn_blocking`, which may block on `flock` for up to 30 seconds. If the caller is cancelled after acquiring the mutex but before `spawn_blocking` completes, the guard drops and the blocking task runs unserialized.

**Fix:** Document that the implementation must ensure the mutex guard is not dropped until `spawn_blocking` completes (e.g., by holding it in a variable that outlives the await). Verify in code review.

---

### M-5: `export_toml()` Returns Bare `String` — Hidden Unwrap

**Domain:** Safety (Rule 21)  
**Location:** Section 2.1, line 65

`export_toml(&self) -> String` requires the implementation to unwrap a `toml::to_string()` result internally.

**Fix:** Change to `pub fn export_toml(&self) -> Result<String, WritebackError>`.

---

### M-6: No Tracing Spans for Reload/Writeback Operations

**Domain:** Observability (Rule 24)  
**Location:** Section 7.2, Section 3.1-3.2

Log events are defined but no tracing spans wrap the multi-stage reload/writeback operations. Without spans, there's no way to correlate stages (flock → parse → diff → store) in a trace viewer.

**Fix:** Define `config.reload` and `config.writeback` parent spans with child spans for each stage. Carry structured fields: `config.file_path`, `config.server_count`, `config.diff.servers_added`, etc.

---

### M-7: No Multi-Layer Subscriber Stack Specified

**Domain:** Observability (Rule 25)  
**Location:** Section 7, Section 2.3

`GlobalConfig` defines `log_level` and `log_format` but the document never specifies the tracing-subscriber layer composition (stdout + OTLP, format selection, EnvFilter).

**Fix:** Add a "Tracing Subscriber Stack" subsection specifying Layer 1 (fmt to stderr) + Layer 2 (OTLP to collector) composed via `tracing_subscriber::registry()`.

---

### M-8: `broadcast_lagged_total` Label May Be Unbounded

**Domain:** Observability (Rule 29)  
**Location:** Section 7.1, line 436

The `subscriber` label on `broadcast_lagged_total` could become unbounded if subscribers use dynamic identifiers.

**Fix:** Constrain to a fixed enum: `{server_manager, queue, router, transport}`. Document explicitly.

---

### M-9: Log Events Lack Explicit Structured Field Names

**Domain:** Observability (Rule 24)  
**Location:** Section 7.2, lines 441-451

The "Content" column describes fields in prose ("File paths, server count, parse duration") rather than specifying structured field names. This invites printf-style formatting.

**Fix:** Add a "Fields" column with explicit names: `config.file_path: String`, `config.server_count: u32`, `config.parse_duration_ms: u64`, etc.

---

### M-10: Distributed Trace Context for API-Triggered Operations

**Domain:** Observability (Rule 27)  
**Location:** Section 3.1, Section 3.2

API-triggered reloads and writeback's outbound HTTP to llama-server don't mention trace context propagation.

**Fix:** Document that: (1) Transport Layer middleware extracts `traceparent` and creates a parent span before calling Config Manager, (2) the outbound HTTP call to llama-server in writeback must inject `traceparent`, (3) SIGHUP-triggered reloads create root spans.

*Downgraded from HIGH (Observability sub-agent) to MEDIUM. Trace context extraction is a Transport Layer responsibility.*

---

## Suggestions (LOW)

### L-1: `GlobalConfig`, `ServerConfig`, `PriorityRulesConfig` Lack Builders or Default

**Domain:** Trait Design (Rule 8)  
**Location:** Section 2.3

12-14 field structs with no builder or `Default` derive. Programmatic construction in tests is painful.

**Fix:** Add `#[derive(Default)]` at minimum. Consider `typed-builder` for test ergonomics.

---

### L-2: `ConfigDiff` Could Use a `ConfigChange` Enum for Extensibility

**Domain:** Trait Design (Rule 9)  
**Location:** Section 2.5

The flat struct mixes leaf changes (`global_changed: bool`) and composite changes (`servers_changed: Vec<...>`). As config grows in v1.1, this accumulates fields.

**Fix:** Consider `Vec<ConfigChange>` where `ConfigChange` is an enum: `Global`, `PriorityRules`, `Server { id, diff }`.

---

### L-3: `ConfigDiff` Should Use `Arc` Through Broadcast Channel

**Domain:** Safety (Rule 23)  
**Location:** Section 2.5

`broadcast::Sender::send` requires `Clone`, which deep-copies all `Vec<String>` in `ServerConfigDiff`. Since reloads are rare this is cold-path, but `Arc<ConfigDiff>` would be zero-cost.

**Fix:** `broadcast::Sender<Arc<ConfigDiff>>`.

---

### L-4: Error Types Wrap Infrastructure Crate Types Directly

**Domain:** Boundaries (Rule 12)  
**Location:** Section 2.4, lines 186, 214

`ConfigLoadError::ParseError` wraps `toml::de::Error` and `WritebackError::SerializeError` wraps `toml::ser::Error`. Consumers matching on these variants take a transitive dependency on `toml`.

**Fix:** Use opaque `Box<dyn Error + Send + Sync>` in the source field, or a `message: String` field with the infrastructure error converted to text.

---

### L-5: Health Signals Lack Quantitative Alerting Thresholds

**Domain:** Observability (Rule 29)  
**Location:** Section 7.3

No thresholds for "reload too slow" or "too many failed reloads."

**Fix:** Add an alerting guidance table with starting thresholds (e.g., p99 reload > 5s = warning, `rate(error)` > 0 = critical).

---

### L-6: No Telemetry Transport Configuration

**Domain:** Observability (Rule 28)  
**Location:** Section 7

No mention of `BatchSpanProcessor`, compression, or scrape model.

**Fix:** Add a brief cross-reference to the system-level telemetry design. Low severity for a low-volume component.

---

## Commendations

### K-1: ArcSwap + Immutable Snapshot Concurrency Model

The ArcSwap + atomic pointer swap + immutable `Arc<AppConfig>` snapshots is an excellent pattern for a config system. It eliminates read-side contention entirely, provides consistent-snapshot semantics, and enables the `CachedConfigReader` optimization for hot paths. This is textbook Rust concurrency design.

### K-2: Correct `spawn_blocking` for All Filesystem I/O

Invariant 6 ("No blocking on reactor") is explicitly stated and consistently applied. Every `flock`, `read`, `write`, `fsync`, and `rename` runs inside `spawn_blocking`. The justification blockquote in the main design doc is thorough. This prevents the most common async-Rust production issue.

### K-3: Thorough Cancellation Safety Analysis

The cross-cutting concerns section correctly identifies both `.await` points in the reload path and analyzes cancellation at each one. The observation that cancelled `spawn_blocking` still runs to completion and the file is the source of truth is a subtlety most designs miss.

### K-4: Comprehensive Failure Mode Analysis

Six explicit failure modes with state-preserved/state-lost analysis, recovery procedures, and honest acknowledgment of the truncation fallback's crash window. FM-1 through FM-6 map cleanly to observable signals in Section 7.

### K-5: Clean Error Type Segregation

Three error types separated by operation (`ConfigLoadError`, `ConfigReloadError`, `WritebackError`), each with focused variants and no catch-all. Consumers of `snapshot()` never encounter `WritebackError`.

### K-6: `CachedConfigReader` as Composition-Based Optimization

Rather than baking caching into `ConfigHandle`, the design provides an opt-in `CachedConfigReader` wrapper. This is composition over inheritance in practice — the hot-path optimization is a decorator.

### K-7: Bounded Label Cardinality Awareness

Section 7.1 explicitly documents: "All labels are bounded enums (max 3-4 values). No unbounded labels." This prevents the most common Prometheus operational failure.

### K-8: Histogram-Based Latency Metrics

All duration metrics use Histogram type, enabling percentile-based alerting rather than averages. This is the correct operational choice.

---

## Review Coverage

| Domain | Sub-agent | Findings | Status |
|--------|-----------|----------|--------|
| Trait Design | rust-review-traits | 6 | Reviewed |
| Boundaries | rust-review-boundaries | 6 | Reviewed |
| Runtime Safety | rust-review-safety | 5 | Reviewed |
| Observability | rust-review-observability | 8 | Reviewed |
| Crate Hygiene | rust-review-crates | — | Skipped (pre-implementation, no Cargo.toml) |
| CI Pipeline | rust-review-ci | — | Skipped (pre-implementation, no CI config) |

---

## Deduplication Notes

The following findings were identified by multiple sub-agents and merged:

1. **ConfigHandle no trait** — flagged by both Traits (F-4, Rule 6) and Boundaries (F-2, Rule 13). Merged into H-3 + M-1 (boundary finding is primary; trait finding is cross-referenced).
2. **ConfigMutation undefined** — flagged by Traits (F-5) as "should be sealed enum." Already covered in the separate adversarial review (C-2) as a missing type. Not duplicated here; the Traits finding about `#[non_exhaustive]` is additive.
3. **Serde on config types** — flagged by Boundaries (F-1, Rule 14) as HIGH. Downgraded to M-3 because Config Manager is infrastructure, not domain.
4. **Trace context propagation** — flagged by Observability (F-4, Rule 27) as HIGH. Downgraded to M-10 because this is Transport Layer's responsibility.

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 4     |
| MEDIUM   | 10    |
| LOW      | 6     |
| Total    | 20    |

**Verdict: FAIL** — 4 HIGH findings require resolution before implementation. The most impactful changes are: (1) newtype wrappers for domain identities (H-1, H-2), (2) `ConfigSource` port trait for testability and extensibility (H-3), and (3) observability boundary pattern (H-4). These establish architectural patterns that all downstream components will depend on.
