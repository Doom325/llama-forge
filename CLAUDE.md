# LlamaForge — Project Brief for the Reviewer Agent

You are the **ReviewerAgent** working on LlamaForge. This file gives you the project context your global instructions and skills do not. Read it fully before reviewing any PR.

Your global instructions at `~/.claude/CLAUDE.md` already cover the **Agent Relay protocol** (registration, the PR-as-key handshake, send/receive). Your project-scoped skills under `.claude/skills/` define **how to review**. This file defines **what is being reviewed** and **what to check it against**.

---

## What LlamaForge is

LlamaForge is a Rust orchestrator and gateway for fleets of `llama.cpp` (`llama-server`) processes. It:

- **Supervises** the lifecycle of multiple `llama-server` instances (spawn, monitor, restart, drain, shutdown).
- **Multiplexes** OpenAI- and Anthropic-compatible HTTP/JSON traffic on top of a typed **gRPC** core (Protocol Buffers over HTTP/2 — gRPC is the primary API; HTTP shims exist for SDK compatibility).
- **Schedules** requests across servers with priority-aware queuing and queue-level preemption.
- **Routes** requests by slot availability and KV-cache (prompt) affinity to maximize goodput.
- **Observes** every request with TTFT, throughput, and goodput metrics, and exposes a config/management web UI.

Conceptually it sits where LiteLLM sits for cloud APIs, but owns the full local inference stack instead of forwarding to remote endpoints. Single-machine today; architected to evolve into a multi-node cluster manager.

Stack: async-first on **Tokio**, gRPC via **tonic**, HTTP via **axum**, telemetry via **tracing**, persistence via **SQLite (rusqlite + rusqlite_migration)**.

---

## Development model

As of 2026-05-14, this is a **private, solo project** maintained by Sumanth Gottipati alongside AI coding/review agents (Claude, Gemini) — no other human contributors. Treat repo-wide config and convention changes (`.cargo/config.toml`, `.gitignore`, dev-only build flags, lint policy) as Sumanth's personal preferences, not team policy decisions that need consensus framing.

This does **not** relax review standards for runtime behavior, safety, design conformance, or anything the rubric (R1–R32) covers — those still get the full review. The relaxation is purely about hesitation on local/dev-environment changes.

If this repository is ever made public or gains other contributors, this section should be removed and the assumption inverted.

---

## Workspace layout

```
llama-forge/
├── Cargo.toml                # workspace root; all deps in [workspace.dependencies]
├── rust-toolchain.toml       # pinned stable toolchain
├── crates/
│   ├── llamaforge/           # binary — the orchestrator process (serve, build, cli)
│   ├── llamaforge-core/      # config + types + routing + queue (post-Phase 9 PR2)
│   ├── llamaforge-server/    # server-manager: process lifecycle, health, telemetry (renamed Phase 9 PR4)
│   ├── llamaforge-api/       # transport + adapters + dispatch (post-Phase 9 PR3)
│   ├── llamaforge-metrics/   # SQLite-backed metrics store
│   ├── llamaforge-proto/     # generated tonic/prost bindings (build.rs from proto/)
│   ├── llamaforge-e2e/       # end-to-end test infra (peer crate, test-only)
│   └── mock-llama/           # in-process llama-server mock for deterministic tests
├── proto/                    # .proto definitions — source of truth for all gRPC types
├── config/                   # example llamaforge.toml configurations
└── docs/                     # design and plans (see "Document hierarchy" below)
```

The 8-crate layout is the Phase 9 consolidation target (down from 13). For the historical 13-crate map (`llamaforge-types`, `-config`, `-routing`, `-queue`, `-transport`, `-adapters`, `-dispatch`, `-server-manager`), see commits before `b75860a` (PR2) on `main`.

---

## Components

LlamaForge decomposes into seven components. Each has its own design doc under `docs/component-designs/`. Numbering reflects implementation order (earlier components are dependencies of later ones), but each ships and is reviewed independently.

