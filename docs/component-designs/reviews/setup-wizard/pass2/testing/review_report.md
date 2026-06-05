# Testing & Defensive Programming Review (Pass 2) — LlamaForge Setup Wizard

**Reviewer:** design-review-testing
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` — Sections 5.3, 5.6, 10, 11, 12, 13 (full document, focus on testing strategy and boundary validation)
**Verdict:** PASS

## Prior Finding Resolution

### Finding 1 — HIGH R22: No backend API tests
**Status: RESOLVED**

Section 10.6 "Backend API Tests" has been added with comprehensive coverage:
- Input validation boundary tests for every endpoint (`scan-models`, `scan-binary`, `build`, `recommend`, `save-config`)
- Happy-path response contract tests verifying documented JSON shapes
- Error response contract tests verifying structured JSON error envelopes
- Backend integration tests with mocked shell commands and filesystem fixtures (`tempdir` with `.gguf` files, mock `llama-server` binary, mock `git`/`cmake` scripts)

This directly addresses the original gap. The tests are specific, with concrete inputs and expected outcomes (e.g., `scan-models` with empty `directories` array -> 400, `save-config` with path to read-only location -> 403).

---

### Finding 2 — HIGH R22: Build endpoint no boundary validation
**Status: RESOLVED**

Section 10.6 now includes:
- `build` with `backend` not in (`cuda` | `metal` | `vulkan` | `cpu`) -> 400 (enum validation)
- `build` with `install_dir` outside home directory -> 400 (path sanitization)
- `build` with `jobs` = 0 or negative -> 400 (lower bound)
- `build` with `jobs` > `num_cpus` -> clamped with warning (upper bound, not rejected)

Section 11.4 also now specifies resource controls: jobs cap at `min(requested_jobs, num_cpus / 2)`, process priority (`nice 10`), and disk space check (5 GB minimum). The validation is specified at the contract level and tested at the boundary level. Fully addressed.

---

### Finding 3 — HIGH R24: Frontend-backend contract gap
**Status: RESOLVED**

Section 10.7 "Contract Tests" has been added with two approaches:
1. JSON schema snapshot tests generated from backend test output, stored in version control, validated against frontend TypeScript types
2. Preferred alternative: `ts-rs` crate to generate TypeScript types directly from Rust response structs at build time

Coverage requirement is explicit: every response type in Section 11 must have either a snapshot or a `ts-rs` derivation, and CI fails if a new response struct is added without a corresponding contract test. This is a strong mechanism to prevent contract drift.

---

### Finding 4 — MEDIUM R25: Recovery paths untested
**Status: RESOLVED**

Section 10.2 now includes a dedicated "Recovery path tests" subsection with five specific scenarios:
- Build failure -> retry -> success (mock failed SSE, verify error UI, simulate retry, mock success, verify transition)
- Build failure -> skip -> manual entry -> next step (verify manually-entered binary path carries through to Review step)
- Hardware detection failure -> manual entry -> config (verify recommendations use manually-entered values, not defaults)
- SSE reconnect during build (disconnect without terminal event, verify reconnection, verify log continues without duplicates)
- SSE reconnect exhaustion (3 consecutive failed reconnects, verify "Connection lost" UI, verify manual retry)

Each test verifies the full recovery path end-to-end, not just that the error state appears. This was the core ask, and it is fully met.

---

### Finding 5 — MEDIUM R24: SSE streaming undertested
**Status: RESOLVED**

Section 10.2 now includes a dedicated "SSE streaming tests" subsection with five specific tests:
- SSE event parsing: each event type (`phase`, `progress`, `log`, `complete`, `failed`) tested individually with correct UI update verification
- SSE unexpected disconnect: closes without `complete`/`failed`, verify reconnect behavior (not false success/error)
- SSE malformed event: invalid JSON in `data` field, verify graceful handling (console log, continue processing, no crash)
- Long build log: 5000+ lines, verify auto-scroll, bounded memory, UI responsiveness
- SSE navigation-away cleanup: verify `EventSource` closed, no further events processed

This covers the event parsing, error handling, memory management, and lifecycle cleanup concerns from the original finding.

---

### Finding 6 — MEDIUM R22: Form validation no upper bounds
**Status: RESOLVED**

Section 5.6 now specifies concrete bounds with test cases:
- `gpu_layers`: -1 (auto) or 0-512. Tests: -2 -> rejected, 513 -> rejected, -1 -> accepted, 0 -> accepted
- `ctx_size`: 0 (auto) or 128-1048576. Tests: 64 -> rejected, 2000000 -> rejected
- `parallel_slots`: 1-64. Tests: 0 -> rejected, 65 -> rejected
- `port`: 1024-65535, unique across servers. Tests: 1023 -> rejected, 65536 -> rejected
- `binary_path`: non-empty string
- `model_path`: non-empty string

The bounds are sensible and aligned with real-world `llama-server` constraints. Each bound has explicit boundary test cases specified inline.

---

### Finding 7 — MEDIUM R25: Standalone wizard mode untested
**Status: RESOLVED**

Section 10.8 "Standalone Wizard Mode Tests" has been added with four specific test scenarios:
- Save failure -> server stays alive -> retry -> success -> clean shutdown
- Ctrl+C -> SIGINT -> exit 130, no zombie process (including child process cleanup)
- Idle timeout -> clean shutdown after configured duration (with `--timeout=5s` for testing)
- Concurrent tabs -> first save wins, second gets 409 or waits (mutex behavior verification, no file corruption)

These cover the lifecycle, signal handling, timeout, and concurrency concerns from the original finding. The idle timeout test (Finding 7 originally asked about "browser close" — the design now has an explicit idle timeout mechanism in Section 12.5 that addresses this, and the test verifies it).

---

### Finding 8 — MEDIUM R23: Hardware detection parser untested
**Status: RESOLVED**

Section 10.6 now includes hardware detection parser tests:
- Mocked `nvidia-smi` / `system_profiler` output -> correct parsing of GPU name, VRAM, driver version
- Hardware detection with gibberish command output -> error (not 200 with `vram_mb: 0`)
- Hardware detection with empty output (no GPU) -> 200 with empty `gpus` array + `recommended_backend: "cpu"`
- Multi-GPU `nvidia-smi` output -> all GPUs in the `gpus` array with correct per-GPU VRAM
- `system_profiler` with Apple Silicon unified memory -> correct `vram_mb` reflecting shared GPU memory

The "gibberish output -> error" test directly addresses the Rule 23 concern about silent default-on-error masking failures. The multi-GPU and Apple Silicon cases cover the major platform variations.

---

### Finding 9 — LOW R24: No contract tests
**Status: RESOLVED**

Same as Finding 3. Section 10.7 addresses this comprehensively.

---

### Finding 10 — LOW R22: Model scanner no input limits
**Status: RESOLVED**

Section 11.3 now specifies:
- `max_depth` parameter (default 5) for recursion depth
- `max_results` parameter (default 100) for result count
- 30-second scan timeout on the backend
- `truncated` boolean and `truncation_reason` enum (`"timeout"` | `"max_results"` | `"max_depth"`) in the response
- `warnings` array that includes permission denied and symlink cycle messages

Section 10.6 includes:
- `scan-models` with > 10 directories -> 400 (directory count limit)
- `scan-models` with path traversal -> 400 or sanitized

The symlink cycle protection is confirmed in Section 11.3's example response (`"Symlink cycle detected: /data/models/loop"` in warnings) and in Section 11.7's observability spec (WARN-level log for symlink cycles with `kind = "symlink_cycle"`).

---

## New Feature Test Coverage Assessment

The first-pass findings drove significant additions to the design. I now check whether these new features have corresponding test coverage.

### Mode State Machine (Section 13.2) — Adequate

The Mode state machine (`Setup -> Transitioning -> Running`, with `Transitioning -> Setup` fallback on hot-reload failure) is exercised by the CompletionScreen verification tests (Section 13.7) and the save-config flow (Section 13.5). The polling logic in `verifySetupComplete()` tests the `Running` transition. The hot-reload failure path tests the `Transitioning -> Setup` revert. No gap.

### Build Job Pattern (Section 11.4) — Adequate

The build endpoint was redesigned from a single POST to a job pattern (POST creates, GET polls, GET `/stream` reconnects, DELETE cancels). The SSE streaming tests in Section 10.2 cover the stream endpoint. The backend API tests in Section 10.6 cover the POST validation (backend enum, install_dir, jobs). The SSE reconnect tests cover the reconnection semantics. The DELETE endpoint (cancel) is not explicitly tested in Section 10, but the `BuildProgress` component contract in Section 4.2 includes an `oncancel` event, and the Section 10.8 Ctrl+C test verifies subprocess cleanup. Minor gap — see finding below.

### Backup-Before-Overwrite (Sections 11.4 and 11.6) — Adequate

Both binary backup (Section 11.4, SSE `complete` event with `replaced_existing` and `backup_path`) and config backup (Section 11.6, response with `backup_path`, 5-backup retention with pruning) are specified. The SSE event parsing tests in Section 10.2 would verify the `complete` event shape. The backend API tests in Section 10.6 cover save-config happy paths. No explicit test for "save-config with existing file and force=true -> backup created and returned in response," but this is a straightforward backend contract test covered by the happy-path response contract tests requirement.

### Validation-Launch (Section 13.5) — Adequate

The validation-launch step (spawn test server, health check, 30s timeout) is tested through the CompletionScreen verification flow (Section 13.7). The response shapes for validation-launch success and failure are documented in Section 13.5, and Section 10.7's contract tests cover all response types. The failure case (`validation_launch.success: false`) is explicitly handled in the CompletionScreen verification sequence with "Go Back" and "Retry" buttons.

### Step State Machine with Downstream Invalidation (Section 5.3) — Minor Gap

The step transition table with `canAdvance` guards and `invalidatesOnReentry` is well-specified. The integration tests in Section 10.2 include "Navigation: Verify back/forward between steps preserves entered data." However, there is no explicit test for the downstream invalidation behavior: "Go back to Hardware, re-run detection, verify Configure and Review steps are invalidated (TanStack Query data removed, client state cleared)." This is a testable interaction. See finding below.

---

## Violations Found

### [RULE 24 — Test Interactions, Not Just Units] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 10.2 — Integration Tests)

**What's wrong:** The build cancel flow (`DELETE /api/setup/build/{id}`) is not explicitly tested. Section 11.4 defines the endpoint (`DELETE -> 200 { cancelled: true }`) and Section 4.2 gives the `BuildProgress` component an `oncancel` event, but no integration test walks the cancel path: start a build, cancel mid-build, verify the build process is killed, verify the SSE stream emits no further events, verify the temp directory is cleaned up.

**Why it matters:** Build cancellation is a user-facing action (the BuildProgress component has a cancel button). If the cancel endpoint fails to kill the subprocess or the SSE stream keeps emitting events after cancellation, the UI will be in an inconsistent state. This is an interaction test — it involves the frontend cancel action, the DELETE endpoint, the backend subprocess lifecycle, and the SSE stream.

**Suggested fix:** Add to Section 10.2 Integration Tests:

```markdown
- **Build cancel:** Start a build (open SSE connection). Trigger cancel 
  (DELETE /api/setup/build/{id}). Verify the SSE stream receives no further 
  `log` or `progress` events after a `cancelled` terminal event (or the 
  stream closes). Verify the backend cleans up the temp directory. Verify 
  the UI transitions to the cancelled state.
