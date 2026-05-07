# Observability Review: Config Manager Design

**Reviewer:** Observability & Telemetry Architecture Review (Rules 24-29)  
**Document:** `component-designs/01_CONFIG_MANAGER.md`  
**Date:** 2026-05-07  
**Verdict:** CONDITIONAL PASS — 3 findings require resolution before implementation

---

## Summary

The Config Manager design document defines 6 Prometheus metrics, 8 log events, and 3 health signals (Section 7). The metric design is solid with bounded label cardinality and histogram-based latency capture. However, the document has gaps in tracing architecture, subscriber configuration, and domain/observability boundary enforcement. The findings below range from HIGH to LOW severity.

---

## Findings

### Finding 1: No Structured Tracing Spans for Reload/Writeback Operations

**Rule:** 24 — Structured Tracing, Not Printf Logging  
**Severity:** MEDIUM  
**Location:** Section 7.2 (lines 441-451), Section 3.1-3.2 (lines 248-301)

**Violation:** Section 7.2 defines 8 log events (e.g., `config.reloaded`, `config.writeback`) but describes them only as discrete events with a level and content summary. There is no mention of spans wrapping the reload or writeback operations. The reload flow (Section 3.1) has multiple stages — mutex acquisition, flock, TOML parse, diff, ArcSwap store, broadcast — but no span is defined to correlate these stages or capture end-to-end duration as a trace.

The `config.reload_duration_seconds` histogram captures wall-clock duration, but without a parent span, there is no way to see the breakdown (how much time was flock vs. parse vs. diff) in a distributed trace viewer.

**Suggested Fix:** Define explicit spans in the design:
- A `config.reload` span wrapping the entire reload path, with child spans for `flock_acquire`, `toml_parse`, `diff_compute`, and `arcswap_store`.
- A `config.writeback` span wrapping the writeback path, with child spans for `flock_acquire`, `toml_reread`, `mutation_apply`, `atomic_write`, and `arcswap_store`.
- Each span should carry structured fields: `config.file_path`, `config.server_count`, `config.diff.servers_added`, etc.

This enables drill-down in Jaeger/Tempo without relying solely on log grep.

---

### Finding 2: No Multi-Layer Subscriber Architecture Specified

**Rule:** 25 — Multi-Layer Subscriber Architecture  
**Severity:** MEDIUM  
**Location:** Section 7 (lines 426-457), Section 2.3 (lines 109-111 — `log_level` and `log_format` fields)

**Violation:** The `GlobalConfig` struct defines `log_level: LogLevel` and `log_format: LogFormat` fields, implying some subscriber configuration exists. However, the design document does not specify the tracing-subscriber layer stack. There is no mention of:
- A human-readable stdout/stderr layer for operator use.
- A structured JSON or OTLP export layer for centralized log aggregation.
- How these layers compose (e.g., `tracing_subscriber::registry().with(stdout_layer).with(otlp_layer)`).
- Whether `log_format` toggles between `fmt::format::Full` and `fmt::format::Json`, or something else entirely.

**Suggested Fix:** Add a subsection (e.g., 7.4 "Tracing Subscriber Stack") that specifies:
1. Layer 1: `tracing_subscriber::fmt` layer writing to stderr, format controlled by `log_format` (human-readable or JSON), filtered by `log_level`.
2. Layer 2: OTLP exporter layer (when `otlp_endpoint` is configured) for structured span/event export to a collector.
3. Both layers composed via `tracing_subscriber::registry()`.

This is especially important because Config Manager is the component that owns `log_level` and `log_format` — it must document how those fields drive the subscriber stack.

---

### Finding 3: No Boundary Enforcement Between Domain Logic and Observability

**Rule:** 26 — Observability Must Not Leak Into Domain Logic  
**Severity:** HIGH  
**Location:** Section 3.1-3.2 (lines 248-301), Section 7.1-7.2 (lines 428-451)

**Violation:** The design defines 6 metrics and 8 log events but does not specify where in the architecture these are emitted. The data flow diagrams in Section 3.1 and 3.2 show the reload/writeback logic as a single pipeline with no separation between domain operations (parse, diff, ArcSwap store) and observability concerns (metric increments, event emission).

Without explicit guidance, implementers will naturally place `tracing::info!("config.reloaded", ...)` and `config_reloads_total.inc()` calls directly inside the `reload()` and `writeback()` methods — embedding telemetry in domain logic.

**Suggested Fix:** The design should specify an observability boundary pattern. Two options:

**Option A — Decorator/Wrapper:** Define an `InstrumentedConfigHandle` that wraps `ConfigHandle`, adding spans, metrics, and events at the boundary. Domain methods on `ConfigHandle` remain telemetry-free.

```
InstrumentedConfigHandle::reload()
  -> span("config.reload")
  -> config_reloads_total.inc()
  -> inner.reload()   // pure domain logic, no tracing
  -> record duration to histogram
  -> emit config.reloaded event with diff fields
```