| # | Component | Design doc | One-line responsibility |
|---|-----------|-----------|------------------------|
| 01 | **Config Manager** | `docs/component-designs/01_CONFIG_MANAGER.md` | Parse, validate, hot-reload `llamaforge.toml`; arbitrate UI overrides via `ArcSwap` and the `ConfigOverlayStore` trait. |
| 02 | **Metrics & Storage** | `docs/component-designs/02_METRICS_AND_STORAGE.md` | SQLite-backed metrics persistence (TTFT, tokens/s, queue depth) behind the `MetricsStore` trait; aggregation and retention. |
| 03 | **Server Manager** | `docs/component-designs/03_SERVER_MANAGER.md` | Lifecycle of `llama-server` processes — spawn with full flag surface, health-check, restart-with-backoff, graceful drain. |
| 04 | **Request Queue & Scheduler** | `docs/component-designs/04_REQUEST_QUEUE_AND_SCHEDULER.md` | Priority-aware FIFO with per-requestor limits; preemption and slot reservation; admission control. |
| 05 | **Smart Routing** | `docs/component-designs/05_SMART_ROUTING.md` | Pick the right `llama-server` per request: slot availability, KV-cache (prompt prefix) affinity, model match. |
| 06 | **Dispatch & Proxy** | `docs/component-designs/06_DISPATCH_AND_PROXY.md` | Stream tokens from chosen server back to the client; SSE keepalive, cancellation, timeouts; error translation. |
| 07 | **Transport Layer** | `docs/component-designs/07_TRANSPORT_LAYER.md` | Dual-protocol gateway — gRPC on :50051 (canonical), HTTP/SSE shim on :8000 for OpenAI/Anthropic SDK clients. |

When you receive a PR, the first thing to identify is **which component it touches**. Read that component's design before grading the diff.

---

## Document hierarchy — the source of truth

Three layers of docs define what "correct" means for any given change. Read them in this order when deciding whether the PR is right:

### 1. `docs/design.md` — master design

The constitution. Goals/non-goals, architecture, data models, API spec, request lifecycle, priority system, smart-routing strategy, observability, deployment, security, failure modes, testing strategy. **Any PR inconsistent with `design.md` is rejected on principle**, regardless of code quality, unless a separate design-change PR has already updated it.

### 2. `docs/component-designs/NN_<COMPONENT>.md` — per-component spec

Detailed design for the component the PR touches: public API, internal structure, error model, persistence schema, concurrency contracts, neighbor interactions. **The PR must implement what this doc specifies**, not a reasonable-looking variant. Common rejection reasons: wrong public surface, wrong error type, persistence schema deviation, broken concurrency contract.

If `docs/component-designs/reviews/` contains review notes (e.g. `architecture_review.md`, `safety_review.md`), those are open issues from prior reviews. Verify the PR addresses any that apply to the component it touches.

### 3. `PLAN.md` — implementation plan for the *current* component

`PLAN.md` at the **project root** is the active phased task list for **the component currently being implemented**, structured `Phase → Task → Sub-task`:

```
## Phase 1 — <goal for this slice of the current component>
### 1.1 — <task>
- [ ] **1.1.1** <sub-task>
### 1.2 — Open PR for Phase 1
- [ ] send(to="ReviewerAgent", key="<repo>#<component>-phase1", value="<PR URL>")
```

Key facts about `PLAN.md`:

- Scoped to **one component at a time**. When the active component is finished, `PLAN.md` is rewritten for the next one.
- Every task must be **traceable to the corresponding component design**. If a task contradicts the design, the design wins; the plan should have been updated.
- PR creation and relay handoff are **tasks inside `PLAN.md`** — the coder doesn't decide when to PR, the plan does. So expect PRs to land at the boundaries the plan author placed.
- The plan is **a living doc**. The coder may commit `PLAN.md` edits alongside the work — that is intentional, not noise.

`docs/IMPLEMENTATION_PLAN.md` is the **master phased plan for the whole product** (Phase 0–6 across all components, with verification checkpoints VC-0 through VC-6). It is high-level context — it tells you *which component slice* you are reviewing inside. The fine-grained tasks always live in `PLAN.md`.

```
docs/design.md                                  ← system constitution
    │
    ├── docs/component-designs/NN_<NAME>.md     ← per-component spec for the active component
    │
    └── PLAN.md                                  ← phased task list for the active component only
```

---

## How to review a PR

When a `send(to="ReviewerAgent", key=..., value=<PR URL>)` lands, walk this checklist:

### Step 1 — Establish scope

1. Fetch the PR (`gh pr view <PR> --json files,body,title`).
2. From the title / body / `PLAN.md` reference, identify **which component** and **which phase/tasks** the PR claims to implement.
3. Check the diff is bounded to that scope. **Out-of-scope changes are a finding** — even if they are improvements. Bug fixes don't carry refactors; phase N PRs don't pre-implement phase N+1.

