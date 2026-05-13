# Smart Routing — Implementation Plan

**Component:** Smart Routing (`05_SMART_ROUTING.md`)  
**Estimated LOC:** ~1,350 (~900 production + ~450 tests)  
**Estimated Duration:** 5–7 working days (single engineer) / 3–4 days (two engineers, PR3/PR4 parallelizable)  
**Design Document:** `component-designs/05_SMART_ROUTING.md` (R2 PASS — all 24 findings addressed)  
**Branch Prefix:** `routing/`  
**Target:** `main` (via squash-merge PRs)  
**Upstream Dependencies:** Config Manager (`llamaforge-config`), Server Manager (`llamaforge-types` with `ServerStateView` trait), Request Queue (`llamaforge-types` with `QueuedRequest`, `ModelTarget`)

> **Branching workflow:** Each PR is squash-merged to `main` before the next PR branches. All `git checkout -b` commands branch from `main`, which is correct because the predecessor PR has already been merged. The "Depends on" field on each PR indicates which predecessor(s) must be merged to `main` before that PR's branch is created. Do NOT branch from a predecessor's feature branch — always branch from `main` after the predecessor is merged.

---

## Ground Rules for CoderAgents

1. **Read the design doc first.** Every PR description must reference the specific design doc sections it implements.
2. **No `unwrap()` or `expect()` in production code paths.** Use `?` propagation, `unwrap_or_else`, or explicit `match`. Mutex locks use `unwrap_or_else(|p| p.into_inner())` for poison recovery.
3. **All Router methods are synchronous.** Async callers MUST wrap calls in `spawn_blocking`. Never add `.await` inside the Router.
4. **Every public type gets a doc comment.** Reference the design doc section number.
5. **Newtype wrappers for domain IDs.** Use `SessionId`, `LoraAdapterId`, `ServerId` — never raw `String` for domain identifiers.
6. **Tests run with `cargo test --workspace`.** Every PR must pass before merge.
7. **`cargo clippy -- -D warnings` must pass.** Zero warnings policy.
8. **Commit messages:** `routing: <imperative summary>` (e.g., `routing: add Router struct with scoring formula`).
9. **Port traits over concrete types.** The Router depends on `dyn ServerStateView`, `dyn DispatchCountView` — never on concrete `ServerManager` or `DispatchTracker`. Consumers depend on `dyn CapacityChecker`, `dyn ServerSelector`, `dyn RouteValidator` — never on `Arc<Router>` directly.
10. **Private fields, public getters.** `RoutingDecision` fields are `pub(crate)` or private. Consumers access via `server_id()`, `reason()`, `into_server_id()`. Newtypes (`SessionId`, `LoraAdapterId`) have private inner fields with `new()` constructors and `as_str()` getters.

---

## Dependency Graph (PR merge order)

```
PR1 ─── PR2 ──┬── PR3 ──┬── PR5 ─── PR6
              └── PR4 ──┘
```

PR3 and PR4 can be developed concurrently after PR2 merges. PR5 depends on both PR3 and PR4 being merged. PR6 is strictly sequential after PR5.

---

## PR1: Crate Scaffold & Port Traits

**Branch:** `routing/01-scaffold`  
**Merge Target:** `main`  
**Size:** Small (~150 lines)  
**Estimated Time:** 2–3 hours

### Purpose

Create the `llamaforge-routing` library crate. Define the port trait that Smart Routing owns (`DispatchCountView`) and the three consumer-facing port traits (`CapacityChecker`, `ServerSelector`, `RouteValidator`). Wire workspace dependencies.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 1.1 | Create `llamaforge-routing` lib crate | `cargo init --lib crates/llamaforge-routing`. This crate owns all Smart Routing logic. | `crates/llamaforge-routing/` |
| 1.2 | Add workspace member | Add `crates/llamaforge-routing` to workspace `Cargo.toml` `[workspace]` members list. | `Cargo.toml` |
| 1.3 | Add workspace-level dependencies | `lru = "0.12"`. Verify `thiserror = "2"`, `tracing`, `metrics`, `serde`, `serde_json` already present from prior components. | `Cargo.toml` |
| 1.4 | Wire crate dependencies | `llamaforge-routing/Cargo.toml` inherits workspace deps: `llamaforge-types`, `llamaforge-config`, `lru`, `tracing`, `metrics`, `thiserror`, `serde`. Dev-deps: `serde_json` (for serde roundtrip tests). | `crates/llamaforge-routing/Cargo.toml` |
| 1.5 | Create module structure | `lib.rs` declares modules: `pub mod router;`, `pub mod affinity;`, `pub mod types;`, `pub mod weights;`, `pub mod error;`, `pub mod config;`. Stub each module file with `//! Module: <name>` doc comment. | `crates/llamaforge-routing/src/` |
| 1.6 | Define `DispatchCountView` port trait | `pub trait DispatchCountView: Send + Sync { fn in_flight_count(&self, server_id: &ServerId) -> usize; }`. Doc comment explaining it breaks the bidirectional dependency between routing and dispatch crates. See design doc Section 2.1a. | `src/lib.rs` |
| 1.7 | Define `CapacityChecker` consumer trait | `pub trait CapacityChecker: Send + Sync { fn has_capacity(&self, target: &ModelTarget) -> bool; }`. Open (unsealed) — test crates can implement this for mocking. Doc comment: "Used by Request Queue's dispatch iterator (Component 04)." See design doc Section 2.1a. Note: if `llamaforge-types` already defines a sealed `CapacityChecker` from Component 04's PR1, unseal it or define this as a separate open trait. Add a code comment documenting the decision. | `src/lib.rs` |
| 1.8 | Define `ServerSelector` consumer trait | `pub trait ServerSelector: Send + Sync { fn select_server(&self, request: &QueuedRequest) -> Result<RoutingDecision, RoutingError>; }`. Doc comment: "Used by Dispatch & Proxy (Component 06) to select a server after dequeue." | `src/lib.rs` |
| 1.9 | Define `RouteValidator` consumer trait | `pub trait RouteValidator: Send + Sync { fn validate_routable(&self, model: &ModelTarget, capabilities: &RequestCapabilities) -> Result<(), RoutingError>; }`. Doc comment: "Used by Transport Layer (Component 07) at admission time." | `src/lib.rs` |
| 1.10 | Set up `lib.rs` re-exports | Re-export key types from submodules so consumers can import from `llamaforge_routing::*`: `Router`, `RoutingDecision`, `RoutingError`, `DispatchCountView`, `CapacityChecker`, `ServerSelector`, `RouteValidator`. Use `pub use` statements. Leave types as stubs for now (PR2 fills them in). | `src/lib.rs` |
| 1.11 | Verify build | `cargo build --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`. | — |

