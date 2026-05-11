# Server Manager — Implementation Plan

**Component:** Server Manager (`03_SERVER_MANAGER.md`)  
**Estimated LOC:** ~3,000  
**Estimated Duration:** 10–12 working days (single engineer) / 7–8 days (two engineers, PR4 and PR5 can be developed in parallel)  
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
8. **Tests use mock binaries.** Do not require an actual `llama-server` binary or GPU for unit/integration tests. Use a mock HTTP server and simple bash/rust dummy processes.
9. **Commit messages:** `server-manager: <imperative summary>` (e.g., `server-manager: add deadlock detection logic`).

### Conventions from Previous Components (MUST follow)

10. **Module visibility:** All modules declared as `pub(crate) mod` in `lib.rs`, with explicit `pub use` re-exports for the public API.
11. **Sealed traits:** All internal traits use the sealed trait pattern (`trait Sealed {}` in a `private` module).
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
**Size:** Medium (~300 lines)  
**Estimated Time:** 3–4 hours

### Purpose

Create the `llamaforge-server-manager` library crate, define all domain types from design doc Section 2.3, and error types from Section 2.4. This is the type foundation — every other PR builds on it.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [ ] 1.1 | Add `llamaforge-server-manager` to workspace | New lib crate `crates/llamaforge-server-manager`. Add to workspace members. Depends on `llamaforge-config` (for `ServerId`, `ConfigHandle`) and `llamaforge-metrics`. | `Cargo.toml`, `crates/llamaforge-server-manager/Cargo.toml` |
| [ ] 1.2 | Wire workspace dependencies | Inherit: `serde`, `thiserror`, `tokio` (with `process`, `sync`), `tracing`, `dashmap`, `reqwest`. Add dev-deps: `tempfile`, `tokio` (with `test-util`). | `crates/llamaforge-server-manager/Cargo.toml` |
| [ ] 1.3 | Create module structure | `lib.rs` declares `pub(crate) mod`: `types`, `error`, `telemetry`, `manager`, `process`, `state`, `health`. Explicit `pub use` re-exports for the public API. | `src/lib.rs`, `src/*.rs` |
| [ ] 1.4 | Implement `ServerLifecycle` enum | `Stopped`, `Starting`, `Running`, `Draining`, `Restarting`, `Failed`. Derive `Debug, Clone, Copy, PartialEq, Eq`. | `src/types.rs` |
| [ ] 1.5 | Implement `ServerCapabilities` & `SlotState` | From Section 2.3. Derive `Debug, Clone`. `SlotState` includes `id`, `is_processing`, `n_past`, `n_decoded`, `prompt_hash`. | `src/types.rs` |
| [ ] 1.6 | Implement `ServerStateSnapshot` | Fields: `id`, `lifecycle`, `pid`, `port`, `model_alias`, `parallel_slots`, `ctx_size`, `slots`, `last_health_check`, `restart_count`, `cumulative_requests`, `capabilities`. | `src/state.rs` |
| [ ] 1.7 | Implement events & errors | `ClientRefreshEvent` struct. `ServerManagerError` using `thiserror` (variants: `ServerNotFound`, `InsufficientVram`, `StartFailed`, `MaxRestartsExceeded`, `SpawnError`, `ConfigInvalid`). | `src/types.rs`, `src/error.rs` |
| [ ] 1.8 | Unit tests: type construction | Construct types, verify `Debug` and `Clone` behavior. Verify `ServerManagerError` formatting. | `src/types.rs` (tests) |

### PR1 Acceptance Criteria

- [ ] `cargo build --workspace` succeeds with zero warnings
- [ ] `llamaforge-server-manager` inherits workspace deps and uses Config types
- [ ] All public enums have `#[non_exhaustive]`
- [ ] `lib.rs` uses `pub(crate) mod` + explicit `pub use` pattern

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
| [ ] 2.1 | Define `GpuDevice` struct | Basic struct representing a GPU (ID, Name, Total VRAM). | `src/telemetry.rs` |
| [ ] 2.2 | Define `GpuTelemetry` trait | `free_vram_bytes`, `utilization_percent`, `list_devices`. `Send + Sync + 'static`. Sealed. | `src/telemetry.rs` |
| [ ] 2.3 | Define `TelemetryError` | Variants: `Unavailable`, `DeviceNotFound`, `FfiError`. | `src/telemetry.rs` |
| [ ] 2.4 | Implement `StubTelemetry` | Returns `Err(TelemetryError::Unavailable)` for all trait methods. Used as fallback. | `src/telemetry.rs` |
| [ ] 2.5 | Cargo feature flags | Add `gpu-nvml`, `gpu-metal`, `gpu-rocm` to Cargo.toml (empty features for now, implementations deferred). | `Cargo.toml` |
| [ ] 2.6 | Unit tests: telemetry | Test `StubTelemetry` returns correct error variants. | `src/telemetry.rs` (tests) |

