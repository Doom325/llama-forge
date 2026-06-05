# Simplicity & Cognitive Load Review — LlamaForge Setup Wizard

**Reviewer:** design-review-simplicity
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` (full document, Sections 1-15)
**Verdict:** PASS WITH WARNINGS

## Violations Found

### [RULE 6 — Every Piece of Complexity Must Earn Its Place] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 4.1, lines 154-183)

**What's wrong:** The component hierarchy specifies four dedicated single-field input components (`GpuLayersInput`, `CtxSizeInput`, `ParallelSlotsInput`, `RecommendationBadge`) as children of `ServerConfigForm`. Each wraps a single numeric input with a range constraint. This is premature decomposition — the validation rules (Section 5.6) are trivial range checks on integers, and each component adds a file, a contract, and an indirection for what amounts to `<input type="number" min={0}>` with a label.

**Why it matters:** Four components that each render one input field quadruple the number of files a developer must open to understand the config form. The "recommendation" display (a badge next to the field) is not complex enough to warrant its own component either — it is a `<span>` with conditional text. When a developer needs to debug "why is gpu_layers showing the wrong recommendation," they now must trace through `ConfigStep` -> `ServerConfigForm` -> `GpuLayersInput` + `RecommendationBadge` instead of reading one form component. This violates the 3 AM test: under pressure, fewer files to open is always better.

**Suggested fix:** Collapse `GpuLayersInput`, `CtxSizeInput`, `ParallelSlotsInput`, and `RecommendationBadge` into `ServerConfigForm` itself. If a reusable "numeric input with recommendation" pattern emerges after the wizard ships, extract it then (rule of three). The hierarchy becomes:

```
ConfigStep (orchestrator)
└── ServerConfigForm (presentational — per-model config form)
```

One component, one file, all form fields inline with their recommendations. If the form grows beyond ~150 lines later, extract at that point with a clear motivation.

---

### [RULE 6 — Every Piece of Complexity Must Earn Its Place] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Sections 12-13, lines 702-738)

**What's wrong:** The dual-mode architecture (standalone wizard via `llamaforge init --interactive` + first-run detection in `llamaforge serve`) creates two distinct server startup paths that must be independently implemented, tested, and maintained. The standalone mode (Section 12) spins up a minimal axum server with only UI + setup endpoints. The first-run mode (Section 13) boots the full transport layer and adds a `mode` field to `/api/status` that the frontend must check to conditionally redirect. These are two different boot sequences, two different shutdown behaviors (standalone exits on config save; serve transitions to running mode), and two different test matrices.

**Why it matters:** For a solo-maintained project, two boot paths means two places to fix when the setup API surface changes. The justification — that standalone mode needs no gRPC or inference engine — is an optimization for a process that runs for 2-10 minutes during initial setup. The ergonomic benefit of `llamaforge init --interactive` (no running server needed) is real, but the implementation cost of maintaining a separate minimal-server path alongside a "setup mode" in the full server is not trivial.

**Suggested fix:** Consider whether the standalone mode can reuse the same "setup mode" boot path from Section 13 — the full transport layer starts but with no servers configured, which is exactly what already happens in the first-run case. The only real difference is "exit on save" vs. "transition to running." That could be a single boolean flag (`--exit-after-setup`) rather than an entirely separate server assembly. This collapses two boot paths into one with a flag:

```rust
Init { interactive: bool } => {
    // Same as `serve` but with:
    //   1. Force "setup mode" regardless of config state
    //   2. After config save, exit 0 instead of transitioning
    serve_with_options(SetupModeOptions { exit_on_save: true, force_setup: true })
}
```

If startup time of the full transport layer is a concern (unlikely for a localhost tool), profile it before building the separate path. Make it work with one path first.

---

### [RULE 6 — Every Piece of Complexity Must Earn Its Place] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11, lines 519-699)

**What's wrong:** Seven API endpoints are specified. Six of them (`hardware`, `scan-binary`, `scan-models`, `build`, `recommend`, `save-config`) are justified by the user flows. The seventh, `check-path` (Section 11.7), exists solely to provide an on-blur "is this path writable?" check for the save-path input field. This is a dedicated endpoint for a single UI micro-interaction.

**Why it matters:** The `check-path` endpoint must be implemented, tested, and maintained for a feature that provides marginal UX improvement over just attempting the save and showing the error. The user enters a save path once, at the very end of the wizard. If the path is not writable, the `save-config` endpoint already returns `403 Forbidden` with a descriptive message (Section 11.6). The `check-path` endpoint duplicates this validation in a separate code path.

**Suggested fix:** Drop the `check-path` endpoint. Let the user click "Save," and if the path is not writable, show the 403 error inline with a "Choose a different path" action. This is the normal web pattern for save operations and eliminates one endpoint, one handler, one test suite, and one potential divergence between the check and the actual save. If the on-blur UX is truly important after user testing, add it then.

---

### [RULE 7 — Optimize for the Reader, the Debugger, and the Operator] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 4.1, lines 162-168)

**What's wrong:** The `BinaryStep` has five children: `BinaryFound`, `BinaryNotFound`, `BuildConfigurator`, `BuildProgress`, and `BinaryManualEntry`. These represent mutually exclusive states of the binary detection flow — exactly one is visible at any time. Decomposing mutually exclusive view states into separate components is reasonable for complex views, but three of these (`BinaryFound`, `BinaryNotFound`, `BinaryManualEntry`) are single-state display components (a path + version, a "not found" message + build button, a text input). They could be inline `{#if}` blocks in `BinaryStep` without loss of clarity.

**Why it matters:** Five files for one wizard step increases cognitive load when tracing the binary detection flow. A developer debugging "why does the manual entry not show after a failed build" must open `BinaryStep` to understand the state machine, then open `BinaryManualEntry` to verify the rendering, then check the event flow back up. If the three simple states were inline, the entire flow would be visible in one file.

**Suggested fix:** Keep `BuildConfigurator` and `BuildProgress` as separate components (they have real complexity — a form with backend selector and a streaming log viewer respectively). Inline `BinaryFound`, `BinaryNotFound`, and `BinaryManualEntry` into `BinaryStep` as conditional blocks. This reduces five children to two.

---

### [RULE 8 — Boring Technology Is Beautiful Technology] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 6.1, lines 346-349)

**What's wrong:** The design specifies prefetching the next step's data while the user is on the current step (e.g., prefetching the binary scan while the user reviews hardware results). This is a performance optimization for a wizard that the user runs once, where each step's API call takes at most a few seconds. The complexity of managing prefetch lifecycle (what if the user goes back? what if the prefetched data depends on a user choice that changed?) is an innovation token spent on a flow where 2 seconds of loading time per step is perfectly acceptable.

**Why it matters:** Prefetching introduces subtle state management questions. The binary scan (Step 2) depends on no user input, so prefetching it from Step 1 is safe. But the pattern is documented as a general principle ("when the user is on Step N, prefetch Step N+1 if deterministic"), which invites future developers to try prefetching Step 4 (config recommendations) from Step 3 (model selection) — where the user's model selection is still in progress. The generalization is more dangerous than the specific case.

**Suggested fix:** Remove the prefetching strategy from v1. Each step fetches its own data on entry. If any step's load time is unacceptable in practice (profile first), add targeted prefetching for that specific transition with a comment explaining why. Do not document it as a general wizard pattern.

---

### [RULE 9 — Make It Work, Make It Right, Make It Fast — In That Order] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 4.4, lines 244-249)

**What's wrong:** The design specifies a "progress estimation timer" for the build step that "updates the estimated time remaining every second based on build log heuristics (cmake configure -> make with N targets)." Estimating build time from log heuristics is notoriously unreliable — the time per compilation unit varies wildly depending on template instantiation, optimization level, and system load. This is a premature feature design for a v1 wizard.

**Why it matters:** Implementing build time estimation requires parsing cmake's target count from log output, tracking elapsed time per target, and extrapolating. The estimation will be wrong frequently enough to be misleading (e.g., linking takes disproportionately long). An inaccurate "ETA: 2 minutes" that becomes "ETA: 8 minutes" damages user trust more than showing no estimate.

**Suggested fix:** For v1, show only the current phase ("Cloning...", "Configuring...", "Compiling...") and the build log. Drop the time estimation. The build SSE already provides `percent` for the clone phase (where progress is deterministic from git) and build phase (from cmake's `[N/M]` output). Show `[N/M] targets compiled` as a progress indicator, which is factual, not estimated. Add wall-clock elapsed time ("Building... 2m 34s elapsed") which is always accurate.

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 0     |
| MEDIUM   | 2     |
| LOW      | 4     |

## Commendations

1. **State management is well-right-sized.** The `SetupWizardStore` (Section 5.3) is a single flat class with seven fields, instantiated via Svelte context and garbage-collected on unmount. No store proliferation, no nested reactive state, no global singletons. The decision to use context scoping instead of a global store is exactly right for session-scoped wizard state.

2. **TanStack Query with `staleTime: Infinity` is the correct call.** Hardware detection results and model scan results genuinely do not go stale within a single wizard session. Using infinite stale time avoids background refetches that would make no sense in a linear wizard flow. This is a case where the simpler configuration is also the correct one.

3. **The "Alternatives Considered" section (Section 15) is exemplary.** Documenting rejected approaches with rationale (standalone React app, prebuilt binary downloads, URL params vs. client store, linear vs. free navigation) saves future developers from re-evaluating the same trade-offs. The explanations are concrete and grounded in project constraints, not abstract.

4. **Shell-command hardware detection over Rust FFI (Section 15, third alternative).** Choosing `nvidia-smi` and `system_profiler` over `nvml-wrapper`, Metal FFI, and `ash` is a textbook "boring technology" decision. The parsing layer is testable, the commands are well-documented, and the dependency cost is zero. This saves multiple innovation tokens for a once-per-setup operation.

5. **No `{@html}` anywhere.** The TOML preview uses `renderMarkdown()` with a code block, and the build log uses `<pre>` text content. This is disciplined adherence to the project's security convention without shortcuts.

6. **URL-per-step navigation design (Section 5.4).** Making each wizard step a distinct route gives back/forward for free, makes refreshing predictable, and keeps the router as the single source of truth for "which step am I on." No custom history stack, no state machine for step transitions — the browser's own navigation model does the work. Simple and correct.
