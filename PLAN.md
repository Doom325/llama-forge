# Dispatch & Proxy — Implementation Plan

**Component:** Dispatch & Proxy (`06_DISPATCH_AND_PROXY.md`)  
**Estimated LOC:** ~2,700 (~1,900 production + ~750 tests)  
**Estimated Duration:** 10–12 working days (single engineer) / 7–8 days (two engineers)  
**Design Document:** `component-designs/06_DISPATCH_AND_PROXY.md` (R2 PASS WITH WARNINGS — all 28 findings addressed)  
**Branch Prefix:** `dispatch/`  
**Target:** `main` (via squash-merge PRs)  
**Upstream Dependencies:** Config Manager (`llamaforge-config`), Server Manager (`llamaforge-server-mgr` types), Smart Routing (`llamaforge-routing` — `RoutingDecision`, `DispatchCountView`), Metrics & Storage (`llamaforge-metrics` — `MetricsIngest`), Request Queue (`llamaforge-queue` — `DispatchCandidate`)

> **Stub strategy for `DispatchCandidate`:** If `llamaforge-queue` (Component 04) is not yet implemented when PR5 begins, define a minimal `DispatchCandidate` struct in `llamaforge-types` with fields `request_id: RequestId`, `prompt: String`, `params: CompletionParams`, `routing: RoutingDecision`. When `llamaforge-queue` lands, migrate to its canonical definition and remove the stub. This avoids blocking the dispatch implementation on the queue component.

> **Branching workflow:** Each PR is squash-merged to `main` before the next PR branches. All `git checkout -b` commands branch from `main`, which is correct because the predecessor PR has already been merged. The "Depends on" field on each PR indicates which predecessor(s) must be merged to `main` before that PR's branch is created. Do NOT branch from a predecessor's feature branch — always branch from `main` after the predecessor is merged.

---

## Ground Rules for CoderAgents

1. **Read the design doc first.** Every PR description must reference the specific design doc sections it implements.
2. **No `unwrap()` or `expect()` in production code paths.** Use `?` propagation, `unwrap_or_else`, or explicit `match`. Mutex locks use `unwrap_or_else(|p| { tracing::warn!("...poisoned..."); p.into_inner() })` for poison recovery.
3. **No blocking under `.await`.** `std::sync::Mutex` critical sections are pure computation (insert, remove, len). Lock guards are acquired, used, and dropped within a single synchronous expression. Never hold a guard across an `.await` point. If async work is needed, clone the data out first.
4. **Every public type gets a doc comment.** Reference the design doc section number.
5. **Newtype wrappers for domain IDs.** Use `ServerId`, `RequestId`, `SlotId`, `HttpStatus`, `CancellationPhase` — never raw integers or strings for domain identifiers.
6. **Tests run with `cargo test --workspace`.** Every PR must pass before merge.
7. **`cargo clippy -- -D warnings` must pass.** Zero warnings policy.
8. **Commit messages:** `dispatch: <imperative summary>` (e.g., `dispatch: add DispatchToken RAII guard with poison-recovering mutex`).
9. **Port traits over concrete types.** Dispatcher uses `Arc<dyn ServerStateView>`, `Arc<dyn BackendClient>`, `Arc<dyn ClientManager>` — never `Arc<Router>` or `Arc<ServerManager>`. The only deliberately concrete type is `MetricsIngest` (documented rationale in design doc Section 2.1).
10. **reqwest confined to `client.rs`.** No other source file in `llamaforge-dispatch` may import `reqwest`. All HTTP interaction goes through the `BackendClient` / `ClientManager` port traits.
11. **Cross-crate port traits live in `llamaforge-types`.** `SlotDispatchView`, `SlotErasure`, `SlotNotifier`, `SlotError`, `ClientRefreshEvent` are owned by `llamaforge-types`, not `llamaforge-dispatch`. This avoids dependency cycles with `llamaforge-server-mgr` and `llamaforge-queue`.
12. **Visibility is intentional.** `DispatchTracker::insert()` and `remove()` are `pub(crate)`. `DispatchToken.released` is private. All internal structs (`SlotDispatchMapInner`, `DispatchTrackerRegistry.inner`) are private. See design doc Section 2.7 Visibility Annotations.

---

## Dependency Graph (PR merge order)

```
PR1 ─── PR2 ─── PR3 ─── PR4 ─── PR5 ─── PR6 ─── PR7 ─── PR8
```

PR3 and PR4 are now sequential (PR4 depends on PR3 because `DispatchToken` calls `tracker.insert()`/`remove()` defined in PR3). PR6 → PR7 → PR8 are strictly sequential after PR5.

> **Note:** The original plan had PR3 and PR4 parallelizable, but PR4's `DispatchToken` compiles against `DispatchTracker` from PR3 — they cannot truly be developed on separate branches without stubs. Making them sequential adds ~1 day to the single-engineer path but is the cleanest approach.

---

## PR1: Crate Scaffold & Shared Types in `llamaforge-types`

**Branch:** `dispatch/01-scaffold`  
**Merge Target:** `main`  
**Size:** Small (~200 lines)  
**Estimated Time:** 2–3 hours

### Purpose