### PR2 Acceptance Criteria

- [ ] Trait compiles and is object-safe
- [ ] `StubTelemetry` provides a safe fallback
- [ ] Feature flags correctly defined in Cargo.toml

### PR2 Creation Steps

```bash
git checkout -b server-manager/02-gpu-telemetry main
# ... implement tasks ...
git add -A && git commit -m "server-manager: add GpuTelemetry trait and StubTelemetry fallback"
git push -u origin server-manager/02-gpu-telemetry
```

---

## PR3: Core State Management & VRAM Ledger

**Branch:** `server-manager/03-core-state`  
**Base:** `server-manager/02-gpu-telemetry`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 4–5 hours

### Purpose

Implement the `ServerManager` shell, `DashMap` for state tracking, and the VRAM Ledger logic (Section 3.3).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [ ] 3.1 | Implement `ServerManager` shell | Struct with `servers: Arc<DashMap>`, `vram_ledger: Arc<Mutex<HashMap>>`, `gpu_telemetry`, `config`, `metrics_ingest`, `metrics_store`, `shutdown_token`. | `src/manager.rs` |
| [ ] 3.2 | Implement VRAM Ledger | `pre_spawn_vram_check()` checking NVML free VRAM minus ledger reservations against estimated usage. Return `InsufficientVram` if needed. Add/remove reservation methods. | `src/manager.rs` |
| [ ] 3.3 | Implement state accessors | `server_state()`, `running_servers()`, `slot_states()`. Extract snapshots from the `DashMap`. | `src/manager.rs` |
| [ ] 3.4 | Implement cumulative requests | `cumulative_requests()`, `increment_requests()`. Log warning at `i32::MAX / 2`. | `src/manager.rs` |
| [ ] 3.5 | Implement `subscribe_client_refresh` | Setup `tokio::sync::broadcast` channel for `ClientRefreshEvent`. | `src/manager.rs` |
| [ ] 3.6 | Unit tests: VRAM logic | Test `pre_spawn_vram_check` arithmetic. Test VRAM reservation adding and removing. | `src/manager.rs` (tests) |
| [ ] 3.7 | Unit tests: accessors | Test `running_servers` filters non-Running states correctly. | `src/manager.rs` (tests) |

### PR3 Acceptance Criteria

- [ ] `ServerManager` can be instantiated
- [ ] VRAM ledger arithmetic is correct and synchronous (no `.await` over Mutex)
- [ ] Broadcast channel provides subscribers correctly

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
| VC-1.1 | State Filtering | `running_servers()` correctly ignores Failed/Stopped. | Tests pass |
| VC-1.2 | VRAM Math | Pre-spawn check correctly subtracts ledger from raw GPU free VRAM. | Tests pass |
| VC-1.3 | Mutex Safety | `Mutex` for VRAM ledger is not held across `.await` points. | Manual code review |

---

## PR4: Process Execution & Server Task Shell

**Branch:** `server-manager/04-process-exec`  
**Base:** `server-manager/03-core-state`  
**Size:** Large (~450 lines)  
**Estimated Time:** 6–8 hours

### Purpose

Implement the main per-server Tokio task, process spawning, and child wait loop (Section 3.1 & 3.2).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [ ] 4.1 | Implement `start()` method | Iterates over configured servers, calls `spawn_server_task` for each. | `src/manager.rs` |
| [ ] 4.2 | Implement `spawn_server_task` | `tokio::spawn` a new task. Sets up the main `tokio::select!` loop. | `src/task.rs` |
| [ ] 4.3 | Implement `spawn_child_process` | Uses `tokio::process::Command::spawn`. Maps config options to `llama-server` CLI args. Sets `setsid()` via `CommandExt::process_group(0)` on Unix for orphan recovery. | `src/process.rs` |
| [ ] 4.4 | Implement `child.wait()` loop | In `select!`, await `child.wait()`. If exits, update state to `Restarting` and log event. Remove VRAM ledger reservation immediately. | `src/task.rs` |
| [ ] 4.5 | Implement `upsert_managed_process` | On successful spawn, send PID to `metrics_store.upsert_managed_process()`. Remove on exit. | `src/process.rs` |
| [ ] 4.6 | Test helper: Mock binary | Create a dummy script/binary that sleeps, for use in tests. | `tests/helpers/mod.rs` |
| [ ] 4.7 | Unit tests: process lifecycle | Spawn task with mock binary. Send external kill signal. Verify state changes to `Restarting`. | `tests/process_tests.rs` |

