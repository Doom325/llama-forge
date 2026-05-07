# LlamaForge — Project Brief for the Coder Agent

You are the **CoderAgent** working on LlamaForge. This file gives you the project context the global Rust-coder extension does not have. Read it fully before touching code.

The global extension at `~/.gemini/extensions/rust-coder/GEMINI.md` owns **how to code** (idiomatic Rust rules, inner dev loop, PR workflow, reviewer rubric). This file owns **what to code** and **where to find the source of truth** for any change.

---

## What LlamaForge is

LlamaForge is a Rust orchestrator and gateway for fleets of `llama.cpp` (`llama-server`) processes. It:

- **Supervises** the lifecycle of multiple `llama-server` instances (spawn, monitor, restart, drain, shutdown).
- **Multiplexes** OpenAI- and Anthropic-compatible HTTP/JSON traffic on top of a typed **gRPC** core (Protocol Buffers over HTTP/2 — gRPC is the primary API; HTTP shims exist for SDK compatibility).
- **Schedules** requests across servers with priority-aware queuing and queue-level preemption.
- **Routes** requests by slot availability and KV-cache (prompt) affinity to maximize goodput.
- **Observes** every request with TTFT, throughput, and goodput metrics, and exposes a config/management web UI.

Conceptually it sits where LiteLLM sits for cloud APIs, but owning the full local inference stack instead of forwarding to remote endpoints. Single-machine today; architected to evolve into a multi-node cluster manager.

Written in Rust, async-first on **Tokio**, gRPC via **tonic**, HTTP via **axum**, telemetry via **tracing**, persistence via **SQLite (rusqlite + rusqlite_migration)**.

---

## Workspace layout

```
llama-forge/
├── Cargo.toml                # workspace root; deps live in [workspace.dependencies]
├── rust-toolchain.toml       # pinned stable toolchain — do not bump silently
├── crates/
│   ├── llamaforge/           # binary crate — the orchestrator process
│   └── llamaforge-proto/     # generated tonic/prost bindings (build.rs from proto/)
├── proto/                    # .proto definitions (source of truth for all gRPC types)
├── config/                   # example llamaforge.toml configurations
└── docs/                     # source of truth for design and plans (see below)
```

When in doubt about *where* code goes, consult the active component design first; it dictates module layout.

---

## Components

LlamaForge decomposes into seven components, each with its own design doc under `docs/component-designs/`. They are numbered to indicate the natural implementation order — earlier components are dependencies of later ones, but each is reviewed and shipped independently.

| # | Component | Design doc | One-line responsibility |
|---|-----------|-----------|------------------------|
| 01 | **Config Manager** | `docs/component-designs/01_CONFIG_MANAGER.md` | Parse, validate, hot-reload `llamaforge.toml`; arbitrate UI overrides via `ArcSwap` and the `ConfigOverlayStore` trait. |
| 02 | **Metrics & Storage** | `docs/component-designs/02_METRICS_AND_STORAGE.md` | SQLite-backed metrics persistence (TTFT, tokens/s, queue depth) behind the `MetricsStore` trait; aggregation and retention. |
| 03 | **Server Manager** | `docs/component-designs/03_SERVER_MANAGER.md` | Lifecycle of `llama-server` processes — spawn with full flag surface, health-check, restart-with-backoff, graceful drain. |
| 04 | **Request Queue & Scheduler** | `docs/component-designs/04_REQUEST_QUEUE_AND_SCHEDULER.md` | Priority-aware FIFO with per-requestor limits; preemption and slot reservation; admission control. |
| 05 | **Smart Routing** | `docs/component-designs/05_SMART_ROUTING.md` | Pick the right `llama-server` per request: slot availability, KV-cache (prompt prefix) affinity, model match. |
| 06 | **Dispatch & Proxy** | `docs/component-designs/06_DISPATCH_AND_PROXY.md` | Stream tokens from chosen server back to the client; SSE keepalive, cancellation, timeouts; error translation. |
| 07 | **Transport Layer** | `docs/component-designs/07_TRANSPORT_LAYER.md` | The dual-protocol gateway — gRPC on :50051 (canonical), HTTP/SSE shim on :8000 for OpenAI/Anthropic SDK clients. |

Cross-cutting concerns (gRPC service definitions, GGUF parsing, mock-llama harness, audit log) are addressed in the master design and in `IMPLEMENTATION_PLAN.md` phases — not in any single component design.

---

## Document hierarchy — the source of truth

Three layers of docs drive every change. **Read them in this order whenever a task is ambiguous**:

### 1. `docs/design.md` — master design

The full system design: goals/non-goals, related work, architecture, data models, API spec, request lifecycle, priority system, smart-routing strategy, observability, deployment, security, failure modes, testing strategy. Treat it as the constitution: any code change must be consistent with it. If you need to deviate, that's a design conversation, not a coding decision.

### 2. `docs/component-designs/NN_<COMPONENT>.md` — per-component spec

The detailed design for a single component: its public API, internal structure, error model, persistence schema, concurrency contracts, and interaction with neighboring components. Always read the design for the component you're touching **before** writing code in its module — even small changes can violate invariants documented here.