### PR1 Acceptance Criteria

- [x] `cargo build --workspace` succeeds with zero warnings
- [x] `cargo clippy --workspace -- -D warnings` passes clean
- [x] `Arc<dyn DispatchCountView>` compiles (trait is object-safe)
- [x] `Arc<dyn CapacityChecker>` compiles (trait is object-safe)
- [x] `Arc<dyn ServerSelector>` compiles (trait is object-safe)
- [x] `Arc<dyn RouteValidator>` compiles (trait is object-safe)
- [x] `CapacityChecker` is NOT sealed — external crates can implement it

### PR1 Creation Steps

```bash
git checkout -b routing/01-scaffold main
# ... implement tasks 1.1–1.11 ...
git add -A && git commit -m "routing: bootstrap llamaforge-routing crate with port traits"
git push -u origin routing/01-scaffold
# Open PR: "Smart Routing PR1: Crate Scaffold & Port Traits"
# Description: "Creates llamaforge-routing library crate. Defines DispatchCountView
# (breaks bidirectional dependency with Dispatch & Proxy), and three consumer-facing
# port traits: CapacityChecker (open, for Request Queue), ServerSelector (for Dispatch
# & Proxy), RouteValidator (for Transport Layer). Module stubs for router, affinity,
# types, weights, error, config.
# Design doc Sections 1.1, 2.1a."
```

---

## PR2: Types, Errors & Weights Builder

**Branch:** `routing/02-types`  
**Depends on:** PR1 merged to `main`  
**Size:** Medium (~300 lines)  
**Estimated Time:** 4–6 hours

### Purpose

Define every data type from design doc Sections 2.2, 2.1a: routing decision types (`SlotTarget`, `RoutingDecision`, `RoutingReason`), domain newtypes (`SessionId`, `LoraAdapterId`), the `RoutingWeights` builder with validation, `RequestCapabilities`, `RoutingConstraint` enum, error types, and the config DTO. Getting the type foundation right early prevents cascading refactors.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 2.1 | Implement `SlotTarget` enum | `Auto`, `Pinned(SlotId)`. Derive `Debug, Clone, Copy`. Doc comment: "Eliminates the `-1` sentinel for slot assignment." See design doc Section 2.2. | `src/types.rs` |
| 2.2 | Implement `RoutingReason` enum | `HighestScore { score: f64 }`, `SessionAffinity { session_id: SessionId }`, `RoundRobin { counter: u64 }`. Derive `Debug, Clone`. | `src/types.rs` |
| 2.3 | Implement `RoutingDecision` struct | All fields private or `pub(crate)`: `server_id: ServerId` (private), `reason: RoutingReason` (private), `pub(crate) score: f64`, `pub(crate) slot_target: SlotTarget`. Implement getters: `server_id() -> &ServerId`, `reason() -> &RoutingReason`, `into_server_id(self) -> ServerId`. Implement `pub(crate) fn new(server_id, reason, score, slot_target) -> Self` constructor. Derive `Debug, Clone`. | `src/types.rs` |
| 2.4 | Implement `SessionId` newtype | `pub struct SessionId(Arc<str>)` — private inner field. `new(impl AsRef<str>) -> Result<Self, IdValidationError>` validates non-empty, trimmed, max-length via `llamaforge-types`' `validate_id`. `as_str() -> &str` getter. Derive `Debug, Clone, PartialEq, Eq, Hash`. | `src/types.rs` |
| 2.5 | Implement `LoraAdapterId` newtype | `pub struct LoraAdapterId(Arc<str>)` — private inner field. Same validation as `SessionId`. `#[derive(Serialize)]` with `#[serde(transparent)]`. Manual `Deserialize` impl routing through validated constructor via `String::deserialize` + `LoraAdapterId::new` + `map_err(serde::de::Error::custom)`. Derive `Debug, Clone, PartialEq, Eq, Hash`. | `src/types.rs` |
| 2.6 | Implement `RequestCapabilities` struct | `lora_adapter: Option<LoraAdapterId>`, `multimodal: bool`, `embedding: bool`, `reranking: bool`. Derive `Debug, Clone, Default`. **Create in `llamaforge-types`** (not `llamaforge-routing`) so that both Transport Layer and Smart Routing can reference it without a crate dependency on `llamaforge-routing`. `LoraAdapterId` must also live in or be re-exported from `llamaforge-types` for this to work. See design doc Section 2.2, line 198-199. | `crates/llamaforge-types/src/lib.rs` |
| 2.7 | Implement `RoutingConstraint` enum | `Model`, `LoraAdapter`, `Multimodal`, `Embedding`, `Reranking`, `Architecture`. Derive `Debug, Clone, PartialEq, Eq`. Implement `Display` with snake_case formatting. See design doc Section 2.2. | `src/error.rs` |
| 2.8 | Implement `RoutingError` enum | Three variants: `ModelNotFound { model: String }`, `NoCapableServer { constraint: RoutingConstraint, requested: String }`, `ModelUnavailable { model: String, server_states: Vec<(ServerId, ServerLifecycle)> }`. Derive `Debug` with `#[derive(thiserror::Error)]`. See design doc Section 2.2. | `src/error.rs` |
| 2.9 | Implement `WeightsValidationError` enum | `OutOfRange { field: &'static str, value: f64 }`. Derive `Debug` with `#[derive(thiserror::Error)]`. Error message: `"weight '{field}' out of range 0.0..=1.0: {value}"`. | `src/weights.rs` |
| 2.10 | Implement `RoutingWeights` struct | Private fields: `w_slots: f64` (default 0.4), `w_kv: f64` (default 0.2), `w_queue: f64` (default 0.2), `w_affinity: f64` (default 0.2). Public getters: `w_slots()`, `w_kv()`, `w_queue()`, `w_affinity()`. `builder() -> RoutingWeightsBuilder`. Do NOT derive `Deserialize`. See design doc Section 2.2. | `src/weights.rs` |
| 2.11 | Implement `RoutingWeightsBuilder` | `Default` impl with (0.4, 0.2, 0.2, 0.2). Setter methods: `w_slots(f64)`, `w_kv(f64)`, `w_queue(f64)`, `w_affinity(f64)` — each consumes and returns `Self`. `build() -> Result<RoutingWeights, WeightsValidationError>`: validate each weight in 0.0..=1.0, warn (not error) if sum != 1.0 via `tracing::warn!`. | `src/weights.rs` |
| 2.12 | Implement `RoutingWeightsDto` | Serde DTO for config file deserialization. `#[derive(Deserialize)]` with `#[serde(default)]` on all fields. Implement `TryFrom<RoutingWeightsDto> for RoutingWeights` routing through the builder. | `src/config.rs` |
| 2.13 | Unit tests: `SessionId` validation | Valid ID succeeds, empty string fails, whitespace-only fails. | `src/types.rs` (tests) |
| 2.14 | Unit tests: `LoraAdapterId` serde roundtrip | Serialize → JSON string → deserialize back → equal. Invalid input during deserialization returns error. | `src/types.rs` (tests) |
| 2.15 | Unit tests: `RoutingWeightsBuilder` rejects out-of-range | `w_slots(1.5).build()` → `Err(WeightsValidationError::OutOfRange { .. })`. | `src/weights.rs` (tests) |
| 2.16 | Unit tests: `RoutingWeightsBuilder` warns non-unit sum | Weights summing to 0.5 → `build()` succeeds. Verify `tracing::warn` emitted (using `tracing-test` subscriber). | `src/weights.rs` (tests) |
| 2.17 | Unit tests: `RoutingWeightsBuilder` default values | Default builder builds successfully with (0.4, 0.2, 0.2, 0.2). | `src/weights.rs` (tests) |
| 2.18 | Unit tests: `RoutingDecision` encapsulation | Verify `server_id()` and `reason()` getters return correct values. Verify `into_server_id()` consumes the decision. | `src/types.rs` (tests) |
| 2.19 | Unit tests: `RoutingConstraint` Display | Each variant displays as its snake_case name. Exhaustive match compiles. | `src/error.rs` (tests) |
| 2.20 | Unit tests: `RoutingWeightsDto` conversion | Valid DTO converts to `RoutingWeights`. DTO with out-of-range value returns error. | `src/config.rs` (tests) |