### PR4 Acceptance Criteria

- [ ] `tokio::process::Command` successfully spawns the binary
- [ ] `child.wait()` correctly detects process exit
- [ ] VRAM ledger entries are reliably cleared on process exit
- [ ] PID is persisted to SQLite via metrics store

### PR4 Creation Steps

```bash
git checkout -b server-manager/04-process-exec main
# ... implement tasks ...
git add -A && git commit -m "server-manager: implement process execution and tokio select loop"
git push -u origin server-manager/04-process-exec
```

---

## PR5: Polling & Health Checks

**Branch:** `server-manager/05-polling`  
**Base:** `server-manager/04-process-exec`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 4–6 hours

### Purpose

Implement the periodic `/health`, `/slots`, and `/metrics` HTTP polling inside the server task (Section 3.1).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [ ] 5.1 | Implement `reqwest` client | Create a shared client for polling per server. | `src/health.rs` |
| [ ] 5.2 | Implement `health_tick` | 2s interval in `select!`. GET `/health`. On success, if `Starting`, transition to `Running` + emit `ClientRefreshEvent`. | `src/task.rs` |
| [ ] 5.3 | Implement `slots_tick` | 5s interval. GET `/slots`. Parse JSON into `SlotState` vector. Update `DashMap`. | `src/task.rs` |
| [ ] 5.4 | Implement `metrics_tick` | 10s interval. GET `/metrics`. Parse and send via `MetricsIngest::send_telemetry`. | `src/task.rs` |
| [ ] 5.5 | Implement failure bounds | Track consecutive health check failures. If > 3, log warning and force transition to `Restarting`. | `src/task.rs` |
| [ ] 5.6 | Test helper: Mock HTTP | Use `httpmock` or similar to serve fake endpoints. | `tests/helpers/mod.rs` |
| [ ] 5.7 | Unit tests: state transitions | Mock 200 OK -> verify `Running`. Mock 500 -> verify `Restarting` after 3 tries. | `tests/polling_tests.rs` |

### PR5 Acceptance Criteria

- [ ] Ticks run independently using `tokio::time::interval`
- [ ] State correctly transitions from `Starting` -> `Running`
- [ ] `ClientRefreshEvent` is broadcasted upon reaching `Running`
- [ ] 3 consecutive failures correctly triggers restart logic

### PR5 Creation Steps

```bash
git checkout -b server-manager/05-polling main
# ... implement tasks ...
git add -A && git commit -m "server-manager: implement health and slots polling"
git push -u origin server-manager/05-polling
```

---

## PR6: Restart Coordinator & Deadlock Detection

**Branch:** `server-manager/06-restart-deadlock`  
**Base:** `server-manager/05-polling`  
**Size:** Medium (~400 lines)  
**Estimated Time:** 5–7 hours

### Purpose

Implement the advanced reliability features: backoff with jitter, staggered spawns, and deadlock detection (Section 3.1 & 3.4).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [ ] 6.1 | Implement Restart Coordinator | Track `restart_count`. Calculate exponential backoff (10s, 20s, 40s) + 20% jitter. Max restarts bound checks. | `src/manager.rs` |
| [ ] 6.2 | Implement per-GPU semaphore | `DashMap<GpuId, Arc<Semaphore(1)>>`. Acquire before spawning to prevent VRAM allocation races (Thundering Herd). | `src/manager.rs` |
| [ ] 6.3 | Implement `is_deadlocked()` | Function checking 4 conditions: active work, no progress (`n_decoded` stalled), GPU idle (via telemetry), previously healthy. | `src/health.rs` |
| [ ] 6.4 | Integrate deadlock check | Call `is_deadlocked` in the health tick. If true, emit SIGKILL, transition to `Restarting`. | `src/task.rs` |
| [ ] 6.5 | Implement metrics recording | Add `metrics` crate `gauge!` for server status and `counter!` for restarts/events. | `src/telemetry.rs` |
| [ ] 6.6 | Unit tests: Deadlock logic | Mock states satisfying all 4 conditions, verify returns true. Change one, verify false. | `src/health.rs` (tests) |
| [ ] 6.7 | Unit tests: Backoff math | Verify monotonicity and jitter bounds. | `src/manager.rs` (tests) |

