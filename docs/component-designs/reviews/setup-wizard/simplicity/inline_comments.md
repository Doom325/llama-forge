<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 174-182 RULE: 6 SEVERITY: MEDIUM -->
**Rule 6 — Every Piece of Complexity Must Earn Its Place**
`GpuLayersInput`, `CtxSizeInput`, `ParallelSlotsInput`, and `RecommendationBadge` are four separate components wrapping single form fields with trivial validation (integer range checks). This is premature decomposition — each adds a file, a contract, and an indirection for what amounts to `<input type="number">` with a label and a `<span>` badge.

Collapse all four into `ServerConfigForm` itself. If a reusable "numeric input with recommendation" pattern emerges after the wizard ships and you see it needed in three places, extract then. Right now this quadruples the files a developer must open to understand the config form.

Suggested hierarchy:
```
ConfigStep (orchestrator)
└── ServerConfigForm (presentational — all fields inline)
```

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 702-738 RULE: 6 SEVERITY: MEDIUM -->
**Rule 6 — Every Piece of Complexity Must Earn Its Place**
The standalone wizard (Section 12) and first-run detection (Section 13) create two distinct server boot paths: a minimal axum server vs. the full transport layer in "setup mode." Both serve the same UI and the same `/api/setup/*` endpoints, but they are assembled differently, shut down differently (exit vs. transition), and must be tested independently.

For a solo-maintained project, consider collapsing these into one boot path. The first-run "setup mode" already starts the full server with no configured instances — which is exactly what standalone mode needs. The only behavioral difference is "exit on config save" vs. "transition to running." That is a single boolean flag, not a separate server assembly:

```rust
// Instead of two paths:
serve_with_options(SetupModeOptions {
    exit_on_save: true,   // standalone: exit 0 after save
    force_setup: true,    // standalone: always enter setup mode
})
```

One boot path, one test matrix, one place to update when the setup API surface changes. If the full transport layer's startup time matters (unlikely for localhost), profile before building the separate path.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 693-699 RULE: 6 SEVERITY: LOW -->
**Rule 6 — Every Piece of Complexity Must Earn Its Place**
The `POST /api/setup/check-path` endpoint (Section 11.7) exists solely for an on-blur "is this writable?" check on the save-path input. The `save-config` endpoint (Section 11.6) already returns `403 Forbidden` with a descriptive error if the path is not writable.

Drop this endpoint for v1. Let the user click "Save" and handle the error inline — this is the standard web pattern. One fewer endpoint to implement, test, and keep in sync with the actual save logic. Add the on-blur check later only if user testing shows the save-error UX is insufficient.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 162-168 RULE: 7 SEVERITY: LOW -->
**Rule 7 — Optimize for the Reader, the Debugger, and the Operator**
`BinaryStep` has five children, three of which (`BinaryFound`, `BinaryNotFound`, `BinaryManualEntry`) are trivial single-state display components (a path + version string, a "not found" prompt, a single text input). These are mutually exclusive states — only one shows at a time.

Keep `BuildConfigurator` and `BuildProgress` as separate components (genuine complexity). Inline the other three as `{#if}` blocks in `BinaryStep` so the entire binary-detection flow is visible in one file:

```svelte
{#if scanResult.found}
  <!-- path + version display -->
{:else if showManualEntry}
  <!-- text input for manual path -->
{:else}
  <!-- "not found" prompt + build button -->
{/if}
```

This reduces five files to two for this step.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 346-349 RULE: 8 SEVERITY: LOW -->
**Rule 8 — Boring Technology Is Beautiful Technology**
Prefetching the next step's data while the user is on the current step is a performance optimization for a wizard that runs once, where each step's API call takes a few seconds at most. The generalized principle ("prefetch Step N+1 if deterministic") invites unsafe future prefetching where the user's input is still in progress.

Remove prefetching from v1. Each step fetches on entry. If a specific step's load time proves unacceptable in practice, add targeted prefetching for that one transition with a comment explaining the motivation. Do not document it as a general wizard pattern.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 244-249 RULE: 9 SEVERITY: LOW -->
**Rule 9 — Make It Work, Make It Right, Make It Fast — In That Order**
The "progress estimation timer" for the build step estimates time remaining from build log heuristics. Build time estimation is notoriously unreliable — linking takes disproportionately long, template-heavy files vary wildly, and system load is unpredictable. An inaccurate "ETA: 2 minutes" that becomes "ETA: 8 minutes" damages trust more than no estimate.

For v1, show factual progress only:
- Clone phase: git's own percentage (already in the SSE `progress` field)
- Build phase: `[N/M] targets compiled` (from cmake output, already parseable)
- Wall-clock elapsed time: "Building... 2m 34s elapsed" (always accurate)

Drop the time estimation. Add it in a future iteration only if users request it, and only after measuring actual build time distributions across representative hardware.