### PR2 Acceptance Criteria

- [x] All types compile with required derives
- [x] `SessionId::new("")` returns `Err`
- [x] `LoraAdapterId` serde roundtrip preserves value
- [x] `RoutingWeightsBuilder::build()` validates range 0.0..=1.0
- [x] `RoutingDecision` fields `score` and `slot_target` are NOT accessible outside the crate
- [x] All error types implement `std::error::Error` via `thiserror`
- [x] `cargo test --workspace` passes all 8 unit test suites
- [x] No raw `String` used for domain identifiers — all use newtypes

### PR2 Creation Steps

```bash
git checkout -b routing/02-types main
# ... implement tasks 2.1–2.20 ...
git add -A && git commit -m "routing: add routing types, error types, and weights builder"
git push -u origin routing/02-types
# Open PR: "Smart Routing PR2: Types, Errors & Weights Builder"
# Description: "Implements all data types from design doc Sections 2.2, 2.1a.
# SlotTarget enum (eliminates -1 sentinel), RoutingDecision with private fields
# and getters, SessionId/LoraAdapterId validated newtypes with Arc<str>,
# RoutingWeights builder with range validation, RoutingConstraint enum with
# Display, RoutingError with thiserror, RoutingWeightsDto config DTO.
# 8 unit test suites."
```

---

## VC-1: Types Verification Checkpoint

**Gate:** PR2 merged. All types compile, all tests pass.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-1.1 | Newtype privacy | Attempt to access `SessionId.0` from outside the crate → compiler rejects | Does not compile |
| VC-1.2 | RoutingDecision encapsulation | Attempt to access `decision.score` from outside the crate → compiler rejects | Does not compile |
| VC-1.3 | Weights validation | `RoutingWeights::builder().w_slots(-0.1).build()` → `Err(OutOfRange)` | Error variant matches |
| VC-1.4 | LoraAdapterId deserialization | `serde_json::from_str::<LoraAdapterId>("\"\"")` → deserialization error | Error returned |
| VC-1.5 | SlotTarget default | All v1 code paths produce `SlotTarget::Auto` | Code inspection |

---

## PR3: Router Core — Scoring & Constraint Filtering

**Branch:** `routing/03-router-core`  
**Depends on:** PR2 merged to `main`  
**Size:** Medium–Large (~300 lines)  
**Estimated Time:** 6–8 hours

### Purpose