### PR6 Acceptance Criteria

- [ ] Consecutive restarts delay exponentially
- [ ] Deadlock accurately identifies Xid 79 / command buffer hangs
- [ ] GPU spawns are synchronized per-GPU via Semaphore
- [ ] Server enters `Failed` state once max restarts exceeded

### PR6 Creation Steps

```bash
git checkout -b server-manager/06-restart-deadlock main
# ... implement tasks ...
git add -A && git commit -m "server-manager: add restart coordinator and deadlock detection"
git push -u origin server-manager/06-restart-deadlock
```

---

## PR7: Config Diffs & Graceful Shutdown

**Branch:** `server-manager/07-config-shutdown`  
**Base:** `server-manager/06-restart-deadlock`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 4–6 hours

### Purpose

Implement dynamic configuration reloading (Section 5) and the clean graceful shutdown sequence.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [ ] 7.1 | Implement `apply_config_diff` | Receive `ConfigDiff`. Spawn tasks for `Added` servers. Send shutdown signal to `Removed` servers. | `src/manager.rs` |
| [ ] 7.2 | Implement `stop_server` | Signal `shutdown_rx`. In task: transition to `Draining`, wait `shutdown_drain_timeout_secs`, send SIGTERM, wait 10s, send SIGKILL. | `src/task.rs` |
| [ ] 7.3 | Implement `shutdown_all` | Iterate over all servers, spawn `stop_server` concurrently using a `JoinSet`, await all. | `src/manager.rs` |
| [ ] 7.4 | Implement Orphan Recovery | On `start()`: query `metrics_store.list_managed_processes()`. Use `kill(pid, 0)` to check if alive. If alive, adopt into `Running` (FM-1). | `src/process.rs` |
| [ ] 7.5 | Unit tests: config diffs | Apply diff adding a server, verify task spawned. | `tests/config_tests.rs` |
| [ ] 7.6 | Unit tests: graceful shutdown | Verify drain timeout occurs before SIGTERM. | `tests/process_tests.rs` |

### PR7 Acceptance Criteria

- [ ] `shutdown_all` guarantees all processes are SIGKILL'd if they refuse to gracefully exit
- [ ] `apply_config_diff` cleanly adds/removes running tasks without restarting unaffected servers
- [ ] Orphan processes from previous crashes are adopted or reaped

### PR7 Creation Steps

```bash
git checkout -b server-manager/07-config-shutdown main
# ... implement tasks ...
git add -A && git commit -m "server-manager: implement config diff application and graceful shutdown"
git push -u origin server-manager/07-config-shutdown
```

---

## PR8: Verification, Tests & Public API

**Branch:** `server-manager/08-verification`  
**Base:** `server-manager/07-config-shutdown`  
**Size:** Medium (~300 lines)  
**Estimated Time:** 4–5 hours

### Purpose

Finalize the component with property tests, complete E2E integration tests, and clean up the API exports.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| [x] 8.1 | Property test: Backoff | Ensure `forall n: backoff(n) <= backoff(n+1)` modulo jitter. | `tests/proptest_manager.rs` |
| [x] 8.2 | Property test: Ledger | Assert `VRAM ledger entries == servers in Starting state` under arbitrary spawn/exit sequences. | `tests/proptest_manager.rs` |
| [x] 8.3 | Integration test: Full Lifecycle | `start()` -> Wait for `Running` via mock health -> Dispatch -> `stop_server()` -> verify exit. | `tests/e2e_lifecycle.rs` |
| [x] 8.4 | Integration test: Orphan recovery | Pre-populate sqlite with running PID. Start manager, assert adopted. | `tests/e2e_lifecycle.rs` |
| [x] 8.5 | Finalize `lib.rs` exports | Ensure `ServerManager`, `ServerLifecycle`, `ClientRefreshEvent`, and `GpuTelemetry` are clean `pub use`. | `src/lib.rs` |
| [x] 8.6 | Tag slow tests | Add `#[ignore]` to tests containing actual process sleeps or HTTP timeouts. | `tests/*.rs` |

### PR8 Acceptance Criteria

- [x] All property tests pass (256 cases)
- [x] E2E lifecycle tests successfully transition through all states
- [x] `cargo doc --no-deps -p llamaforge-server-manager` runs clean without warnings

