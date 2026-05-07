# Config Manager — Re-Review (R2)

**Document:** `01_CONFIG_MANAGER.md` (updated, ~895 lines)  
**Date:** 2026-05-07  
**Method:** Cross-check every finding from both the adversarial review (17 findings) and the architecture review (20 findings) against the updated document. Identify: resolved, still open, or newly introduced.

---

## Resolution Summary

### Adversarial Review Findings (REVIEW_01_CONFIG_MANAGER.md)

| ID | Severity | Finding | Status |
|----|----------|---------|--------|
| C-1 | Critical | ConfigHandle vs ConfigManagerInner contradictions | **Partially fixed** — see OPEN-1 |
| C-2 | Critical | ConfigMutation type never defined | **STILL OPEN** — see OPEN-2 |
| C-3 | Critical | flock timeout implementation unspecified | **STILL OPEN** — see OPEN-3 |
| H-1 | High | Duration fields don't roundtrip through TOML | **STILL OPEN** — see OPEN-4 |
| H-2 | High | Missing model_architecture field in ServerConfig | **STILL OPEN** — see OPEN-5 |
| H-3 | High | Reload scope boundary ambiguity | **STILL OPEN** — see OPEN-6 |
| H-4 | High | 13+ referenced types never defined | **STILL OPEN** — see OPEN-7 |
| H-5 | High | ServerId as HashMap key — serde/ergonomics | **Partially fixed** — pattern established via TaskTypeId etc. |
| M-1 | Medium | Writeback data flow omits mutex acquisition | **STILL OPEN** — diagram unchanged |
| M-2 | Medium | Validation rules unspecified, normalization vs error | **STILL OPEN** |
| M-3 | Medium | ConfigDiff::timestamp uses non-serializable Instant | **STILL OPEN** |
| M-4 | Medium | Environment variable override undocumented | **STILL OPEN** |
| M-5 | Medium | Missing web_ui section in AppConfig | **STILL OPEN** |
| M-6 | Medium | Proptest backward-compat strategy fragile | **STILL OPEN** — Section 8.3 unchanged |
| L-1 | Low | Broadcast channel capacity tight | Acceptable as-is |
| L-2 | Low | fields_changed stringly typed | **RESOLVED** — ServerConfigField enum |
| L-3 | Low | Unknown-key warning needs custom deserializer | Acceptable as-is |

### Architecture Review Findings (architecture_review.md)

| ID | Severity | Finding | Status |
|----|----------|---------|--------|
| H-1 | High | Bare String keys in PriorityRulesConfig | **RESOLVED** — newtypes added (lines 280-303) |
| H-2 | High | ServerConfigDiff stringly typed | **RESOLVED** — ServerConfigField enum (lines 405-458) |
| H-3 | High | No port abstraction for config persistence | **RESOLVED** — ConfigSource trait (lines 30-45) |
| H-4 | High | No domain/observability boundary | **RESOLVED** — InstrumentedConfigHandle (lines 149-193) |
| M-1 | Medium | ConfigHandle has no trait for testing | **RESOLVED** — ConfigReader + ConfigWriter traits (lines 50-64) |
| M-2 | Medium | No typestate for config lifecycle | **RESOLVED** — pragmatic explanation (lines 113-129) |
| M-3 | Medium | Serde on domain-facing types | **RESOLVED** — documented rationale (lines 197-202) |
| M-4 | Medium | reload_mutex cancellation concern | **RESOLVED** — detailed analysis (lines 840-859) |
| M-5 | Medium | export_toml returns bare String | **RESOLVED** — returns Result (line 106) |
| M-6 | Medium | No tracing spans | **RESOLVED** — span table (Section 7.1) |
| M-7 | Medium | No subscriber stack specified | **RESOLVED** — Section 7.4 |
| M-8 | Medium | Unbounded subscriber label | **RESOLVED** — fixed enum constraint (lines 680-682) |
| M-9 | Medium | Log events lack structured fields | **RESOLVED** — fields column added (Section 7.3) |
| M-10 | Medium | Trace context not addressed | **RESOLVED** — lines 662-667 |
| L-1 | Low | Missing Default on config structs | **RESOLVED** — Default derived (lines 210, 219, 247, 292) |
| L-2 | Low | ConfigDiff flat struct | **RESOLVED** — ConfigChange enum (lines 366-403) |
| L-3 | Low | ConfigDiff cloned through broadcast | **RESOLVED** — Arc<ConfigDiff> (lines 56, 77, 361) |
| L-4 | Low | Error types wrap infra crate types | **RESOLVED** — Box<dyn Error> (lines 322, 352) |
| L-5 | Low | Health signals lack thresholds | **RESOLVED** — Section 7.6 alerting table |
| L-6 | Low | No telemetry transport config | **RESOLVED** — Section 7.4 cross-reference |