Create the `llamaforge-dispatch` library crate and extend `llamaforge-types` with the cross-crate port traits and types required by Dispatch & Proxy: `SlotDispatchView`, `SlotErasure`, `SlotNotifier`, `SlotError`, `ClientRefreshEvent`, `SlotId`, and `HttpStatus`.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 1.1 | Create `llamaforge-dispatch` lib crate | `cargo init --lib crates/llamaforge-dispatch`. This crate owns Dispatcher, DispatchToken, proxy tasks, slot tracking, and the reqwest adapter. | `crates/llamaforge-dispatch/` |
| 1.2 | Add workspace member | Add `llamaforge-dispatch` to workspace `Cargo.toml` `[workspace]` members list. | `Cargo.toml` |
| 1.3 | Add workspace-level dependencies | `arc-swap = "1"`, `async-trait = "0.1"`, `pin-project-lite = "0.2"`, `futures-core = "0.3"`, `bytes = "1"`. Verify `reqwest`, `dashmap`, `tokio`, `tracing`, `metrics`, `thiserror` already present from prior PRs. **Note:** `pin-project-lite`, `futures-core`, and `bytes` are not listed in the design doc's crate placement section but are essential implementation deps (`futures-core` for `Stream` trait used by `BackendResponseStream`, `bytes` for `Bytes` chunks in SSE parsing, `pin-project-lite` for `Pin` projections in streaming). Update the design doc Section 1.1 deps list in this PR. | `Cargo.toml` |
| 1.4 | Add `SlotId` newtype to `llamaforge-types` | `pub struct SlotId(pub(crate) u32)`. Derive `Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Display`. Ensure usable as `HashMap` key component. | `crates/llamaforge-types/src/lib.rs` |
| 1.5 | Add `HttpStatus` newtype to `llamaforge-dispatch` | `pub struct HttpStatus(pub u16)` with `is_server_error()`, `is_client_error()`, `Display` impl. Lives in dispatch crate (domain error type). | `crates/llamaforge-dispatch/src/error.rs` |
| 1.6 | Add `CancellationPhase` enum to `llamaforge-dispatch` | Four variants: `PreConnect`, `PreStream`, `Streaming`, `Cleanup`. With `Debug, Clone, Copy, PartialEq, Eq` derives and `Display` impl. | `crates/llamaforge-dispatch/src/error.rs` |
| 1.7 | Add `SlotDispatchView` trait to `llamaforge-types` | `pub trait SlotDispatchView: Send + Sync` with `get_request_for_slot()` and `has_unresolved()`. In `crates/llamaforge-types/src/ports.rs`. | `crates/llamaforge-types/src/ports.rs` |
| 1.8 | Add `SlotErasure` trait to `llamaforge-types` | `#[async_trait] pub trait SlotErasure: Send + Sync` with `erase_slot()`. Same file. | `crates/llamaforge-types/src/ports.rs` |
| 1.9 | Add `SlotError` enum to `llamaforge-types` | `ErasureFailed { server_id, slot_id, reason }` and `ServerUnreachable { server_id }`. With `thiserror`. | `crates/llamaforge-types/src/ports.rs` |
| 1.10 | Add `ClientRefreshEvent` struct to `llamaforge-types` | `pub struct ClientRefreshEvent { pub server_id: ServerId, pub new_port: u16, pub tls_config: Option<TlsConfig> }`. Derive `Debug, Clone`. | `crates/llamaforge-types/src/lib.rs` |
| 1.10a | Add `SlotNotifier` trait to `llamaforge-types` | `pub trait SlotNotifier: Send + Sync { fn notify_slot_available(&self, server_id: &ServerId); }`. Called by proxy task cleanup (primary path) and by `DispatchToken::Drop` (safety-net path) to wake the scheduler when a slot frees. Lives alongside `SlotDispatchView` and `SlotErasure`. | `crates/llamaforge-types/src/ports.rs` |
| 1.11 | Wire crate dependencies | `llamaforge-dispatch/Cargo.toml` inherits workspace deps: `llamaforge-types`, `llamaforge-routing`, `llamaforge-config`, `llamaforge-metrics`, `reqwest`, `dashmap`, `arc-swap`, `async-trait`, `tokio`, `tokio-util`, `tracing`, `thiserror`, `metrics`, `bytes`, `futures-core`, `pin-project-lite`. Dev-deps: `tokio` (with `test-util`, `macros`, `rt-multi-thread`), `tokio-stream`, `wiremock`. | `crates/llamaforge-dispatch/Cargo.toml` |
| 1.12 | Create module structure in `llamaforge-dispatch` | `lib.rs` declares modules: `pub mod dispatcher;`, `pub mod token;`, `pub mod tracker;`, `pub mod slot_map;`, `pub mod proxy;`, `pub mod client;`, `pub mod error;`. Stub each module file with doc comments referencing design doc sections. | `crates/llamaforge-dispatch/src/` |
| 1.13 | Verify build | `cargo build --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`. | — |

### PR1 Acceptance Criteria

- [x] `cargo build --workspace` succeeds with zero warnings
- [x] `cargo clippy --workspace -- -D warnings` passes clean
- [x] `SlotId` compiles with `Eq + Hash` (usable as `HashMap` key)
- [x] `Arc<dyn SlotDispatchView>` compiles (trait is object-safe)
- [x] `Arc<dyn SlotErasure>` compiles (trait is object-safe)
- [x] `Arc<dyn SlotNotifier>` compiles (trait is object-safe)
- [x] `ClientRefreshEvent` can cross crate boundaries (no private fields)
- [x] `crates/llamaforge-dispatch/src/lib.rs` compiles as a library

### PR1 Creation Steps

```bash
git checkout -b dispatch/01-scaffold main
# ... implement tasks 1.1–1.13 ...
git add -A && git commit -m "dispatch: bootstrap llamaforge-dispatch crate and shared types"
git push -u origin dispatch/01-scaffold
# Open PR: "Dispatch & Proxy PR1: Crate Scaffold & Shared Types"
# Description: "Creates llamaforge-dispatch library crate and extends llamaforge-types
# with cross-crate port traits (SlotDispatchView, SlotErasure, SlotNotifier) and types
# (SlotId, SlotError, ClientRefreshEvent). Design doc Sections 1.1, 2.4, 2.6."
```

---

## PR2: Error Types, Domain Types & Port Traits

**Branch:** `dispatch/02-types`  
**Depends on:** PR1 merged to `main`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 4–6 hours

### Purpose

Define every domain type, error type, and port trait from design doc Sections 2.5, 2.6: `DispatchError`, `StreamError`, `CancellationPhase`, `HttpStatus`, `CompletionRequest`, `BackendResponseStream`, `BackendClient`, `ClientManager`. Getting the type foundation right early prevents cascading refactors.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 2.1 | Implement `DispatchError` enum | Four variants: `ConnectionFailed` (opaque `Box<dyn Error>`), `BackendError` (uses `HttpStatus`), `Cancelled` (uses `CancellationPhase`), `Timeout`. All with `thiserror`. See design doc Section 2.6. | `src/error.rs` |
| 2.2 | Implement `StreamError` enum | Six variants: `BackendDisconnect`, `Timeout`, `Cancelled`, `ChunkParse { reason }`, `ChannelClosed`, `Internal(Box<dyn Error>)`. NOT `anyhow`. See design doc Section 2.6. | `src/error.rs` |
| 2.3 | Implement `CompletionRequest` struct | `prompt: String`, `n_predict: Option<u32>`, `temperature: f32`, `stream: bool`, `stop: Vec<String>`, `id_slot: Option<SlotId>`. Derive `Debug, Clone`. Does NOT derive `Deserialize`. See design doc Section 2.5. | `src/dispatcher.rs` |
| 2.4 | Define `BackendResponseStream` type alias | `pub type BackendResponseStream = Pin<Box<dyn Stream<Item = Result<Bytes, StreamError>> + Send>>`. See design doc Section 2.5. | `src/client.rs` |
| 2.5 | Implement `BackendClient` trait | `#[async_trait] pub trait BackendClient: Send + Sync` with `post_completion(&self, server_id, payload) -> Result<BackendResponseStream, DispatchError>`. | `src/client.rs` |
| 2.6 | Implement `ClientManager` trait | `pub trait ClientManager: Send + Sync` with `refresh(&self, server_id, event)`. | `src/client.rs` |
| 2.7 | Unit tests: DispatchError Display | Verify all four variants format correctly. Verify `ConnectionFailed.source()` returns the inner error. | `src/error.rs` (tests) |
| 2.8 | Unit tests: StreamError variants | Verify all six variants format correctly. Verify `Internal` wraps and exposes the inner error. | `src/error.rs` (tests) |
| 2.9 | Unit tests: HttpStatus helpers | `HttpStatus(500).is_server_error()` → true. `HttpStatus(404).is_client_error()` → true. `HttpStatus(200).is_server_error()` → false. | `src/error.rs` (tests) |
| 2.10 | Unit tests: CancellationPhase Display | Verify all four variants: `"pre-connect"`, `"pre-stream"`, `"streaming"`, `"cleanup"`. | `src/error.rs` (tests) |
| 2.11 | Compile-time verification | `Arc<dyn BackendClient>` compiles (object-safe). `BackendResponseStream` is `Send`. | — |

### PR2 Acceptance Criteria

- [x] All error types implement `std::error::Error` via `thiserror`
- [x] No `reqwest` types appear in any public error variant
- [x] No `anyhow` dependency anywhere in the crate
- [x] `BackendClient` and `ClientManager` are object-safe
- [x] `CompletionRequest` does NOT derive `Deserialize`
- [x] All 4 unit test suites pass
- [x] `cargo test --workspace` passes

### PR2 Creation Steps

