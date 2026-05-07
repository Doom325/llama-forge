# Adversarial Review: Config Manager Component Design

**Document:** `01_CONFIG_MANAGER.md`  
**Reviewer:** Adversarial Design Reviewer  
**Date:** 2026-05-07  
**Methodology:** Cross-reference against main design doc (`llama-forge-design.md` §5.5), R3 review resolutions, and idiomatic Rust / Tokio patterns. Focus on implementation-blocking gaps, concurrency bugs, type system weaknesses, and boundary contract violations.

**Summary:** 17 findings — 3 Critical, 5 High, 6 Medium, 3 Low

---

## Critical Findings

### C-1: `ConfigHandle` vs `ConfigManagerInner` — Contradictory Struct Definitions

**Location:** Section 2.1 (lines 31-66) vs Section 3.3 (lines 314-320)

**Problem:** Two structs are defined with overlapping, contradictory fields:

```rust
// Section 2.1 — Public API
pub struct ConfigHandle {
    inner: arc_swap::ArcSwap<AppConfig>,
    reload_mutex: tokio::sync::Mutex<()>,
    writeback_notify: tokio::sync::Notify,  // never referenced anywhere else
}

// Section 3.3 — Internal Architecture
struct ConfigManagerInner {
    config: ArcSwap<AppConfig>,
    change_tx: tokio::sync::broadcast::Sender<ConfigDiff>,
    reload_mutex: tokio::sync::Mutex<()>,
    paths: ConfigPaths,
}
```

The relationship between these is never defined. Does `ConfigHandle` wrap `ConfigManagerInner`? Are they the same struct under different names? Key contradictions:

1. **`reload_mutex` appears in both.** If `ConfigHandle` wraps `ConfigManagerInner`, it would delegate to the inner mutex. But both define their own.
2. **`broadcast::Sender` is missing from `ConfigHandle`.** `ConfigHandle::subscribe()` returns a `broadcast::Receiver<ConfigDiff>`, which requires calling `sender.subscribe()` — but `ConfigHandle` has no sender. `ConfigManagerInner` has `change_tx` but is never referenced in the public API.
3. **`writeback_notify: Notify` in `ConfigHandle` is never used.** No method signature, data flow diagram, invariant, or failure mode references it. It's a phantom field.
4. **`ConfigPaths` stored in `ConfigManagerInner` but passed as parameter in `ConfigHandle` methods.** Both `reload()` and `writeback()` take `&ConfigPaths` as a parameter, yet `ConfigManagerInner` stores `paths: ConfigPaths`. If one wraps the other, the parameter is redundant. If they're separate, which stores the paths?

**Impact:** An implementer cannot determine the correct struct layout. This will block implementation on day one.

**Recommendation:** Unify into a single struct. Store `ConfigPaths`, `broadcast::Sender`, and `ArcSwap` together. Remove `writeback_notify` or document its purpose. Remove the `paths` parameter from `reload()` and `writeback()` if paths are stored internally.

---

### C-2: `ConfigMutation` Type Never Defined

**Location:** Section 2.1 (line 58)

**Problem:** `writeback()` accepts a `ConfigMutation` parameter:

```rust
pub async fn writeback(
    &self,
    paths: &ConfigPaths,
    mutation: ConfigMutation,
) -> Result<ConfigDiff, WritebackError>;
```

But `ConfigMutation` is never defined anywhere in the document — not in Section 2.3 (Config Types), not in Section 2.4 (Error Types), not in Section 3. There is no enum, struct, or even prose description of what mutations are possible.

**Impact:** The writeback API is unimplementable. The entire writeback data flow (Section 3.2) references "apply_mutation(parsed)" without defining what a mutation looks like, how it's validated, or what the variant set is.

**Recommendation:** Define `ConfigMutation` as an enum:

```rust
pub enum ConfigMutation {
    SetLoraAdapters {
        server_id: ServerId,
        adapters: Vec<LoraAdapterConfig>,
    },
    // v1.1: additional variants
}
```

Include validation rules (e.g., server_id must exist in current config).

---

### C-3: `flock` Timeout Is Specified but Physically Impossible with `flock(2)`

**Location:** Section 6 (line 416: "flock timeout: 30 seconds, Hardcoded")

**Problem:** The document specifies a 30-second timeout on `flock` acquisition. However, `flock(2)` has only two modes:

- `LOCK_EX` — blocks indefinitely until the lock is available
- `LOCK_EX | LOCK_NB` — returns immediately with `EWOULDBLOCK` if the lock is held

There is no built-in timeout parameter. To implement a 30-second timeout, you'd need one of:

1. **`LOCK_NB` retry loop with sleep** — introduces polling overhead and sleep granularity issues
2. **`tokio::time::timeout` around `spawn_blocking`** — but cancelling `spawn_blocking` doesn't interrupt the blocked `flock` call; the thread remains blocked in the kernel
3. **`alarm(2)` signal** — process-global, interferes with Tokio's signal handling
4. **`fcntl(F_SETLK)` with a separate timer thread** — different semantics from `flock`

Option 2 (the most likely approach) has a subtle problem: `tokio::time::timeout` will cancel the `spawn_blocking` future, but the blocking thread continues running `flock()` in the kernel indefinitely. The thread is leaked until the lock is released. Under pathological conditions (dead process holding lock), this leaks one blocking thread per timeout, eventually exhausting the blocking thread pool.

**Impact:** The timeout mechanism is unspecified and all obvious implementations have gotchas. An implementer who picks the wrong approach gets either leaked threads or broken semantics.

**Recommendation:** Specify the implementation strategy explicitly. Recommended: `LOCK_NB` in a retry loop with `thread::sleep(100ms)`, counted iterations (300 × 100ms = 30s), inside `spawn_blocking`. This is fully cancellation-safe (the loop can check a flag), doesn't leak threads, and provides predictable timeout behavior. Document that the blocking thread is occupied for up to 30s during contention.

---

## High Findings

### H-1: `Duration` Fields in Config Types Don't Roundtrip Through TOML

**Location:** Section 2.3 (lines 119-120)

**Problem:** `GlobalConfig` contains:

```rust
pub queue_default_timeout: Duration,
pub restart_stagger: Duration,
```

But `std::time::Duration` has no `serde::Serialize`/`Deserialize` implementation that produces human-readable TOML. The standard serde impl serializes as `{"secs": 300, "nanos": 0}` — a TOML table, not the integer `300` that the main design doc specifies (`queue_default_timeout_secs = 300`, line 1023).

The main design doc uses `_secs` suffix naming (`queue_default_timeout_secs`, `restart_stagger_secs`, `check_interval_secs`, `keepalive_interval_secs`), implying integer seconds in the TOML. The component design uses `Duration` types, implying a different serialization.

**Impact:** `export_toml()` will produce unreadable output. The proptest roundtrip property (`parse(serialize(config)) == config`) will fail. Config files won't match the documented TOML format.

**Recommendation:** Either use `u64` fields with `_secs` suffix (matching the main design doc) and convert to `Duration` at usage sites, or use `#[serde(with = "serde_duration_secs")]` custom serializer. Document the choice. The `_secs` suffix approach is simpler and consistent with the main design doc.

---

### H-2: Missing `model_architecture` Field in `ServerConfig`

**Location:** Section 2.3 (lines 129-157)

**Problem:** The main design doc TOML (line 1051) includes:

```toml
model_architecture = "transformer"
```

The Smart Routing component design (Section 3.2) explicitly depends on this field:

> For servers with `model_architecture = "recurrent"` (Mamba, RWKV): `kv_cache_usage` component → 0.0

But `ServerConfig` in the Config Manager design has no `model_architecture` field. This is not just a missing optional field — Smart Routing's scoring formula changes behavior based on it. Without it, recurrent models will be scored incorrectly (KV cache component will be non-zero for models that have no KV cache).

**Impact:** Cross-component contract violation. Smart Routing cannot function correctly for recurrent models.

**Recommendation:** Add `model_architecture: ModelArchitecture` to `ServerConfig` with `#[serde(default)]` defaulting to `Transformer`. Define `ModelArchitecture` as an enum: `{ Transformer, Recurrent }`.

---

### H-3: Reload Scope Boundary Confusion — Does `reload()` Include Server Lifecycle?

**Location:** Section 2.1 (line 54) vs Section 4.2 (line 352)

**Problem:** The `reload()` method signature returns `Result<ConfigDiff, ConfigReloadError>` — suggesting it parses TOML, diffs, and updates ArcSwap. But Section 4.2 guarantees:

> **Transport Layer:** `POST /api/config/reload` returns only after the full reload completes **(including server lifecycle actions triggered by diff)**.

The main design doc (lines 1224-1229) specifies Steps 1-5 under the reload mutex, where Steps 3-5 are server lifecycle operations (stop/restart/start servers). These are async operations that involve Server Manager — not Config Manager.

The ambiguity: does `ConfigHandle::reload()` include server lifecycle orchestration (Steps 3-5) or just the parse-diff-swap (Steps 1-2)?

- **If reload() includes Steps 3-5:** Config Manager depends on Server Manager, contradicting the dependency list (line 7: "Dependencies: Filesystem, Metrics & Storage"). It also means `ConfigReloadError` needs variants for server start/stop failures.
- **If reload() is Steps 1-2 only:** The boundary guarantee about "returns only after server lifecycle actions" is misplaced — it's a Transport Layer handler concern, not a Config Manager guarantee. But then: who holds the reload_mutex during Steps 3-5? If the mutex is released after Step 2, a second SIGHUP could trigger another reload before Step 5 completes, violating the serialization guarantee.

**Impact:** This boundary ambiguity will cause either circular dependencies or broken serialization guarantees.

**Recommendation:** Clarify that `reload()` performs Steps 1-2 (parse, diff, ArcSwap store, broadcast). The Transport Layer handler is responsible for Steps 3-5 (awaiting Server Manager actions). The reload_mutex must be held for the entire duration (Steps 1-5), which means `reload()` should accept a callback or return an RAII guard:

```rust
pub async fn reload(&self, paths: &ConfigPaths) -> Result<ReloadGuard, ConfigReloadError>;
// ReloadGuard holds the mutex; caller performs lifecycle actions; 
// mutex released on Drop.
```

Alternatively, `reload()` accepts a `Fn(ConfigDiff) -> Future` callback that executes under the mutex.

---

### H-4: Missing Type Definitions — `GrpcConfig`, `PrometheusConfig`, and Supporting Types

**Location:** Section 2.3 (lines 95-96)

**Problem:** `AppConfig` references several types that are never defined in this document:

- `GrpcConfig` — referenced at line 95, never defined. Main design doc (lines 1028-1035) shows `max_message_size_mb`, `keepalive_interval_secs`, `keepalive_timeout_secs`, `max_concurrent_streams`, and nested `tls` section.
- `PrometheusConfig` — referenced at line 96, never defined. Main design doc (lines 1037-1039) shows `enabled` and `endpoint`.
- `LogLevel`, `LogFormat`, `Backend`, `DeviceSpec`, `CacheType`, `FlashAttnMode`, `LoraAdapterConfig`, `SpeculativeMethod`, `StarvationConfig`, `FairShareConfig`, `PreemptionConfig` — all referenced in type definitions, none defined.

**Impact:** An implementer must reverse-engineer these from the main design doc or guess. The component design is supposed to be self-contained enough for implementation. Missing 13+ type definitions makes it incomplete.

**Recommendation:** Either define all types inline (adds verbosity but is complete), or add a "Shared Types" reference section that lists each type with its fields and references the main design doc section where it's specified. At minimum, `GrpcConfig` and `PrometheusConfig` must be defined since they're direct fields of `AppConfig`.

---

### H-5: `ServerId` as HashMap Key — Serde Deserialization and Ergonomics Unaddressed

**Location:** Section 2.3 (line 98)

**Problem:** `AppConfig` uses `HashMap<ServerId, ServerConfig>`. Per R3 resolution IR-1/IR-6, `ServerId` is a newtype: `ServerId(Arc<str>)`.

Using a newtype as a `HashMap` key with `serde` requires:

1. **`Deserialize` for `ServerId` as a map key.** TOML tables like `[servers.llama-8b-gpu0]` parse as `HashMap<String, _>`. With a newtype key, serde needs `impl Deserialize for ServerId` that converts from string. This works with `#[serde(transparent)]` or a custom impl — but it must be specified.
2. **`Hash` + `Eq` derived on `ServerId`.** `Arc<str>` implements `Hash` + `Eq` (comparing by content), so this works, but needs explicit `#[derive(Hash, Eq)]`.
3. **Ergonomic lookups.** `map.get("llama-8b-gpu0")` won't compile because `&str != &ServerId`. You need `impl Borrow<str> for ServerId` or callers must construct `ServerId` for every lookup. This is a frequent ergonomic pain point with newtypes.