Implement the `Router` struct with its three public methods: `has_capacity()`, `select_server()`, and `validate_routable()`. This includes the hard constraint filter, the scoring formula with dispatch tracker compensation, round-robin tie-breaking, and recurrent model handling. This is the core routing engine.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 3.1 | Implement `Router` struct | `config: Arc<ConfigHandle>`, `server_state: Arc<dyn ServerStateView>`, `dispatch_counts: Arc<dyn DispatchCountView>`, `round_robin: DashMap<ModelAlias, AtomicU64>`. Constructor takes all three port trait deps. **Note:** `affinity_cache: Mutex<SessionAffinityCache>` is added in PR5 (task 5.1) after PR4 builds the cache. PR3's Router operates without affinity — the `check_affinity()` placeholder (task 3.6) returns `None`. | `src/router.rs` |
| 3.2 | Implement hard constraint filter | Private method `fn eligible_servers(&self, model: &ModelTarget, capabilities: &RequestCapabilities) -> Result<Vec<ServerSnapshot>, RoutingError>`. Filters by: (1) model match, (2) `Running` state, (3) LoRA adapter availability, (4) multimodal capability, (5) embedding/reranking capability, (6) architecture match. Returns `RoutingError::ModelNotFound` if no servers serve the model at all. Returns `RoutingError::NoCapableServer` with the specific `RoutingConstraint` that failed. Returns `RoutingError::ModelUnavailable` if servers exist but none are `Running` — populate `server_states: Vec<(ServerId, ServerLifecycle)>` by querying `ServerStateView` for ALL servers for the model (including non-Running ones) and collecting their `(id, lifecycle)` pairs. This requires `ServerStateView` to expose server lifecycle state even for non-Running servers (e.g., via `all_servers_for_model()` or equivalent). See design doc Section 3.1. | `src/router.rs` |
| 3.3 | Implement scoring formula | Private method `fn score_server(&self, server: &ServerSnapshot, weights: &RoutingWeights) -> f64`. Zero-guard: if `total_slots == 0` → `free_ratio = 0.0`. Dispatch compensation: `effective_free = idle_slots.saturating_sub(self.dispatch_counts.in_flight_count(&server.id))`. `free_ratio = effective_free as f64 / total_slots as f64`. Score: `w_slots × free_ratio + w_kv × (1.0 - kv_usage) + w_queue × (1.0 - deferred_ratio) + w_affinity × 0.0`. End with `debug_assert!(score.is_finite())`. See design doc Section 3.1. | `src/router.rs` |
| 3.4 | Implement recurrent model handling | Inside `score_server`: if `server.model_architecture == "recurrent"` → zero the KV cache and affinity components. Score reduces to: `w_slots × free_ratio + w_queue × (1.0 - deferred_ratio)`. See design doc Section 3.2. | `src/router.rs` |
| 3.5 | Implement round-robin tie-breaking | Private method `fn break_tie(&self, model: &ModelAlias, candidates: &[ScoredCandidate]) -> usize`. Uses `self.round_robin.entry(model.clone()).or_default()` to handle hot-reloaded models. `AtomicU64::fetch_add(1, Relaxed) % candidates.len()`. Returns index into candidates slice. See design doc Section 3.1. | `src/router.rs` |
| 3.6 | Implement `Router::select_server()` | Call `eligible_servers()`. **Affinity placeholder:** add a private method `fn check_affinity(&self, ...) -> Option<ServerId>` that returns `None` unconditionally (the real `SessionAffinityCache` is built in PR4 and wired in PR5). Score all eligible servers. Find max score. If tie, use `break_tie()`. Build `RoutingDecision` with `SlotTarget::Auto`. Add a `// TODO(PR5): record affinity after scoring` comment at the call site. See design doc Section 3.1. | `src/router.rs` |
| 3.7 | Implement `Router::has_capacity()` | For each server serving the model: compute `effective_free = idle_slots.saturating_sub(dispatch_counts.in_flight_count(id))`. If any `effective_free > 0`, return `true`. Otherwise `false`. O(S) where S = servers for the model. See design doc Section 4.2. | `src/router.rs` |
| 3.8 | Implement `Router::validate_routable()` | Call `eligible_servers()` with the model target and capabilities. If `Ok(_)` (at least one eligible server exists), return `Ok(())`. If `Err(e)`, propagate the `RoutingError`. This is a pure constraint check — no scoring needed. See design doc Section 4.2. | `src/router.rs` |
| 3.9 | Implement `CapacityChecker` for `Router` | `impl CapacityChecker for Router { fn has_capacity(&self, target: &ModelTarget) -> bool { self.has_capacity(target) } }` | `src/router.rs` |
| 3.10 | Implement `ServerSelector` for `Router` | `impl ServerSelector for Router { fn select_server(&self, request: &QueuedRequest) -> Result<RoutingDecision, RoutingError> { self.select_server(request) } }` | `src/router.rs` |
| 3.11 | Implement `RouteValidator` for `Router` | `impl RouteValidator for Router { fn validate_routable(&self, model: &ModelTarget, capabilities: &RequestCapabilities) -> Result<(), RoutingError> { self.validate_routable(model, capabilities) } }` | `src/router.rs` |
| 3.12 | Add routing metrics | `llamaforge_routing_decisions_total` counter (labels: `reason={score,affinity,round_robin}`), `llamaforge_routing_score` histogram (label: `server_id`), `llamaforge_routing_rejections_total` counter (labels: `reason={not_found,no_capable,unavailable}`). Record at appropriate call sites in `select_server()` and `validate_routable()`. See design doc Section 7.1. | `src/router.rs` |
| 3.13 | Add routing log events | `routing.selected` at DEBUG (per-dispatch: server_id, score, reason), `routing.rejected` at WARN (on admission reject: model, constraint, error). See design doc Section 7.2. | `src/router.rs` |
| 3.14 | Unit test: prefers server with more free slots | Server A: 3/4 free, Server B: 1/4 free → A selected. | tests |
| 3.15 | Unit test: dispatch tracker compensation | Mock `DispatchCountView` returning `in_flight_count=2`. Server: 4 slots, 3 idle. Effective free = 1. `free_ratio = 0.25`. | tests |
| 3.16 | Unit test: model not found at admission | Request for "nonexistent-model" → `RoutingError::ModelNotFound`. | tests |
| 3.17 | Unit test: round-robin on tie | Two servers with identical scores. Dispatch 4 requests → 2 to each. | tests |
| 3.18 | Unit test: zero total slots returns zero score | Server with `total_slots = 0`. Assert `free_ratio = 0.0`, score is finite, no NaN. | tests |
| 3.19 | Unit test: recurrent model zeroes KV/affinity | Recurrent server → KV and affinity components both 0.0 in score. | tests |
| 3.20 | Unit test: `has_capacity` with tracker compensation | Server: 4 slots, 4 idle, `in_flight_count=4` → `has_capacity` returns false. Same with `in_flight_count=3` → returns true. | tests |
| 3.21 | Unit test: `validate_routable` LoRA constraint | Request needs LoRA "my-adapter", no server has it → `NoCapableServer { constraint: LoraAdapter, .. }`. | tests |

### PR3 Acceptance Criteria