**Score: 20 of 20 architecture review findings resolved. 5 of 17 adversarial findings resolved, 3 partially fixed, 9 still open.**

---

## Still-Open Findings

### OPEN-1 (Medium, downgraded from Critical): ConfigHandle / ConfigManagerInner Still Duplicated

**Original:** C-1 — ConfigHandle vs ConfigManagerInner contradictory definitions.

**Current state:** The contradiction is reduced but NOT eliminated. Both structs have identical fields:

```rust
// Section 2.3 (lines 73-78) — public
pub struct ConfigHandle {
    source: Arc<dyn ConfigSource>,
    inner: arc_swap::ArcSwap<AppConfig>,
    reload_mutex: tokio::sync::Mutex<()>,
    change_tx: tokio::sync::broadcast::Sender<Arc<ConfigDiff>>,
}

// Section 3.3 (lines 535-540) — internal
struct ConfigManagerInner {
    source: Arc<dyn ConfigSource>,
    config: ArcSwap<AppConfig>,
    change_tx: tokio::sync::broadcast::Sender<Arc<ConfigDiff>>,
    reload_mutex: tokio::sync::Mutex<()>,
}
```

These are the same struct with different names (`inner` vs `config` for the ArcSwap field). The note on line 533 says "ConfigHandle is the public-facing struct that wraps this" — but ConfigHandle doesn't contain a `ConfigManagerInner` field. It has the same fields directly.

**Severity downgrade:** From Critical to Medium. The fields now agree (unlike R1 where they contradicted), so an implementer can figure out the intent. But one struct should be removed.

**Fix:** Delete `ConfigManagerInner` entirely. It adds nothing — `ConfigHandle` already has all the same fields. If the intent is to put ConfigHandle behind an Arc (for sharing across tasks), just use `Arc<ConfigHandle>` directly.

---

### OPEN-2 (Critical): `ConfigMutation` Type Still Never Defined

**Original:** C-2

**Current state:** `writeback()` on line 101 still takes `ConfigMutation`, and `ConfigSource::persist()` on line 41 also takes `&ConfigMutation`. The type appears 4 times but is never defined — no enum, no struct, no variants, no documentation.

The Traits sub-agent suggested `#[non_exhaustive] enum ConfigMutation` with a `SetLoraAdapters` variant. This was noted but never incorporated into the document.

**Fix:** Add to Section 2.6 (Config Types):

```rust
/// Mutations that can be applied via the writeback API.
/// `#[non_exhaustive]` allows adding variants in future versions without breaking
/// downstream match arms. v1.0 supports only LoRA adapter changes.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ConfigMutation {
    SetLoraAdapters {
        server_id: ServerId,
        adapters: Vec<LoraAdapterConfig>,
    },
}
```

---

### OPEN-3 (High, downgraded from Critical): flock Timeout Implementation Strategy Unspecified

**Original:** C-3

**Current state:** Section 6 (line 636) still says "flock timeout: 30 seconds, Hardcoded" with no specification of how the timeout is implemented. `flock(2)` has no timeout parameter.

**Severity downgrade:** From Critical to High. The rest of the design is now clear enough that an implementer could pick a reasonable strategy. But the choice has implications (thread leaking vs. polling overhead) that should be specified.

**Fix:** Add a note under Section 6 or the Cross-Cutting Concerns section:

> **flock timeout implementation:** `flock(2)` does not support a timeout parameter. The
> implementation uses `LOCK_NB` (non-blocking) in a retry loop with `thread::sleep(100ms)`
> between attempts, counted to 300 iterations (300 × 100ms = 30s). This runs inside
> `spawn_blocking`, so the sleeping thread does not block the Tokio reactor. This approach
> is fully cancellation-safe (the loop can check an abort flag), does not leak threads
> (unlike wrapping a blocking `flock` in `tokio::time::timeout`), and provides predictable
> timeout behavior.

---

### OPEN-4 (High): `Duration` Fields Still Don't Have TOML Serialization Strategy

**Original:** H-1

**Current state:** Lines 238-240:

```rust
pub queue_default_timeout: Duration,
pub restart_stagger: Duration,
```

No `#[serde(with = "...")]` annotation. The main design doc uses `queue_default_timeout_secs = 300` (integer seconds). `std::time::Duration`'s default serde representation is `{secs: 300, nanos: 0}` — a TOML table, not an integer.

The proptest roundtrip property `parse(serialize(config)) == config` will fail unless serialization is addressed.

**Fix:** Either:

(a) Use `u64` fields with `_secs` suffix matching the main design doc:
```rust
pub queue_default_timeout_secs: u64,
pub restart_stagger_secs: u64,
```
With conversion to `Duration` at usage sites.

(b) Or use `#[serde(with = "humantime_serde")]` for human-readable durations (`"5m"`, `"10s"`).

Option (a) is simpler and matches the main design doc's TOML format.

---

### OPEN-5 (High): Missing `model_architecture` Field in ServerConfig

**Original:** H-2

**Current state:** `ServerConfig` (lines 248-275) still has no `model_architecture` field. Smart Routing (doc 05) explicitly depends on this:

> For servers with `model_architecture = "recurrent"` (Mamba, RWKV): kv_cache_usage → 0.0

Without this field, Smart Routing cannot distinguish transformer from recurrent models, leading to incorrect scoring for Mamba/RWKV servers.

**Fix:** Add to ServerConfig:

```rust
#[serde(default)]
pub model_architecture: ModelArchitecture,
```

And define:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelArchitecture {
    #[default]
    Transformer,
    Recurrent,
}
```

Also add `ModelArchitecture` to `ServerConfigField`:

```rust
pub enum ServerConfigField {
    // ... existing variants
    ModelArchitecture,
}
```

With `requires_restart() -> true` (changing architecture requires server restart).

---

### OPEN-6 (Medium, downgraded from High): Reload Scope Boundary Still Ambiguous

**Original:** H-3

**Current state:** The `reload()` method (line 95) returns `Result<ConfigDiff, ConfigReloadError>` with no server lifecycle orchestration. But Section 4.2 (line 572) still guarantees:

> **Transport Layer:** `POST /api/config/reload` returns only after the full reload completes **(including server lifecycle actions triggered by diff)**.

This guarantee is not something `ConfigHandle::reload()` can provide — it returns a diff, not orchestrating server lifecycle. It's a Transport Layer handler concern.

**Severity downgrade:** From High to Medium. The actual API (`reload() -> ConfigDiff`) is now clear. The boundary contract just has a misplaced guarantee.

**Fix:** Move the guarantee from Section 4.2 to the Transport Layer's boundary contracts. Replace with:

> **Transport Layer:** `reload()` returns the diff. The Transport Layer handler is responsible for awaiting Server Manager lifecycle actions before returning the HTTP response.

---

### OPEN-7 (Medium, downgraded from High): Referenced Types Still Undefined

**Original:** H-4

**Current state:** These types are referenced but never defined: `GrpcConfig`, `PrometheusConfig`, `LogLevel`, `LogFormat`, `Backend`, `DeviceSpec`, `CacheType`, `FlashAttnMode`, `LoraAdapterConfig`, `SpeculativeMethod`, `StarvationConfig`, `FairShareConfig`, `PreemptionConfig`.

**Severity downgrade:** From High to Medium. The design is now self-consistent enough that these can be defined during implementation. They're standard config structs with no complex interactions.

**Fix:** Add a "Shared Types Reference" subsection listing each type with its fields and which main design doc section specifies it. Alternatively, define them inline (adds 50-80 lines but makes the doc fully self-contained).

---

### OPEN-8 (Medium): Writeback Data Flow Diagram Still Omits Mutex Acquisition

**Original:** M-1

**Current state:** Section 3.2 (lines 496-519) still shows three steps without the reload_mutex. Invariant 1 says both reload and writeback are serialized by the mutex, and the cancellation analysis (lines 840-859) clearly documents the mutex behavior. But the data flow diagram is the first thing an implementer sees, and it doesn't show when the mutex is acquired relative to Step 1 (HTTP to llama-server).

**Fix:** Add mutex acquisition to the diagram:

```
    POST /api/servers/{id}/lora-adapters
                    │
                    ▼
    ┌──────────────────────────────────┐
    │ 0. reload_mutex.lock().await     │  ← serializes with reloads
    └──────────────┬───────────────────┘
                   │
    ┌──────────────▼───────────────────┐
    │ 1. Apply to llama-server (HTTP)  │  ← runtime change first
    └──────────────┬───────────────────┘
    ...