None of this is addressed, and the newtype is defined in a different component (it's a shared type). Where does it live? Who owns the `ServerId` definition?

**Impact:** First implementer who tries to parse TOML into `HashMap<ServerId, ServerConfig>` will hit a wall.

**Recommendation:** Document that `ServerId` requires `#[derive(Hash, Eq, Serialize, Deserialize)]` with `#[serde(transparent)]`, and `impl Borrow<str> for ServerId`. Specify where shared newtypes are defined (likely a `types` crate or `common` module).

---

## Medium Findings

### M-1: Writeback Data Flow Doesn't Show Reload Mutex Acquisition

**Location:** Section 3.2 (lines 278-301)

**Problem:** The writeback data flow diagram shows three steps: (1) Apply to llama-server via HTTP, (2) spawn_blocking { flock, re-read, mutate, write }, (3) Update ArcSwap. The reload_mutex acquisition is not shown.

Invariant 1 (line 325) says: "Only one reload or writeback executes at a time (enforced by `reload_mutex`)."

But when is the mutex acquired relative to Step 1 (HTTP to llama-server)?

- **If before Step 1:** The reload_mutex is held during an HTTP request to llama-server (potentially seconds of latency). All reloads are blocked during LoRA changes. This is heavy-handed but safe.
- **If between Steps 1 and 2:** There's a window where the runtime change is applied to llama-server but the mutex isn't held. A concurrent reload could read the old TOML and broadcast a diff that doesn't include the LoRA change. The writeback then writes the mutation to disk and stores it. Subscribers see two diffs. This is correct but surprising — the LoRA change appears to "disappear" and "reappear" from the config perspective.

**Impact:** Ambiguous concurrency semantics. The "correct" interleaving depends on when the mutex is acquired, which isn't documented.

**Recommendation:** Add the mutex acquisition to the data flow diagram. Recommended: acquire mutex before Step 1 (conservative, simple). Document the latency implication explicitly.

---

### M-2: Validation Rules Referenced but Never Specified

**Location:** Section 2.4 (line 189) and Section 8.1 (line 473)

**Problem:** `ConfigLoadError::ValidationError` exists, and the testing strategy mentions "Validation: fair_share percentages sum to 100, port conflicts, device spec validation." But no validation logic is specified:

1. **What is validated?** Only fair_share sums? Or also: port uniqueness across servers, device spec format, model_path non-empty, ctx_size > 0, parallel_slots > 0?
2. **When does validation run?** After parse but before ArcSwap store? Only at startup? Also during reload?
3. **Normalization vs error:** The main design doc (line 1105) says fair_share values are "normalized proportionally (preserving ratios) and a warning is logged" if they don't sum to 100. But `ConfigLoadError::ValidationError` implies it's an error, not a normalization. Which is it?

**Impact:** Inconsistency between the main design doc (normalize + warn) and the component design (error). An implementer will make an arbitrary choice.

**Recommendation:** Add a "Validation Rules" subsection listing each validation, when it runs, and whether it errors or normalizes. Align with the main design doc: fair_share normalizes with a warning; other validations (port conflicts, invalid device specs) error.

---

### M-3: `ConfigDiff::timestamp` Uses `Instant` — Not Displayable, Not Serializable

**Location:** Section 2.5 (line 233)

**Problem:** `ConfigDiff` uses `std::time::Instant` for `timestamp`:

```rust
pub struct ConfigDiff {
    // ...
    pub timestamp: std::time::Instant,
}
```

Issues:

1. **`Instant` is not `Serialize`/`Deserialize`.** If `ConfigDiff` ever needs to be serialized (audit trail, JSON API response for reload status), this field prevents it.
2. **`Instant` is not displayable.** You can compute duration since another Instant, but you can't format it as a human-readable timestamp for logs. The log event `config.reloaded` (line 445) would want to show when the reload happened.
3. **`ConfigDiff` derives `Clone` but not `Serialize`.** This is fine for broadcast, but limits future use.

**Impact:** Minor now but creates tech debt. The first time someone tries to log or serialize a `ConfigDiff`, they'll need to refactor.

**Recommendation:** Use `std::time::SystemTime` (or `time::OffsetDateTime` if the `time` crate is in scope) for human-readable timestamps. Keep an `Instant` if you need monotonic ordering, but add a wall-clock field for display.

---

### M-4: No Environment Variable Override Mechanism Documented

**Location:** Section 2.3 and Section 3.3

**Problem:** The main design doc (line 1141) references `$LLAMAFORGE_STATE_DIR` as an environment variable override. The component design's `ConfigPaths` struct (line 307) has `state_dir: PathBuf` but never explains how the resolution works:

1. Does `state_dir` come from CLI args, env var, or the TOML `[global].state_dir` field?
2. What's the precedence? CLI arg > env var > TOML > default?
3. Other potential env overrides: `$LLAMAFORGE_CONFIG` for the main config path?
4. Where does `ConfigPaths` get constructed? `ConfigHandle::load()` takes `&ConfigPaths`, implying the caller resolves paths. But the resolution logic (including env vars) is undocumented.

**Impact:** Implementer must guess the path resolution strategy. Different choices lead to different CLI ergonomics.

**Recommendation:** Add a "Path Resolution" subsection documenting precedence: CLI arg > env var > default. List all supported env vars. Specify that `ConfigPaths` is constructed in `main()` before `ConfigHandle::load()`.

---

### M-5: Missing `web_ui` Section in `AppConfig`

**Location:** Section 2.3 (lines 91-99)

**Problem:** `AppConfig` includes `grpc: GrpcConfig` and `prometheus: PrometheusConfig`, but the main design doc TOML (lines 1041-1043) also has a `[web_ui]` section:

```toml
[web_ui]
enabled = true
serve_path = "/ui"
```

This is absent from `AppConfig`. If it's intentionally deferred (Phase 5), the document should say so. But `GrpcConfig` is included despite gRPC being Phase 4 (Transport Layer). The inconsistency suggests `web_ui` was simply forgotten.

Additionally, `[grpc.tls]` is shown in the main design doc (lines 1033-1035) as a nested section under `[grpc]`. If `GrpcConfig` contains TLS settings, those need to be modeled.

**Impact:** A parsed TOML with `[web_ui]` will trigger an "unknown key" warning (per Open Question 3), but operators following the main design doc will include it. Confusing.

**Recommendation:** Add `web_ui: WebUiConfig` to `AppConfig` with `#[serde(default)]`. Even if the feature is deferred, parsing the section prevents spurious warnings.

---

### M-6: Proptest Backward-Compatibility Strategy Is Fragile

**Location:** Section 8.3 (line 485)

**Problem:** The proposed property test is:

> `∀ config, ∀ subset of fields removed: parse(serialize(subset)) succeeds with defaults`

This is testing one direction (field removal = old config read by new code). But how do you generate "a subset of fields removed"? The obvious approach — remove random keys from the serialized TOML string — is brittle: removing a key mid-table can produce invalid TOML syntax.

Also, the reverse direction isn't tested: what if a v1.0 config file is read by v1.1 code that has new *required* fields? The `serde(default)` strategy handles this, but the property test should verify it explicitly by generating configs with fields from a "future" extended struct.

**Impact:** Property test may be hard to implement as specified and may miss the most important scenario (forward compatibility).

**Recommendation:** Define a concrete strategy: generate a full `AppConfig`, serialize to TOML, deserialize into a struct with fewer fields (simulating old code reading new config), and vice versa. Use two struct versions maintained in the test module.

---

## Low Findings

### L-1: Broadcast Channel Capacity of 16 May Be Tight Under Rapid Reloads

**Location:** Section 6 (line 415)

**Problem:** The broadcast channel has a hardcoded capacity of 16. If a subscriber is slow (e.g., Server Manager is in the middle of a server restart that takes 10+ seconds), and an operator sends SIGHUPs in a loop (or a CI script triggers reloads rapidly), the channel fills and the subscriber gets `Lagged`.

While this is handled (FM-5), the recovery path — "subscriber must re-read full config snapshot and compute its own diff" — is complex and essentially duplicates the diff logic that Config Manager already computed. Every subscriber must implement this recovery path or risk missing changes.

**Impact:** Low in practice (rapid SIGHUP is rare), but the recovery path is non-trivial for every consumer.

**Recommendation:** Consider capacity of 64 (still bounded, 64KB max buffer). Or document that consumers MUST implement the lagged-recovery path and provide a helper function for it.

---

### L-2: `ServerConfigDiff::fields_changed` Is Stringly Typed

**Location:** Section 2.5 (line 239)

**Problem:** `fields_changed: Vec<String>` is a list of field names as strings. This is fragile:

1. If a field is renamed during refactoring, the string won't be updated by the compiler.
2. Consumers who pattern-match on field names (e.g., `if fields_changed.contains(&"model_path".to_string())`) get no compile-time safety.
3. What are the valid values? "model_path"? "ctx_size"? "lora_adapters"? Not documented.

**Impact:** Low — `fields_changed` is primarily for logging/debugging, and the `requires_restart` / `hot_reloadable` booleans are the actionable fields. But it's a maintenance footgun.

**Recommendation:** Define a `ServerConfigField` enum and use `Vec<ServerConfigField>`. Or if it's logging-only, rename to `fields_changed_display: Vec<String>` to signal it's not for programmatic use.

---

### L-3: No Mention of `#[deny(unknown_fields)]` Option for Strict Parsing

**Location:** Section 9, Open Question 3 (line 539)

**Problem:** Open Question 3 discusses unknown TOML keys, deciding to "log a warning for unknown keys at INFO level during parse." But `serde` doesn't provide an INFO-level warning mechanism. The options are:

1. `#[serde(deny_unknown_fields)]` — hard error on unknown keys (too strict for backward compat)
2. Default behavior — silently ignore unknown keys (too permissive, typos go unnoticed)
3. Custom deserializer that collects unknown keys and returns them alongside the parsed config

Option 3 is what's implied but it requires a non-trivial custom deserializer (or using `toml::Value` as a first pass, diffing known keys against parsed keys). This is more work than the casual "log a warning" phrasing suggests.

**Impact:** Low — the feature is nice-to-have. But it shouldn't be listed as a resolved decision when the implementation complexity isn't acknowledged.

**Recommendation:** Either defer unknown-key warnings to v1.1, or document the two-pass parse strategy: parse to `toml::Value` first, extract known keys, log any remaining keys as warnings, then deserialize from the Value.

---

## Summary Table

| ID | Severity | Category | One-line Summary |
|----|----------|----------|------------------|
| C-1 | Critical | Architecture | `ConfigHandle` vs `ConfigManagerInner` contradictory definitions |
| C-2 | Critical | Completeness | `ConfigMutation` type never defined |
| C-3 | Critical | Implementation | `flock` timeout physically impossible with `flock(2)` as specified |
| H-1 | High | Type Safety | `Duration` fields don't roundtrip through TOML serialization |
| H-2 | High | Cross-Component | Missing `model_architecture` field breaks Smart Routing |
| H-3 | High | Boundary | Reload scope ambiguity — does `reload()` include server lifecycle? |
| H-4 | High | Completeness | 13+ referenced types never defined |
| H-5 | High | Type Safety | `ServerId` newtype as HashMap key — serde/ergonomics unaddressed |
| M-1 | Medium | Concurrency | Writeback data flow omits reload_mutex acquisition point |
| M-2 | Medium | Correctness | Validation rules referenced but never specified; normalization vs error conflict |
| M-3 | Medium | Type Design | `ConfigDiff::timestamp` uses non-serializable `Instant` |
| M-4 | Medium | Completeness | Environment variable override mechanism undocumented |
| M-5 | Medium | Completeness | Missing `web_ui` section in `AppConfig` |
| M-6 | Medium | Testing | Proptest backward-compat strategy is fragile as specified |
| L-1 | Low | Capacity | Broadcast channel capacity of 16 may be tight |
| L-2 | Low | Type Safety | `fields_changed` is stringly typed |
| L-3 | Low | Implementation | Unknown-key warning requires non-trivial custom deserializer |

---

## Blocking Assessment

**Implementation-blocking findings (must resolve before coding):** C-1, C-2, C-3, H-3

These four findings make the component unimplementable as currently specified. The struct layout is contradictory (C-1), a core type is missing (C-2), a specified behavior is physically impossible (C-3), and a critical boundary is ambiguous (H-3).

**Should-resolve findings (high risk of rework if deferred):** H-1, H-2, H-4, H-5

These will cause compilation failures or cross-component breakage during implementation. Resolving them during implementation is possible but wasteful — the design should specify the answer.

**Can-defer findings (implement and revisit):** M-1 through M-6, L-1 through L-3

These are real issues but can be resolved during implementation without rework risk.
