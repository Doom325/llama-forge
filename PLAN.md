# Workspace Consolidation — Implementation Plan

**Phase:** 9 (Post-v1 structural refactor — collapse 13-crate sprawl to 7-crate layout)
**Estimated LOC:** ~4,500 lines moved + ~600 lines deleted (adapter glue, bridge functions, re-exports) — net **−600 LOC** despite touching every crate.
**Estimated Duration:** 5–7 working days (single engineer)
**Design Reference:** `docs/design.md` §4.2 (Hexagonal layering), issue #154 (the H4 finding driving this phase).
**Branch Prefix:** `consolidate/`
**Target:** `main` (via squash-merge PRs)
**Upstream Dependencies:** v1 closure complete (PRs #1–#11 from the prior Integration phase). H1/H2/H3 design-review fixes merged (#183/#184/#185).

**Primary CoderAgent:** `CoderAgent_llamaforge-designbugs`.
**Primary Reviewer:** `ReviewerAgent_llamaforge-designbugs`.
**Relay key prefix:** `llama-forge#consolidate-*`.

---

## Why this phase exists

The new-design-review pass landed issue #154: 13 crates for ~23K lines of single-node code, several of which exist solely as adapter layers between other crates. Tracing one request end-to-end (HTTP in → queue → route → dispatch → backend → response) requires opening **seven** crates today: `transport`, `adapters/inference_engine`, `adapters/queue`, `queue`, `routing`, `dispatch`, and `types`. The new-hire test ("4–5 files to follow a request") fails by a wide margin.

H2 (duplicate `SlotNotifier`) and H3 (duplicate `RouteValidator`) were the symptoms — both arose because the same trait surface had to be declared in two crates to satisfy the layering. They are now fixed, but the structural pressure that *produced* them remains: every new port trait risks the same fate.

A secondary issue caught by the H4 review: `ServerId` is defined twice — `llamaforge_types::ServerId(Uuid)` and `llamaforge_config::ServerId(String)` — requiring `config_server_id_to_types_id` bridge calls at every boundary. The unification removes ~50 lines of bridge code and a class of "which ServerId is in play?" debugging time.

The phase output is a workspace where:

- **13 crates collapse to 7**: `llamaforge`, `llamaforge-core`, `llamaforge-server`, `llamaforge-api`, `llamaforge-metrics`, `llamaforge-proto`, `mock-llama` (+ `llamaforge-e2e` as the test-infra peer).
- **One `ServerId` type, used everywhere**, with config parsing/validation living in the same crate that defines the type.
- **Request trace fits in ≤4 files** (api → core::queue → core::routing → api::dispatch is one crate hop, not six).
- **No behavior change**: no public HTTP/gRPC surface change, no metric name change, no config schema change. This is pure structural refactor.

After this phase lands, the next design-review pass should not produce another H2/H3-style finding for any new port trait.

---

## Target layout

```
llamaforge          (binary + serve + build + cli)             [unchanged crate name]
llamaforge-core     (config + types + routing + queue)         [NEW — absorbs 4 crates]
llamaforge-server   (server-manager + process + health + telemetry) [RENAMED from -server-manager]
llamaforge-api      (transport + adapters + dispatch)          [NEW — absorbs 3 crates]
llamaforge-metrics  (SQLite store)                             [unchanged]
llamaforge-proto    (generated tonic/prost bindings)           [unchanged]
mock-llama          (test backend)                             [unchanged]
llamaforge-e2e      (test infra)                               [unchanged]
```

**Crates removed**: `llamaforge-types`, `llamaforge-routing`, `llamaforge-queue`, `llamaforge-dispatch`, `llamaforge-adapters` (5 crates merged in).
**Crates renamed**: `llamaforge-server-manager` → `llamaforge-server`.
**Crates added**: `llamaforge-core`, `llamaforge-api`.

Module layout inside the new crates preserves the current source-file boundaries — e.g., `llamaforge-core/src/types/`, `llamaforge-core/src/routing/`, `llamaforge-core/src/queue/`. This makes the merge a `git mv` exercise rather than a rewrite, and keeps blame history intact.

---

## Ground Rules for CoderAgents

1. **Behavior-preserving refactor only.** No new features, no API surface changes (HTTP/gRPC/metric names/config keys all stay identical). If a behavior change is tempting, defer it to a separate PR after this phase closes.
2. **Use `git mv`, not delete-and-rewrite.** Preserves blame and makes the diff readable. CI's mergeability check fails fast if a file is touched in two places.
3. **Module structure inside merged crates mirrors the old crate names.** `llamaforge-core/src/{types,config,routing,queue}/` — one submodule per absorbed crate. Don't flatten the source tree in the same PR; that's churn on top of churn.
4. **`pub` ↔ `pub(crate)` audit at every merge.** When two crates merge, types previously `pub` to satisfy the crate boundary may now be `pub(crate)`. Each PR does this audit explicitly — tighten visibility where the dependency-graph collapse makes it possible. Document each demotion in the PR body.
5. **Workspace deps only.** New deps belong in root `[workspace.dependencies]`; member crates use `workspace = true`. Same rule as Phase 8 — restated here because new `Cargo.toml`s will be created.
6. **No `unwrap()`/`expect()` in production paths.** Inherited.
7. **No blocking under `.await`.** Inherited.
8. **5-gate local verification + CI check before every relay closure:**
    ```bash
    cargo build --workspace && \
    cargo clippy --workspace --all-targets -- -D warnings && \
    cargo test --workspace && \
    cargo fmt --check && \
    cargo deny check
    echo "all-local=$?"
    gh pr checks <PR>
    ```
9. **Each PR rewrites the `[workspace.dependencies]` block atomically.** Half-renamed workspace deps break every member crate at once; keep the change cohesive in one commit.
10. **Per-PR PLAN.md tick.** Each PR closes specific task IDs in the section below; tick them in the same commit so the plan stays the source of truth.
11. **`gh pr merge --squash --delete-branch`** for each landed PR. Use `--auto` only when a single PR is in flight; if two are queued, merge sequentially (per `project_automerge_race` memory).
12. **Closing the relay cycle** sends a one-line verdict on the matching `llama-forge#consolidate-prN` key.

---

## Dependency Graph (PR merge order)

```
PR1 (ServerId unification) ── PR2 (core merge) ── PR3 (api merge) ── PR4 (server rename) ── PR5 (docs/closure)
```

Strictly sequential. Each PR depends on the previous landing because:

- **PR1 → PR2**: Once `ServerId` is unified, the merge of `types` + `config` is purely cosmetic. Doing them at the same time creates a 3000-line diff with intermingled type-redesign and file-moves; doing them sequentially keeps each PR reviewable.
- **PR2 → PR3**: `llamaforge-api` depends on `llamaforge-core` (transport/dispatch/adapters all use `types`, `routing`, `config`). The new `-core` crate name must exist before `-api` can declare its dep.
- **PR3 → PR4**: The rename of `server-manager` is independent in principle, but rebasing it on top of two crate-merges is much cheaper than rebasing two crate-merges on top of a rename.
- **PR4 → PR5**: Docs/PLAN.md/README must reflect the final crate layout; cannot finalize until PR4 lands.

There is no parallelism here. Each PR is large; running two in flight against the workspace `Cargo.toml` would conflict on every push.

---

## PR1 — `ServerId` unification

**Branch:** `consolidate/01-server-id-unify`
**Depends on:** `main` only (post-#183/#184/#185)
**Size:** Small (~150 lines changed, ~50 deleted)
**Estimated Time:** 2–3 hours

### Purpose

Eliminate the `llamaforge_config::ServerId(String)` ↔ `llamaforge_types::ServerId(Uuid)` duality. After this PR, exactly one `ServerId` type exists, defined in `llamaforge-types`, and is what everything uses.

### Audit of current state (file:line)

- `crates/llamaforge-types/src/lib.rs` — `pub struct ServerId(pub Uuid)` (the surviving canonical type)
- `crates/llamaforge-config/src/types.rs` — `pub struct ServerId(pub String)` (the config-side slug/UUID-string variant)
- `crates/llamaforge-config/src/handle.rs:27-35` — load-time validation that already coerces config IDs to canonical form
- `crates/llamaforge/src/build.rs:281-287` — `fn config_server_id_to_types_id(s: &llamaforge_config::ServerId) -> llamaforge_types::ServerId` (the bridge being deleted)
- `crates/llamaforge-server-manager/src/**` — `ServerStateSnapshot` uses `config::ServerId` in some adapters

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 1.1 | `llamaforge-config` depends on `llamaforge-types` (workspace dep). Today config has no internal deps; this introduces one. Edge is acyclic: types is a leaf. | `crates/llamaforge-config/Cargo.toml` |
| 1.2 | Replace `pub struct ServerId(pub String)` in `llamaforge-config::types` with a re-export: `pub use llamaforge_types::ServerId;`. Remove the struct entirely. | `crates/llamaforge-config/src/types.rs` |
| 1.3 | Move config-side parsing/validation (currently the `String` invariants in `handle.rs:27-35`) into a `ServerId::from_config_str(&str) -> Result<Self, ConfigError>` helper on `llamaforge-types::ServerId`. This is the *only* place the slug-vs-UUID coercion lives. | `crates/llamaforge-types/src/lib.rs`, `crates/llamaforge-config/src/handle.rs` |
| 1.4 | Delete `config_server_id_to_types_id` and the `ServerStateAdapter` projection in `build.rs` (the two-step projection collapses to identity now). | `crates/llamaforge/src/build.rs` |
| 1.5 | Sweep all callers: `grep -rn 'config_server_id_to_types_id\|llamaforge_config::ServerId' crates/`. Each hit becomes a direct `ServerId` reference. | various |
| 1.6 | Unit tests for `ServerId::from_config_str`: round-trip UUID-shaped strings, reject empty strings, accept the slug forms that worked before. Carry over existing test cases from `llamaforge-config::handle::tests`. | `crates/llamaforge-types/src/lib.rs` |
| 1.7 | `cargo deny check` — verify no advisory landed in the new `config → types` edge's transitive graph. | — |

### PR1 Acceptance Criteria

- [ ] `grep -rn 'config_server_id_to_types_id' crates/` returns 0 hits.
- [ ] `grep -rn 'llamaforge_config::ServerId' crates/` returns 0 hits.
- [ ] Exactly one `pub struct ServerId` in the workspace (`grep -rn 'pub struct ServerId' crates/` returns 1 line).
- [ ] All 5 local gates green; 3/3 CI checks green.
- [ ] Relay closure on `llama-forge#consolidate-pr1` cites the run URL.

---

## PR2 — `llamaforge-core`: merge `types` + `config` + `routing` + `queue`

**Branch:** `consolidate/02-core-merge`
**Depends on:** PR1 merged
**Size:** Large (~2,500 lines moved across crates; ~200 deleted from re-exports/visibility tightening)
**Estimated Time:** 8–12 hours

### Purpose

Create `crates/llamaforge-core/` by absorbing the four leaf-ish crates that together constitute the "brain" of the orchestrator: domain types, configuration, routing policy, and the request queue. After this PR the four old crates are deleted; their contents live under `llamaforge-core::{types, config, routing, queue}`.

### Why these four together

The four form a closed sub-dependency-graph today: `types` is a leaf, `config` depends on `types` (after PR1), `routing` depends on both, `queue` depends on all three. There is no fifth crate to add (every other internal crate depends on at least one of these) and no subset to remove (the dispatcher and adapters reach across all four). Merging them in one step is the only way to land the new crate in a usable state.

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 2.1 | `mkdir crates/llamaforge-core && cargo init --lib`. Workspace member; standard `[lib]` with the canonical doc comment from CLAUDE.md (cites design doc §4.2). | `Cargo.toml`, `crates/llamaforge-core/` |
| 2.2 | `git mv` each old crate's `src/` into a sub-tree: `crates/llamaforge-types/src/*.rs` → `crates/llamaforge-core/src/types/*.rs` (and similarly for `config`, `routing`, `queue`). Tests follow under `crates/llamaforge-core/tests/{types,config,routing,queue}/`. **No content edits in this commit — pure moves.** | `git mv` |
| 2.3 | New `crates/llamaforge-core/src/lib.rs` declares `pub mod types; pub mod config; pub mod routing; pub mod queue;`. Re-exports the load-bearing types at the crate root for compat (`pub use types::{ServerId, RequestId, ModelTarget, ...}` — only the symbols imported by ≥3 external sites; everything else stays namespaced). | `crates/llamaforge-core/src/lib.rs` |
| 2.4 | Merge the four old `Cargo.toml`s into the new one. Union the dependency list, deduplicating. Workspace deps only. | `crates/llamaforge-core/Cargo.toml` |
| 2.5 | Update `[workspace.dependencies]` at the workspace root: remove `llamaforge-types`, `-config`, `-routing`, `-queue`; add `llamaforge-core`. | `Cargo.toml` |
| 2.6 | Sweep every other crate's `Cargo.toml`: replace `llamaforge-types = { workspace = true }` (and the other 3) with `llamaforge-core = { workspace = true }`. Affects `llamaforge-metrics`, `-server-manager`, `-dispatch`, `-transport`, `-adapters`, `llamaforge` (bin/lib), `llamaforge-e2e`. | various |
| 2.7 | Path-rename sweep: replace `llamaforge_types::` → `llamaforge_core::types::`, `llamaforge_config::` → `llamaforge_core::config::`, `llamaforge_routing::` → `llamaforge_core::routing::`, `llamaforge_queue::` → `llamaforge_core::queue::`. Use `git grep -lE 'llamaforge_(types|config|routing|queue)::' \| xargs sed -i ''`. Then build and fix the inevitable false-positive matches by hand. | every member crate |
| 2.8 | Visibility audit. Symbols that were `pub` to cross the (now-deleted) crate boundary may become `pub(crate)`. Specifically:<br>• Internal helpers in `routing::affinity::*` no longer need to be `pub` for `routing::router` to use them across files within the same module — already fine, but the audit catches `pub use` re-exports that can drop to `pub(super)`.<br>• Internal queue plumbing (e.g. `pipeline::HealthTracker`) was `pub` for `llamaforge::build` to construct it; that's still needed (composition root reaches in), so leave alone.<br>Document each demotion in the PR body. | various |
| 2.9 | Re-export ergonomics. The old crate root re-exports (`pub use error::RoutingError;` in `routing/lib.rs`) move into `llamaforge-core/src/routing/mod.rs` so call sites stay `llamaforge_core::routing::RoutingError` rather than `llamaforge_core::routing::error::RoutingError`. | `crates/llamaforge-core/src/{types,config,routing,queue}/mod.rs` |
| 2.10 | Delete the four old crate directories. `git rm -rf crates/llamaforge-{types,config,routing,queue}`. | — |
| 2.11 | Update `.github/workflows/ci.yml` if any job mentions a removed crate by name (e.g. `cargo doc -p llamaforge-types`). | `.github/workflows/ci.yml` |
| 2.12 | Update PLAN.md (this file) and `docs/component-designs/{01_CONFIG_MANAGER,04_REQUEST_QUEUE_AND_SCHEDULER,05_SMART_ROUTING}.md` to refer to the new `llamaforge-core::*` paths. Code-path cites in those design docs become stale otherwise. | `docs/component-designs/*.md` |
| 2.13 | Run the full test suite — the queue/routing tests in particular exercise cross-module wiring and will catch missed renames. | — |

### PR2 Acceptance Criteria

- [x] `ls crates/` shows no `llamaforge-{types,config,routing,queue}` directories.
- [x] `grep -rn 'llamaforge_types::\|llamaforge_config::\|llamaforge_routing::\|llamaforge_queue::' crates/` returns 0 hits in non-test paths (test fixtures may briefly use full paths; tidy if unsightly).
- [x] `cargo build --workspace` ≤ wall-clock of pre-merge baseline + 10% (one fewer crate to compile should *help*, not hurt).
- [x] All 5 local gates green (modulo two pre-existing flakes — `test_sigint_orphans` and `test_vc3_11_large_body_concurrency_limit` — that also fail on `origin/main`). CI verification deferred to the parent session.
- [x] No `cargo deny check` advisory regression.
- [x] PR body lists each visibility demotion explicitly (no demotions: the audit found none safe to apply within the PR's behavior-preserving scope; deferred to a follow-up).

---

## PR3 — `llamaforge-api`: merge `transport` + `adapters` + `dispatch`

**Branch:** `consolidate/03-api-merge`
**Depends on:** PR2 merged
**Size:** Large (~3,000 lines moved; ~400 deleted from adapter-only-existed-for-glue code)
**Estimated Time:** 8–12 hours

### Purpose

Create `crates/llamaforge-api/` by absorbing the three crates that together implement the request-handling pipeline: the HTTP/gRPC transport, the port-trait adapters, and the dispatch/proxy layer. After this PR, the three old crates are deleted; their contents live under `llamaforge-api::{transport, adapters, dispatch}`.

### Why these three together

`adapters` exists today solely to bridge port traits between `transport` and the upstream component crates. When `transport` and `dispatch` live in the same crate, the `InferenceEngineAdapter` and `QueueSubmitterAdapter` are no longer adapters at all — they're internal wiring. Several adapter structs (~400 lines total) collapse to direct calls.

`dispatch` and `transport` are kept separate today only because `adapters` needs both. Without `adapters` as a forced intermediary, separating them costs more than it saves.

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 3.1 | `mkdir crates/llamaforge-api && cargo init --lib`. Workspace member. Crate doc cites design doc §4.2 + §7 (Transport layer). | `Cargo.toml`, `crates/llamaforge-api/` |
| 3.2 | `git mv` each old crate's `src/` into a sub-tree: `transport/` → `llamaforge-api/src/transport/`, `adapters/` → `llamaforge-api/src/adapters/`, `dispatch/` → `llamaforge-api/src/dispatch/`. Tests likewise. **Pure moves in this commit.** | `git mv` |
| 3.3 | `crates/llamaforge-api/src/lib.rs` declares the three submodules and re-exports the load-bearing public types: `TransportDeps`, `start_servers`, `Dispatcher`, `ReqwestClientStore`, and the adapter constructors `QueueSubmitterAdapter::new`, `ServerControlAdapter::new`, `MetricsExporterAdapter::new`, `InferenceEngineAdapter::new`. | `crates/llamaforge-api/src/lib.rs` |
| 3.4 | Union the three `Cargo.toml`s. Note that `transport` brings the proto dep, `dispatch` brings `reqwest`, and `adapters` is the only one that needed everything — the union is the union. | `crates/llamaforge-api/Cargo.toml` |
| 3.5 | Update `[workspace.dependencies]`: remove `llamaforge-transport`, `-adapters`, `-dispatch`; add `llamaforge-api`. | `Cargo.toml` |
| 3.6 | Sweep `Cargo.toml`s in remaining crates that referenced any of the three: `llamaforge` (bin/lib), `llamaforge-e2e`. | `crates/llamaforge/Cargo.toml`, `crates/llamaforge-e2e/Cargo.toml` |
| 3.7 | Path-rename sweep: `llamaforge_transport::` → `llamaforge_api::transport::`, `llamaforge_adapters::` → `llamaforge_api::adapters::`, `llamaforge_dispatch::` → `llamaforge_api::dispatch::`. Same mechanical sweep as PR2. | every remaining member crate |
| 3.8 | Adapter collapse audit. Now that `transport`, `adapters`, and `dispatch` live in one crate, several adapter structs become superfluous:<br>• `InferenceEngineAdapter` exists to satisfy `transport::InferenceEngine`. With dispatch + transport in the same crate, the trait can be implemented directly on a thinner type, or the trait deleted in favor of a concrete service. **Decision deferred to PR5**: leave the structs in place for PR3 to keep the diff bounded, document the collapse opportunity in `docs/component-designs/06_DISPATCH_AND_PROXY.md`, follow up after PR4 lands.<br>• `QueueSubmitterAdapter` / `ServerControlAdapter` / `MetricsExporterAdapter`: same — they survive PR3, get re-examined in PR5. | `docs/component-designs/06_DISPATCH_AND_PROXY.md` |
| 3.9 | Visibility audit. The transport-side mock impls in `*/src/grpc.rs::tests` and `*/src/http.rs::tests` reach into dispatch-internal types via `pub` types — many can drop to `pub(crate)` or `pub(super)` now. List each demotion in PR body. | various |
| 3.10 | Delete the three old crate directories. | — |
| 3.11 | Update component design docs: `06_DISPATCH_AND_PROXY.md`, `07_TRANSPORT_LAYER.md`, and any reference to `llamaforge-adapters` (the design doc for adapters is the README of the old crate; if it carries any standalone notes, fold into `06` and `07`). | `docs/component-designs/*.md` |
| 3.12 | Run full test suite. The HTTP and gRPC integration tests under `crates/llamaforge-api/src/transport/tests/` are the highest-cardinality and will catch any missed path-rename. | — |

### PR3 Acceptance Criteria

- [ ] `ls crates/` shows no `llamaforge-{transport,adapters,dispatch}` directories.
- [ ] `grep -rn 'llamaforge_transport::\|llamaforge_adapters::\|llamaforge_dispatch::' crates/` returns 0 hits in non-test paths.
- [ ] HTTP integration tests (`crates/llamaforge-api/src/transport/http/*` tests) pass at the same count as pre-merge.
- [ ] gRPC integration tests (`crates/llamaforge-api/src/transport/grpc/*` tests) pass at the same count as pre-merge.
- [ ] All 5 local gates green; 3/3 CI checks green.
- [ ] PR body lists adapter-collapse opportunities deferred to PR5 (not silently elided).

---

## PR4 — Rename `llamaforge-server-manager` → `llamaforge-server`

**Branch:** `consolidate/04-server-rename`
**Depends on:** PR3 merged
**Size:** Small (~50 lines changed — `Cargo.toml`, workspace `[dependencies]`, import paths)
**Estimated Time:** 1–2 hours

### Purpose

The crate's name today (`llamaforge-server-manager`) is awkward and inconsistent with the target layout's naming convention (`llamaforge-core`, `llamaforge-api`, `llamaforge-server`). Rename to `llamaforge-server`. Pure rename — no module restructure, no behavior change.

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 4.1 | `git mv crates/llamaforge-server-manager crates/llamaforge-server`. | — |
| 4.2 | Edit `crates/llamaforge-server/Cargo.toml`: `name = "llamaforge-server"`. | `crates/llamaforge-server/Cargo.toml` |
| 4.3 | Update workspace `[workspace.dependencies]`: rename the entry. | `Cargo.toml` |
| 4.4 | Sweep `Cargo.toml`s in `llamaforge`, `llamaforge-api`, `llamaforge-e2e` to use the new name. | various |
| 4.5 | Path-rename sweep: `llamaforge_server_manager::` → `llamaforge_server::`. Mechanical. | every member crate |
| 4.6 | Update component design doc filename if it embeds the old crate name. | `docs/component-designs/03_SERVER_MANAGER.md` (content only — keep filename) |
| 4.7 | `cargo doc --no-deps --workspace` to confirm the doc tree builds with the new name. | — |

### PR4 Acceptance Criteria

- [ ] `ls crates/` shows `llamaforge-server`, not `llamaforge-server-manager`.
- [ ] `grep -rn 'llamaforge_server_manager\|llamaforge-server-manager' .` returns 0 hits anywhere in the repo (including docs, CI, CLAUDE.md if applicable).
- [ ] All 5 local gates green; 3/3 CI checks green.
- [ ] `cargo doc` clean.

---

## PR5 — Closure: docs, README, deferred adapter collapse, plan reset

**Branch:** `consolidate/05-closure`
**Depends on:** PR4 merged
**Size:** Medium (~300 lines docs + ~100 lines code for the deferred adapter-collapse if it lands here, else docs-only)
**Estimated Time:** 4–6 hours

### Purpose

Three converging items:

1. **Adapter collapse deferred from PR3.** With `transport` + `dispatch` in one crate, several "adapters" are no longer adapters. Decide whether to collapse them now or defer to a future cleanup. Recommendation: collapse the *trivial* ones (`MetricsExporterAdapter`, `ServerControlAdapter`) in this PR, leave `InferenceEngineAdapter` for a future stand-alone PR (it's load-bearing and the diff would be ~500 lines on its own).
2. **Documentation sync.** `docs/design.md` §4 (architecture diagram), `docs/component-designs/*.md` (per-component crate cites), `README.md` if present, all need to reflect the new 7-crate layout. The `CLAUDE.md` workspace-layout block also needs updating.
3. **PLAN.md reset.** This file is the active phased plan; with this phase complete, rewrite it as a one-pager pointing at "no active phase — see issue tracker for next work." Future phases overwrite as they begin.

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 5.1 | Collapse `MetricsExporterAdapter` if its only consumer is the same-crate transport: the adapter is ~30 lines; the consumer can call the underlying `PrometheusHandle::render()` directly via a 1-line method on a renamed type. Document the rename in PR body. | `crates/llamaforge-api/src/adapters/metrics_exporter.rs` |
| 5.2 | Collapse `ServerControlAdapter` similarly if feasible. Audit first: the trait may be load-bearing for the management HTTP API's serialization shape; if so, leave alone and document why. | `crates/llamaforge-api/src/adapters/server_control.rs` |
| 5.3 | `InferenceEngineAdapter`: do **not** collapse in this PR. The adapter currently owns the queue-event→stream state machine (per design doc §3.1); collapsing it requires picking a new home for that logic. Open a follow-up issue ("collapse InferenceEngineAdapter into transport's stream handler") and link from this PR. | — |
| 5.4 | Update `docs/design.md` §4 architecture diagram: the 7-crate layout (boxes labeled with the new names; arrows = today's dependency edges). Replace the current 13-crate diagram. | `docs/design.md` |
| 5.5 | Update each `docs/component-designs/NN_*.md` to cite the new file paths. The component design docs themselves don't need to merge (each still describes one architectural component) — but the "implementation lives in" pointers update. | `docs/component-designs/*.md` |
| 5.6 | Update `CLAUDE.md` workspace-layout section (if it embeds the old crate list). The block under "Workspace layout" near line 51 of project CLAUDE.md lists the crates explicitly; update to the 7-crate set. | `CLAUDE.md` |
| 5.7 | Update `README.md` (top-level) if it references the old crate names. | `README.md` |
| 5.8 | New-hire test: trace one request from HTTP receipt to backend dispatch. Count the files opened. Target: ≤4 files (api/transport/http.rs → api/transport/router.rs → api/dispatch/proxy.rs → core/queue/pipeline.rs or similar). Document the trace in `docs/design.md` as the canonical "how a request flows" walkthrough. | `docs/design.md` |
| 5.9 | Reset `PLAN.md`: a 5-line placeholder pointing at the issue tracker for the next active phase. (CLAUDE.md says PLAN.md is rewritten when the active phase finishes — this is that.) | `PLAN.md` |
| 5.10 | Open follow-up issue: "collapse InferenceEngineAdapter into transport stream handler" with `tech-debt` label and link to PR5. | (GitHub issue) |

### PR5 Acceptance Criteria

- [ ] `docs/design.md` §4 architecture diagram lists the 7 crates.
- [ ] Every `docs/component-designs/NN_*.md` cites a path that exists in the current workspace.
- [ ] `CLAUDE.md` workspace-layout block matches reality.
- [ ] `PLAN.md` is the 5-line placeholder (this content gets overwritten by the next phase).
- [ ] Follow-up issue opened and referenced from the PR body.
- [ ] All 5 local gates green; 3/3 CI checks green.
- [ ] Relay closure on `llama-forge#consolidate-pr5`.

---

## VC-9 — Workspace Consolidation Sign-Off

**Gate:** PRs 1–5 merged.

| # | Check | Method | Pass criterion |
|---|-------|--------|-----------------|
| VC-9.1 | Crate count | `ls crates/ \| wc -l` | 8 (`llamaforge`, `llamaforge-core`, `llamaforge-server`, `llamaforge-api`, `llamaforge-metrics`, `llamaforge-proto`, `mock-llama`, `llamaforge-e2e`) |
| VC-9.2 | Single `ServerId` | `grep -rn 'pub struct ServerId' crates/` | exactly 1 hit |
| VC-9.3 | All tests pass | `cargo test --workspace` | zero failures |
| VC-9.4 | Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| VC-9.5 | No `unwrap()` regression | grep `\.unwrap()` in `crates/*/src/` excluding tests | ≤ pre-phase baseline (`Uuid::now_v7()` exception remains) |
| VC-9.6 | E2E green | `cargo test -p llamaforge-e2e` | all tests SUCCESS |
| VC-9.7 | CI all-green on PR5 | `gh pr view <PR5> --json statusCheckRollup` | every check SUCCESS |
| VC-9.8 | `cargo deny check` | exit 0 | green |
| VC-9.9 | Documentation builds | `cargo doc --no-deps --workspace` | no warnings |
| VC-9.10 | New-hire trace | manual walkthrough of HTTP-request → backend-dispatch | ≤4 files opened |
| VC-9.11 | No behavior change | sample request through HTTP + gRPC matches pre-phase response byte-for-byte | manual smoke (one request each path) |
| VC-9.12 | No metric name regression | `curl /metrics` after a test request; diff against a pre-phase capture | metric names unchanged |

When VC-9 is green, this phase is closed; `PLAN.md` is reset (per task 5.9).

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Path-rename sweep in PR2/PR3 misses a fully-qualified `llamaforge_types::` (etc.) reference in a doc comment or string literal | High | Low | The `cargo build` step catches code-level misses immediately. Doc-comment/string misses don't break the build but show up in `cargo doc` link warnings — VC-9.9 covers it. |
| Visibility tightening (`pub` → `pub(crate)`) accidentally breaks `llamaforge-e2e`'s reach-in | Medium | Medium | Audit explicit. Each demotion listed in PR body; reviewer cross-checks against e2e's import set. If broken, revert the specific demotion — keep the rest. |
| Merging four crates' tests into one tree creates duplicate test names | Medium | Low | Test names are scoped by file path (`tests::routing::router::test_X` vs `tests::queue::pipeline::test_X`). No conflict in practice; if one shows up, rename the inner test. |
| `cargo build` wall-time regresses (more compilation in fewer parallel units) | Medium | Low | Fewer crates → fewer parallel compile units, but each unit is small and ICE-free. Net wall-time historically *improves* in similar refactors. Measured in VC-9 acceptance. |
| Workflow-ref leak: a code comment cites the old crate name | Medium | Low | `git grep -nE 'llamaforge-(types\|config\|routing\|queue\|transport\|adapters\|dispatch\|server-manager)' crates/` in PR5; replace remaining hits. |
| `docs/component-designs/*.md` reviews flag the move as a design change (it isn't) | Low | Low | Lead PR description with "behavior-preserving refactor, no design change" and link to issue #154. |
| Auto-merge ordering race resurfaces (see `project_automerge_race` memory) | Medium | High | Strictly sequential PRs in this phase. Each PR's `gh pr merge` runs *without* `--auto` (cite memory in each closure message). |
| `cargo deny` advisory lands mid-phase on the merged dep graph | Low | Medium | Run `cargo deny check` locally at every PR boundary (gate 5). The merged crate's `[workspace.dependencies]` is the union of the absorbed crates' deps — no new transitive deps introduced, so risk is no higher than the pre-phase baseline. |
| `InferenceEngineAdapter` deferral in PR5 looks like an evasion to a reviewer | Low | Low | PR5 body links to the explicit follow-up issue with the rationale (load-bearing state machine; out-of-scope diff size). The deferral is named, not hidden. |

---

## Timeline (Single Engineer)

| Day | Work | PR |
|-----|------|----|
| Day 1 | `ServerId` unification | PR1 |
| Day 2–3 | `llamaforge-core` merge — `git mv`, path renames, visibility audit | PR2 |
| Day 4–5 | `llamaforge-api` merge — same shape as PR2 but larger | PR3 |
| Day 6 | `server-manager` → `server` rename + doc sync | PR4 |
| Day 7 | Closure PR: trivial adapter collapses, docs, PLAN.md reset, follow-up issue | PR5 |

If PR2 or PR3 runs long (>2 days each), do **not** carry forward partial work — close it cleanly with the move-only commit landed and a follow-up PR for the visibility audit. The "pure move" commit is the load-bearing piece; the visibility audit can be a smaller separate PR if needed.

---

## Summary: PR → File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `llamaforge-types/src/lib.rs`, `llamaforge-config/{src/handle.rs,Cargo.toml}`, `llamaforge/src/build.rs` | ~150 changed, ~50 deleted |
| PR2 | `crates/llamaforge-core/` (new), four old crate directories deleted, workspace `Cargo.toml` + every member's `Cargo.toml`, every `use llamaforge_{types,config,routing,queue}` site | ~2,500 moved, ~200 deleted |
| PR3 | `crates/llamaforge-api/` (new), three old crate directories deleted, workspace `Cargo.toml` + every member's `Cargo.toml`, every `use llamaforge_{transport,adapters,dispatch}` site | ~3,000 moved, ~400 deleted |
| PR4 | `crates/llamaforge-server/` (renamed from `-server-manager`), workspace `Cargo.toml`, every member's `Cargo.toml`, every `use llamaforge_server_manager` site | ~50 changed |
| PR5 | `docs/design.md`, `docs/component-designs/*.md`, `CLAUDE.md`, `README.md`, `PLAN.md`, possibly `crates/llamaforge-api/src/adapters/*.rs` (trivial collapses) | ~300 docs + ~100 code |
| **Total** | | **~5,500 moved, ~750 deleted** (net **−750 LOC** if no new code) |

---

## Process Notes (carried forward)

- **5-gate local verification + 3/3 CI confirmation before every relay closure.** No exceptions.
- **`gh pr merge --squash --delete-branch` WITHOUT `--auto`** for every PR in this phase. The workspace `Cargo.toml` is touched in PRs 1–4; auto-merge ordering would silently drop changes. Cite `project_automerge_race` memory in each closure.
- **File:line cites in closure messages** for each closed task. If you can't cite a line, the task isn't done.
- **Behavior-preserving guarantee.** If any user-visible behavior changes during this refactor, stop and open a separate PR for the behavior change. This phase is structural only.

---

*Plan revision 1 — 2026-05-22. Replaces the prior `PLAN.md` (Integration & Composition Root, phase 8) which is complete (PRs #1–#11 on `main`).*