- [x] `select_server()` returns `RoutingDecision` with `SlotTarget::Auto` for all v1 decisions
- [x] Scoring formula matches design doc Section 3.1 exactly
- [x] `saturating_sub` prevents underflow on dispatch compensation
- [x] Zero-guard on `total_slots == 0` prevents division by zero / NaN
- [x] `debug_assert!(score.is_finite())` present after score computation
- [x] Round-robin uses `AtomicU64` with `.entry().or_default()` for hot-reloaded models
- [x] Recurrent model detection zeros KV cache and affinity components
- [x] All 3 port trait impls compile and delegate correctly
- [x] All 8 unit test suites pass
- [x] No `unwrap()` in production code

### PR3 Creation Steps

```bash
git checkout -b routing/03-router-core main
# ... implement tasks 3.1–3.21 ...
git add -A && git commit -m "routing: implement Router with scoring formula and constraint filtering"
git push -u origin routing/03-router-core
# Open PR: "Smart Routing PR3: Router Core — Scoring & Constraint Filtering"
# Description: "Implements the Router struct (design doc Sections 2.1, 3.1, 3.2, 3.3, 4.2).
# Hard constraint filter (model, LoRA, multimodal, architecture), scoring formula with
# dispatch tracker compensation (saturating_sub, zero-guard), recurrent model handling,
# round-robin tie-breaking with AtomicU64, all three consumer port trait impls
# (CapacityChecker, ServerSelector, RouteValidator), 5 Prometheus metrics, 2 log events.
# 8 unit test suites."
```

---

## PR4: Session Affinity Cache

**Branch:** `routing/04-affinity`  
**Depends on:** PR2 merged to `main` (can be developed concurrently with PR3)  
**Size:** Small–Medium (~150 lines)  
**Estimated Time:** 3–4 hours

### Purpose

Implement the `SessionAffinityCache` — an LRU cache mapping `SessionId → ServerId`. Protected by `std::sync::Mutex` with poison recovery via `into_inner()`. Session affinity is advisory: it yields to scoring when the target server is full.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 4.1 | Implement `SessionAffinityCache` struct | `cache: lru::LruCache<SessionId, ServerId>`. Internal struct (not public). Constructor takes `max_entries: usize` (default 10,000). See design doc Section 2.3. | `src/affinity.rs` |
| 4.2 | Implement `SessionAffinityCache::get()` | `fn get(&mut self, session_id: &SessionId, available_servers: &[ServerId]) -> Option<ServerId>`. Look up in LRU. If found AND server is in `available_servers`, return `Some`. Otherwise return `None` (stale entry, server went away). LRU touch on hit. | `src/affinity.rs` |
| 4.3 | Implement `SessionAffinityCache::set()` | `fn set(&mut self, session_id: SessionId, server_id: ServerId)`. Insert/update in LRU. Evicts oldest entry if at capacity. | `src/affinity.rs` |
| 4.4 | Implement `SessionAffinityCache::invalidate_server()` | `fn invalidate_server(&mut self, server_id: &ServerId)`. Iterate all entries, collect keys where value == server_id, remove them. Called when server enters `Draining` or `Failed` state. | `src/affinity.rs` |
| 4.5 | Export `SessionAffinityCache` as `pub(crate)` | Mark the struct `pub(crate)` (not `pub`) so that `router.rs` in the same crate can construct and hold it. Export the constructor `pub(crate) fn new(max_entries: usize) -> Self`. | `src/affinity.rs` |
| 4.8 | Unit test: affinity hit | Set affinity (session1 → serverA). Query with serverA available → returns `Some(serverA)`. | tests |
| 4.9 | Unit test: affinity miss (server unavailable) | Set affinity (session1 → serverA). Query with only serverB available → returns `None`. | tests |
| 4.10 | Unit test: invalidate_server removes entries | Set affinity for 3 sessions to serverA. `invalidate_server(serverA)`. All 3 return `None`. | tests |
| 4.11 | Unit test: LRU eviction | Cache with max_entries=3. Insert 4 entries. Assert oldest evicted. | tests |

### PR4 Acceptance Criteria

- [x] `SessionAffinityCache` is `pub(crate)` — visible within `llamaforge-routing` but not to external crates
- [x] LRU cache correctly evicts oldest entries at capacity
- [x] `invalidate_server` removes all entries for the given server
- [x] PR4 does NOT modify `src/router.rs` — all Router integration deferred to PR5
- [x] All 4 unit test suites pass

### PR4 Creation Steps

```bash
git checkout -b routing/04-affinity main
# ... implement tasks 4.1–4.11 ...
git add -A && git commit -m "routing: implement SessionAffinityCache with LRU"
git push -u origin routing/04-affinity
# Open PR: "Smart Routing PR4: Session Affinity Cache"
# Description: "Implements SessionAffinityCache in isolation (design doc Section 2.3).
# LRU cache (lru crate, max 10,000 entries) mapping SessionId → ServerId.
# Server invalidation for Draining/Failed states. pub(crate) visibility for
# Router integration in PR5. Does NOT modify router.rs — all Router wiring
# deferred to PR5 to avoid merge conflicts with concurrent PR3.
# 4 unit test suites."
```

---

## VC-2: Core Components Verification Checkpoint

**Gate:** PR3 + PR4 merged. Router core and affinity cache work together.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-2.1 | Scoring correctness | Server A (3/4 free, 0.1 KV, 0.0 deferred), Server B (1/4 free, 0.5 KV, 0.3 deferred) → verify A scores higher | Score comparison passes |
| VC-2.2 | Dispatch compensation | 4 slots, 3 idle, `in_flight_count=3` → `effective_free=0`, `has_capacity=false` | Assertion passes |
| VC-2.3 | Session affinity lifecycle | Set affinity → hit → invalidate server → miss → fall through to scoring | All assertions pass |
| VC-2.4 | Constraint filtering | Request with LoRA + multimodal, only 1 of 3 servers has both → that server selected | Server ID matches |
| VC-2.5 | Round-robin fairness | Two identical servers, 100 dispatches → each gets 50 ± 1 | Count assertion passes |
| VC-2.6 | Mutex poison recovery | Poison cache, then dispatch 10 requests → all succeed with score-based routing | No panics, 10 decisions |

---

## PR5: Integration Wiring & Config Weights Log Event

**Branch:** `routing/05-integration`  
**Depends on:** PR3 and PR4 both merged to `main`  
**Size:** Medium (~250 lines)  
**Estimated Time:** 5–7 hours

### Purpose