**Option B — Callback/Event Bus:** The `reload()` method returns a `ReloadOutcome` struct containing timing and diff data. A separate observability module subscribes to these outcomes and emits metrics/events.

Either approach keeps Rule 26 satisfied. The design should pick one and document it.

---

### Finding 4: Distributed Trace Context Not Addressed for API-Triggered Reloads

**Rule:** 27 — Distributed Trace Context Propagation  
**Severity:** HIGH  
**Location:** Section 3.1 (line 250 — "POST /api/config/reload"), Section 3.2 (line 279 — "POST /api/servers/{id}/lora-adapters")

**Violation:** Both reload and writeback can be triggered via HTTP API endpoints. The design does not mention:
- Extracting W3C `traceparent`/`tracestate` headers from inbound HTTP requests.
- Propagating trace context into the reload/writeback span so that an API caller's trace includes the config operation.
- Injecting trace context into the outbound HTTP call to llama-server (Section 3.2, step 1: "Apply to llama-server (HTTP)").

Without this, a request to change a LoRA adapter will show a gap in the distributed trace between the API gateway and the config writeback, and between the config writeback and the llama-server apply call.

**Suggested Fix:** Add to the design:
1. Inbound: The Transport Layer (which owns HTTP) must extract `traceparent` and create a tracing span. Config Manager's reload/writeback span should be a child of that extracted context.
2. Outbound: The HTTP call to llama-server in the writeback flow (Section 3.2, step 1) must inject `traceparent`/`tracestate` headers. Note this as a contract requirement on whoever implements that HTTP call.
3. Add a note that SIGHUP-triggered reloads create a root span (no parent context available).

---

### Finding 5: No Telemetry Transport Configuration Specified

**Rule:** 28 — Telemetry Transport Efficiency at Scale  
**Severity:** LOW  
**Location:** Section 7 (lines 426-457)

**Violation:** The design does not specify how metrics and traces are exported. For a Config Manager (low-volume component — reloads are rare), this is low severity, but the design should still document:
- Whether Prometheus metrics are scraped (pull) or pushed (push gateway).
- Whether OTLP export uses `BatchSpanProcessor` or `SimpleSpanProcessor`.
- Whether gzip/zstd compression is enabled on the OTLP exporter.

**Suggested Fix:** Add a brief note in Section 7 or a cross-cutting concerns section:
- Metrics: exposed via `/metrics` endpoint (pull model, handled by Transport Layer).
- Traces: exported via OTLP with `BatchSpanProcessor` and gzip compression (configured globally, not per-component).
- Config Manager does not own these decisions but should reference the system-level telemetry design.

This is LOW severity because Config Manager is low-volume and the transport design is likely a cross-cutting concern owned elsewhere.

---

### Finding 6: `broadcast_lagged_total` Metric Has Potentially Unbounded `subscriber` Label

**Rule:** 29 — Actionable, Symptom-Based Alerting  
**Severity:** MEDIUM  
**Location:** Section 7.1 (line 436)

**Violation:** The metric `llamaforge_config_broadcast_lagged_total` uses a `subscriber` label. Section 7.1 claims "All labels are bounded enums (max 3-4 values)" but the `subscriber` label's cardinality depends on how subscribers identify themselves. If subscribers are named by dynamic identifiers (e.g., request IDs, connection IDs), this becomes an unbounded label — a classic Prometheus cardinality bomb that causes memory growth in the metrics server.

**Suggested Fix:** Constrain the `subscriber` label to a fixed enum of known subscriber types: `{server_manager, queue, router, transport}`. Document this constraint explicitly. If a new subscriber type is added, it must be added to this enum — never use a dynamic identifier.

---

### Finding 7: Health Signals Lack Quantitative Thresholds

**Rule:** 29 — Actionable, Symptom-Based Alerting  
**Severity:** LOW  
**Location:** Section 7.3 (lines 454-457)

**Violation:** The three health signals (Ready, Live, Degraded) are well-defined qualitatively but lack quantitative alerting guidance:
- No threshold for "reload taking too long" (e.g., `config_reload_duration_seconds` p99 > 5s).
- No threshold for "too many failed reloads" (e.g., `config_reloads_total{result="error"}` increasing).
- The "Degraded" signal for writeback-disabled is good, but there is no signal for "writeback is succeeding but slow" (flock contention).

**Suggested Fix:** Add an alerting guidance table:

| Alert | Condition | Severity |
|-------|-----------|----------|
| ConfigReloadSlow | `config_reload_duration_seconds` p99 > 5s for 5m | Warning |
| ConfigReloadFailing | `rate(config_reloads_total{result="error"}[5m]) > 0` | Critical |
| ConfigFlockContention | `config_flock_wait_seconds` p99 > 2s for 5m | Warning |
| ConfigWritebackDisabled | Degraded health signal active | Warning |

