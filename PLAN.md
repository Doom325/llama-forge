# Server Manager — Implementation Plan

**Component:** Server Manager (`03_SERVER_MANAGER.md`)  
**Estimated LOC:** ~3,400  
**Estimated Duration:** 13–15 working days (single engineer) / 8–9 days (two engineers, PR4 and PR5 can be developed in parallel)  
**Design Document:** `docs/component-designs/03_SERVER_MANAGER.md`  
**Branch Prefix:** `server-manager/`  
**Target:** `main` (via squash-merge PRs)  
**Prerequisite:** Config Manager (PR1–PR7 merged) and Metrics & Storage (PR1–PR8 merged). Depends on `ConfigHandle`, `MetricsIngest`, `MetricsStore`, and `ServerId`.  
**Codebase Location:** `~/git/llama-forge` (Cargo workspace root)  
**Existing Crates:** `llamaforge` (binary), `llamaforge-config` (implemented), `llamaforge-metrics` (implemented), `llamaforge-proto` (stub)

---

## Ground Rules for CoderAgents

1. **Read the design doc first.** Every PR description must reference the specific design doc sections it implements.
2. **No `unwrap()` or `expect()` anywhere** — not in production code, except where statically impossible (e.g. arithmetic bounds with comments). Panics in the server task can lead to unmanaged zombie processes.
3. **Graceful before forceful.** SIGTERM always precedes SIGKILL (with a 10s timeout between).
4. **Child process reaping is sacred.** `child.wait()` must always be awaited in the server task. No zombie processes allowed.
5. **State is a projection.** `ServerState` must reflect reality. Do not transition to `Running` until `/health` actively passes.
6. **VRAM Ledger consistency.** Ledger entries exist ONLY for servers in the `Starting` state. Remove them on ANY process exit or transition to `Running`.
7. **Every public type gets a doc comment.** Reference the design doc section number (e.g., `/// See design doc Section 3.2.`).
8. **Tests use `mock-llama`.** Do not require an actual `llama-server` binary or GPU for unit/integration tests. All integration tests drive against the `mock-llama` test binary (PR4 task 4.6), configured via env-var knobs (`MOCK_HEALTH_FAIL_AFTER`, `MOCK_HANG_AFTER`, `MOCK_EXIT_AFTER_SECS`, `MOCK_MODEL_ALIAS`, `MOCK_TPS`). No `httpmock`, no shell scripts.
9. **Commit messages:** `server-manager: <imperative summary>` (e.g., `server-manager: add deadlock detection logic`).

### Conventions from Previous Components (MUST follow)

10. **Module visibility:** All modules declared as `pub(crate) mod` in `lib.rs`, with explicit `pub use` re-exports for the public API.
11. **Sealed traits:** All *consumer-facing* internal traits use the sealed trait pattern (`trait Sealed {}` in a `private` module). **Exception:** port traits (`GpuTelemetry`, `MetricsStore`, `ConfigOverlayStore`, …) MUST NOT be sealed — they are dependency-injection seams and need free external impls for adapters and test mocks.
12. **`#[non_exhaustive]`:** All public enums get `#[non_exhaustive]` for forward compatibility.
13. **`metrics` crate:** Use the `metrics` facade crate (`counter!`, `gauge!`) for observability, NEVER `prometheus` directly.
14. **`#[allow(async_fn_in_trait)]`** on async trait definitions. No `#[async_trait]` macro.

---

## Dependency Graph (PR merge order)

```
PR1 ─── PR2 ─── PR3 ─── PR4 ─── PR5 ─── PR6 ─── PR7 ─── PR8
                          │               │
                          └── can overlap ─┘
                       (PR4 task logic vs PR5 polling)
```

PR1 → PR2 → PR3 → PR4 → PR5 → PR6 → PR7 → PR8 (strict merge ordering; PR4 code and PR5 code can be developed concurrently after PR3 merges, but integration requires PR4 to be merged first).

---

## PR1: Crate Setup & Domain Types

**Branch:** `server-manager/01-types`  
**Merge Target:** `main`  
**Size:** Medium (~400 lines)  
**Estimated Time:** 4–5 hours

### Purpose