Wire the session affinity cache into the Router (moved from PR4 to avoid merge conflicts with concurrent PR3), implement the `startup_wait_secs` grace period for Starting servers, add the `routing.weights_non_unit` log event for config loading, and implement the full integration test suite from the design doc. This PR ensures all components work together as a cohesive routing engine.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 5.1 | Wire `SessionAffinityCache` into Router | Add `affinity_cache: std::sync::Mutex<SessionAffinityCache>` field to `Router` struct. Update `Router::new()` to construct `SessionAffinityCache` with capacity from `config.routing.session_affinity_max_entries` (default 10,000). Replace the placeholder `check_affinity()` (from PR3 task 3.6) with the real implementation. | `src/router.rs` |
| 5.2 | Implement poison recovery wrapper | Private method `fn get_affinity(&self, session_id: &SessionId, available: &[ServerId]) -> Option<ServerId>`. Wraps `self.affinity_cache.lock()` with `match Ok/Err(poisoned)` pattern. On poison: `tracing::warn!("session affinity cache poisoned, degrading to score-only routing")`, then `poisoned.into_inner().get(...)`. See design doc Cross-Cutting Concerns (Panic Safety). | `src/router.rs` |
| 5.3 | Implement affinity integration in `select_server` | Replace the PR3 `// TODO(PR5)` stub. After `eligible_servers()` and before scoring: if `request.session_id` is `Some`, call `get_affinity()`. If hit AND target server has `free_ratio > 0`, return immediately with `RoutingReason::SessionAffinity`. If hit but server is full, fall through to scoring. After scoring, call `set_affinity()` to record the decision. See design doc Section 3.1. | `src/router.rs` |
| 5.4 | Add affinity metrics | `llamaforge_session_affinity_hits_total` counter, `llamaforge_session_affinity_misses_total` counter. Record in `select_server()` after affinity check. See design doc Section 7.1. | `src/router.rs` |
| 5.5 | Implement `startup_wait_secs` grace period | In `validate_routable()`: if all servers for the model are in `Starting` state AND the Router has been alive for less than `config.routing.startup_wait_secs` (default 30), return `Ok(())` instead of `RoutingError::ModelUnavailable`. This allows the request to enqueue during the startup window. Store `started_at: Instant` in the Router struct (set in `Router::new()`). See design doc Section 4.3, Section 6 config table. | `src/router.rs` |
| 5.6 | Add `routing.weights_non_unit` log event | On config load/reload: if weights sum != 1.0, emit `tracing::warn!(sum, "routing weights sum to {sum:.3}, not 1.0 — consider normalizing")`. This may already exist in the builder's `build()` method (task 2.11) — verify it fires on config reload path too. The hot-reload path MUST reconstruct weights through the builder (via `TryFrom<RoutingWeightsDto>`) to ensure the warning fires. See design doc Section 7.2. | `src/router.rs` or `src/config.rs` |
| 5.7 | Integration test: full routing lifecycle | Create Router with mock `ServerStateView` (2 servers), mock `DispatchCountView`. Submit 10 requests → verify all get valid `RoutingDecision`s. Verify decisions spread across servers. | tests |
| 5.8 | Integration test: affinity + scoring fallback | Set affinity via first dispatch. Second dispatch with same session → affinity server. Make affinity server full → third dispatch falls to scoring. | tests |
| 5.9 | Integration test: validate_routable → select_server pipeline | Call `validate_routable()` → Ok. Then `select_server()` → valid decision. Then `validate_routable()` with bad capabilities → `NoCapableServer`. | tests |
| 5.10 | Integration test: config hot-reload weights | Change routing weights via config. Verify next `select_server()` uses new weights (observable via different server selection when weights heavily favor KV cache vs slots). Verify hot-reload path goes through `TryFrom<RoutingWeightsDto>` (and thus through the builder, which fires the non-unit-sum warning). | tests |
| 5.11 | Integration test: all servers down | All servers in `Failed` state → `select_server()` returns `ModelUnavailable`. `has_capacity()` returns false. `validate_routable()` returns `ModelUnavailable`. | tests |
| 5.12 | Integration test: `SlotTarget::Auto` in all v1 decisions | Route 20 requests → verify every `RoutingDecision` has `slot_target == SlotTarget::Auto`. | tests |
| 5.13 | Integration test: mock `DispatchCountView` configurable | Create a reusable `MockDispatchCountView` that returns configurable per-server counts. Verify it works with the Router. This mock will be useful for downstream components' tests. | tests |
| 5.14 | Integration test: affinity bypassed when full | Affinity → serverA, but serverA has `free_ratio = 0`. Assert scoring selects serverB. | tests |
| 5.15 | Integration test: mutex poison recovery | Poison the affinity cache mutex (spawn a thread that panics while holding the lock). Call `select_server()` → returns a valid `RoutingDecision` (score-based, no panic). | tests |
| 5.16 | Integration test: startup grace period | All servers `Starting`, Router age < `startup_wait_secs` → `validate_routable()` returns `Ok(())`. After grace period elapses → returns `ModelUnavailable`. | tests |
| 5.17 | Expose `Router::invalidate_affinity()` | `pub fn invalidate_affinity(&self, server_id: &ServerId)`. Thin wrapper: locks the affinity cache mutex (with poison recovery), calls `invalidate_server(server_id)`. Intended to be called by the composition root or Dispatch & Proxy when a server enters `Draining`/`Failed`. The caller is NOT wired in this component — documented as integration seam in Integration Points section. | `src/router.rs` |

### PR5 Acceptance Criteria

- [x] `SessionAffinityCache` wired into Router with `Mutex<SessionAffinityCache>` field
- [x] Mutex poison recovered via `into_inner()` with `tracing::warn!`
- [x] Affinity is advisory — bypassed when target server is full
- [x] `startup_wait_secs` grace period allows enqueue during startup window
- [x] `routing.weights_non_unit` log event fires on config reload with non-unit sum
- [x] Config hot-reload goes through the builder (via `TryFrom<RoutingWeightsDto>`)
- [x] Router constructor reads `session_affinity_max_entries` from config
- [x] All 10 integration test suites pass
- [x] Mock `DispatchCountView` is reusable for downstream component tests
- [x] Full lifecycle (validate → select → affinity → fallback) works end-to-end
- [x] No `unwrap()` on mutex lock — all use `unwrap_or_else(|p| p.into_inner())`