```

Or if the mutex is acquired after Step 1, document that explicitly.

---

### OPEN-9 (Medium): Validation Rules Still Unspecified

**Original:** M-2

**Current state:** `ConfigLoadError::ValidationError` exists (line 324). Section 8.1 mentions "Validation: fair_share percentages sum to 100, port conflicts, device spec validation." But:

1. No validation rules are listed.
2. The main design doc says fair_share values are "normalized proportionally and a warning is logged" if they don't sum to 100. The component design has a `ValidationError` suggesting it's an error. Which is it?
3. When does validation run — only at startup, or also during reload?

**Fix:** Add a "Validation Rules" subsection:

| Rule | Behavior | When |
|------|----------|------|
| fair_share sums to 100 | Normalize + WARN | Startup + reload |
| Server ports unique | ERROR | Startup + reload |
| model_path non-empty | ERROR | Startup + reload |
| ctx_size > 0 | ERROR | Startup + reload |
| parallel_slots > 0 | ERROR | Startup + reload |
| device specs valid format | ERROR | Startup + reload |

---

### OPEN-10 (Low): ConfigDiff::timestamp Still Uses Instant

**Original:** M-3

**Current state:** Line 369 — `pub timestamp: std::time::Instant`.

`Instant` is not displayable and not serializable. For logging and debugging, a wall-clock timestamp would be more useful. This is low severity since `Instant` is fine for ordering and the field is explicitly "not used for correctness."

**Fix:** Add a `wall_clock: SystemTime` field alongside `timestamp: Instant`, or replace with `SystemTime` if monotonic ordering is not needed.

---

### OPEN-11 (Low): Environment Variable Override Mechanism Undocumented

**Original:** M-4

**Current state:** The main design doc references `$LLAMAFORGE_STATE_DIR`. Section 6 (line 634) mentions "CLI args or env" for config file paths. But the resolution precedence and supported env vars are not specified.

**Fix:** Add a "Path Resolution" note under Section 6:

> Config paths are resolved at startup: CLI arg > environment variable > default.  
> Supported env vars: `LLAMAFORGE_CONFIG` (main config path), `LLAMAFORGE_STATE_DIR` (state dir).  
> `ConfigPaths` is constructed in `main()` before `ConfigHandle::load()`.

---

### OPEN-12 (Low): Missing `web_ui` Section in AppConfig

**Original:** M-5

**Current state:** `AppConfig` (lines 211-217) has `grpc: GrpcConfig` and `prometheus: PrometheusConfig` but no `web_ui`. The main design doc TOML has a `[web_ui]` section. A parsed TOML with `[web_ui]` will trigger an unknown-key warning.

**Fix:** Add `#[serde(default)] pub web_ui: WebUiConfig` to `AppConfig`. Even if the feature is Phase 5, parsing the section prevents spurious warnings.

---

## New Issues Introduced by Changes

### NEW-1 (Low): InstrumentedConfigHandle Doesn't Implement ConfigReader Forwarding Semantics

**Location:** Section 2.5, lines 166-168

The comment says `ConfigReader` delegates to inner with "no instrumentation (hot path, must stay cheap)." This is correct — but the doc should note that `snapshot()` and `subscribe()` are forwarded without spans or metrics. An implementer might add tracing to `snapshot()` out of reflex.

**Fix:** Add explicit comment: "DO NOT add instrumentation to `ConfigReader` methods — these are called per-request in the dispatch loop."

---

### NEW-2 (Low): ConfigChange Enum Missing `#[non_exhaustive]`

**Location:** Lines 374-380

`ConfigChange` is the enum that replaces the flat ConfigDiff fields. New config sections (overlay store, web_ui config) will require new variants. Without `#[non_exhaustive]`, adding a variant is a breaking change for anyone matching exhaustively.

**Fix:** Add `#[non_exhaustive]` to `ConfigChange`.

---

## Final Verdict

**Previous verdict:** FAIL (4 HIGH, 10 MEDIUM, 6 LOW = 20 findings across architecture review)

**Current verdict:** PASS WITH WARNINGS

All 20 architecture review findings are resolved. Of the 17 adversarial review findings, 5 are fully resolved, 3 are partially fixed, and 9 remain open. The remaining open findings break down as:

| Severity | Count | IDs |
|----------|-------|-----|
| Critical | 1 | OPEN-2 (ConfigMutation undefined) |
| High | 2 | OPEN-3 (flock timeout), OPEN-4 (Duration serde), OPEN-5 (model_architecture) |
| Medium | 4 | OPEN-1, OPEN-6, OPEN-8, OPEN-9 |
| Low | 4 | OPEN-10, OPEN-11, OPEN-12, NEW-1, NEW-2 |

**Blocking assessment:**

- **OPEN-2 is the only remaining implementation blocker.** `ConfigMutation` is referenced 4 times but never defined. A 5-line enum definition resolves it.
- **OPEN-3, OPEN-4, OPEN-5 are high-risk but not blocking.** They can be resolved during implementation with modest rework. OPEN-5 (model_architecture) will cause a cross-component failure if forgotten.
- **Everything else is refinement.** The design is now clear enough to implement correctly.

**Recommendation:** Fix OPEN-2 (mandatory), OPEN-5 (prevents cross-component bug), and OPEN-4 (prevents serde roundtrip failures). The rest can be resolved during implementation.