### PR8 Creation Steps

```bash
git checkout -b server-manager/08-verification main
# ... implement tasks ...
git add -A && git commit -m "server-manager: finalize verification and public API"
git push -u origin server-manager/08-verification
```

---

## VC-4: Full Component Verification Checkpoint

**Gate:** All 8 PRs merged. Server Manager is feature-complete.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-4.1 | Lifecycle E2E | `start()` -> `/health` mock -> `stop_server()` | Transitions `Starting` -> `Running` -> `Draining` -> `Stopped` cleanly |
| VC-4.2 | Deadlock Recovery | Trigger 4 deadlock conditions on running mock | SIGKILL sent, transitions to `Restarting` |
| VC-4.3 | Config Hot-Reload | Send `ConfigDiff` removing server 1, adding server 2 | Server 1 Draining, Server 2 Starting. Server 3 untouched. |
| VC-4.4 | VRAM Ledger | Request multiple concurrent starts on GPU 0 | Spawns are staggered, VRAM checked, no over-allocation |

---

## VC-5: Final Sign-Off Checkpoint

**Gate:** VC-4 passes.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-5.1 | All tests pass | `cargo test --workspace && cargo test --workspace -- --ignored` | Zero failures |
| VC-5.2 | Zero clippy warnings | `cargo clippy --workspace -- -D warnings` | Exit code 0 |
| VC-5.3 | Unrwap audit | `rg 'unwrap\(\)' crates/llamaforge-server-manager/src/` | Zero hits in production code |
| VC-5.4 | Expect audit | `rg 'expect\(' crates/llamaforge-server-manager/src/` | Zero hits in production code |

---

## Summary: PR → File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `src/types.rs`, `src/error.rs`, `src/state.rs`, `src/lib.rs` | ~300 |
| PR2 | `src/telemetry.rs` (Trait + Stub) | ~200 |
| PR3 | `src/manager.rs` (DashMap, VRAM Ledger) | ~350 |
| PR4 | `src/task.rs`, `src/process.rs` (Spawn, wait) | ~450 |
| PR5 | `src/health.rs`, `src/task.rs` (Ticks) | ~350 |
| PR6 | `src/manager.rs`, `src/health.rs` (Backoff, Deadlock) | ~400 |
| PR7 | `src/manager.rs`, `src/process.rs` (Diff, Shutdown, Orphan) | ~350 |
| PR8 | `tests/proptest_*.rs`, `tests/e2e_*.rs`, `src/lib.rs` | ~300 |
| **Total** | | **~2,700** |

---

## Timeline (Single Engineer)

| Day | Work | PRs |
|-----|------|-----|
| Day 1 | Domain types + Telemetry interfaces | PR1, PR2 |
| Day 2 | Core state (DashMap) + VRAM ledger math | PR3 |
| Day 3 | Process execution shell + child reaping | PR4 |
| Day 4 | Health/Slots polling HTTP loops | PR5 |
| Day 5 | Restart coordination + GPU semaphore | PR6 (part 1) |
| Day 6 | Deadlock detection + metrics | PR6 (part 2) |
| Day 7 | Config diff application | PR7 (part 1) |
| Day 8 | Graceful shutdown + Orphan recovery | PR7 (part 2) |
| Day 9 | Integration + Property testing | PR8 |
| Day 10–12 | Full verification (VC-4, VC-5), docs | — |

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Zombie Processes | Low | High | `child.wait()` is unconditionally awaited in `select!`. `setsid()` allows `-pgid` kill. |
| VRAM Overallocation | Medium | Medium | Ledger strictly subtracts during `Starting` phase. Concurrent spawns blocked by GPU semaphore. |
| Deadlock False Positive | Medium | High | Deadlock detection requires 4 distinct independent conditions before killing (Section 3.4). |
| `reqwest` memory leak | Low | Low | One shared Client per server. Long-lived, no rapid re-instantiation. |
| SQLite locking during orphan | Low | Low | DB accesses are fast. Retries handled by `deadpool-sqlite`. |

---

## Integration Points (for next components)

After Server Manager is complete:

1. **Smart Routing** will poll `ServerManager::slot_states()` to assign requests via prompt hash affinity.
2. **Dispatch & Proxy** will call `ServerManager::server_state()` to verify the target server is `Running`. It will subscribe to `subscribe_client_refresh()` to dynamically swap `reqwest::Client` instances after a backend restart.
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