```bash
git checkout -b dispatch/02-types main
# ... implement tasks 2.1–2.11 ...
git add -A && git commit -m "dispatch: add error types, CompletionRequest, and BackendClient port trait"
git push -u origin dispatch/02-types
# Open PR: "Dispatch & Proxy PR2: Error Types, Domain Types & Port Traits"
# Description: "Implements all error types (DispatchError, StreamError with structured
# variants), domain types (CompletionRequest, BackendResponseStream), and port traits
# (BackendClient, ClientManager). Design doc Sections 2.5, 2.6. No reqwest or anyhow
# in public API. 4 test suites."
```

---

## VC-1: Types Verification Checkpoint

**Gate:** PR2 merged. All types compile, all tests pass.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-1.1 | Type safety | Attempt to pass `u32` where `SlotId` is expected → compiler rejects | [x] Does not compile |
| VC-1.2 | HttpStatus safety | Attempt to pass `u16` where `HttpStatus` is expected → compiler rejects | [x] Does not compile |
| VC-1.3 | No reqwest leak | `grep -rn "reqwest" crates/llamaforge-dispatch/src/ --include="*.rs"` — only in `client.rs` | [x] Only `client.rs` matches |
| VC-1.4 | Object safety | `let _: Arc<dyn BackendClient> = ...;` compiles | [x] Compiles |

---

## PR3: DispatchTracker, DispatchTrackerRegistry & DispatchCountAdapter

**Branch:** `dispatch/03-tracker`  
**Depends on:** PR2 merged to `main`  
**Size:** Medium (~300 lines)  
**Estimated Time:** 4–6 hours

### Purpose