```

---

### [RULE 24 — Test Interactions, Not Just Units] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 10.2 — Integration Tests)

**What's wrong:** The downstream invalidation behavior of the step state machine (Section 5.3) is not explicitly tested. The design specifies that re-entering a completed step (e.g., navigating back to "Hardware" from "Review") triggers `invalidatesOnReentry` which removes TanStack Query data for downstream steps and clears associated client state. The integration tests mention "back/forward between steps preserves entered data" but do not test the invalidation direction: "re-entering an earlier step causes downstream steps to lose their data and refetch."

**Why it matters:** Downstream invalidation is a correctness property that protects users from stale data. If a user changes their hardware detection results, the Configure and Review steps must use the new hardware data, not the old recommendations. This is an interaction between the step state machine, TanStack Query cache management, and the client store — exactly the kind of multi-component interaction that Rule 24 targets.

**Suggested fix:** Add to Section 10.2 Integration Tests:

```markdown
- **Step re-entry invalidation:** Complete the wizard through to Review. 
  Navigate back to Hardware step. Re-run hardware detection (mock a 
  different GPU result). Advance to Configure step. Verify the config 
  recommendations are based on the new hardware data (not cached from 
  the first pass). Verify the Review step's TOML preview reflects the 
  updated recommendations.
```

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 0     |
| MEDIUM   | 0     |
| LOW      | 2     |

All 10 prior findings are **RESOLVED**. The two new LOW findings are non-blocking improvement opportunities related to test coverage for the build cancel flow and step invalidation behavior.

## Commendations

1. **Thorough resolution of all prior findings.** Every finding from pass 1 has been addressed with specific, concrete test descriptions — not vague "add tests for this." The recovery path tests, SSE streaming tests, standalone mode tests, and backend API tests each contain enough detail to implement directly without ambiguity.

2. **Contract test strategy with escalation path.** Section 10.7 provides both a pragmatic approach (JSON schema snapshots) and a preferred approach (`ts-rs` codegen) with explicit CI enforcement ("CI fails if a new response struct is added without a corresponding contract test"). This is a mature approach to an easy-to-neglect problem.

3. **Boundary test cases specified inline with validation rules.** Section 5.6 embeds the test cases directly next to the validation rules (e.g., `gpu_layers: -1 (auto) or 0-512. Tests: -2 -> rejected, 513 -> rejected`). This co-location makes it trivial to verify test coverage and impossible for the tests to drift from the specified bounds.

4. **Hardware parser tests with platform-specific fixtures.** Section 10.6 specifies parser tests against real-world output patterns (multi-GPU `nvidia-smi`, Apple Silicon `system_profiler`, empty output for no GPU) and explicitly tests the failure-masking anti-pattern (gibberish output -> error, not 200 with `vram_mb: 0`). This directly addresses the Rule 23 concern about silent degradation.

5. **Standalone mode tests cover the full lifecycle.** Section 10.8's four test scenarios cover the complete lifecycle from startup through clean shutdown, including signal handling, idle timeout, concurrent access, and error recovery. The SIGINT test explicitly checks for zombie processes, which is a detail most test plans miss.

6. **Model scanner limits are defense-in-depth.** The `max_depth`, `max_results`, scan timeout, `truncated` flag, and `truncation_reason` enum form a complete bounding strategy with user-facing transparency. The response tells the user *why* results were truncated, not just that they were.
