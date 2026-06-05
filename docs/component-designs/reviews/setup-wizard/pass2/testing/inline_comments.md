<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 519-539 RULE: 24 SEVERITY: LOW -->
**Rule 24 — Test Interactions, Not Just Units**
The SSE streaming tests and recovery path tests are thorough, but the build cancel flow is missing. Section 11.4 defines `DELETE /api/setup/build/{id}` and `BuildProgress` has an `oncancel` event (Section 4.2), yet no integration test walks the full cancel path: start build -> cancel mid-build -> verify subprocess killed -> verify SSE stream terminates -> verify temp directory cleaned up -> verify UI transitions to cancelled state.

Suggested addition to Section 10.2 "SSE streaming tests":
```markdown
- **Build cancel mid-stream:** Start a build (open SSE connection, receive several `log` events).
  Trigger cancel (DELETE /api/setup/build/{id}). Verify the SSE stream receives no further
  `log` or `progress` events after the cancellation. Verify the UI transitions to the
  cancelled/idle state. Verify the backend cleans up the temp directory (backend integration
  test: confirm temp dir is removed after cancel).
```
---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 519-524 RULE: 24 SEVERITY: LOW -->
**Rule 24 — Test Interactions, Not Just Units**
The integration tests mention "Navigation: Verify back/forward between steps preserves entered data" but do not cover the *invalidation* direction from the step state machine (Section 5.3). The `invalidatesOnReentry` mechanism removes TanStack Query cache and client state for downstream steps when a user revisits an earlier step. This is a multi-component interaction (state machine + query client + store) that needs its own test.

Suggested addition to Section 10.2 "Integration Tests":
```markdown
- **Step re-entry invalidation:** Complete the wizard through to Review. Navigate back to
  the Hardware step via the step indicator. Re-run hardware detection (mock returns a
  different GPU — e.g., RTX 3060 instead of RTX 4090). Advance forward to Configure step.
  Verify the config recommendations reflect the new GPU (lower gpu_layers, smaller ctx_size).
  Advance to Review step. Verify the TOML preview uses the updated config values. This
  exercises the invalidatesOnReentry mechanism end-to-end.
```
---