If `docs/component-designs/reviews/` contains review notes for the active component (e.g. `architecture_review.md`, `safety_review.md`), those are the open issues a reviewer has flagged on the current iteration of that component's design or implementation. Treat them as authoritative outstanding work.

### 3. `PLAN.md` — implementation plan for the *current* component

`PLAN.md` at the **project root** holds the active phased task list for **the component currently being implemented**, structured as `Phase → Task → Sub-task` (sub-tasks may nest):

```
## Phase 1 — <goal for this slice of the current component>
### 1.1 — <task>
- [ ] **1.1.1** <sub-task>
- [ ] **1.1.2** <sub-task>
### 1.2 — Open PR for Phase 1
- [ ] **1.2.1** Push branch + open PR
- [ ] **1.2.2** send(to="ReviewerAgent", key="<repo>#<component>-phase1", value="<PR URL>")
- [ ] **1.2.3** Wait for verdict; address findings under same key
```

Key facts about `PLAN.md`:

- It is **scoped to one component at a time.** Whichever component the team is currently building, its phased tasks live in `PLAN.md`. When that component is finished and merged, `PLAN.md` is replaced (or rewritten) with the next component's plan.
- It is **the executable layer of the component design.** Every task in `PLAN.md` should map to something specified in the corresponding `docs/component-designs/NN_<COMPONENT>.md`. If a task contradicts the component design, the design wins — fix the plan, not the code.
- It is **a living document.** If a sub-task turns out to be wrong, missing, or in the wrong order, edit `PLAN.md` and commit the plan change separately so the reviewer can see the scope adjustment.
- **PR creation, relay handoff, and review iteration are tasks in `PLAN.md`,** placed at the right boundaries by the plan author. Your job is to follow the next unchecked task — never to decide on your own when to open a PR. (See the global extension's "Workflow loop" section for the mechanics of executing each task kind.)

The relationship across the three layers:

```
docs/design.md                                  ← system constitution (rare changes)
    │
    ├── docs/component-designs/NN_<NAME>.md     ← per-component spec for the active component
    │                                             (occasional revisions; reviewed separately)
    │
    └── PLAN.md                                  ← phased task list for the active component only
                                                   (the day-to-day execution plan)
```

`docs/IMPLEMENTATION_PLAN.md` is the **master phased plan for the whole product** (Phase 0–6 across all components, rough timeline and verification checkpoints). It is read-only context for the agent — it tells you which component's implementation slice you're inside. The detailed task list always lives in `PLAN.md`.

---

## Conventions specific to LlamaForge

- **Workspace deps only.** Add new dependencies to `[workspace.dependencies]` in the root `Cargo.toml`; member crates reference them with `workspace = true`. Never add a version-pinned dep directly to a member crate's `Cargo.toml`.
- **gRPC is canonical.** All inter-service contracts live in `proto/`. The HTTP shim translates *into* the gRPC types — it is never authoritative. If a feature needs a new field, add it to the `.proto` first and let `tonic-build` regenerate.
- **`llamaforge-proto` is generated code.** Do not hand-edit. Edit `proto/*.proto` and let `build.rs` rebuild it.
- **SQLite is behind traits.** All persistence goes through `MetricsStore` and `ConfigOverlayStore` (or future siblings). Never reach for `rusqlite::Connection` directly outside the trait implementation; doing so couples callers to SQLite and blocks the documented future backend swap.
- **Telemetry stays out of domain logic** (R26 in the reviewer rubric). Use `#[instrument]` at adapter boundaries; do not sprinkle `tracing::info!` calls inside scheduling, routing, or queue code.
- **Hot-reload safety.** Config is wrapped in `ArcSwap`. Readers must `.load()` once per request and treat the snapshot as immutable for the request's lifetime — do not re-read mid-request.
- **`mock-llama` exists for tests.** Integration tests must drive against `mock-llama` (with deterministic delays and error injection) rather than a real `llama-server`. Real-server tests are gated behind an explicit feature flag and excluded from default CI.
- **Respect `rust-toolchain.toml`.** Do not `rustup override` to a newer toolchain to make a change compile.

---

## Where the workflow lives

Don't reproduce workflow rules here. The global extension at `~/.gemini/extensions/rust-coder/GEMINI.md` is authoritative for:

- Idiomatic Rust rules (error handling, borrowing, types, modules, docs, tests, unsafe, deps, concurrency, performance).
- Pre-commit verification gate (`fmt`, `clippy -D warnings`, `test`, `doc`).
- Conventional Commits format and commit hygiene.
- The reviewer rubric (R1–R32) — what the ReviewerAgent will grade your PR against.
- The workflow loop: **follow the next unchecked task in `PLAN.md`**, with specific mechanics for code-change tasks, PR-creation tasks, and review-iteration tasks.

This project's only addition to the workflow is the document hierarchy above: every task in `PLAN.md` should be backed by a sentence in `docs/design.md` and elaborated in the active component's design under `docs/component-designs/`. When those three are inconsistent, stop and ask — don't pick one to follow silently.