Create the `llamaforge-server-manager` library crate, define all domain types from design doc Section 2.3, and error types from Section 2.4. This is the type foundation — every other PR builds on it.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [x] 1.0 | **Pre-flight: verify `ManagedProcess` contract** | Before any code in this crate, confirm `llamaforge-metrics::types::ManagedProcess` carries the fields required for orphan recovery (PR7 Task 7.4) and PID persistence (PR4 Task 4.5): `pid: ProcessId`, `port: u16`, `model_alias: String`, `server_id: ServerId`, `started_at: chrono::DateTime<Utc>`. If any field is missing, land a small additive PR in `llamaforge-metrics` first (extend the struct, the SQL migration, and the row DTO) — block this crate's work on its merge. This is a hard cross-component contract; PR4 will not compile without it. | (verification step; no files in this crate) |
| [x] 1.1 | Add `llamaforge-server-manager` to workspace | New lib crate `crates/llamaforge-server-manager`. Add to workspace members. Depends on `llamaforge-config` (for `ServerId`, `ConfigHandle`) and `llamaforge-metrics`. | `Cargo.toml`, `crates/llamaforge-server-manager/Cargo.toml` |
| [x] 1.2 | Wire workspace dependencies | Inherit (already in `[workspace.dependencies]` from Config Manager / Metrics): `serde`, `thiserror`, `tokio` (with `process`, `sync`, `signal`, `macros`, `rt-multi-thread`), `tracing`, `dashmap`, `reqwest` (with `json`), `parking_lot`, `nix` (with `signal`, `process`), `tokio-util` (with `rt`) for `CancellationToken` + `JoinSet`, `metrics`, `chrono`. Inherit dev-deps: `tempfile`, `proptest`, `tokio` (with `test-util` for `tokio::time::pause()`/`advance()`), `tracing-subscriber` (with `env-filter`, for `#[traced_test]` in unit tests asserting on log events), `typed-builder`, `metrics-util` (with `debugging` feature, for VC-3.6's in-process recorder). Add `axum` and `tower` as **`test-utils`-feature-gated** deps (not dev-deps) — they're consumed by the `mock-llama` binary in Task 4.6 which is itself feature-gated. If any are not yet in `[workspace.dependencies]`, add there first (workspace-deps-only convention). | `Cargo.toml` (workspace), `crates/llamaforge-server-manager/Cargo.toml` |
| [x] 1.3 | Create module structure | `lib.rs` declares `pub(crate) mod`: `types`, `error`, `state`, `telemetry` (GPU FFI port — `GpuTelemetry` trait), `manager`, `task`, `process`, `health`, `vram_estimate`, `observability` (Prometheus metric registration via the `metrics` facade — separate from `telemetry.rs` to avoid the GPU-FFI/Prometheus-metric collision), `instrumented`. Stub every file. Explicit `pub use` re-exports for the public API at the bottom of `lib.rs` (populated as tasks implement them). Follow the exact pattern from `llamaforge-config/src/lib.rs` and `llamaforge-metrics/src/lib.rs`. | `src/lib.rs`, `src/*.rs` |
| [x] 1.4 | Implement `ServerLifecycle` enum | `#[non_exhaustive]`. Variants: `Stopped`, `Starting`, `Running`, `Draining`, `Restarting`, `Failed`. Derive `Debug, Clone, Copy, PartialEq, Eq`. | `src/types.rs` |
| [x] 1.5 | Implement `ServerCapabilities` & `SlotState` | From Section 2.3. Derive `Debug, Clone`. `SlotState` includes `id`, `is_processing`, `n_past`, `n_decoded`, `prompt_hash`. `ServerCapabilities` includes `models: Vec<String>`, `lora_adapters: Vec<String>`, `multimodal: bool`, `speculative: SpeculativeMethod`, `architecture: ModelArchitecture`. | `src/types.rs` |
| [x] 1.5a | Implement `SpeculativeMethod` enum | `#[non_exhaustive]`. Variants per llama.cpp: `None`, `Draft`, `Lookup`, `Mtp`. Derive `Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize`. | `src/types.rs` |
| [x] 1.5b | Implement `ModelArchitecture` enum | `#[non_exhaustive]`. Variants for common families: `Llama`, `Mistral`, `Mixtral`, `Qwen`, `Gemma`, `Phi`, `DeepSeek`, `Other(String)`. Derive `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`. | `src/types.rs` |
| [x] 1.6 | Implement `ServerStateSnapshot` | **Public** read-only snapshot returned by accessors. Fields: `id`, `lifecycle`, `pid: Option<u32>`, `port`, `model_alias`, `parallel_slots`, `ctx_size`, `slots`, `last_health_check`, `restart_count: u32`, `cumulative_requests: u64`, `capabilities`. Derive `Debug, Clone`. | `src/state.rs` |
| [x] 1.6a | Implement internal `ServerState` | **Internal mutable state**, `pub(crate)` only. Stored in `Arc<DashMap<ServerId, ServerState>>`. Fields: all of `ServerStateSnapshot`'s data PLUS internal bookkeeping: `first_health_ok_at: Option<Instant>` (used by deadlock detection), `last_n_decoded: HashMap<u32, u32>` (per-slot decode counter for deadlock detection), `gpu_id: GpuId`, `gguf_metadata: Option<Arc<GgufMetadata>>` (cached after first parse, PR3). Methods: `pub(crate) fn to_snapshot(&self) -> ServerStateSnapshot`. Task-local state (not in DashMap) stays in the per-server task: `consecutive_health_failures: u32`, `consecutive_degraded_ticks: u32`, `last_total_tokens_decoded: u64`, `warned_overflow: AtomicBool` (this last one is shared via `Arc` if accessed from accessors). Doc-comment which fields live where and why. | `src/state.rs` |
| [x] 1.7 | Implement events & errors | `ClientRefreshEvent { server_id: ServerId, new_port: u16 }` (`Debug, Clone`). `#[non_exhaustive] ServerManagerError` using `thiserror` with 6 variants per design Section 2.4: `ServerNotFound { id }`, `InsufficientVram { gpu, needed_mb, free_mb }`, `StartFailed { id, reason }`, `MaxRestartsExceeded { id, max }`, `SpawnError { source: std::io::Error }`, `ConfigInvalid { message }`. | `src/types.rs`, `src/error.rs` |
| [x] 1.8 | Unit tests: type construction | Construct types, verify `Debug` and `Clone` behavior. Verify `ServerManagerError` formatting. Verify all public enums are `#[non_exhaustive]` (compile-time match must use wildcard). Verify `ServerState::to_snapshot()` round-trips matching fields. | `src/types.rs` (tests) |

### PR1 Acceptance Criteria

- [ ] `ManagedProcess` pre-flight (Task 1.0) confirmed — required fields present in `llamaforge-metrics`, or an additive PR has landed
- [ ] `cargo build --workspace` succeeds with zero warnings
- [ ] `llamaforge-server-manager` inherits workspace deps and uses Config types
- [ ] All public enums have `#[non_exhaustive]`; matches must use wildcard arm
- [ ] `lib.rs` uses `pub(crate) mod` + explicit `pub use` pattern; module list includes `vram_estimate` and `instrumented` stubs
- [ ] `ServerState::to_snapshot()` produces a snapshot whose fields exactly match `ServerStateSnapshot` (round-trip test passes)
- [ ] `SpeculativeMethod` and `ModelArchitecture` serde round-trip (TOML and JSON)

### PR1 Creation Steps

```bash
git checkout -b server-manager/01-types main
# ... implement tasks ...
git add -A && git commit -m "server-manager: add crate, domain types, and errors"
git push -u origin server-manager/01-types
```

---

## PR2: GPU Telemetry Trait & Implementations

**Branch:** `server-manager/02-gpu-telemetry`  
**Base:** `server-manager/01-types`  
**Size:** Small (~200 lines)  
**Estimated Time:** 2–3 hours

### Purpose

Define the `GpuTelemetry` trait (Section 2.2) and its fallback implementation.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [x] 2.1 | Define `GpuDevice` struct | Basic struct representing a GPU (ID, Name, Total VRAM). | `src/telemetry.rs` |
| [x] 2.2 | Define `GpuTelemetry` trait | `free_vram_bytes`, `utilization_percent`, `list_devices`. `Send + Sync + 'static`. **NOT sealed** — this is a port trait per design Section 2.2; feature-gated adapters (`NvmlTelemetry`, `MetalTelemetry`, `RocmTelemetry`) and downstream-component test mocks must be free to implement it. Object-safe (used as `Arc<dyn GpuTelemetry>` in `ServerManager`). | `src/telemetry.rs` |
| [x] 2.3 | Define `TelemetryError` | Variants: `Unavailable`, `DeviceNotFound`, `FfiError`. | `src/telemetry.rs` |
| [x] 2.4 | Implement `StubTelemetry` | Returns `Err(TelemetryError::Unavailable)` for all trait methods. Used as fallback. | `src/telemetry.rs` |
| [x] 2.5 | Cargo feature flags | Add `gpu-nvml`, `gpu-metal`, `gpu-rocm` to Cargo.toml (empty features for now, implementations deferred). | `Cargo.toml` |
| [x] 2.6 | Unit tests: telemetry | Test `StubTelemetry` returns correct error variants. | `src/telemetry.rs` (tests) |

### PR2 Acceptance Criteria

- [x] Trait compiles and is object-safe
- [x] `StubTelemetry` provides a safe fallback
- [x] Feature flags correctly defined in Cargo.toml

### PR2 Creation Steps

```bash
git checkout -b server-manager/02-gpu-telemetry main
# ... implement tasks ...
git add -A && git commit -m "server-manager: add GpuTelemetry trait and StubTelemetry fallback"
git push -u origin server-manager/02-gpu-telemetry
```

---

## PR3: Core State Management, VRAM Ledger & GGUF Estimator

**Branch:** `server-manager/03-core-state`  
**Base:** `server-manager/02-gpu-telemetry`  
**Size:** Large (~500 lines)  
**Estimated Time:** 10–12 hours (1.5 days — GGUF header parsing is the bulk of the work)

### Purpose

Implement the `ServerManager` shell, `DashMap` for state tracking, and the VRAM Ledger logic (Section 3.3).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [x] 3.1 | Implement `ServerManager` shell | Struct fields: `servers: Arc<DashMap<ServerId, ServerState>>`, `vram_ledger: Arc<parking_lot::Mutex<HashMap<GpuId, u64>>>` (**`parking_lot::Mutex`** — `std::sync::Mutex::lock().unwrap()` violates Ground Rule 2), `gpu_telemetry: Arc<dyn GpuTelemetry>`, `config: Arc<ConfigHandle>`, `metrics_ingest`, `metrics_store: Arc<dyn MetricsStore>`, `shutdown_token: CancellationToken`, `client_refresh_tx: tokio::sync::broadcast::Sender<ClientRefreshEvent>` (capacity 32; subscribers via Task 3.5), **`pending_dispatches: Arc<DashMap<ServerId, AtomicU64>>`** (introduced here, not PR7, because PR6's `is_deadlocked()` reads it as condition 1 "active work"; PR7 just bolts on the drain semantics). | `src/manager.rs` |
| [x] 3.1a | Implement `pending_dispatches` API surface | Three `pub` methods on `ServerManager` (must be `pub`, not `pub(crate)`, because the future `llamaforge-dispatch` crate calls them): `pub fn pending_dispatches(&self, id: &ServerId) -> u64`, `pub fn increment_pending(&self, id: &ServerId)`, `pub fn decrement_pending(&self, id: &ServerId)`. All atomic via `AtomicU64::fetch_add`/`fetch_sub`/`load` with `Ordering::Relaxed`. Lazily insert an entry on first `increment_pending` for an id. **v1:** Dispatch & Proxy (component 06) is not yet built, so callers don't exist and the gauge stays at 0 — that's intentional and the drain path in PR7 handles it. | `src/manager.rs` |
| [x] 3.2 | Implement VRAM Ledger | `pre_spawn_vram_check(&ServerConfig, &GgufMetadata) -> Result<(), ServerManagerError>` per design Section 3.3. Caller must have parsed (or fetched cached) `GgufMetadata` first (Task 3.2a) — this function does NO file I/O. Acquire `parking_lot::Mutex` synchronously (no `.await` while held — Cancellation Safety, design Section 9.1), query `gpu_telemetry.free_vram_bytes()` (fallback to `u64::MAX` on `Unavailable` + WARN log), subtract ledger reservations, compare against `compute_vram_estimate(config, meta)`. Return `InsufficientVram { gpu, needed_mb, free_mb }` on failure. On success, record reservation. Separate `release_vram_reservation(gpu, bytes)` and `clear_vram_reservation(server_id)` methods for use on exit / Starting→Running transition. | `src/manager.rs` |
| [x] 3.2a | Implement GGUF metadata parser | Per design Section 9 Open Question 3 (decision: "Rust implementation"). New module `src/vram_estimate.rs`. **All types `pub(crate)`** — they are internal to VRAM estimation, no external consumer needs them; if that ever changes, promote and add to `lib.rs` re-exports. `pub(crate) struct GgufMetadata { architecture: ModelArchitecture, block_count: u32, embedding_length: u32, head_count: u32, head_count_kv: u32, quantization: Quantization, file_size_bytes: u64 }`. `pub(crate) enum Quantization { Q4_K_M, Q5_K_M, Q6_K, Q8_0, F16, F32, Other(String) }` (`#[non_exhaustive]`). `pub(crate) enum GgufError { Malformed, Io(std::io::Error), UnsupportedVersion(u32) }` (`#[non_exhaustive]`, `thiserror`). **Derive `Debug, thiserror::Error` only — NOT `Clone`** (`std::io::Error` is not `Clone`). If a `Clone`-able shape is ever needed later, wrap as `Io(Arc<std::io::Error>)`. Parser entry point: `pub(crate) async fn parse_gguf(path: &Path) -> Result<GgufMetadata, GgufError>`. **The actual file read MUST run on `tokio::task::spawn_blocking` with parent-span propagation** — sync `std::fs::File` reads on the tokio reactor violate Cancellation Safety. Idiomatic pattern: <pre>let span = tracing::Span::current();<br>tokio::task::spawn_blocking(move \|\| {<br>    let _enter = span.enter();<br>    sync_parse(path)<br>}).await?</pre> Defensive parser: bounded buffer (read first 1 MiB max), validate magic `GGUF`, version 2 or 3 (reject others with `UnsupportedVersion`), reject if KV header declares size > 1 MiB. On any parse error return the appropriate `GgufError` variant and let caller decide (PR4 Task 4.2 specifies the fallback). | `src/vram_estimate.rs` |
| [x] 3.2a-i | Implement `compute_vram_estimate` | `pub(crate) fn compute_vram_estimate(config: &ServerConfig, meta: &GgufMetadata) -> u64`. If `config.estimated_vram_mb.is_some()` → return that (operator override). Otherwise: weight footprint by quant type (table-driven bytes-per-param: Q4_K_M ≈ 4.5 bits/p, Q5_K_M ≈ 5.5, Q6_K ≈ 6.6, Q8_0 ≈ 8.5, F16 = 16, F32 = 32). KV cache: `2 * block_count * head_count_kv * (embedding_length / head_count) * ctx_size * bytes_per_elem * parallel_slots`. Add 10% safety margin. Pure-function, no I/O — easy to unit test. | `src/vram_estimate.rs` |
| [x] 3.2a-ii | Cache `GgufMetadata` on `ServerState` | After first successful parse (in the per-server task spawn path), store `Arc<GgufMetadata>` on `ServerState` (field added in PR1 Task 1.6a). Re-spawns reuse the cache; only a config diff that changes `model_path` invalidates it (handled in PR7). | `src/manager.rs` |
| [x] 3.2b | Implement `shutdown_drain_timeout_secs` resolver | Per design Section 6: `Auto: max(60, min(ctx_size/30, 600))`. Helper `fn resolve_drain_timeout(server: &ServerConfig) -> Duration`: returns `Duration::from_secs(config.shutdown_drain_timeout_secs)` if set, else applies the formula. Unit-tested separately (design Section 8.1 lists "Drain timeout formula" as a pure-logic target). | `src/manager.rs` |
| [x] 3.3 | Implement state accessors | `server_state()`, `running_servers()`, `slot_states()`. Extract snapshots from the `DashMap`. `running_servers()` filters to `ServerLifecycle::Running` only (never `Starting`/`Failed`/`Stopped` — guarantee to Transport Layer per design Section 4.2). | `src/manager.rs` |
| [x] 3.4 | Implement cumulative requests | `cumulative_requests()`, `increment_requests()`. Log warning **once** at `i32::MAX / 2` (latch a `warned_overflow: AtomicBool` so the warning fires exactly once per server lifetime — counter is `u64` so increments continue safely; warning is an operator advisory to rolling-restart, per design Section 3.5 invariant 7). | `src/manager.rs` |
| [x] 3.5 | Implement `subscribe_client_refresh` | Setup `tokio::sync::broadcast` channel for `ClientRefreshEvent` with capacity 32. Stored in `ServerManager`; `subscribe_client_refresh(&self) -> broadcast::Receiver<ClientRefreshEvent>`. | `src/manager.rs` |
| [x] 3.6 | Unit tests: VRAM logic | Test `pre_spawn_vram_check` arithmetic (effective_free = nvml_free - reserved). Test reservation add/remove. Test `Unavailable` telemetry falls back to skip-check. | `src/manager.rs` (tests) |
| [x] 3.6a | Unit tests: VRAM estimation | Per design Section 8.1 "VRAM estimation formula (various quant types, ctx sizes, architectures)". Table-driven `compute_vram_estimate` tests: (Q4_K_M, 7B-class meta, ctx 4096) → expected MB; (Q8_0, 13B-class, ctx 8192) → expected MB; explicit `estimated_vram_mb` override wins over formula. GGUF parser test against a **generated fixture** — implement `fn make_tiny_gguf(tmp: &TempDir, arch: &str, quant: Quantization) -> PathBuf` test helper that writes ≤8KB of synthetic header bytes (magic `GGUF`, version 3, minimal KV block, no real tensor data) to a tmpdir per run. Keeps no binary in the repo. Include a malformed-bytes test (truncate the header) → asserts `GgufError::Malformed`; an unsupported-version test (write version 1) → asserts `GgufError::UnsupportedVersion(1)`. | `tests/vram_estimate.rs`, `tests/helpers/gguf_fixture.rs` |
| [x] 3.6b | Unit tests: drain timeout formula | `ctx_size=0` → 60s. `ctx_size=3000` → 100s. `ctx_size=20000` → 600s (capped). Explicit override wins. | `src/manager.rs` (tests) |
| [x] 3.7 | Unit tests: accessors | Test `running_servers` filters non-Running states correctly. Test `cumulative_requests` warning latches at `i32::MAX/2`. | `src/manager.rs` (tests) |

### PR3 Acceptance Criteria

- [x] `ServerManager` can be instantiated
- [x] VRAM ledger arithmetic is correct and synchronous (no `.await` over Mutex)
- [x] Broadcast channel provides subscribers correctly
- [x] GGUF header parser runs under `spawn_blocking` and produces correct metadata for fixture
- [x] Malformed GGUF returns `GgufError::Malformed` (no panic)
- [x] `compute_vram_estimate` is within 15% of measured allocation on the fixture; explicit override wins
- [x] Drain timeout formula matches design Section 6: 60s floor, ctx-scaled middle, 600s cap
- [x] Cumulative request warning latches at `i32::MAX/2` (fires exactly once per server lifetime)

### PR3 Creation Steps

```bash
git checkout -b server-manager/03-core-state main
# ... implement tasks ...
git add -A && git commit -m "server-manager: implement core state DashMap and VRAM ledger"
git push -u origin server-manager/03-core-state
```

---

## VC-1: Contracts & State Verification Checkpoint

**Gate:** PR1 + PR2 + PR3 merged. Data structures and basic logic in place.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-1.1 | State Filtering | `running_servers()` correctly ignores Failed/Stopped/Starting. | Tests pass |
| VC-1.2 | VRAM Math | Pre-spawn check correctly subtracts ledger from raw GPU free VRAM. | Tests pass |
| VC-1.3 | Mutex Safety | `parking_lot::Mutex` for VRAM ledger is not held across `.await` points. | Manual code review |
| VC-1.4 | VRAM Estimator | GGUF parser + estimator produce sane values within 15% of measured llama-server allocation on a fixture model. | Tests pass |
| VC-1.5 | Convention Compliance | All public enums `#[non_exhaustive]`. All modules `pub(crate)` with explicit `pub use`. `GpuTelemetry` is NOT sealed. | `rg "pub mod" src/lib.rs` returns 0; manual audit |

---

## PR4: Process Execution & Server Task Shell

**Branch:** `server-manager/04-process-exec`  
**Base:** `server-manager/03-core-state`  
**Size:** Large (~550 lines)  
**Estimated Time:** 10–12 hours (1.5 days — the restart loop + cancellation-safe cleanup + `mock-llama` are each non-trivial)

### Purpose

Implement the main per-server Tokio task, process spawning, and child wait loop (Section 3.1 & 3.2).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [x] 4.1 | Implement `start()` method | Iterates over configured servers, calls `spawn_server_task` for each. Collects per-server handles into a `JoinSet` owned by `ServerManager` for coordinated shutdown (design "Graceful Shutdown" cross-cutting concern). **Composition signature:** `start()` returns `Result<Arc<ServerManager>, ServerManagerError>` (bare manager, matching design Section 2.1). The composition root in `main.rs` wraps it: `let mgr = InstrumentedServerManager::new(ServerManager::start(...).await?);` — keeps `ServerManager` independently testable without the metrics decorator and keeps telemetry-out-of-domain (R26). Consumers (Smart Routing, Dispatch & Proxy) hold the wrapped `Arc<InstrumentedServerManager>` (which delegates by `Deref`). | `src/manager.rs` |
| [x] 4.2 | Implement `spawn_server_task` skeleton | `tokio::spawn` the per-server task. **Outer restart loop** owns spawn/restart cycles: `loop { parse_gguf_or_fallback → pre_spawn_vram_check → spawn child → run monitor_loop → if Failed or shutdown: break; else: backoff_sleep(restart_count); restart_count += 1; }`. After loop exits, run final cleanup (kill child if still alive, clear ledger, remove `managed_processes` row). **GGUF parse-failure handling** (uses `GgufError` from Task 3.2a): on first spawn, call `parse_gguf(model_path).await`. (a) `Ok(meta)` → cache `Arc<GgufMetadata>` on `ServerState`, proceed. (b) `Err(GgufError::Io(_))` (file missing/unreadable) → log ERROR `server.gguf_unreadable`, transition to `Failed`, break out of restart loop. (c) `Err(GgufError::Malformed)` or `Err(GgufError::UnsupportedVersion(_))` → if `config.estimated_vram_mb.is_some()`: log WARN `server.gguf_parse_failed` (include path + error), construct a placeholder `GgufMetadata::override_only(estimate_mb)` so the cache field still exists, proceed (estimate path uses the override anyway). Else: log ERROR `server.gguf_required`, transition to `Failed`, break. Cached metadata is reused on re-spawns; invalidated only when PR7 sees `model_path` change in a config diff. | `src/task.rs` |
| [x] 4.2a | Implement `monitor_loop` (inner `select!`) | Branches: `child.wait()` (Task 4.4), health/slots/metrics ticks (PR5), `shutdown_rx.cancelled()`. **The `shutdown_rx` branch MUST run cleanup before returning to the outer loop:** (a) send SIGTERM via `nix::sys::signal::kill(pid, SIGTERM)`, (b) `tokio::time::timeout(Duration::from_secs(10), child.wait())`, (c) if timeout: `kill(-pgid, SIGKILL)` (group kill), (d) await `child.wait()` to reap, (e) clear ledger reservation, (f) delete `managed_processes` row. Returning a sentinel result tells the outer loop to break (not restart). This is the cancellation-safety guarantee from design Section 9.1 — dropping the `Child` handle without an explicit kill leaves an orphan on Unix. | `src/task.rs` |
| [x] 4.3 | Implement `spawn_child_process` | Uses `tokio::process::Command::spawn`. Maps config options to `llama-server` CLI args. Sets `setsid()` via `CommandExt::process_group(0)` on Unix (enables `kill(-pgid, …)` for group SIGKILL on drain timeout). | `src/process.rs` |
| [x] 4.4 | Implement `child.wait()` loop branch | In `select!`, await `child.wait()`. On exit: (a) log `server.stopped` with exit code/signal, (b) **call `clear_vram_reservation(server_id)` immediately** (Ground Rule 6 — ledger entries removed on ANY exit, regardless of whether spawn-to-healthy completed), (c) remove `managed_processes` row, (d) transition to `Restarting` (or `Failed` if `restart_count >= max_restart_attempts`), (e) re-enter outer restart loop after backoff. | `src/task.rs` |
| [x] 4.5 | Implement `upsert_managed_process` | After spawn returns a PID, **`.await` `metrics_store.upsert_managed_process(ManagedProcess { pid, port, model_alias, server_id, started_at })` to completion before entering `monitor_loop`** — the SQLite row must be durable before the task can be killed, otherwise orphan recovery on next startup misses it. Delete on exit (Task 4.4) and on graceful shutdown (Task 4.2a cleanup). | `src/process.rs` |
| [x] 4.6 | Test helper: `mock-llama` binary | Per LlamaForge convention "Tests use `mock-llama`", build a minimal Rust test binary serving `/health`, `/slots`, `/metrics`. Configurable via env vars: `MOCK_PORT`, `MOCK_HEALTH_FAIL_AFTER=N` (return 500 after N successful health calls), `MOCK_HANG_AFTER=N` (stop incrementing `n_decoded` after N slots ticks), `MOCK_EXIT_AFTER_SECS=N` (process exits), `MOCK_MODEL_ALIAS` (echoed in `/health` JSON for orphan recovery tests), `MOCK_TPS` (synthetic tokens/sec for degradation tests). **Cargo target:** `[[bin]]` with `required-features = ["test-utils"]` so it's gated out of normal `cargo build` but `CARGO_BIN_EXE_mock-llama` env var IS populated for tests (which `[[example]]` would NOT do — that was the bug in earlier drafts). Concretely: <pre>[[bin]]<br>name = "mock-llama"<br>path = "src/bin/mock-llama.rs"<br>required-features = ["test-utils"]<br><br>[features]<br>test-utils = ["dep:axum", "dep:tower"]</pre> Tests run with `cargo test --features test-utils` (CI default) and locate the binary via `env!("CARGO_BIN_EXE_mock-llama")`. The `mock-llama.rs` file itself uses `#![cfg(feature = "test-utils")]` at the top so it compiles to nothing without the flag. | `crates/llamaforge-server-manager/src/bin/mock-llama.rs`, `crates/llamaforge-server-manager/Cargo.toml`, `tests/helpers/mod.rs` |
| [x] 4.7 | Unit tests: process lifecycle | Spawn task with `mock-llama`. (a) Send external kill signal → verify state → `Restarting` within 100ms and ledger cleared. (b) Cancel `shutdown_token` mid-run → verify child is SIGTERM'd then SIGKILL'd (no orphan PID survives task drop). (c) Verify `managed_processes` row written on spawn, removed on exit. | `tests/process_tests.rs` |

### PR4 Acceptance Criteria

- [x] `tokio::process::Command` successfully spawns the binary
- [x] `child.wait()` correctly detects process exit
- [x] VRAM ledger entries are reliably cleared on process exit
- [x] PID is persisted to SQLite via metrics store
- [x] `upsert_managed_process` is **awaited to completion before `monitor_loop` starts** — verified by code review (the `.await` appears textually before the monitor loop) AND by a unit test that starts the task, waits for the first health tick to arrive, then asserts `metrics_store.list_managed_processes()` already contains the row at that point
- [x] `shutdown_token` cancellation triggers explicit SIGTERM→SIGKILL of child before task exits (no orphans on cooperative shutdown)
- [x] Process group set via `process_group(0)` so `kill(-pgid, …)` is available
- [x] GGUF parse failure paths: `Io` → `Failed`; `Malformed`/`UnsupportedVersion` with `estimated_vram_mb` set → WARN + proceed; without override → `Failed`
- [x] `mock-llama` example binary builds and supports all 6 env-var knobs: `MOCK_PORT`, `MOCK_HEALTH_FAIL_AFTER`, `MOCK_HANG_AFTER`, `MOCK_EXIT_AFTER_SECS`, `MOCK_MODEL_ALIAS`, `MOCK_TPS`

### PR4 Creation Steps

```bash
git checkout -b server-manager/04-process-exec main
# ... implement tasks ...
git add -A && git commit -m "server-manager: implement process execution and tokio select loop"
git push -u origin server-manager/04-process-exec
```

---

## PR5: Polling, Health Checks & Degradation

**Branch:** `server-manager/05-polling`  
**Base:** `server-manager/04-process-exec`  
**Size:** Medium (~450 lines)  
**Estimated Time:** 7–9 hours (1 day — rolling TPS + degradation guard takes most of it)

### Purpose

Implement the periodic `/health`, `/slots`, and `/metrics` HTTP polling inside the server task (Section 3.1).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [x] 5.1 | Implement `reqwest` client | One long-lived `reqwest::Client` per server task (built once on spawn, recreated on restart). Connect timeout 1s, request timeout 5s for health, 10s for slots/metrics. | `src/health.rs` |
| [x] 5.2 | Implement `health_tick` | 2s `tokio::time::interval` (monotonic, design "Clock Assumptions") with random initial jitter 0–500ms to prevent synchronized bursts (design Section 3.5 invariant 6). GET `/health`. On HTTP 200 while `Starting`: (a) **call `clear_vram_reservation(server_id)`** — the ledger entry exists only during spawn-to-healthy (Ground Rule 6 + design Section 3.5 invariant 1), (b) transition lifecycle to `Running`, (c) record `first_health_ok_at = Some(Instant::now())`, (d) broadcast `ClientRefreshEvent { server_id, new_port: port }`, (e) reset `consecutive_health_failures = 0`. Health check validates HTTP 200 only — no JSON parsing (design Section 9 Open Question 2). | `src/task.rs` |
| [x] 5.3 | Implement `slots_tick` | 5s interval. GET `/slots`. Parse JSON into `Vec<SlotState>`. Update `DashMap` entry's `slots`. Also update `last_n_decoded` map (used by deadlock detection in PR6). On error: log debug, leave previous snapshot in place. | `src/task.rs` |
| [x] 5.4 | Implement `metrics_tick` | 10s interval. GET `/metrics` (Prometheus text format from llama-server). Lightweight line-based parser (no full Prometheus client dep) — extract `llamacpp:n_decode_total` (cumulative tokens decoded across all slots). Compute `current_tps = (n_decode_total - last_total_tokens_decoded).max(0) / 10.0` (clamp on counter reset, e.g. after restart). Update `last_total_tokens_decoded`. Send full Prometheus body via `MetricsIngest::send_telemetry`. Feed `current_tps` into the rolling TPS window (Task 5.5a) and invoke degradation check (Task 5.5b). | `src/task.rs` |
| [x] 5.5 | Implement health failure bounds | Track `consecutive_health_failures: u32`. Increment on each non-2xx or timeout. **On the 3rd consecutive failure** (i.e. `>= 3`, design Section 4.3 and FM table — *not* `> 3`), log `server.health_fail` WARN, transition to `Restarting`. A single transient timeout does NOT count as a failure relative to state (no transition) but DOES increment the counter; success resets to 0. | `src/task.rs` |
| [x] 5.5a | Implement rolling TPS window | Per design Section 9 ("Rolling TPS window (5 min)"), Section 1 ("performance degradation tracking"), Section 6 (`restart_on_degradation`), Section 7.2 (`server.performance_degraded` log event). New struct `RollingTpsWindow` with a fixed 50-sample ring buffer of `(Instant, f64)` token-per-second observations. API: `record(tps: f64)`, `mean_5min() -> Option<f64>`, `mean_lifetime() -> Option<f64>` (uses a separately tracked lifetime sum + count, not just the ring), `sample_count() -> usize`. Samples older than 5 minutes are pruned on `record()`. **Held inside the per-server task (not behind a lock)** — domain state, not observability. | `src/health.rs` |
| [x] 5.5b | Implement degradation detection (with cold-start guard) | Each `metrics_tick`: get `current_tps` (Task 5.4). **Skip the check entirely** if (a) `window.sample_count() < 30` (under 5 minutes of data — baseline isn't trustworthy yet) OR (b) any slot has `is_processing == true` AND `n_past < ctx_size / 10` (mid-prefill — TPS legitimately low). Otherwise: compare `current_tps < 0.5 * mean_lifetime()`. If true for **3 consecutive ticks** AND `server.config.restart_on_degradation == true`: log `server.performance_degraded` WARN with `current_tps` and `rolling_avg`, then transition to `Restarting` with reason `degradation`. If `restart_on_degradation == false`: log WARN only, no state change. Reset the consecutive counter on a single non-degraded tick or whenever the guard skips. | `src/task.rs` |
| [x] 5.6 | Mock HTTP via `mock-llama` | Reuse the `mock-llama` binary from PR4 task 4.6 — it already serves `/health`, `/slots`, `/metrics` with deterministic-failure env vars. No `httpmock` dependency. | `tests/helpers/mod.rs` |
| [x] 5.7 | Unit tests: state transitions | (a) Mock 200 OK while Starting → verify `Running`, `ClientRefreshEvent` broadcast, ledger entry cleared. (b) Mock 500 three times consecutively → verify state is `Restarting` exactly on the 3rd failure, not the 4th. (c) Two failures then one success → verify counter resets, no restart. (d) Verify health interval has initial jitter (two tasks started together don't tick at the same wall instant). | `tests/polling_tests.rs` |
| [x] 5.7a | Unit tests: degradation | (a) Rolling window: insert 50 samples, verify `mean_5min()` correct, prune past 5 min. (b) `current_tps < 0.5 * baseline` for 3 consecutive ticks + `restart_on_degradation=true` → state → `Restarting`. (c) Same conditions + `restart_on_degradation=false` → WARN logged, state unchanged. (d) Single recovery tick resets the consecutive counter. **(e) Cold-start guard: `sample_count=20` + low TPS → no restart, no WARN.** **(f) Prefill guard: any slot with `n_past < ctx_size/10` + low TPS → no restart.** | `tests/polling_tests.rs` |

### PR5 Acceptance Criteria

- [x] Ticks run independently using `tokio::time::interval` with initial jitter on health
- [x] State correctly transitions from `Starting` -> `Running` AND ledger reservation is cleared at that transition
- [x] `ClientRefreshEvent` is broadcasted upon reaching `Running`
- [x] Exactly 3 consecutive failures triggers `Restarting` (not 4)
- [x] Rolling TPS window prunes samples older than 5 min and reports correct means
- [x] `restart_on_degradation=true` triggers `Restarting` after 3 consecutive degraded ticks; `=false` only logs
- [x] Degradation check is skipped entirely while either guard holds (`sample_count < 30` OR any slot in prefill with `n_past < ctx_size/10`); only after both guards clear AND the 3-consecutive-tick rule fires does a restart happen

### PR5 Creation Steps

```bash
git checkout -b server-manager/05-polling main
# ... implement tasks ...
git add -A && git commit -m "server-manager: implement health and slots polling"
git push -u origin server-manager/05-polling
```

---

## VC-2: Process + Polling Verification Checkpoint

**Gate:** PR4 + PR5 merged. Per-server task can spawn, monitor, and reap a real process.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-2.1 | Lifecycle Transitions | `mock-llama` 200 OK → `Starting` → `Running` within 4s | Lifecycle matches; ledger cleared on `Running` |
| VC-2.2 | Crash Detection | External `kill -9` of mock-llama → state → `Restarting` ≤100ms | `child.wait()` branch fires; ledger cleared |
| VC-2.3 | Health Threshold | Mock returns 500 three times → state transitions on 3rd failure | Exactly 3 (not 2, not 4) |
| VC-2.4 | Cancellation Safety | `shutdown_token.cancel()` mid-run → child SIGTERM'd, then SIGKILL'd, then reaped | No zombie / orphan PID survives |
| VC-2.5 | Degradation | `restart_on_degradation=true` + 3 sub-baseline TPS ticks → state → `Restarting` | Triggers; log event recorded |
| VC-2.6 | Upsert Ordering | Start a task, wait for first health tick to fire, then query `metrics_store.list_managed_processes()` | Row present at the moment monitor loop reaches first tick (i.e. upsert completed before monitor entry) |

---

## PR6: Restart Coordinator, Deadlock Detection & Observability Decorator

**Branch:** `server-manager/06-restart-deadlock`  
**Base:** `server-manager/05-polling`  
**Size:** Medium (~550 lines)  
**Estimated Time:** 9–11 hours (1.5 days — full metrics surface + decorator wrapping is larger than the original "just backoff" estimate)

### Purpose

Implement the advanced reliability features: backoff with jitter, staggered spawns, and deadlock detection (Section 3.1 & 3.4).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [x] 6.1 | Implement Restart Coordinator | Track `restart_count`. Backoff formula uses the per-server `restart_cooldown_secs` config knob (design Section 6, default 10) as the **base**: `delay(n) = restart_cooldown_secs * 2^(n-1)` capped at `restart_cooldown_secs * 4`, then apply `±20%` jitter. With defaults this yields `10±2s, 20±4s, 40±8s, 40±8s, …` matching design Section 3.1. When `restart_count >= max_restart_attempts`: emit `server.failed` ERROR, transition permanently to `Failed`, do NOT remove from `servers` map (consumer queries still resolve, just see `Failed`); recovery requires operator config-reload. | `src/manager.rs` |
| [x] 6.2 | Implement per-GPU semaphore | `DashMap<GpuId, Arc<Semaphore::new(1)>>` (lazily inserted on first spawn touching a GPU). Acquire **before** `pre_spawn_vram_check` and hold until either spawn fails or health check passes (transition to `Running`). Prevents VRAM races and the FM-4 thundering herd. Also enforces `restart_stagger_secs` between releases for the same GPU. | `src/manager.rs` |
| [x] 6.3 | Implement `is_deadlocked()` | Per design Section 3.4. Four conditions: (1) active work (`is_processing` on any slot OR pending tracker > 0), (2) no decode progress (`n_decoded == last_n_decoded` for all slots), (3) GPU idle (`utilization_percent() < 10`, **with fallback to `process_cpu_usage() < 50` on `TelemetryError::Unavailable`** — design Section 3.4 explicitly specifies this fallback for Metal/CPU/Vulkan; on other telemetry errors return `false` to avoid false-positive kills), (4) `first_health_ok_at.is_some()` (previously healthy). All 4 must hold. | `src/health.rs` |
| [x] 6.4 | Integrate deadlock check | Call `is_deadlocked()` from health tick. If true: log `server.deadlock_detected` ERROR with slot state + GPU util + duration, send **SIGKILL** directly (not SIGTERM — design FM-2: deadlocked process can't handle signals), transition to `Restarting` with reason `deadlock`. | `src/task.rs` |
| [x] 6.5 | Implement full metrics surface | Per design Section 7.1 — all 9 metrics via the `metrics` facade crate: `llamaforge_server_status` (Gauge, label `server_id`, values 0/1/2/3), `llamaforge_server_restarts_total` (Counter, labels `server_id, reason` with `reason ∈ {crash, health_fail, degradation, deadlock, config}`), `llamaforge_server_uptime_seconds` (Gauge, label `server_id`), `llamaforge_vram_reserved_bytes` (Gauge, label `gpu_id`), `llamaforge_vram_free_bytes` (Gauge, label `gpu_id`), `llamaforge_gpu_utilization_pct` (Gauge, label `gpu_id`), `llamaforge_server_cumulative_requests` (Counter, label `server_id`), `llamaforge_pending_dispatches` (Gauge, label `server_id`), `llamaforge_health_check_duration_seconds` (Histogram, label `server_id`). Each registered with `describe_*!` once at startup. **File `src/observability.rs` (NOT `src/telemetry.rs`)** — `telemetry.rs` owns the `GpuTelemetry` FFI port from PR2; mixing GPU-FFI types and Prometheus registration is the same anti-pattern Metrics crate avoided with its `metrics_recording.rs` split. | `src/observability.rs` |
| [x] 6.5a | Implement `InstrumentedServerManager` decorator | Per CLAUDE.md project convention "Telemetry stays out of domain logic" (R26). Wrapper struct holding `inner: Arc<ServerManager>` that delegates all public methods, layering `#[instrument]` and `metrics::*!` calls at the adapter boundary. Domain code in `manager.rs` / `task.rs` stays free of telemetry except for genuine domain log events (`server.started`, `server.stopped`, etc., which are part of the contract per design Section 7.2). Mirrors `InstrumentedConfigHandle` in `llamaforge-config`. | `src/instrumented.rs` |
| [x] 6.6 | Unit tests: Deadlock logic | All 4 conditions met → true. Each condition independently flipped → false. **Specific test from design Section 8.4:** `is_processing=true, n_decoded=0, GPU util=95%` (legitimate prefill) → false. Telemetry `Unavailable` + low CPU → true (fallback path). Telemetry `Unavailable` + high CPU → false. Telemetry `FfiError` → false (don't kill). | `src/health.rs` (tests) |
| [x] 6.7 | Unit tests: Backoff math | (a) Attempt 1 delay ∈ [8s, 12s]. Attempt 2 ∈ [16s, 24s]. Attempt 3+ ∈ [32s, 48s]. (b) Monotonicity property: `backoff(n).mean <= backoff(n+1).mean`. (c) After `max_restart_attempts`: state → `Failed`, no further backoff invoked. | `src/manager.rs` (tests) |
| [x] 6.8 | Unit tests: per-GPU serialization | Two concurrent `pre_spawn` calls on `gpu:0` → second blocks until first reaches `Running` or fails. Calls on different GPUs proceed in parallel. | `tests/gpu_semaphore.rs` |

### PR6 Acceptance Criteria

- [x] Consecutive restarts delay exponentially with 20% jitter, capped at 40±8s
- [x] Deadlock accurately identifies Xid 79 / command buffer hangs and uses CPU fallback when telemetry is `Unavailable`
- [x] GPU spawns are synchronized per-GPU via Semaphore; cross-GPU spawns parallel
- [x] Server enters `Failed` state once max restarts exceeded (permanent until config reload)
- [x] All 9 metrics from design Section 7.1 are emitted with correct labels
- [x] `InstrumentedServerManager` decorator carries all `#[instrument]` and `metrics::*!` calls; domain code in `manager.rs` / `task.rs` contains no `metrics::counter!` etc.

### PR6 Creation Steps

```bash
git checkout -b server-manager/06-restart-deadlock main
# ... implement tasks ...
git add -A && git commit -m "server-manager: add restart coordinator and deadlock detection"
git push -u origin server-manager/06-restart-deadlock
```

---

## PR7: Config Diffs, Graceful Shutdown & Orphan Recovery

**Branch:** `server-manager/07-config-shutdown`  
**Base:** `server-manager/06-restart-deadlock`  
**Size:** Medium (~400 lines)  
**Estimated Time:** 6–8 hours

### Purpose

Implement dynamic configuration reloading (Section 5) and the clean graceful shutdown sequence.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [x] 7.1 | Implement `apply_config_diff` | Receive `ConfigDiff`. For each entry: `Added` → spawn task; `Removed` → signal shutdown; `Changed` with `requires_restart=true` (e.g. `devices`, `model_path`) → graceful stop + respawn; `Changed` with hot-reloadable fields only → update `ServerState` in place, no restart. Servers not mentioned in the diff are untouched (design Section 4.2 guarantee). Operations applied sequentially with `restart_stagger_secs` between sequential starts on the same GPU. The async snapshot is `.load()`ed once per call (CLAUDE.md hot-reload contract). | `src/manager.rs` |
| [x] 7.2 | Implement `stop_server` | Signal per-server `shutdown_rx`. Server task's shutdown branch (PR4 task 4.2a): (a) transition to `Draining`, (b) wait up to `resolve_drain_timeout(server)` (Task 3.2b — formula `max(60, min(ctx_size/30, 600))`) OR until `pending_dispatches(id) == 0` if available, whichever is sooner. **v1 simplification:** Dispatch & Proxy (component 06) is not yet built, so `pending_dispatches` always returns 0; the timeout path is the only effective gate. (`pending_dispatches` and the increment/decrement API were defined in PR3 Task 3.1a — Dispatch & Proxy will call them in component 06 without re-plumbing.) (c) send SIGTERM via `nix::sys::signal::kill(pid, SIGTERM)`, (d) wait 10s for exit (design Section 6 hardcoded timeout), (e) send SIGKILL group-wide via `kill(Pid::from_raw(-pgid), SIGKILL)`, (f) await `child.wait()` to reap, (g) cleanup as in PR4 task 4.2a. | `src/task.rs`, `src/manager.rs` |
| [x] 7.3 | Implement `shutdown_all` | Iterate over all servers, spawn `stop_server` concurrently using a `JoinSet`, await all. After all join: clear VRAM ledger (all entries), delete all rows from `managed_processes` (clean state for next startup, design "Graceful Shutdown" cross-cutting concern). | `src/manager.rs` |
| [x] 7.4 | Implement Orphan Recovery (FM-1) | On `start()`: query `metrics_store.list_managed_processes()`. For each row: (a) `kill(pid, 0)` to check liveness. If `Err(ESRCH)` → not alive → delete the row and continue. If alive: (b) **issue `GET http://localhost:{row.port}/health`** with a short timeout (1s). If 200 OK AND `row.model_alias` matches a configured server's model: log `server.orphan_reclaimed` INFO and adopt into `Running` state with the existing PID (skip spawn, skip VRAM ledger entry, set `first_health_ok_at = now()`, attach `child.wait()` via `nix::sys::wait::waitpid(pid, WNOHANG)` polling). (c) If liveness check passes but `/health` fails or model mismatches: log WARN `orphan_unowned`, **leave the process alone** (do NOT kill — could be unrelated; per design FM-1), delete the row so we don't try again. | `src/process.rs` |
| [x] 7.5 | Unit tests: config diffs | (a) Apply diff adding a server → verify task spawned. (b) Apply diff removing a server → verify graceful shutdown, untouched servers unaffected. (c) Apply diff with `requires_restart=false` change → verify no restart, state updated in place. **(d) FM-5: apply diff changing `devices` (gpu:0 → gpu:1) on a running server → verify old GPU ledger released, new GPU pre-spawn check runs, new server spawns on gpu:1, gpu:0 free VRAM increases.** | `tests/config_tests.rs` |
| [x] 7.6 | Unit tests: graceful shutdown | Verify drain timeout occurs before SIGTERM. Verify SIGKILL 10s after SIGTERM if still alive. Verify `managed_processes` rows deleted on `shutdown_all`. | `tests/process_tests.rs` |
| [x] 7.7 | Unit tests: orphan recovery | (a) Pre-populate `managed_processes` with PID of a running `mock-llama` serving matching model on its port → start manager → assert adopted into `Running`. (b) Pre-populate with a PID that is alive but `/health` fails → assert NOT adopted, row deleted, process left alone. (c) Pre-populate with a dead PID → assert row deleted, no adoption attempt. (d) Pre-populate with alive PID whose `/health` returns a different model_alias → assert `orphan_unowned` WARN, row deleted. | `tests/process_tests.rs` |

### PR7 Acceptance Criteria

- [ ] `shutdown_all` guarantees all processes are SIGKILL'd if they refuse to gracefully exit
- [ ] `apply_config_diff` cleanly adds/removes running tasks without restarting unaffected servers
- [ ] Orphan processes from previous crashes are adopted or reaped
- [ ] Orphan `/health` verification: alive PIDs whose `/health` returns a non-matching `model_alias` are left running and `orphan_unowned` WARN is logged; their `managed_processes` row is deleted so the check is idempotent
- [ ] FM-5: changing a server's `devices` field via config diff releases the old GPU ledger entry and runs `pre_spawn_vram_check` on the new GPU
- [ ] `pending_dispatches`, `increment_pending`, `decrement_pending` are exposed as `pub` methods on `ServerManager` so the future Dispatch & Proxy crate can wire in without re-plumbing

### PR7 Creation Steps

```bash
git checkout -b server-manager/07-config-shutdown main
# ... implement tasks ...
git add -A && git commit -m "server-manager: implement config diff application and graceful shutdown"
git push -u origin server-manager/07-config-shutdown
```

---

## VC-3: Restart, Deadlock & Shutdown Verification Checkpoint

**Gate:** PR6 + PR7 merged. Reliability features integrated.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-3.1 | Backoff Schedule | Force 4 consecutive crashes under `tokio::time::pause()` + `tokio::time::advance()` (no real wall-clock waits — 110+s of real time would otherwise be needed) → measure inter-restart intervals | Match 10±2, 20±4, 40±8, 40±8 ranges |
| VC-3.2 | Deadlock SIGKILL | Mock satisfies all 4 conditions → assert SIGKILL (not SIGTERM) sent | Direct SIGKILL; state → `Restarting` |
| VC-3.3 | GPU Serialization | Two `pre_spawn` on `gpu:0` concurrently → measure overlap | Second waits; cross-GPU spawns are parallel |
| VC-3.4 | Graceful Shutdown | `stop_server` with mock in-flight → drain → SIGTERM → SIGKILL | Order observed; child reaped |
| VC-3.5 | Orphan `/health` Verification | Alive PID with wrong model_alias on `/health` → NOT adopted, left alone | `orphan_unowned` WARN; row deleted |
| VC-3.6 | Metrics Completeness | Install `metrics_util::debugging::DebuggingRecorder` (or `PrometheusBuilder::build_recorder()`) **in-process** for the test, exercise a full scenario (spawn → run → restart → deadlock-kill → degradation), then snapshot the recorder. Server Manager has no HTTP `/metrics` endpoint of its own — Transport Layer (component 07) wraps it later. | All 9 metric names present with ≥1 observation; `restarts_total` carries exactly the 5 documented `reason` values from design Section 7.1 (`crash`, `health_fail`, `degradation`, `deadlock`, `config`); `server_id` / `gpu_id` labels match the test scenario |

---

## PR8: Verification, Tests & Public API

**Branch:** `server-manager/08-verification`  
**Base:** `server-manager/07-config-shutdown`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 5–6 hours

### Purpose

Finalize the component with property tests, complete E2E integration tests, and clean up the API exports.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [x] 8.1 | Property test: Backoff | `∀ n: backoff(n).mean <= backoff(n+1).mean` modulo jitter; bounds `[0.8x, 1.2x]`. | `tests/proptest_manager.rs` |
| [x] 8.2 | Property test: Ledger | Under arbitrary spawn/exit/healthy sequences, assert ledger entries always == set of servers in spawn-to-healthy state (i.e. `Starting`). | `tests/proptest_manager.rs` |
| [x] 8.2a | Property test: Stagger | `∀ concurrent restarts on same GPU: time between starts >= restart_stagger_secs` (design Section 8.3). | `tests/proptest_manager.rs` |
| [x] 8.3 | Integration test: Full Lifecycle | `start()` -> Wait for `Running` via `mock-llama` -> Dispatch fake increments -> `stop_server()` -> verify drain → SIGTERM → SIGKILL → reaped → `managed_processes` row deleted. | `tests/e2e_lifecycle.rs` |
| [x] 8.4 | Integration test: Orphan recovery | (a) Pre-populate sqlite with running `mock-llama` PID + matching model_alias → start manager → assert adopted. (b) Same but wrong model_alias → assert NOT adopted, row deleted, process untouched. | `tests/e2e_lifecycle.rs` |
| [x] 8.5 | Finalize `lib.rs` exports | `pub use` for: `ServerManager`, `InstrumentedServerManager`, `ServerLifecycle`, `ServerStateSnapshot`, `SlotState`, `ServerCapabilities`, `SpeculativeMethod`, `ModelArchitecture`, `ClientRefreshEvent`, `ServerManagerError`, `GpuTelemetry`, `GpuDevice`, `TelemetryError`, `StubTelemetry`. **Internal-only (NOT re-exported):** `ServerState`, `GgufMetadata`, `GgufError`, `Quantization`, `RollingTpsWindow`. If `cargo-public-api` is installed, run `cargo public-api list --simplified`; otherwise a manual `rg '^pub ' src/lib.rs` review to confirm nothing else leaks. | `src/lib.rs` |
| [x] 8.6 | Tag slow tests | Add `#[ignore]` to tests containing actual process sleeps or HTTP timeouts. | `tests/*.rs` |
| [x] 8.7 | id_task overflow test | Per design Section 8.4: set `cumulative_requests` to `i32::MAX/2 - 1`, increment, assert warning logged exactly once (latch). | `tests/e2e_lifecycle.rs` |

### PR8 Acceptance Criteria

- [ ] All property tests pass (256 cases)
- [ ] E2E lifecycle tests successfully transition through all states
- [ ] `cargo doc --no-deps -p llamaforge-server-manager` runs clean without warnings

### PR8 Creation Steps

```bash
git checkout -b server-manager/08-verification main
# ... implement tasks ...
git add -A && git commit -m "server-manager: finalize verification and public API"
git push -u origin server-manager/08-verification
```

---

## PR9: End-to-End Integration Test Suite

**Branch:** `server-manager/09-e2e`
**Base:** `main` (after PR8 merges)
**Size:** Medium (~400 lines, all under `tests/`)
**Estimated Time:** 6–8 hours

### Purpose

Close the E2E coverage gaps identified in the PR8 reviewer feedback. The existing integration tests (PR4–PR8) cover individual features in isolation against `mock-llama`, but they do not chain those features together in a single scenario, and several plan checkpoints (VC-3.1, VC-3.6, VC-4.5) were never realized as runnable tests. This PR adds the missing scenarios so VC-4 can pass from automated tests rather than manual inspection.

### Background — what already exists

| Test | Covers |
|------|--------|
| `e2e_lifecycle::test_e2e_lifecycle` | Single server: start → Running → drain → SIGTERM → reaped |
| `e2e_lifecycle::test_id_task_overflow` | i32::MAX/2 latch fires exactly once |
| `process_tests::test_process_lifecycle_external_kill` | External SIGKILL → Restarting ≤100ms |
| `process_tests::test_process_lifecycle_shutdown` | shutdown_token cancellation → child killed |
| `process_tests::test_orphan_recovery` | 4 cases (adopt / health-fail / dead PID / model mismatch) |
| `polling_tests::test_health_transition_and_refresh` | /health 200 → Running + `ClientRefreshEvent` |
| `polling_tests::test_consecutive_health_failures` | 3rd 500 → Restarting |
| `config_tests::test_apply_config_diff` | Added/removed/hot-reload/FM-5 GPU change |
| `gpu_semaphore::test_gpu_semaphore` | Same-GPU stagger ≥ `restart_stagger_secs` |

### What's missing

1. No combined scenario that exercises multiple features in one run (VC-3.6).
2. No integration test for the degradation restart path (VC-4.5, PR5 review F3 deferral).
3. No automated assertion of backoff schedule timing (VC-3.1).
4. No test that snapshots the Prometheus metrics surface after a realistic run (VC-3.6).
5. No multi-server orchestration test (3+ servers, mixed lifecycles, concurrent diff application).
6. CI invocation guidance for the `--ignored --features test-utils` matrix is undocumented.

### Ground rules

- **All tests in this PR run against `mock-llama`** — same convention as PR4–PR8. No real `llama-server` binary, no GPU.
- **Each test must be self-contained in port allocation**: use the existing `static NEXT_PORT: AtomicU16` pattern from PR7 (start the counter at `19000` to avoid collisions with prior tests).
- **`#[ignore]` on every test in this PR** — they take seconds-to-minutes each. CI runs them via `--ignored`.
- **No `std::env::set_var`** — pass mock-llama config via the comma-packed `--alias "name,health_fail_after=N,..."` CLI surface (PR5 F1 fix).
- **`#![cfg(feature = "test-utils")]`** at the top of every new file.
- **Use `metrics_util::debugging::DebuggingRecorder`** for metric-snapshot tests (it's already a workspace dev-dep, added in PR1 Task 1.2).
- **Use `tokio::time::pause()` / `advance()`** where wall-clock waits would otherwise exceed 30 seconds. Note that `pause()` cannot coexist with real TCP listeners — wall-clock tests that drive `mock-llama` cannot pause time. Pure-fn / state-machine tests can.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [ ] 9.1 | **Combined scenario test (VC-3.6)** | Spawn 1 server with `mock-llama`, wait for Running, install a per-test `DebuggingRecorder` via `metrics::set_global_recorder` (note: global, so this test must run alone — see Task 9.6), then drive a sequence: (a) wait for first `/health` success → assert `llamaforge_server_status` = 2.0, `llamaforge_server_uptime_seconds` > 0; (b) kill the mock with SIGKILL → wait for Restarting → on restart, assert `restarts_total{reason="crash"}` incremented by 1; (c) inject deadlock by spawning mock-llama with `--alias "mock,hang_after=2"` so n_decoded stalls, AND configure `MockTelemetry` to return `utilization_percent = 5` → assert `restarts_total{reason="deadlock"}` increments AND SIGKILL is sent (verify via PID liveness check); (d) trigger degradation via `--alias "mock,tps=0.1"` plus pre-warmed rolling window (use the test-only `set_cumulative_requests_for_test`-style helper if needed, gated behind `test-utils`) and wait for 3 consecutive degraded ticks → assert `restarts_total{reason="degradation"}`. **Final assertion**: snapshot all 9 metrics from design Section 7.1 and verify each has ≥1 recorded observation with the documented label set. | `tests/scenario_full_stack.rs` |
| [ ] 9.2 | **Degradation integration test (VC-4.5)** | Standalone test for the degradation path (separate from 9.1 to keep it focused). Spawn server with `restart_on_degradation = true` and `--alias "mock,tps=0.1"`. To bypass the cold-start guard without waiting 5 minutes of wall time, add a feature-gated `#[cfg(any(test, feature = "test-utils"))] pub fn force_warm_tps_window_for_test(&self, id, samples, baseline)` on `ServerManager` (mirrors `set_cumulative_requests_for_test` from PR8). Pre-seed 30 samples at 10.0 TPS, then let the manager run; mock returns near-zero TPS via /metrics; after ~30 seconds wall-clock (3 × 10s metrics ticks), assert lifecycle transitioned to `Restarting`, `server.performance_degraded` log fired, `restarts_total{reason="degradation"}` += 1. Mirror test with `restart_on_degradation = false`: assert WARN logged but lifecycle stays `Running`. | `tests/degradation_e2e.rs` |
| [ ] 9.3 | **Backoff schedule timing test (VC-3.1)** | Pure-Rust async test (no `mock-llama`). Use `tokio::time::pause()` + `advance()` to fast-forward through 4 restart cycles. Provide a stub spawn function that always fails so the manager hits the backoff path. Measure `Instant`-stamped restart attempts (with `Instant::now()` referring to paused time). Assert: attempt 1 ∈ [8s, 12s], attempt 2 ∈ [16s, 24s], attempt 3 ∈ [32s, 48s], attempt 4 ∈ [32s, 48s] (capped). After `max_restart_attempts` reached, assert lifecycle = `Failed` and no further attempts fire even after advancing the clock another 5 minutes. Note: time::pause() requires `--start-paused` or explicit `tokio::time::pause()` at test entry; mock-llama-based tests cannot use this. | `tests/backoff_schedule.rs` |
| [ ] 9.4 | **Multi-server orchestration test** | 3 servers configured: two on gpu:0 (with `restart_stagger_secs = 2`), one on gpu:1. Call `start()`. Assert: (a) the two gpu:0 servers reach Running in sequence with ≥ 2s gap; (b) the gpu:1 server reaches Running in parallel (overlapping the gpu:0 sequence); (c) after all three are Running, kill the gpu:0 first server externally → assert it transitions to Restarting and the gpu:0 second server is *not* disturbed; (d) call `apply_config_diff` removing the gpu:1 server while the gpu:0 first is restarting → assert gpu:1 drains/stops cleanly and gpu:0 servers are untouched; (e) `shutdown_all` → all reaped, SQLite `managed_processes` table empty, VRAM ledger empty. Wall-clock budget: ≤ 60 seconds total. | `tests/multi_server.rs` |
| [ ] 9.5 | **Prometheus metric snapshot test** | (Subset of 9.1 isolated as its own test so it can run without the full scenario.) Spawn one server, wait for Running, fire 5 `increment_requests` calls + 5 `increment_pending` / `decrement_pending` round-trips. Snapshot the `DebuggingRecorder`. Assert all 9 design-Section-7.1 metric names appear with correct label keys: `llamaforge_server_status{server_id}`, `llamaforge_server_restarts_total{server_id, reason}` (empty until restarts), `llamaforge_server_uptime_seconds{server_id}`, `llamaforge_vram_reserved_bytes{gpu_id}`, `llamaforge_vram_free_bytes{gpu_id}` (may be absent if free-VRAM gauge isn't emitted by the poller yet — flag this as a follow-up if so), `llamaforge_gpu_utilization_pct{gpu_id}` (likely absent — same flag), `llamaforge_server_cumulative_requests{server_id}` = 5, `llamaforge_pending_dispatches{server_id}` = 0, `llamaforge_health_check_duration_seconds{server_id}` (histogram, ≥1 observation). Any metric that's documented but not emitted produces a `FIXME: emit <metric>` finding in PR9's output rather than a blocking failure — surface them as known gaps for follow-up. | `tests/metrics_surface.rs` |
| [ ] 9.6 | **Test ordering / isolation** | The `metrics::set_global_recorder` API is install-once-per-process. Tests in Tasks 9.1 and 9.5 cannot both install a recorder. Two acceptable shapes: (a) gate both behind `#[serial_test::serial]` from the `serial_test` crate (add to workspace dev-deps additively), or (b) make 9.5 a subset of 9.1 (drop the standalone) and only install the recorder once. Pick (a) for clarity. Add `serial_test = "3"` to `[workspace.dependencies]` and inherit in server-manager's dev-deps. | `Cargo.toml` (workspace + server-manager), all metrics-recorder-installing tests |
| [ ] 9.7 | **CI invocation note** | Add `docs/component-designs/03_SERVER_MANAGER_TESTING.md` (or extend existing) with: the two test invocations (`cargo test -p llamaforge-server-manager` for fast unit/proptest, `cargo test -p llamaforge-server-manager --features test-utils -- --ignored` for the integration suite), the leaked-port cleanup tip (`lsof -i:18100-19200` + kill), and any per-test wall-clock budget warnings. This is informational, not a test. | `docs/component-designs/03_SERVER_MANAGER_TESTING.md` |

### PR9 Acceptance Criteria

- [ ] `cargo test -p llamaforge-server-manager` (default features, no `--ignored`) still completes in < 5 seconds with no failures
- [ ] `cargo test -p llamaforge-server-manager --features test-utils -- --ignored` runs all integration + scenario tests and passes
- [ ] Each new test file declares `#![cfg(feature = "test-utils")]` at the top
- [ ] Combined scenario test (9.1) exercises crash + deadlock + degradation in a single run and the `restarts_total` counter shows all three reasons
- [ ] Backoff schedule test (9.3) runs in < 1 second wall-clock via `tokio::time::pause()` (verified by `timeout 10s cargo test ... test_backoff_schedule` not actually being needed)
- [ ] Multi-server test (9.4) completes in < 60 seconds wall-clock
- [ ] Any metric documented in design Section 7.1 but absent from the recorder snapshot is captured as a `FIXME: emit X` comment in the test output, not a panic — this is the discovery surface for follow-up emission gaps
- [ ] `serial_test` added to workspace deps additively; no version pins on member crates
- [ ] Testing doc lands at `docs/component-designs/03_SERVER_MANAGER_TESTING.md`

### Reviewer notes

This PR is intentionally test-only — no production code changes beyond optional test-helper additions (`force_warm_tps_window_for_test` in Task 9.2), which must be gated behind `#[cfg(any(test, feature = "test-utils"))]` and `#[doc(hidden)]` per the PR8 F1 lesson. If any test discovers a real bug in the production code, file it as a separate finding in the PR description and either fix it in a sibling PR or `#[ignore]` the failing test with a `// BUG: <issue>` comment.

The proptest_manager.rs tests (PR8) are tautological by design — don't try to fix them in this PR; they're a separate cleanup item.

### PR9 Creation Steps

```bash
git checkout -b server-manager/09-e2e main
# ... implement tasks ...
git add -A && git commit -m "server-manager: add E2E scenario tests and metrics snapshot suite"
git push -u origin server-manager/09-e2e
```

---

## VC-4: Full Component Verification Checkpoint

**Gate:** All 8 PRs merged. Server Manager is feature-complete.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-4.1 | Lifecycle E2E | `start()` -> `/health` mock -> `stop_server()` | Transitions `Starting` -> `Running` -> `Draining` -> `Stopped` cleanly |
| VC-4.2 | Deadlock Recovery | Trigger 4 deadlock conditions on running mock | SIGKILL sent, transitions to `Restarting` |
| VC-4.3 | Config Hot-Reload | Send `ConfigDiff` removing server 1, adding server 2 | Server 1 Draining, Server 2 Starting. Server 3 untouched. |
| VC-4.4 | VRAM Ledger E2E | End-to-end: spawn 3 servers on gpu:0 sequentially with `mock-llama`, force OOM on third → ledger correctly serialized, third returns `InsufficientVram`, gpu:0 free reflects actual reservations (E2E counterpart to VC-3.3's unit-level semaphore test) | No over-allocation; correct error returned |
| VC-4.5 | Performance Degradation | Mock TPS drops to <50% of baseline for 3 ticks with `restart_on_degradation=true` | `server.performance_degraded` logged, restart triggered |

---

## VC-5: Final Sign-Off Checkpoint

**Gate:** VC-4 passes.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-5.1 | All tests pass | `cargo test --workspace && cargo test --workspace -- --ignored` | Zero failures |
| VC-5.2 | Zero clippy warnings | `cargo clippy --workspace -- -D warnings` | Exit code 0 |
| VC-5.3 | Unwrap audit | `rg 'unwrap\(\)' crates/llamaforge-server-manager/src/` | Zero hits in production code |
| VC-5.4 | Expect audit | `rg 'expect\(' crates/llamaforge-server-manager/src/` | Zero hits in production code |
| VC-5.5a | No `metrics::*!` in domain code | `rg 'metrics::(counter\|gauge\|histogram\|describe)!' crates/llamaforge-server-manager/src/manager.rs crates/llamaforge-server-manager/src/task.rs crates/llamaforge-server-manager/src/health.rs crates/llamaforge-server-manager/src/process.rs` | Zero hits (only `src/observability.rs` and `src/instrumented.rs` may invoke metric macros) |
| VC-5.5b | No `#[instrument]` in domain code | `rg '#\[instrument' crates/llamaforge-server-manager/src/manager.rs crates/llamaforge-server-manager/src/task.rs crates/llamaforge-server-manager/src/health.rs crates/llamaforge-server-manager/src/process.rs` | Zero hits (decorator owns spans) |
| VC-5.5c | Tracing log events in domain are bounded | `rg 'tracing::(info\|warn\|error)!' crates/llamaforge-server-manager/src/manager.rs crates/llamaforge-server-manager/src/task.rs` | Each hit must correspond to a named event in design Section 7.2 (`server.started`, `server.stopped`, `server.health_fail`, `server.deadlock_detected`, `server.performance_degraded`, `server.vram_insufficient`, `server.restarting`, `server.failed`, `server.orphan_reclaimed`, `server.id_task_overflow_warning`, `orphan_unowned`) — manual audit |
| VC-5.6 | Non-exhaustive audit | All public enums have `#[non_exhaustive]` | `rg '^pub enum' crates/llamaforge-server-manager/src/` cross-checked against attributes |

---

## Summary: PR → File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `src/types.rs`, `src/error.rs`, `src/state.rs` (incl. internal `ServerState`), `src/lib.rs` (+ `SpeculativeMethod`, `ModelArchitecture`) | ~400 |
| PR2 | `src/telemetry.rs` (Trait + Stub, **not sealed**) | ~200 |
| PR3 | `src/manager.rs` (DashMap, VRAM Ledger), `src/vram_estimate.rs` (GGUF parser + estimator) | ~500 |
| PR4 | `src/task.rs` (outer restart loop + monitor `select!`), `src/process.rs` (Spawn, wait, cancellation-safe cleanup), `src/bin/mock-llama.rs` (feature-gated `[[bin]]`) | ~550 |
| PR5 | `src/health.rs` (rolling TPS), `src/task.rs` (Ticks + degradation) | ~450 |
| PR6 | `src/manager.rs`, `src/health.rs` (Backoff, Deadlock w/ CPU fallback), `src/observability.rs` (full 9 metrics; separate from `telemetry.rs`), `src/instrumented.rs` | ~550 |
| PR7 | `src/manager.rs`, `src/process.rs` (Diff, Shutdown, Orphan w/ `/health` check) | ~400 |
| PR8 | `tests/proptest_*.rs`, `tests/e2e_*.rs`, `src/lib.rs` | ~350 |
| **Total** | | **~3,400** |

---

## Timeline (Single Engineer)

| Day | Work | PRs |
|-----|------|-----|
| Day 1 | `ManagedProcess` pre-flight + Domain types (+ `ServerState`, `SpeculativeMethod`, `ModelArchitecture`) + Telemetry interfaces | PR1, PR2 |
| Day 2 | Core state (DashMap) + VRAM ledger | PR3 (part 1) |
| Day 3 | GGUF parser + VRAM estimator + drain timeout formula | PR3 (part 2) |
| Day 4–5 | Process execution + restart loop + cancellation-safe cleanup + `mock-llama` + GGUF-failure handling | PR4 |
| Day 6 | Health/Slots polling + 3-failure transition + ledger clear on Running | PR5 (part 1) |
| Day 7 | Rolling TPS window + degradation detection + cold-start/prefill guards | PR5 (part 2) |
| Day 8 | Restart coordination + GPU semaphore | PR6 (part 1) |
| Day 9 | Deadlock detection (w/ CPU fallback) + full 9 metrics + `InstrumentedServerManager` | PR6 (part 2) |
| Day 10 | Config diff application + `pending_dispatches` API | PR7 (part 1) |
| Day 11 | Graceful shutdown + Orphan recovery with `/health` verification + FM-5 GPU change | PR7 (part 2) |
| Day 12 | Integration + Property testing | PR8 |
| Day 13–15 | Full verification (VC-3, VC-4, VC-5), docs | — |

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Zombie Processes | Low | High | `child.wait()` is unconditionally awaited in `select!`. `setsid()` allows `-pgid` kill. Cooperative shutdown branch explicitly SIGTERM/SIGKILLs before task drop. |
| VRAM Overallocation | Medium | Medium | Ledger strictly subtracts during `Starting` phase. Concurrent spawns blocked by GPU semaphore. |
| Deadlock False Positive | Medium | High | Deadlock detection requires 4 distinct independent conditions before killing (Section 3.4). |
| `reqwest` memory leak | Low | Low | One shared Client per server. Long-lived, no rapid re-instantiation. |
| SQLite locking during orphan | Low | Low | DB accesses are fast. Retries handled by `deadpool-sqlite`. |
| GGUF parse failure / corrupt header | Low | Medium | Defensive parser with bounded buffers (Task 3.2a). On error: fall back to operator-supplied `config.estimated_vram_mb`, or skip-check with WARN. No panics. |
| Cold-start TPS misread as degradation | Medium | High (false restart) | Two-part guard in Task 5.5b: minimum `sample_count >= 30` AND no slot mid-prefill. Restart only after both pass and 3 consecutive degraded ticks. |
| Metrics crate `ManagedProcess` missing required fields | Medium | High (blocks PR4 / PR7) | PR1 Task 1.0 pre-flight verifies `pid`, `port`, `model_alias`, `server_id`, `started_at` are present. If any missing, land an additive PR in `llamaforge-metrics` (extend struct + migration + row DTO) before PR4 begins. |

---

## Integration Points (for next components)

After Server Manager is complete:

1. **Smart Routing** will poll `ServerManager::slot_states()` to assign requests via prompt hash affinity.
2. **Dispatch & Proxy** will call `ServerManager::server_state()` to verify the target server is `Running`, subscribe to `subscribe_client_refresh()` to swap `reqwest::Client` instances after a backend restart, and call `increment_pending(id)` / `decrement_pending(id)` around each dispatch so `stop_server` can drain in-flight requests (API surface defined in PR7 task 7.2; Server Manager exposes `pending_dispatches(id) -> u64`).
3. **Transport Layer** will query `ServerManager::running_servers()` to expose overall gateway health.

---

## Cross-Component Dependency Diagram

```
llamaforge-config         llamaforge-metrics
    (ConfigHandle)           (MetricsStore)
          │                        │
          ▼                        ▼
    llamaforge-server-manager (this plan)
          │
          │ provides: ServerState, slot_states(), ClientRefreshEvent
          ▼
    Smart Routing, Dispatch & Proxy
```