These are suggestions — the operator tunes thresholds, but the design should provide starting points.

---

### Finding 8: Log Events Lack Explicit Structured Field Definitions

**Rule:** 24 — Structured Tracing, Not Printf Logging  
**Severity:** MEDIUM  
**Location:** Section 7.2 (lines 441-451)

**Violation:** The log event table lists events with a "Content" column describing what each event contains in prose (e.g., "File paths, server count, parse duration" for `config.loaded`). This does not specify the actual structured field names that will appear in the tracing event. Without this, implementers may use inconsistent field names or positional formatting.

For example, `config.reloaded` says "Diff summary (servers added/removed/changed)" — but is that:
- `servers_added=2, servers_removed=0, servers_changed=1` (good: named fields)
- `"Reloaded config: 2 added, 0 removed, 1 changed"` (bad: printf-style string)

**Suggested Fix:** Expand the log event table with an explicit "Fields" column:

| Event | Level | Fields |
|-------|-------|--------|
| `config.loaded` | INFO | `config.file_path: String`, `config.server_count: u32`, `config.parse_duration_ms: u64` |
| `config.reloaded` | INFO | `config.servers_added: u32`, `config.servers_removed: u32`, `config.servers_changed: u32`, `config.reload_trigger: {sighup, api}` |
| `config.reload_failed` | ERROR | `config.file_path: String`, `config.error: String`, `config.line_number: Option<u32>` |
| `config.writeback` | INFO | `config.field: String`, `config.old_value: String`, `config.new_value: String`, `config.write_strategy: {atomic_rename, truncate_fallback}` |

This ensures implementers use `tracing::info!(config.server_count = count, config.file_path = %path, "config.loaded")` rather than `tracing::info!("Loaded config from {} with {} servers", path, count)`.

---

## Commendations

### C1: Bounded Label Cardinality Awareness

Section 7.1 explicitly calls out: "All labels are bounded enums (max 3-4 values). No unbounded labels." This is excellent practice and prevents the most common Prometheus operational failure. The `result={success,error}` and `operation={reload,writeback}` labels are well-designed. (Finding 6 notes one exception that needs tightening.)

### C2: Histogram-Based Latency Metrics

The design uses Histogram type for `config_reload_duration_seconds`, `config_writeback_duration_seconds`, and `config_flock_wait_seconds`. This enables percentile-based alerting (p50, p95, p99) rather than relying on averages — a direct alignment with Rule 29. This is the correct choice for operational visibility.

### C3: Failure Mode Documentation Enables Observability Design

Section 5 (Failure Modes FM-1 through FM-6) is thorough and maps directly to observable signals. Each failure mode has a clear detection path through the defined metrics and log events. For example:
- FM-2 (crash during truncation fallback) maps to `config.empty_file_detected` event.
- FM-5 (broadcast overflow) maps to `config_broadcast_lagged_total` metric.
- FM-6 (state dir not writable) maps to the Degraded health signal.

This is a model for how failure modes and observability should be co-designed.

### C4: Separate Reload Trigger Awareness

The design distinguishes SIGHUP-triggered reloads from API-triggered reloads in the data flow (Section 3.1). This is important for observability — operators need to know whether a reload was manual or programmatic when debugging incidents. The log events should carry a `trigger` field to preserve this distinction (noted in Finding 8).

### C5: flock Wait Time as a Dedicated Metric

`llamaforge_config_flock_wait_seconds` with `operation` label is an excellent operational metric. Lock contention is a common root cause of config reload latency, and having it broken out as a separate histogram (rather than buried in the total reload duration) enables precise diagnosis.

---

## Summary Table

| # | Rule | Severity | Section | Status |
|---|------|----------|---------|--------|
| F1 | 24 — Structured Tracing | MEDIUM | 7.2, 3.1-3.2 | No spans defined for reload/writeback |
| F2 | 25 — Multi-Layer Subscriber | MEDIUM | 7, 2.3 | No subscriber stack specified |
| F3 | 26 — Domain/Observability Boundary | HIGH | 3.1-3.2, 7.1-7.2 | No boundary enforcement pattern |
| F4 | 27 — Trace Context Propagation | HIGH | 3.1, 3.2 | No W3C header extraction/injection |
| F5 | 28 — Transport Efficiency | LOW | 7 | No export configuration specified |
| F6 | 29 — Alerting (Cardinality) | MEDIUM | 7.1 | `subscriber` label may be unbounded |
| F7 | 29 — Alerting (Thresholds) | LOW | 7.3 | No quantitative alert thresholds |
| F8 | 24 — Structured Fields | MEDIUM | 7.2 | Log events lack explicit field names |

**HIGH findings (F3, F4) must be addressed before implementation begins.**  
**MEDIUM findings (F1, F2, F6, F8) should be addressed during implementation.**  
**LOW findings (F5, F7) are recommendations for operational maturity.**