### PR5 Creation Steps

```bash
git checkout -b routing/05-integration main
# ... implement tasks 5.1–5.16 ...
git add -A && git commit -m "routing: wire affinity into Router, add startup grace period, integration tests"
git push -u origin routing/05-integration
# Open PR: "Smart Routing PR5: Integration Wiring & Tests"
# Description: "Wires SessionAffinityCache into Router (design doc Section 2.3),
# implements Mutex poison recovery via into_inner(), startup_wait_secs grace
# period for Starting servers (design doc Section 4.3, 6), config reload weights
# log event (design doc Section 7.2), and 10 integration test suites covering:
# full routing lifecycle, affinity + scoring fallback, validate → select pipeline,
# config hot-reload, all-servers-down, SlotTarget::Auto enforcement, affinity
# bypass when full, mutex poison recovery, startup grace period, and reusable
# MockDispatchCountView. Design doc Sections 2.3, 3.1, 4.3, 6, 7.2, 8.2."
```

---

## PR6: Design Doc Test Cases & Final Verification

**Branch:** `routing/06-verification`  
**Depends on:** PR5 merged to `main`  
**Size:** Medium (~250 lines of tests)  
**Estimated Time:** 4–6 hours

### Purpose

The final PR adds all remaining specific test cases from design doc Section 8.2 that weren't covered in PR2–PR5, implements the VC-2 verification checks as automated tests, and performs the final sign-off audit. This PR is pure test code — no production logic changes.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 6.1 | Implement `test_routing_with_mock_dispatch_count_view` | Inject stub `DispatchCountView` returning configurable counts. Assert `free_ratio` correctly compensated. See design doc Section 8.2. | `tests/design_doc_tests.rs` |
| 6.2 | Implement `test_routing_constraint_exhaustive_match` | Match on `RoutingConstraint` in `NoCapableServer` error. Ensures compile-time exhaustiveness. See design doc Section 8.2. | `tests/design_doc_tests.rs` |
| 6.3 | Implement `test_routing_decision_field_encapsulation` | From an external test crate, verify `server_id()` and `reason()` getters work. Attempt to access `decision.score` → compile error (manual verification, add code comment). See design doc Section 8.2. | `tests/design_doc_tests.rs` |
| 6.4 | Implement `test_slot_target_auto_in_v1_decisions` | Route multiple requests. Assert all have `SlotTarget::Auto`. | `tests/design_doc_tests.rs` |
| 6.5 | Implement `test_weights_builder_rejects_negative` | `w_kv(-0.5).build()` → error. | `tests/design_doc_tests.rs` |
| 6.6 | Implement `test_stale_slot_compensation_floor` | `in_flight_count` > `idle_slots` → `effective_free = 0` (not negative). `free_ratio = 0.0`. | `tests/design_doc_tests.rs` |
| 6.7 | Implement VC-2 checks as automated tests | All 6 VC-2 checks from the verification checkpoint above, each as a separate `#[test]` function. | `tests/vc2_routing.rs` |
| 6.8 | Add `#[ignore]` for slow tests | Tag any stress tests (e.g., round-robin 100 dispatches) with `#[ignore]`. Run with `cargo test -- --ignored`. | — |
| 6.9 | Final CI check | `cargo test --workspace`, `cargo test --workspace -- --ignored`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`. | — |

### PR6 Acceptance Criteria

- [ ] All design doc Section 8.2 test cases implemented and passing
- [ ] All 6 VC-2 verification checks pass as automated tests
- [ ] `cargo test --workspace` passes (excluding `#[ignore]` tests)
- [ ] `cargo test --workspace -- --ignored` passes (including slow tests)
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] No production code changes in this PR — test code only

### PR6 Creation Steps

```bash
git checkout -b routing/06-verification main
# ... implement tasks 6.1–6.9 ...
git add -A && git commit -m "routing: add design doc test cases and VC-2 verification"
git push -u origin routing/06-verification
# Open PR: "Smart Routing PR6: Design Doc Tests & Final Verification"
# Description: "Pure test PR. Adds remaining design doc Section 8.2 test cases,
# 6 VC-2 verification checks as automated tests, and final CI validation.
# Total: ~15 new test functions. No production code changes."
```

---

## VC-3: Final Sign-Off Checkpoint

**Gate:** All 6 PRs merged. Smart Routing is complete.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-3.1 | All tests pass | `cargo test --workspace && cargo test --workspace -- --ignored` | Zero failures |
| VC-3.2 | Zero clippy warnings | `cargo clippy --workspace -- -D warnings` | Exit code 0 |
| VC-3.3 | Code coverage | Run `cargo llvm-cov --workspace`. Target > 85% line coverage for `llamaforge-routing`. Note: error paths in `eligible_servers()` (especially `ModelUnavailable` with Starting grace period) and the Mutex poison recovery path require deliberate test construction — PR5 tasks 5.15/5.16 and PR3 tasks 3.16/3.21 cover these. | Coverage report generated |
| VC-3.4 | Public API audit | Review `lib.rs` re-exports. Verify only intended types are public. `SessionAffinityCache` is NOT public. `ScoredCandidate` (if any) is NOT public. | Manual review |
| VC-3.5 | Design doc compliance | Cross-check every Section in `05_SMART_ROUTING.md` against the implementation. Checklist of all specified behaviors. | Manual review with checklist |
| VC-3.6 | Documentation | Every public type, trait, method, and enum variant has a doc comment. `cargo doc --no-deps -p llamaforge-routing` builds without warnings. | `cargo doc` succeeds |
| VC-3.7 | Dependency audit | `cargo deny check` passes. No duplicate dependency versions. | `cargo deny` green |
| VC-3.8 | No `unwrap()` audit | `grep -rn "unwrap()" crates/llamaforge-routing/src/ --include="*.rs"` — only `unwrap_or_else` for poison recovery. Zero bare `unwrap()`. | Zero matches |
| VC-3.9 | Memory bounds check | Session affinity LRU at max capacity (10,000 entries × ~150 bytes) < 2 MB. Scoring is stack-only. | Memory assertion or calculation |
| VC-3.10 | Port trait audit | Router depends on `dyn ServerStateView` + `dyn DispatchCountView` only. Consumers depend on `dyn CapacityChecker` / `dyn ServerSelector` / `dyn RouteValidator` only. No concrete cross-crate type imports. | `grep` for concrete types |

