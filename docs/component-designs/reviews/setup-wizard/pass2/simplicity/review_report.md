# Simplicity & Cognitive Load Review (Pass 2) — LlamaForge Setup Wizard

**Reviewer:** design-review-simplicity
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` (full document, Sections 1-15 + Appendices)
**Verdict:** PASS WITH WARNINGS

## First-Pass Findings — Verification

### Finding 1: MEDIUM R6 — Over-decomposed config form (4 sub-components) — RESOLVED

The four sub-components (`GpuLayersInput`, `CtxSizeInput`, `ParallelSlotsInput`, `RecommendationBadge`) have been removed. Section 4.1 (line 171) now shows:

```
ConfigStep (orchestrator)
└── ServerConfigForm (presentational — per-model config form; handles GPU layers,
    context size, parallel slots, and recommendation display internally)
```

Section 4.2 (lines 203-206) explicitly states the form uses "standard `<input>` and `<label>` elements. No sub-components — each field is a labeled input with an inline 'Recommended: X' hint derived from the `recommendation` prop." This is exactly the fix the first pass requested. Clean resolution.

### Finding 2: MEDIUM R6 — Dual-mode boot path complexity — PARTIALLY RESOLVED

The first pass recommended collapsing the standalone wizard and first-run detection into a single boot path with an `exit_on_save` flag. The document has been revised to share the setup router (Section 12.1, `setup_router()` function) and the handler logic is identical in both modes. This addresses the API divergence concern.

However, Section 12.2 (line 975-978) still describes a distinct "minimal axum server" boot path: "Mount only: `ui_router().merge(setup_router())`. No gRPC, no inference engine, no config handle, no SSE events, no management API." Section 13.4 (lines 1127-1133) describes a different boot path: "Boot the full transport layer (HTTP + gRPC) on the default ports" with `full_app_router().merge(setup_router())`.

These are still two different server assemblies — one minimal, one full. The shared router mitigates the worst risk (API endpoint divergence), but the two boot paths still require separate integration tests and separate reasoning about what is and is not available. The task list shows "#15 — Simplify dual-mode boot into single path with exit_on_save flag" as completed, but the document retains the dual-path structure.

**Assessment:** The shared router is a meaningful improvement, and the remaining difference (minimal vs. full server assembly) may be a deliberate design choice to avoid booting gRPC for a one-time setup CLI. This is acceptable for v1 given the shared handler code, but the residual complexity should be noted. See NEW finding below.

### Finding 3: LOW R6 — Unnecessary check-path endpoint — NOT FULLY RESOLVED

The `check-path` endpoint has been removed from the body of the document — there is no Section 11.x defining its contract, and no frontend code references it. However, the route is still registered in the `setup_router()` code block in Section 12.1, line 966:

```rust
.route("/api/setup/check-path", post(check_path_handler))
```

This is a stale reference. The endpoint has no contract, no handler specification, no tests, and no frontend consumer. It will cause confusion during implementation: a developer reading the router will attempt to implement `check_path_handler` and find no spec for it.

### Finding 4: LOW R7 — BinaryStep children too decomposed — RESOLVED

Section 4.1 (line 163) now describes `BinaryStep` as:

```
BinaryStep (orchestrator — uses inline {#if} blocks for found/not-found/manual states)
├── BuildConfigurator (presentational — backend selector)
└── BuildProgress (presentational — streaming log)
```

The three trivial display states (`BinaryFound`, `BinaryNotFound`, `BinaryManualEntry`) have been inlined as `{#if}` blocks. Only `BuildConfigurator` and `BuildProgress` remain as separate components, which is exactly the first-pass recommendation. Clean resolution.

### Finding 5: LOW R8 — Premature prefetching — RESOLVED

Section 6.1 (line 372) now explicitly states: "Each step fetches its data on mount — no prefetching of subsequent steps. The wizard is a linear flow where each step depends on user confirmation of the previous step's results, so prefetching would either be wasted (user goes back) or premature (user hasn't confirmed yet)."

This is a clear rejection of prefetching with correct rationale. Clean resolution.

### Finding 6: LOW R9 — Unreliable build time estimation — RESOLVED

Section 4.4 (line 240) now states: "The build step displays factual progress only: cmake's `[N/M]` target count (parsed from build log output) and wall-clock elapsed time. No estimated time remaining — build time estimation from log heuristics is unreliable and misleading."

However, a stale reference persists in Section 2.2 (line 82), the Key Screens table, which still says "Streaming build log with progress estimation" in the Loading column for the Build screen. This contradicts Section 4.4 and should say "Streaming build log with target count and elapsed time" or similar. Minor inconsistency.

---

## New Complexity Introduced by Fixes — Assessment

The first-pass fixes and the broader document revision introduced several new mechanisms. Each is evaluated below for whether it earns its complexity.

### [RULE 6 — Every Piece of Complexity Must Earn Its Place] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 12.1, line 966)

**What's wrong:** The `setup_router()` code block registers `check-path` and `validate-launch` routes. The `check-path` endpoint was flagged for removal in pass 1 and its contract has been deleted from the document, but the route registration remains. This is a stale artifact that will mislead implementers.

**Why it matters:** A developer reading Section 12.1 will see 10 routes and attempt to implement all 10. Nine have specifications; `check-path` does not. This creates a "where is the spec?" investigation that wastes time and may lead to an ad-hoc implementation that diverges from the intended "just use save-config" pattern.

**Suggested fix:** Remove line 966 (`.route("/api/setup/check-path", post(check_path_handler))`) from the `setup_router()` function.

---

### Mode State Machine (Section 13.2) — JUSTIFIED

The three-state machine (`Setup -> Transitioning -> Running`) with the rollback edge (`Transitioning -> Setup` on hot-reload failure) is well-motivated. The states map directly to observable behavior differences (inference endpoints return 503 vs. live), and the transition guards prevent the system from entering an inconsistent state (e.g., claiming "running" while the config reload failed). For a server that transitions exactly once per boot, three states is the minimum to represent "not ready," "becoming ready," and "ready." The `Arc<ArcSwap<Mode>>` is the standard Tokio pattern for shared mutable state. No over-engineering here.

### Build Job Pattern — POST+GET+SSE+DELETE (Section 11.4) — JUSTIFIED

The build is a long-running operation (5-30 minutes) that must survive SSE disconnections, support cancellation, and report status to reconnecting clients. The four-verb pattern (POST creates, GET polls, GET /stream reconnects SSE, DELETE cancels) is the standard pattern for long-running jobs in HTTP APIs. The alternative — a single POST that holds the connection open for 30 minutes — would be fragile and non-reconnectable. The 2-minute grace period for disconnected clients is reasonable for a build that takes 5-30 minutes. Complexity is earned.

### Step Transition Table (Section 5.3) — JUSTIFIED

The `STEP_TRANSITIONS` record with `canAdvance` guards and `invalidatesOnReentry` lists is a single, flat data structure that replaces what would otherwise be scattered conditional logic across multiple step components. It makes the wizard's navigation rules readable in one glance and prevents stale downstream data when the user navigates back. The invalidation-on-reentry behavior is specifically motivated (e.g., changing hardware detection results must invalidate config recommendations). Good use of a declarative structure.

### Backup-Before-Overwrite Logic (Sections 11.4, 11.6) — JUSTIFIED

Both binary installation and config save create timestamped backups before overwriting. For a tool that manages inference server configuration and binaries, accidentally destroying a working config or binary with no recovery path would be a real operational failure. The backup mechanism is simple (rename with timestamp), bounded (5 backups max, oldest pruned), and best-effort (failure to prune does not block the operation). The SSE `complete` event surfaces the backup path so the user knows where to find it. Proportionate to the risk.

### Session Correlation (Section 11.7) — JUSTIFIED WITH MINOR RESERVATION

The `X-Setup-Session-Id` header provides end-to-end log correlation across all wizard API calls. For debugging "what happened during this user's setup," this is valuable — it lets you grep a single UUID to reconstruct the entire flow. The implementation is lightweight (frontend generates a UUID, attaches it as a header, backend extracts it into a tracing span field). No server-side session state is stored.

Minor reservation: the document specifies that if the header is missing, the field is set to `"anonymous"`. This is fine, but the document should be explicit that the backend never rejects requests missing this header — it is purely observational. This is already implied by "No server-side session state" but could be stated more directly to avoid a future implementer adding a 400 check.

### Validation-Launch Step in Save-Config (Section 13.5) — JUSTIFIED

Spawning a test `llama-server` process after saving the config and before transitioning to running mode catches configuration errors that static validation cannot detect (e.g., CUDA out of memory, binary incompatible with the detected GPU). The 30-second timeout is reasonable. The validation server is killed immediately after the health check, so it does not interfere with the real server spawn. This prevents the worst first-run experience: config saves successfully, server transitions to running mode, then all inference requests fail because the model does not fit in VRAM. Worth the complexity.

---

### [RULE 6 — Every Piece of Complexity Must Earn Its Place] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Sections 12.2 vs. 13.4)

**What's wrong:** Despite the shared `setup_router()`, the standalone and first-run modes still describe two different server assemblies. Section 12.2 mounts `ui_router().merge(setup_router())` on an ephemeral port. Section 13.4 boots the full transport layer with `full_app_router().merge(setup_router())` on default ports. These are two different `main()` branches with different startup sequences, different port binding strategies, and different shutdown behaviors.

**Why it matters:** Two boot paths means two places to update when adding a new setup endpoint, two integration test configurations, and two mental models for "how does the wizard server start." The shared router mitigates endpoint divergence but does not eliminate the boot-path divergence. For a solo-maintained project, this is maintenance surface that may not earn its keep.

**Suggested fix:** This is borderline. The ergonomic argument for the minimal standalone server (no gRPC listener, ephemeral port, exit-on-save) is real — a user running `llamaforge init --interactive` does not expect gRPC on port 50051 to start. If the dual path is retained, add a note in Section 12 explicitly acknowledging the trade-off: "The standalone mode uses a minimal server assembly to avoid binding gRPC/inference ports that are not needed during setup. This means two boot paths exist and must be maintained. The shared `setup_router()` ensures endpoint behavior is identical; the difference is only in which other services are co-hosted." This makes the deliberate choice visible to future maintainers.

---

### [RULE 7 — Optimize for the Reader, the Debugger, and the Operator] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 2.2, line 82)

**What's wrong:** The Key Screens table (Section 2.2) still says "Streaming build log with progress estimation" in the Loading column for the Build screen. Section 4.4 explicitly states "No estimated time remaining — build time estimation from log heuristics is unreliable and misleading." This is a contradiction within the same document.

**Why it matters:** A developer reading Section 2.2 to understand the wizard's screen states will believe the build screen shows estimated time remaining. When they read Section 4.4 they will find the opposite. This wastes time resolving the contradiction and risks implementing the wrong behavior if the developer reads Section 2.2 but not Section 4.4.

**Suggested fix:** Change line 82's Loading column from "Streaming build log with progress estimation" to "Streaming build log with target count (`[N/M]`) and elapsed time".

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 0     |
| MEDIUM   | 0     |
| LOW      | 3     |

### First-Pass Resolution Status

| # | Finding | Status |
|---|---------|--------|
| 1 | MEDIUM R6: Over-decomposed config form | RESOLVED |
| 2 | MEDIUM R6: Dual-mode boot path | PARTIALLY RESOLVED (shared router good; dual boot path remains) |
| 3 | LOW R6: Unnecessary check-path endpoint | NOT FULLY RESOLVED (stale route in setup_router) |
| 4 | LOW R7: BinaryStep children too decomposed | RESOLVED |
| 5 | LOW R8: Premature prefetching | RESOLVED |
| 6 | LOW R9: Unreliable build time estimation | RESOLVED (stale text in Section 2.2 table) |

### New Findings

| # | Severity | Rule | Description |
|---|----------|------|-------------|
| 1 | LOW | R6 | Stale `check-path` route in `setup_router()` (line 966) |
| 2 | LOW | R6 | Dual boot path trade-off should be documented explicitly |
| 3 | LOW | R7 | Section 2.2 table contradicts Section 4.4 on build progress display |

No new MEDIUM or HIGH findings were introduced by the fixes. The added mechanisms (Mode state machine, build job pattern, step transition table, backup-before-overwrite, session correlation, validation-launch) are all proportionate to their respective problems and appropriate for the project's scale.

## Commendations

1. **Clean resolution of the config form decomposition.** The `ServerConfigForm` contract (Section 4.2, lines 203-206) is explicit: "No sub-components — each field is a labeled input with an inline 'Recommended: X' hint." This is the right level of detail to prevent re-decomposition by a future contributor who thinks single-field components are a good idea.

2. **The step transition table is a better solution than was requested.** The first pass did not ask for a state machine, but the `STEP_TRANSITIONS` table (Section 5.3) elegantly solves a problem the first pass did not anticipate: downstream invalidation when the user navigates back. Having the invalidation rules co-located with the advancement guards in a single declarative structure is cleaner than scattering cache-clearing logic across individual step components.

3. **Prefetching rejection rationale is well-articulated.** Section 6.1 does not just say "no prefetching" — it explains *why* in terms specific to the wizard's linear flow model. This prevents future re-introduction without re-evaluation.

4. **Build progress fix is precise.** Section 4.4 specifies exactly what is shown (cmake's `[N/M]` target count + wall-clock elapsed time) and exactly what is not shown (estimated time remaining), with a one-sentence rationale. No ambiguity for the implementer.

5. **Shared `setup_router()` function (Section 12.1) is a good structural mitigation.** Even though the dual boot path was not fully collapsed, centralizing all route definitions in one function eliminates the highest-risk divergence vector (one mode having endpoints the other lacks, or handlers wired differently). This was a pragmatic partial fix.