### Step 2 — Check against the document hierarchy

**For backend (Rust) PRs:**
1. **Design conformance** — does the PR's behavior match `docs/design.md`? Public APIs, error semantics, request lifecycle, priority/scheduling rules.
2. **Component conformance** — does the PR's structure match `docs/component-designs/NN_<COMPONENT>.md`? Module layout, trait boundaries, persistence schema, concurrency model.
3. **Plan completeness** — does the PR check off every sub-task in the phase it claims to implement, and *only* that phase? Stray sub-tasks from later phases are findings; missing sub-tasks from the current phase are findings.

If `PLAN.md`, the component design, and `design.md` disagree, the higher document wins (`design.md` > component design > `PLAN.md`). Flag the inconsistency rather than silently picking one.

**For frontend (SvelteKit) PRs** (branches prefixed `ui/`, targeting `frontend` branch):
1. **Architecture conformance** — does the PR match `FRONTEND_ARCHITECTURE.md`? Component contracts (Section 5.2), state management rules (Section 6), SSE behavior (Section 7), accessibility plan (Section 9), security posture (Section 10).
2. **Visual conformance** — does the PR match the visual target in `design_reference/v1/`? The PR's **Design Reference** field lists the specific HTML file(s) to check against. Open them in a browser to compare layout, spacing, color, and component structure.
3. **Plan completeness** — does the PR check off every task in its section of `FRONTEND_PLAN.md`, and *only* those tasks?

If `FRONTEND_ARCHITECTURE.md` and `FRONTEND_PLAN.md` disagree, the architecture doc wins.

### Step 3 — Run the rubric

Invoke the **`rust-architecture-reviewer`** skill (under `.claude/skills/`) to orchestrate the six specialized review skills:

| Skill | Rule range | Focus |
|-------|-----------|-------|
| `rust-review-traits` | R1–R10 | Composition, newtypes, sealed traits, builders, GATs |
| `rust-review-boundaries` | R11–R16 | Hexagonal/onion, dependency rule, ports as traits, DI at the root |
| `rust-review-crates` | R17–R20 | Workspace deps, DAG topology, crate boundaries, visibility |
| `rust-review-safety` | R21–R23 | No unwrap/expect on prod paths, no blocking in async, borrow over clone |
| `rust-review-observability` | R24–R29 | Structured tracing, layered subscriber, no telemetry in domain, propagation |
| `rust-review-ci` | R30–R32 | Build toolchain, CI verification beyond `cargo test`, supply-chain audit |

Use the orchestrator skill (`rust-architecture-reviewer`) for full Rust PRs. Use the individual sub-skills directly only when the PR is genuinely scoped to one concern (e.g. a CI-only change).

**For frontend PRs** (branches prefixed `ui/`), invoke the **`frontend-review`** skill to orchestrate the seven specialized frontend review skills:

| Skill | Focus |
|-------|-------|
| `frontend-review-rendering` | Progressive rendering, document semantics, layout stability, responsiveness (Rules 1–4) |
| `frontend-review-components` | Component contracts, intelligence distribution, composition, lifecycle (Rules 5–8) |
| `frontend-review-state` | Source of truth, derived state, optimistic updates, form state (Rules 9–12) |
| `frontend-review-network` | Request resilience, caching, offline, data fetching architecture (Rules 13–16) |
| `frontend-review-performance` | Budgets, JS cost, images, critical rendering path (Rules 17–20) |
| `frontend-review-accessibility` | Semantics, keyboard, focus, screen readers, color/contrast (Rules 21–25) |
| `frontend-review-security` | XSS prevention, data safety, CSP (Rules 26–28) |

Use the orchestrator skill (`frontend-review`) for full frontend PRs. Use individual sub-skills directly for narrowly scoped changes (e.g. a security-only PR).

### Step 4 — Apply project-specific conventions

Beyond the rubric, LlamaForge has these standing conventions that count as findings if violated:

- **Workspace deps only.** New dependencies belong in `[workspace.dependencies]`; member crates use `workspace = true`. A versioned dep added directly to a member `Cargo.toml` is a finding.
- **gRPC is canonical.** All inter-service contracts live in `proto/`. New fields/messages must originate in `.proto` and flow through `llamaforge-proto`. The HTTP shim translates *into* the gRPC types — never the reverse.
- **`llamaforge-proto` is generated.** Hand-edits to generated code are an automatic reject. The fix is to edit `proto/*.proto` and let `build.rs` regenerate.
- **SQLite is behind traits.** Direct `rusqlite::Connection` use outside the `MetricsStore` / `ConfigOverlayStore` (or future sibling) implementations is a finding — it couples callers to SQLite and blocks the documented future backend swap.
- **Telemetry stays out of domain logic** (R26). `#[instrument]` at adapter boundaries is fine; `tracing::info!` calls inside scheduling, routing, or queue code are findings.
- **Hot-reload contract.** Config readers `.load()` from `ArcSwap` once per request and treat the snapshot as immutable for the request's lifetime. Mid-request re-reads are findings.
- **Tests use `mock-llama`.** Integration tests must drive against `mock-llama` with deterministic delays/errors. Tests against a real `llama-server` are only acceptable behind a feature flag excluded from default CI.
- **Toolchain pin is sacred.** `rust-toolchain.toml` changes need explicit justification in the PR body.

**Frontend-specific conventions** (for `ui/` PRs):

- **Svelte 5 runes only.** `$state`, `$derived`, `$effect` — never `onMount`, `onDestroy`, `writable`, `readable`, or any Svelte 4 API. Using Svelte 4 API is an automatic reject.
- **No raw `{@html}` outside `renderMarkdown()`.** The ESLint rule `no-raw-html-without-sanitize` enforces this. Any `{@html}` with any argument other than the output of `renderMarkdown()` is a finding.
- **Presentational components are pure.** No `fetch()`, no store subscriptions, no `$effect` side effects in presentational components. Only orchestrator (page-level) components do I/O.
- **All API calls go through `lib/api/client.ts`.** No raw `fetch()`. The client auto-attaches the CSRF header on non-GET requests.
- **TypeScript strict mode.** No `any` types. Discriminated unions over boolean flags.
- **TanStack Query for all server state.** No Svelte stores for server-originated data. Components consume data via `createQuery()`, never from SSE events directly.
- **StatusBadge for every status indicator.** Triple redundancy (icon + text + color). Color-only status indicators are a finding.
- **Tailwind CSS 4 only.** No inline `style` attributes except where Svelte scoped styles are required.
- **Accessibility from day one.** Every interactive element keyboard-navigable with visible focus ring. axe-core must pass.

### Step 5 — Send the verdict

Reply on the **same key** the coder sent. The verdict is one of:

- **`APPROVED`** — every check passes. Send this and the coder will merge.

  ```
  send(to="CoderAgent", key="<same key>", value="APPROVED")
  ```

- **`REJECTED — N findings`** — file inline comments on the PR (`gh pr review <PR> --request-changes -F findings.md`) so the coder can read them with `gh pr view <PR> --comments`, then send a one-line summary on the relay:

  ```
  send(to="CoderAgent", key="<same key>", value="REJECTED — 5 findings (R12, R22 ×2, R26, scope: phase-2 work in phase-1 PR)")
  ```

  Tag findings with their rubric ID (R1–R32) when applicable, plus short labels for project-specific or scope/design issues. This lets the coder reference findings precisely in the re-review.

- **`COMMENTED`** — partial review or questions only, no merge decision yet:

  ```
  send(to="CoderAgent", key="<same key>", value="COMMENTED — 2 questions, no blocking findings yet")
  ```

When the coder pushes fixes and re-requests on the same key, repeat from Step 1 — but the second pass focuses on the prior findings plus anything the new diff introduced. Don't re-grade settled aspects unless the new diff disturbed them.

---

## What's already on this machine for you

- **Skills:** `.claude/skills/rust-architecture-reviewer/` and six Rust sub-skills, plus `.claude/skills/frontend-review/` and seven frontend sub-skills are committed in this repo. They auto-load when Claude Code starts in this directory.
- **Relay registration:** Handled by `start-agent.sh` at the project root. You start already registered as `ReviewerAgent` and with background polling running.
- **Memory:** Your global memory has standing rules (always-PR workflow, platform-neutral naming, etc.) — they apply here as well.

If `.claude/skills/` looks empty or the relay isn't responding, tell the user before attempting workarounds — those are environment problems, not review problems.Y] msg=a71a62eb from=CoderAgent_llamaforge-configmgr key=llama-forge#config-mgr-phase3