---

## Summary: PR → File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `crates/llamaforge-routing/Cargo.toml`, `src/lib.rs`, module stubs | ~150 |
| PR2 | `src/types.rs`, `src/error.rs`, `src/weights.rs`, `src/config.rs`, `crates/llamaforge-types/src/lib.rs` (`RequestCapabilities`) | ~300 |
| PR3 | `src/router.rs` | ~300 |
| PR4 | `src/affinity.rs` | ~100 |
| PR5 | `src/router.rs` (affinity wiring, poison recovery, startup grace, `invalidate_affinity`), integration tests | ~250 |
| PR6 | `tests/design_doc_tests.rs`, `tests/vc2_routing.rs` | ~250 |
| **Total** | | **~1,350** |

> Note: LOC estimate is lower than design doc's ~1,500 because the design doc includes padding for doc comments and module boilerplate. Actual production code is ~900 lines, tests ~450 lines.

---

## Timeline (Single Engineer)

| Day | Work | PRs |
|-----|------|-----|
| Day 1 | Crate scaffold + port traits + all types/errors/weights | PR1, PR2 |
| Day 2 | Router core — scoring formula, constraint filtering | PR3 (part 1) |
| Day 3 | Router core — round-robin, recurrent, trait impls, metrics | PR3 (part 2) |
| Day 4 | Session affinity cache + mutex poison recovery | PR4 |
| Day 5 | Integration wiring + integration tests | PR5 |
| Day 6 | Design doc test cases + VC-2 + final verification | PR6 |
| Day 7 | VC-3 sign-off, code review fixes, docs | — |

## Timeline (Two Engineers)

| Day | Engineer A | Engineer B |
|-----|-----------|-----------|
| Day 1 | PR1 (scaffold) + PR2 (types) | Reviews PR1/PR2, writes mock `ServerStateView` + `DispatchCountView` test doubles |
| Day 2 | PR3 (router core) | PR4 (session affinity) — parallel after PR2 |
| Day 3 | PR5 (integration wiring + tests) | PR6 (design doc tests + VC-2) |
| Day 4 | VC-3 sign-off, code review fixes | Code review fixes, docs |

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `ServerStateView` trait not yet defined in `llamaforge-types` | Medium | Medium | Define a minimal stub in `llamaforge-types` or use a local trait alias. Coordinate with Server Manager (Component 03) author. |
| Sealed `CapacityChecker` in `llamaforge-types` conflicts with open trait needed here | Medium | Low | Design doc Section 2.1a explicitly calls this out. Unseal in `llamaforge-types`, or define a separate open `CapacityChecker` in `llamaforge-routing` and deprecate the sealed one. |
| `lru` crate API changes between versions | Low | Low | Pin `lru = "0.12"`. API is stable and minimal (get, put, iter). |
| `DashMap` contention on `round_robin` counters | Low | Low | Contention is bounded by model count (typically 1-4). Each entry is `AtomicU64` — no lock held during scoring. |
| Stale `/slots` data causes brief over-dispatch | Medium | Low | By design: dispatch tracker compensation reduces the window. Worst case is 1 extra request per /slots poll interval (5s). Self-correcting. |
| Session affinity cache memory at max capacity | Low | Low | 10,000 entries × ~150 bytes = 1.5 MB. Well within bounds. LRU eviction prevents growth. |
| Mutex poison on affinity cache | Low | Low | Recovered via `into_inner()`. Scoring continues without affinity. Logged as warning. |
| `QueuedRequest` type not yet available from `llamaforge-queue` | Medium | Medium | Use a local stub type during development. Replace with real import when Component 04 is merged. Or define in `llamaforge-types`. |
| `invalidate_server()` call site not wired | Medium | Low | `SessionAffinityCache::invalidate_server()` is implemented (PR4 task 4.4) but no task wires it to server lifecycle changes. The call site depends on Server Manager's state change notification mechanism (Component 03), which may not exist yet. The composition root or Dispatch & Proxy (Component 06) should subscribe to server state changes and call `Router::invalidate_affinity(server_id)` (a thin wrapper around `affinity_cache.lock().invalidate_server()`). Add this public method to Router in PR5 but defer the caller wiring to the composition root PR. |
| `RequestCapabilities` + `LoraAdapterId` in `llamaforge-types` may pull serde into types crate | Low | Low | `LoraAdapterId` has `Serialize` + manual `Deserialize`. If `llamaforge-types` doesn't already depend on `serde`, this adds a new dependency. Verify workspace-level `serde` is already present. |

---

## Integration Points (for next component)

After Smart Routing is complete, the following integration seams are ready:

1. **Request Queue (Component 04)** holds `Arc<dyn CapacityChecker>` and calls `has_capacity()` in the model-aware dispatch scan. The composition root coerces `Arc<Router>` to `Arc<dyn CapacityChecker>`.
2. **Dispatch & Proxy (Component 06)** holds `Arc<dyn ServerSelector>` and calls `select_server()` after dequeue. It also implements `DispatchCountView` on its `DispatchTracker` — the composition root injects `Arc<DispatchTracker>` as `Arc<dyn DispatchCountView>` into the Router. When a server enters `Draining` or `Failed`, the composition root (or Dispatch & Proxy) calls `Router::invalidate_affinity(server_id)` to purge stale session affinity entries.
3. **Transport Layer (Component 07)** holds `Arc<dyn RouteValidator>` and calls `validate_routable()` at admission time.
4. **`main.rs` (composition root)** constructs: `Arc<Router>::new(config, server_state, dispatch_counts)` → coerce to `Arc<dyn CapacityChecker>` for Request Queue, `Arc<dyn ServerSelector>` for Dispatch & Proxy, `Arc<dyn RouteValidator>` for Transport Layer.
5. **Config Manager** provides `Arc<ConfigHandle>` for `RoutingWeights` reads and hot-reload.
6. **Server Manager** provides `Arc<dyn ServerStateView>` for slot state snapshots.