Implement the per-server in-flight request tracking infrastructure: `DispatchTracker` (mutex-guarded HashSet), `DispatchTrackerRegistry` (DashMap wrapper), and `DispatchCountAdapter` (bridges to Component 05's `DispatchCountView` port trait). This is the accounting foundation that DispatchToken and Router depend on.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 3.1 | Implement `DispatchTracker` struct | `inner: std::sync::Mutex<HashSet<RequestId>>`. Four methods: `count()` (pub), `insert()` (pub(crate)), `remove()` (pub(crate)), `snapshot_ids()` (pub). All use `unwrap_or_else(|p| { tracing::warn!(...); p.into_inner() })`. See design doc Section 2.3. | `src/tracker.rs` |
| 3.2 | Implement `DispatchTrackerRegistry` struct | `inner: DashMap<ServerId, Arc<DispatchTracker>>` (private). Methods: `get()`, `get_or_create()`. Wraps DashMap — external code never sees DashMap directly. | `src/tracker.rs` |
| 3.3 | Implement `DispatchCountAdapter` struct | `trackers: Arc<DispatchTrackerRegistry>`. Implements `DispatchCountView` from `llamaforge-routing`. `in_flight_count()` delegates to `trackers.get(server_id).map(|t| t.count()).unwrap_or(0)`. | `src/tracker.rs` |
| 3.4 | Add tracing to poison recovery | All four `DispatchTracker` lock sites emit `tracing::warn!("DispatchTracker mutex poisoned, recovering")` on poison. | `src/tracker.rs` |
| 3.5 | Unit tests: insert/remove/count | Insert 3 IDs → count == 3. Remove 1 → count == 2. Remove non-existent → returns false, count unchanged. | tests |
| 3.6 | Unit tests: snapshot_ids | Insert 5 IDs → `snapshot_ids()` returns all 5 (order-independent). Remove 2 → snapshot returns 3. | tests |
| 3.7 | Unit tests: DispatchTrackerRegistry get_or_create | `get(unknown)` → None. `get_or_create(id)` → creates and returns. Second `get(id)` → returns same tracker (Arc identity). | tests |
| 3.8 | Unit tests: DispatchCountAdapter | Registry with server A (3 in-flight), server B (0 in-flight). `in_flight_count(A)` → 3. `in_flight_count(B)` → 0. `in_flight_count(unknown)` → 0. | tests |
| 3.9 | Unit tests: concurrent insert/remove stress | Spawn 50 tasks: 25 insert, 25 remove (overlapping IDs). Assert: no panics, final count consistent with net operations. | tests |
| 3.10 | Unit tests: visibility enforcement | `DispatchTracker::insert()` is callable from within `llamaforge-dispatch` but NOT from external crates. Document this as a compile-time check in the test. | tests |

### PR3 Acceptance Criteria

- [x] All mutex locks use poison recovery pattern (zero bare `unwrap()`)
- [x] `insert()` and `remove()` are `pub(crate)` — external crates cannot bypass DispatchToken
- [x] `DispatchTrackerRegistry` encapsulates DashMap (no `DashMap` in public API)
- [x] `DispatchCountAdapter` correctly implements `DispatchCountView`
- [x] All 6 test suites pass
- [x] Concurrent stress test passes without panics

### PR3 Creation Steps

```bash
git checkout -b dispatch/03-tracker main
# ... implement tasks 3.1–3.10 ...
git add -A && git commit -m "dispatch: add DispatchTracker, Registry, and DispatchCountAdapter"
git push -u origin dispatch/03-tracker
# Open PR: "Dispatch & Proxy PR3: Tracker Infrastructure"
# Description: "Implements per-server in-flight tracking (design doc Sections 2.3, 2.7).
# DispatchTracker with poison-recovering mutex, DispatchTrackerRegistry DashMap wrapper,
# DispatchCountAdapter bridging to DispatchCountView port trait. 6 test suites
# including concurrent stress test."
```

---

## PR4: DispatchToken, SlotDispatchMap & SlotDispatchView Impl

**Branch:** `dispatch/04-token-slotmap`  
**Depends on:** PR3 merged to `main` (`DispatchToken` calls `tracker.insert()`/`remove()` which are defined in PR3)  
**Size:** Medium (~350 lines)  
**Estimated Time:** 5–7 hours

### Purpose

Implement the RAII slot accounting guard (`DispatchToken`) and the bidirectional reconciliation map (`SlotDispatchMap` with `SlotDispatchView` trait impl). These are the two data structures that ensure GPU slots are never leaked and orphaned slots are always detected.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 4.1 | Implement `DispatchToken` struct | `tracker: Arc<DispatchTracker>`, `request_id: RequestId`, `server_id: ServerId`, `slot_notifier: Arc<dyn SlotNotifier>`, `released: bool` (private). `#[must_use]` attribute. See design doc Section 2.2. | `src/token.rs` |
| 4.2 | Implement `DispatchToken::new()` | Calls `tracker.insert(request_id)`. Returns Self. | `src/token.rs` |
| 4.3 | Implement `DispatchToken::release()` | If `!self.released`: call `tracker.remove()`, set `released = true`. If already released: no-op. | `src/token.rs` |
| 4.4 | Implement `DispatchToken::server_id()` | Returns `&ServerId`. | `src/token.rs` |
| 4.5 | Implement `Drop for DispatchToken` | If `!self.released`: call `tracker.remove()` and `slot_notifier.notify_slot_available(&self.server_id)`. The `Drop` is a safety net for panics — in the normal path, the proxy task's cleanup (PR6 task 6.5 step 3) calls `notify_slot_available()` explicitly after `release()`. In the panic case, the scheduler tolerates a missed or duplicate notification (next poll cycle picks it up). | `src/token.rs` |
| 4.6 | Implement `SlotDispatchMap` struct | `inner: std::sync::Mutex<SlotDispatchMapInner>`. `SlotDispatchMapInner` is private: `by_request: HashMap<RequestId, (ServerId, SlotId, Instant)>`, `by_slot: HashMap<(ServerId, SlotId), RequestId>`. | `src/slot_map.rs` |
| 4.7 | Implement `SlotDispatchMap` inherent methods | `insert()`, `remove()`, `get_slot_for_request()`, `get_dispatched_at()`. All use poison recovery. Forward and reverse maps always updated together under the same lock. | `src/slot_map.rs` |
| 4.8 | Implement `SlotDispatchView for SlotDispatchMap` | `get_request_for_slot()` and `has_unresolved()` exposed only through the trait (not as inherent methods). Delegate to lock-guarded inner maps. **Split API rationale:** inherent methods (`insert`, `remove`, `get_slot_for_request`, `get_dispatched_at`) are used within `llamaforge-dispatch` only; trait methods (`get_request_for_slot`, `has_unresolved`) are the cross-crate boundary consumed by `llamaforge-server-mgr` via `Arc<dyn SlotDispatchView>`. Callers of `get_request_for_slot` must `use SlotDispatchView;`. Add a doc comment on the trait impl explaining this split. Optionally add a `trybuild` compile-fail test showing `map.get_request_for_slot()` fails without the trait import. | `src/slot_map.rs` |
| 4.9 | Add tracing to SlotDispatchMap poison recovery | All lock sites emit `tracing::warn!("SlotDispatchMap mutex poisoned")` on poison. | `src/slot_map.rs` |
| 4.10 | Unit tests: DispatchToken RAII — drop releases | Create token → count == 1. Drop → count == 0. | tests |
| 4.11 | Unit tests: DispatchToken explicit release | Create → release() → count == 0. Drop after release → still count == 0 (no double-decrement). | tests |
| 4.12 | Unit tests: DispatchToken must_use | Verify `#[must_use]` attribute compiles. Document that creating and immediately discarding produces a compiler warning. | tests |
| 4.13 | Unit tests: SlotDispatchMap bidirectional consistency | Insert 3 entries. `get_slot_for_request(r1)` → correct `(server, slot)`. `get_request_for_slot(s1, slot1)` → correct request. Remove r1 → both forward and reverse maps updated. | tests |
| 4.14 | Unit tests: SlotDispatchMap dispatched_at | Insert with known time. `get_dispatched_at()` returns correct `Instant`. Remove → returns None. | tests |
| 4.15 | Unit tests: SlotDispatchView has_unresolved | Insert entry for server A. `has_unresolved(A)` → true. Remove entry. `has_unresolved(A)` → false. `has_unresolved(unknown)` → false. | tests |
| 4.16 | Unit tests: SlotDispatchMap empty after full remove cycle | Insert 10 entries. Remove all 10. Both maps empty. `has_unresolved` → false for all servers. | tests |

### PR4 Acceptance Criteria

- [x] `DispatchToken::Drop` always releases the slot (verified by unit test)
- [x] Double-release via `release()` + `Drop` does not double-decrement
- [x] `SlotDispatchMap` maintains bidirectional consistency through all mutations
- [x] `get_request_for_slot` and `has_unresolved` are only accessible via `SlotDispatchView` trait
- [x] All mutex locks use poison recovery
- [x] All 7 test suites pass

### PR4 Creation Steps

```bash
git checkout -b dispatch/04-token-slotmap main
# ... implement tasks 4.1–4.16 ...
git add -A && git commit -m "dispatch: add DispatchToken RAII guard and SlotDispatchMap"
git push -u origin dispatch/04-token-slotmap
# Open PR: "Dispatch & Proxy PR4: DispatchToken & SlotDispatchMap"
# Description: "Implements RAII slot accounting (design doc Section 2.2) and bidirectional
# reconciliation map (design doc Section 2.4). DispatchToken with #[must_use], explicit
# release, and Drop guard. SlotDispatchMap with SlotDispatchView trait impl.
# 7 test suites."
```

---

## VC-2: Core Data Structures Verification Checkpoint

**Gate:** PR3 + PR4 merged. Tracker and slot map work independently.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-2.1 | Token RAII panic safety | Spawn task, create DispatchToken, panic → verify count decremented (Drop fired) | Count assertion passes |
| VC-2.2 | Slot map consistency | Insert 50 random entries, remove 25 randomly → verify `by_request.len() == by_slot.len()` | Assertion passes |
| VC-2.3 | DispatchCountAdapter accuracy | Insert 10 requests across 3 servers → verify `in_flight_count()` matches per-server `count()` | All counts match |
| VC-2.4 | Concurrent token lifecycle | Spawn 100 tasks, each creates + drops a DispatchToken → final count == 0, no panics | Count and no-panic assertions |

---

## PR5: ReqwestClientStore Adapter & Dispatcher Struct

**Branch:** `dispatch/05-dispatcher`  
**Depends on:** PR3 and PR4 both merged to `main`  
**Size:** Medium (~400 lines)  
**Estimated Time:** 6–8 hours

### Purpose

Wire the infrastructure layer (`ReqwestClientStore` implementing `BackendClient` + `ClientManager`) and the `Dispatcher` struct itself: construction via `new()`, the `dispatch()` method that spawns proxy tasks, and the `refresh_client()` method. The proxy task body is a stub in this PR — full streaming comes in PR6.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 5.1 | Implement `ReqwestClientStore` struct | [x] `clients: DashMap<ServerId, arc_swap::ArcSwap<reqwest::Client>>` (private). Constructor takes server configs and builds initial clients. | `src/client.rs` |
| 5.2 | Implement `ReqwestClientStore::build_client()` | [x] Private helper. Configures `reqwest::ClientBuilder` with: `pool_idle_timeout(10s)`, `pool_max_idle_per_host(max(parallel_slots, 16))`, `SO_KEEPALIVE(idle=10s, interval=5s, probes=3)`. See design doc Section 6 (Adapter-specific config). | `src/client.rs` |
| 5.3 | Implement `BackendClient for ReqwestClientStore` | [x] `post_completion()`: load client via `ArcSwap::load()`, build POST to `http://127.0.0.1:{port}/completion`, serialize `CompletionRequest` to llama-server JSON format, send request, wrap `reqwest::Response` into `BackendResponseStream`. Map `reqwest::Error` → `DispatchError::ConnectionFailed` with `Box<dyn Error>`. | `src/client.rs` |
| 5.4 | Implement `ClientManager for ReqwestClientStore` | [x] `refresh()`: build new client from `ClientRefreshEvent`, atomically swap via `ArcSwap::store()`. Old client drains naturally. Log at `tracing::info!`. | `src/client.rs` |
| 5.5 | Implement `Dispatcher` struct | [x] All fields per design doc Section 2.1: `server_state: Arc<dyn ServerStateView>`, `metrics_ingest: MetricsIngest`, `trackers: Arc<DispatchTrackerRegistry>`, `slot_dispatch_map: Arc<SlotDispatchMap>`, `config: Arc<ConfigHandle>`, `backend_client: Arc<dyn BackendClient>`, `client_manager: Arc<dyn ClientManager>`, `slot_notifier: Arc<dyn SlotNotifier>`, `proxy_tasks: Mutex<JoinSet<()>>` (tracks spawned proxy tasks for graceful shutdown — design doc Graceful Shutdown step 5). All fields private. | `src/dispatcher.rs` |
| 5.6 | Implement `Dispatcher::new()` | [x] Constructor accepting all dependencies. | `src/dispatcher.rs` |
| 5.7 | Implement `Dispatcher::dispatch()` | [x] Step 1: Get or create tracker for `routing.server_id`. Step 2: Create `DispatchToken` (pass `Arc::clone(&self.slot_notifier)`). Step 3: Build `CompletionRequest` from `DispatchCandidate`. Step 4: Add task to `proxy_tasks` JoinSet via `self.proxy_tasks.lock()...spawn(proxy_task(...))` — pass `slot_notifier` clone for explicit cleanup notification. Step 5: Return handle. The `proxy_task` body is a stub for now (PR6 fills it in). | `src/dispatcher.rs` |
| 5.8 | Implement `Dispatcher::tracker()` | [x] Delegate to `trackers.get()`. | `src/dispatcher.rs` |
| 5.9 | Implement `Dispatcher::slot_dispatch_map()` | [x] Return `Arc::clone(&self.slot_dispatch_map)`. | `src/dispatcher.rs` |
| 5.10 | Implement `Dispatcher::refresh_client()` | [x] Delegate to `client_manager.refresh()`. | `src/dispatcher.rs` |
| 5.10a | Implement `Dispatcher::shutdown()` | [x] Async method. Waits for all proxy tasks in `JoinSet` to complete with a 5s timeout (`tokio::time::timeout(Duration::from_secs(5), join_set.join_all())`). On timeout: log warning with count of remaining tasks, return. See design doc Graceful Shutdown step 5. | `src/dispatcher.rs` |
| 5.10b | Implement `Dispatcher::drain_server()` | [x] Async method taking `server_id: &ServerId`. Cancel the per-server `CancellationToken`, then await only that server's tasks. Used by Server Manager when stopping a single backend. Returns number of drained tasks. | `src/dispatcher.rs` |
| 5.11 | Implement stub `proxy_task()` | [x] Async fn that takes all context (token, client, request, channels). Stub: immediately release token and send a placeholder response. PR6 replaces this with the real streaming logic. | `src/proxy.rs` |
| 5.12 | Unit tests: Dispatcher construction | [x] Construct with mock `BackendClient`, mock `ClientManager`, mock `ServerStateView`. Verify all fields accessible. | tests |
| 5.13 | Unit tests: dispatch creates token | [x] Call `dispatch()` → verify `tracker.count() == 1`. Drop the returned token → count == 0. | tests |
| 5.14 | Unit tests: refresh_client delegates | [x] Call `refresh_client()` with mock `ClientManager` → verify mock received the event. | tests |
| 5.15 | Unit tests: ReqwestClientStore ArcSwap | [x] Create store, call `refresh()` → verify new client is returned by subsequent loads. Old loads still work (Arc reference). | tests |
| 5.16 | Integration test: dispatch + stub proxy | [x] Dispatch a request → verify stub proxy completes, token released, response channel receives something. | tests |

### PR5 Acceptance Criteria

- [x] `Dispatcher::new()` accepts only port traits (no concrete types except `MetricsIngest`)
- [x] `dispatch()` spawns a tokio task into the `proxy_tasks` JoinSet
- [x] `shutdown()` and `drain_server()` methods compile and handle empty JoinSet gracefully
- [x] `ReqwestClientStore` is the ONLY file importing `reqwest`
- [x] `ArcSwap::store()` atomically swaps client (no blocking)
- [x] All 5 test suites pass
- [x] Stub proxy task releases token and sends response

### PR5 Creation Steps

```bash
git checkout -b dispatch/05-dispatcher main
# ... implement tasks 5.1–5.16 ...
git add -A && git commit -m "dispatch: add ReqwestClientStore adapter and Dispatcher struct"
git push -u origin dispatch/05-dispatcher
# Open PR: "Dispatch & Proxy PR5: Dispatcher & ReqwestClientStore"
# Description: "Implements Dispatcher struct (design doc Section 2.1) with all dependencies
# injected as port traits. ReqwestClientStore adapter (design doc Section 2.5) with
# ArcSwap for atomic client rotation. Proxy task is a stub — streaming comes in PR6.
# 5 test suites."
```

---

## PR6: Proxy Task — Streaming, Backpressure & Cancellation

**Branch:** `dispatch/06-proxy`  
**Depends on:** PR5 merged to `main`  
**Size:** Large (~400 lines)  
**Estimated Time:** 8–10 hours

### Purpose

Replace the stub proxy task with the real streaming implementation: two-phase `select!` loop, cancellation handling, backpressure timeout, slot release before audit, and all stream lifecycle events. This is the highest-risk code in the entire component.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 6.1 | Implement `proxy_task()` Phase 1: connect | Call `backend_client.post_completion()`. Handle `DispatchError::ConnectionFailed` → release token, send error to response channel. Handle `DispatchError::BackendError` → release token, send error. Record `CancellationPhase::PreConnect` on cancel. | `src/proxy.rs` |
| 6.2 | Implement `proxy_task()` Phase 2: streaming loop | Two-phase `select!` loop per design doc Section 3.3. Phase 1: `biased` select between `cancel_token.cancelled()` and `backend_stream.chunk()`. Phase 2: `biased` select between `cancel_token.cancelled()` and `response_channel.send(parsed)`. | `src/proxy.rs` |
| 6.3 | Implement backpressure timeout | In Phase 2: if `response_channel.send()` fails, apply `tokio::time::timeout(2s, response_channel.reserve())`. On timeout: break loop, disconnect. See design doc Section 3.3 and FM-5. | `src/proxy.rs` |
| 6.4 | Implement SSE chunk parsing | `parse_sse_chunk(&chunk) -> Result<ParsedChunk, StreamError::ChunkParse>`. Extract `data:` fields from SSE stream. Handle `data: [DONE]` sentinel. | `src/proxy.rs` |
| 6.5 | Implement `proxy_task()` Phase 3: cleanup | Step 1: `drop(response_handle)` (close TCP). Step 2: `dispatch_token.release()` (free slot — tracker decremented, but `Drop` notification suppressed since `released=true`). Step 3: `slot_notifier.notify_slot_available(&server_id)` (explicit primary-path notification). Step 4: Detached `tokio::spawn` for audit write with `timeout(10s, audit_tx.send(record))`. Slot is already freed — audit stall never holds hardware. See design doc Section 3.1 steps 6–7. | `src/proxy.rs` |
| 6.6 | Record `CancellationPhase` correctly | `PreConnect` (before POST), `PreStream` (after POST, before first chunk), `Streaming` (during chunk loop), `Cleanup` (during cleanup phase). On cancellation during `Streaming` phase: write a structured `FINISH_REASON_UPSTREAM_FAILURE` error payload to the response channel before breaking, so the client receives a clean termination signal (see design doc Graceful Shutdown step 3). | `src/proxy.rs` |
| 6.7 | Update slot_dispatch_map | After successful slot assignment (from `/slots` poll correlation or `id_slot` header): call `slot_dispatch_map.insert()`. On cleanup: call `slot_dispatch_map.remove()`. | `src/proxy.rs` |
| 6.8 | Add dispatch metrics | Record `llamaforge_dispatches_total`, `llamaforge_dispatch_duration_seconds`, `llamaforge_ttft_seconds`, `llamaforge_tokens_completion_total`, `llamaforge_backpressure_disconnects_total`, `llamaforge_pending_dispatches`. See design doc Section 7.1. | `src/proxy.rs` |
| 6.9 | Add dispatch tracing spans | `dispatch.started` (DEBUG), `dispatch.completed` (DEBUG), `dispatch.failed` (WARN), `dispatch.cancelled` (DEBUG), `dispatch.backpressure` (WARN). See design doc Section 7.2. | `src/proxy.rs` |
| 6.10 | Tokio test: successful streaming | Mock backend returns 5 SSE chunks. Verify: all 5 received by response channel, token released, audit record sent. | tests |
| 6.11 | Tokio test: cancellation before first chunk | Cancel token immediately after dispatch. Verify: `StreamError::Cancelled` yielded, token released, `CancellationPhase::PreStream`. | tests |
| 6.12 | Tokio test: cancellation mid-stream | Mock backend returns 3 chunks, cancel after 2nd. Verify: 2 chunks received, then `Cancelled`, token released. | tests |
| 6.13 | Tokio test: backend disconnect | Mock backend sends 2 chunks then drops connection. Verify: 2 chunks received, then `BackendDisconnect`, token released. | tests |
| 6.14 | Tokio test: backpressure timeout | Mock slow consumer (never reads). Verify: proxy disconnects after 2s, token released, `backpressure_disconnects_total` incremented. | tests |
| 6.15 | Tokio test: slot released before audit | Mock audit channel that blocks forever. Verify: slot freed within 1ms of stream end, audit send eventually times out (10s), slot was never held during audit wait. | tests |
| 6.16 | Tokio test: connection refused | Mock backend returns connection error. Verify: `DispatchError::ConnectionFailed`, token released, no stream started. | tests |
| 6.17 | Tokio test: backend HTTP 500 | Mock backend returns 500. Verify: `DispatchError::BackendError { status: HttpStatus(500) }`, token released. | tests |
| 6.18 | Tokio test: TTFT metric recorded | Mock backend sends first chunk after 100ms. Verify: `llamaforge_ttft_seconds` histogram recorded with value ≈ 0.1s. | tests |
| 6.19 | Tokio test: graceful shutdown | Dispatch 3 requests with slow mock backends (500ms each). Call `dispatcher.shutdown()`. Verify: all 3 proxy tasks receive cancellation, write `FINISH_REASON_UPSTREAM_FAILURE`, complete within 5s timeout. JoinSet empty after shutdown. | tests |
| 6.20 | Tokio test: drain_server | Dispatch 2 requests to server A, 1 to server B. Call `drain_server(A)`. Verify: server A tasks cancelled and drained, server B task unaffected and still running. | tests |

### PR6 Acceptance Criteria

- [ ] Two-phase `select!` loop is correct (no cross-branch variable references)
- [ ] Cancellation checked with `biased` in both phases
- [ ] Slot released BEFORE audit send (invariant #1)
- [ ] Backpressure timeout disconnects after 2s (FM-5)
- [ ] `CancellationPhase` correctly recorded for all cancel paths
- [ ] Graceful shutdown: `dispatcher.shutdown()` drains all proxy tasks within 5s timeout
- [ ] `drain_server()` cancels only the targeted server's tasks
- [ ] `FINISH_REASON_UPSTREAM_FAILURE` written on cancellation during streaming phase
- [ ] All 11 test suites pass
- [ ] No `.unwrap()` in production code

### PR6 Creation Steps

```bash
git checkout -b dispatch/06-proxy main
# ... implement tasks 6.1–6.18 ...
git add -A && git commit -m "dispatch: implement proxy task with streaming, backpressure, and cancellation"
git push -u origin dispatch/06-proxy
# Open PR: "Dispatch & Proxy PR6: Proxy Task"
# Description: "Full proxy task implementation (design doc Sections 3.1, 3.3, 3.4).
# Two-phase select! streaming loop, cooperative cancellation with CancellationPhase,
# 2s backpressure timeout, slot release before audit, SSE chunk parsing, all dispatch
# metrics and tracing spans. 9 test suites including TTFT measurement."
```

---

## VC-3: Full Dispatch Pipeline Verification Checkpoint

**Gate:** PR1–PR6 merged. The entire dispatch → proxy → release pipeline works.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-3.1 | Full lifecycle | Dispatch request → stream 10 chunks → complete → verify: token released, slot_dispatch_map empty, audit sent, metrics recorded | All assertions pass |
| VC-3.2 | Token never leaked | Dispatch 5 requests. Cancel 2 mid-stream, let 2 complete, disconnect 1. Verify: tracker count == 0 for all servers | Count assertion passes |
| VC-3.3 | Cancellation priority | Cancel token and backend chunk arrive simultaneously → verify cancel wins (biased select) | `StreamError::Cancelled` yielded |
| VC-3.4 | Client refresh | Dispatch → backend "restarts" → `ClientRefreshEvent` → next dispatch uses new client | New client verified |
| VC-3.5 | Backpressure disconnect | Slow consumer → verify disconnect after 2s, token released, metric incremented | Timer and metric assertions |
| VC-3.6 | Connection pool poisoning recovery | Stale connection → `ConnectionFailed` → token released → subsequent dispatch with fresh client succeeds | Second dispatch succeeds |
| VC-3.7 | Concurrent dispatches | 32 concurrent dispatches → all complete or error → tracker count == 0 | No leaked tokens |
| VC-3.8 | Panic safety | Inject panic in chunk parsing → verify DispatchToken::Drop fires → tracker count decremented | Count assertion passes |

**Integration test file:** `tests/vc3_dispatch_pipeline.rs`

---

## PR7: Reconciliation Logic

**Branch:** `dispatch/07-reconciliation`  
**Depends on:** PR6 merged to `main`  
**Size:** Medium (~250 lines)  
**Estimated Time:** 4–6 hours

### Purpose

Implement the bidirectional reconciliation logic (forward: detect orphaned slots; reverse: detect completed requests) and its scheduling. This is the safety net that catches state drift between LlamaForge and llama-server.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 7.1 | Implement `first_seen_processing` tracker | `HashMap<(ServerId, SlotId), Instant>`. Tracks when we first observe a slot processing that isn't in `slot_dispatch_map`. Cleaned every cycle. | `src/dispatcher.rs` (or `src/reconciliation.rs`) |
| 7.2 | Implement forward reconciliation | For each slot with `is_processing=true` not in `slot_dispatch_map`: add to `first_seen_processing`. If age > `grace_period`: call `slot_erasure.erase_slot()` (fire-and-forget with 5s timeout). Grace = `max(30s, ctx_size / 500)`. See design doc Section 3.2. | same |
| 7.3 | Implement reverse reconciliation | For each entry in `slot_dispatch_map` for this server: if slot NOT processing AND `dispatched_at > grace_period` → remove from `slot_dispatch_map`. Skip entirely if `has_unresolved()` is true. | same |
| 7.4 | Implement cleanup | Remove `first_seen_processing` entries for slots that are: (a) now in `slot_dispatch_map`, or (b) no longer `is_processing`. | same |
| 7.5 | Implement reconciliation scheduling | Runs on every `/slots` poll (5 seconds). Called by Server Manager after each poll cycle. | same |
| 7.6 | Add reconciliation metrics | `llamaforge_reconciliation_erasures_total` counter with `direction={forward,reverse}` label. | same |
| 7.7 | Add reconciliation tracing | `reconciliation.erasure` (INFO), `reconciliation.skipped` (DEBUG). See design doc Section 7.2. | same |
| 7.8 | Unit test: forward reconciliation detects orphan | Slot processing, not in map, age > grace → erasure called. | tests |
| 7.9 | Unit test: forward reconciliation respects grace | Slot processing, not in map, age < grace → no erasure. | tests |
| 7.10 | Unit test: dynamic grace period | Server with `ctx_size = 128000` → grace = `max(30, 256) = 256s`. Server with `ctx_size = 4000` → grace = `max(30, 8) = 30s`. | tests |
| 7.11 | Unit test: reverse reconciliation cleans stale entry | Entry in map, slot not processing, age > grace → entry removed. | tests |
| 7.12 | Unit test: reverse reconciliation skips when unresolved | Server has unresolved dispatches → reverse reconciliation skipped entirely. | tests |
| 7.13 | Unit test: first_seen_processing cleanup | Entry cleaned when slot enters `slot_dispatch_map` or stops processing. | tests |
| 7.14 | Tokio test: end-to-end reconciliation cycle | Dispatch request → backend completes without our knowledge (simulate by clearing slot_dispatch_map) → next reconciliation detects stale entry → cleanup. | tests |

### PR7 Acceptance Criteria

- [ ] Forward reconciliation uses `slot_erasure.erase_slot()` (not direct HTTP)
- [ ] Reverse reconciliation skips when `has_unresolved()` is true
- [ ] Dynamic grace period formula: `max(30s, ctx_size / 500)`
- [ ] `first_seen_processing` cleaned every cycle (no memory leak)
- [ ] All 7 test suites pass

### PR7 Creation Steps

```bash
git checkout -b dispatch/07-reconciliation main
# ... implement tasks 7.1–7.14 ...
git add -A && git commit -m "dispatch: add bidirectional reconciliation with dynamic grace periods"
git push -u origin dispatch/07-reconciliation
# Open PR: "Dispatch & Proxy PR7: Reconciliation"
# Description: "Implements bidirectional slot reconciliation (design doc Section 3.2).
# Forward: detects orphaned slots via SlotErasure port trait. Reverse: cleans stale
# map entries. Dynamic grace period (max(30s, ctx_size/500)). Cleanup prevents
# memory leaks. 7 test suites."
```

---

## PR8: Property Tests, VC-3 Integration Tests & Design Doc Test Cases

**Branch:** `dispatch/08-verification`  
**Depends on:** PR7 merged to `main`  
**Size:** Medium (~350 lines of tests)  
**Estimated Time:** 5–7 hours

### Purpose

The final PR adds all integration tests (VC-3), the specific test cases from the design doc Section 8.3, and stress/property tests for the dispatch pipeline. This PR is pure test code — no production logic changes.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 8.1 | Implement VC-3 integration tests | All 8 checks from VC-3. Each as a separate `#[tokio::test]` function with mock BackendClient. | `tests/vc3_dispatch_pipeline.rs` |
| 8.2 | Implement design doc Section 8.3 test cases (non-duplicate only) | 5 unique tests for `design_doc_tests.rs`: `test_cancellation_checked_before_chunk`, `test_slot_released_before_audit_send`, `test_client_refresh_on_server_restart`, `test_backpressure_timeout_disconnects`, `test_non_streaming_absolute_timeout`. The remaining 3 design doc tests are already covered: `test_dispatch_token_released_on_drop` (PR4 task 4.10), `test_dispatch_token_explicit_release_before_audit` (PR4 task 4.11), `test_reconciliation_grace_period` (PR7 task 7.10). Add doc comments in `design_doc_tests.rs` cross-referencing the earlier PRs. | `tests/design_doc_tests.rs` |
| 8.3 | Property test: token lifecycle | `∀ dispatch: after completion or error, tracker.count() for that server decrements by 1` | `tests/proptest_dispatch.rs` |
| 8.4 | Property test: slot_dispatch_map consistency | `∀ operations: by_request.len() == by_slot.len()` | `tests/proptest_dispatch.rs` |
| 8.5 | Property test: reconciliation bounded | `∀ cycle: first_seen_processing.len() ≤ total_slots` | `tests/proptest_dispatch.rs` |
| 8.6 | Property test: grace period monotonic | `∀ (ctx_a, ctx_b) where ctx_a > ctx_b: grace(ctx_a) >= grace(ctx_b)` | `tests/proptest_dispatch.rs` |
| 8.7 | Stress test: 100 concurrent dispatches | All complete or error. Final tracker count == 0. No panics. No leaked tokens. | `tests/stress.rs` |
| 8.8 | Stress test: rapid client refresh | 10 refreshes interleaved with 50 dispatches → all dispatches complete, no stale connections used after refresh. | `tests/stress.rs` |
| 8.9 | Add `#[ignore]` slow tests | Tag property-based and stress tests with `#[ignore]`. Run with `cargo test -- --ignored`. | — |
| 8.10 | Final CI check | `cargo test --workspace`, `cargo test --workspace -- --ignored`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`. | — |

### PR8 Acceptance Criteria

- [ ] All 8 VC-3 integration tests pass
- [ ] All 5 unique design doc Section 8.3 test cases pass (3 already covered in PR4/PR7)
- [ ] All 4 property tests pass (256 cases each)
- [ ] Both stress tests pass without panics or leaked tokens
- [ ] `cargo test --workspace` passes (excluding `#[ignore]`)
- [ ] `cargo test --workspace -- --ignored` passes (including slow tests)
- [ ] No production code changes in this PR — test code only

### PR8 Creation Steps

```bash
git checkout -b dispatch/08-verification main
# ... implement tasks 8.1–8.10 ...
git add -A && git commit -m "dispatch: add integration tests, property tests, and stress tests"
git push -u origin dispatch/08-verification
# Open PR: "Dispatch & Proxy PR8: Verification & Stress Tests"
# Description: "Pure test PR. Adds 8 VC-3 integration tests, 8 design doc test cases,
# 4 proptest property-based tests, and 2 stress tests (100 concurrent dispatches,
# rapid client refresh). Total: 22 new test functions. No production code changes."
```

---

## VC-4: Final Sign-Off Checkpoint

**Gate:** All 8 PRs merged. Dispatch & Proxy is complete.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-4.1 | All tests pass | `cargo test --workspace && cargo test --workspace -- --ignored` | Zero failures |
| VC-4.2 | Zero clippy warnings | `cargo clippy --workspace -- -D warnings` | Exit code 0 |
| VC-4.3 | Code coverage | Run `cargo llvm-cov --workspace`. Target > 80% line coverage for `llamaforge-dispatch`. | Coverage report generated |
| VC-4.4 | Public API audit | Review `lib.rs` re-exports. Verify only intended types are public. `SlotDispatchMapInner` is NOT public. `ReqwestClientStore.clients` is NOT public. `DispatchTracker::insert()`/`remove()` are `pub(crate)`. | Manual review |
| VC-4.5 | Design doc compliance | Cross-check every Section in `06_DISPATCH_AND_PROXY.md` against the implementation. Check all 8 Key Invariants. | Manual review with checklist |
| VC-4.6 | Documentation | Every public type, trait, method, and enum variant has a doc comment. `cargo doc --no-deps -p llamaforge-dispatch` builds without warnings. | `cargo doc` succeeds |
| VC-4.7 | Dependency audit | `cargo deny check` passes. No duplicate dependency versions. | `cargo deny` green |
| VC-4.8 | No `unwrap()` audit | `grep -rn "unwrap()" crates/llamaforge-dispatch/src/ --include="*.rs"` — only `unwrap_or_else` for poison recovery. Zero bare `unwrap()`. | Zero matches |
| VC-4.9 | reqwest isolation audit | `grep -rn "reqwest" crates/llamaforge-dispatch/src/ --include="*.rs"` — only in `client.rs`. | Only `client.rs` matches |
| VC-4.10 | Memory bounds | Run dispatch pipeline with 32 concurrent slots across 8 servers. Measure RSS → verify < 5 MB (design doc: ~3.1 MB worst case + overhead). | Memory within bounds |
| VC-4.11 | Lock ordering audit | Verify no code path holds `SlotDispatchMap.inner` and `DispatchTracker.inner` simultaneously. Each lock is independent. | Manual review |

---

## Summary: PR → File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `crates/llamaforge-types/src/ports.rs` (incl. `SlotNotifier`), `crates/llamaforge-dispatch/src/lib.rs`, `Cargo.toml` | ~200 |
| PR2 | `src/error.rs`, `src/client.rs` (types only), `src/dispatcher.rs` (types only) | ~350 |
| PR3 | `src/tracker.rs` | ~300 |
| PR4 | `src/token.rs`, `src/slot_map.rs` | ~350 |
| PR5 | `src/client.rs` (adapter impl), `src/dispatcher.rs` (incl. JoinSet, shutdown, drain), `src/proxy.rs` (stub) | ~450 |
| PR6 | `src/proxy.rs` (full impl incl. graceful shutdown tests) | ~450 |
| PR7 | `src/reconciliation.rs` (or `src/dispatcher.rs` additions) | ~250 |
| PR8 | `tests/vc3_dispatch_pipeline.rs`, `tests/design_doc_tests.rs`, `tests/proptest_dispatch.rs`, `tests/stress.rs` | ~350 |
| **Total** | | **~2,700** |

> Note: LOC estimate exceeds the design doc's ~2,500 because tests account for ~750 lines, the adapter code (ReqwestClientStore) adds ~100 lines, and graceful shutdown infrastructure (JoinSet, `shutdown()`, `drain_server()`) adds ~50 lines. Production code is ~1,900 lines.

---

## Timeline (Single Engineer)

| Day | Work | PRs |
|-----|------|-----|
| Day 1 | Crate scaffold + shared types in llamaforge-types | PR1 |
| Day 2 | Error types, domain types, port traits | PR2 |
| Day 3 | DispatchTracker, Registry, DispatchCountAdapter | PR3 |
| Day 4 | DispatchToken, SlotDispatchMap, SlotDispatchView | PR4 |
| Day 5 | ReqwestClientStore adapter + Dispatcher struct | PR5 |
| Day 6 | Proxy task: connect phase, streaming loop | PR6 (part 1) |
| Day 7 | Proxy task: backpressure, cancellation, metrics | PR6 (part 2) |
| Day 8 | Reconciliation logic + scheduling | PR7 |
| Day 9 | Integration tests, property tests, stress tests | PR8 |
| Day 10 | Final verification (VC-4), code review fixes, docs | — |
| Day 11 | Buffer for review feedback and edge cases | — |

## Timeline (Two Engineers)

| Day | Engineer A | Engineer B |
|-----|-----------|-----------|
| Day 1 | PR1 (scaffold) + PR2 (types) | Reviews PR1/PR2, writes mock BackendClient and test fixtures |
| Day 2 | PR3 (tracker infrastructure) | Reviews PR3, writes test harness for PR4/PR5 |
| Day 3 | PR4 (DispatchToken + SlotDispatchMap) | Reviews PR4, starts PR6 mock backend harness |
| Day 4 | PR5 (ReqwestClientStore + Dispatcher) | Reviews PR5 |
| Day 5 | PR6 (proxy task — full impl, pair programming) | PR6 (pair programming — both engineers on `proxy.rs`, same branch) |
| Day 6 | PR7 (reconciliation) | PR8 (integration tests, property tests) |
| Day 7 | VC-4 verification, docs | Stress tests, code review fixes |
| Day 8 | Buffer for review feedback | Buffer for edge cases |

> **Note on Day 5:** PR6 is a single PR modifying `src/proxy.rs`. Both engineers work on the same branch via pair programming or sequential commits — NOT parallel branches. This is the highest-risk code (streaming select! loop, backpressure, cancellation) and benefits from real-time review.

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Two-phase `select!` incorrectly structured | Medium | High | Design doc Section 3.3 documents the original logic error and correct pattern. PR6 tests verify cancellation wins over ready chunk. Code review must verify no cross-branch variable references. |
| `reqwest` leaks into domain types | Medium | Medium | Ground rule #10 + VC-4.9 grep audit. `client.rs` is the only file importing reqwest. PR2 verifies no reqwest in error types. |
| Mutex poison recovery masks data corruption | Low | Medium | Critical section is pure HashMap manipulation. Poison only happens on logic bug panic. Recovery allows continued operation at risk of 1 misplaced entry. Logged as `tracing::warn!`. |
| `ArcSwap` performance under contention | Low | Low | `ArcSwap::load()` is wait-free. `store()` only called on server restart (rare). Concurrent loads (dispatch) never block. |
| Backpressure timeout too aggressive (2s) | Medium | Low | Hardcoded in v1. If operators report false disconnects on slow networks: make configurable in v1.1. `llamaforge_backpressure_disconnects_total` metric alerts to the issue. |
| Reconciliation grace period too short | Low | Medium | Dynamic formula `max(30s, ctx_size/500)`. For 128K context: 262s grace. Test with real llama-server prefill times during integration. `llamaforge_reconciliation_erasures_total` metric alerts to premature erasures. |
| SlotDispatchMap grows unbounded | Low | Medium | Bounded by `total_slots` across all servers (128 max). Reconciliation cleans stale entries every 5s. Memory bounds table in design doc: 25KB worst case. |
| Tokio task leak from proxy tasks | Low | High | `DispatchToken::Drop` ensures cleanup even on panic. VC-3.2 and VC-3.7 verify zero leaked tokens. `llamaforge_pending_dispatches` gauge alerts to drift. JoinSet for task monitoring. |
| `arc-swap` crate version conflicts | Low | Low | Pin in workspace `Cargo.toml`. Already a workspace dependency from PR1. |
| `first_seen_processing` HashMap grows without bound | Low | Low | Cleaned every reconciliation cycle (5s). Bounded by total_slots (128). Design doc Section 3.2 explicitly documents cleanup. |

---

## Integration Points (for next component)

After Dispatch & Proxy is complete, the following integration seams are ready:

1. **Request Queue** holds `Arc<dyn SlotNotifier>` (trait defined in `llamaforge-types` — PR1 task 1.10a; implemented by Dispatcher — PR5). The proxy task's cleanup phase calls `notify_slot_available()` after `dispatch_token.release()` (primary path). `DispatchToken::Drop` also calls it as a safety net for panics. `DispatchCandidate` is consumed by `Dispatcher::dispatch()`.
2. **Smart Routing** provides `RoutingDecision` consumed by `dispatch()`. `DispatchCountAdapter` implements `DispatchCountView` for Router's scoring. `ServerStateView` provides read-only slot state.
3. **Server Manager** provides `ClientRefreshEvent` on backend restart. Consumes `Arc<dyn SlotDispatchView>` for reconciliation reads. Implements `SlotErasure` for orphan slot erasure. Calls `Dispatcher::drain_server()` before SIGTERM and `Dispatcher::shutdown()` on global shutdown — both with 5s timeout (design doc Graceful Shutdown step 5).
4. **Transport Layer** consumes `BackendResponseStream` (`Pin<Box<dyn Stream<Item = Result<Bytes, StreamError>> + Send>>`). Handles `StreamError` variants for client-facing error messages.
5. **Metrics & Storage** provides `MetricsIngest` (audit channel). Receives audit records via fire-and-forget 10s timeout send.
6. **`main.rs`** (composition root) constructs: `Arc<DispatchTrackerRegistry>` → `Arc<DispatchCountAdapter>` (inject into Router) → `Arc<SlotDispatchMap>` → `ReqwestClientStore` (as `Arc<dyn BackendClient>` + `Arc<dyn ClientManager>`) → `Dispatcher` impl as `Arc<dyn SlotNotifier>` (inject into Request Queue) → `Dispatcher::new()`.