# Testing & Defensive Programming Review — LlamaForge Setup Wizard

**Reviewer:** design-review-testing
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` — Section 10 (Testing Strategy), Section 11 (Backend API Contract), Section 6 (Network Resilience), Section 5 (State Management), Section 9 (Security Posture)
**Verdict:** PASS WITH WARNINGS

## Violations Found

### [RULE 22 — Validate at Boundaries, Trust Nothing] — Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 10 — Testing Strategy / Section 11 — Backend API Contract)

**What's wrong:** The testing strategy (Section 10) does not include any tests for backend API endpoint input validation. Section 11 defines seven new endpoints (`/api/setup/hardware`, `scan-binary`, `scan-models`, `build`, `recommend`, `save-config`, `check-path`) that accept filesystem paths, numeric parameters, and structured JSON payloads — but Section 10 contains zero backend-level tests. All described tests are frontend component tests, frontend integration tests, visual regression, accessibility, or performance tests. No backend unit tests, no backend integration tests, no validation boundary tests.

**Why it matters:** These endpoints accept user-supplied filesystem paths (`directories`, `extra_paths`, `install_dir`, `path`) that the backend resolves and operates on. Path traversal, symlink exploitation, and shell injection through crafted paths are real attack vectors. The `POST /api/setup/build` endpoint executes `git clone` and `cmake` — it runs arbitrary build tooling on the host. Without backend tests that validate boundary inputs (empty arrays, paths with `../`, paths with null bytes, non-UTF-8 sequences, extremely long paths, directories the process cannot read, non-existent paths), the first line of defense is untested. The design states the backend is authoritative for path validation (Section 9.1: "the backend validates and resolves them"), yet the testing strategy does not verify that the backend actually does this.

**Suggested fix:** Add a new subsection `10.6 Backend API Tests` that explicitly covers:

```markdown
### 10.6 Backend API Tests

Each `/api/setup/*` endpoint is tested independently with:

- **Input validation boundary tests:**
  - `scan-models` with empty `directories` array → 400
  - `scan-models` with non-existent directory → partial success (skip invalid, return valid)
  - `scan-models` with path traversal (`../../etc`) → 400 or sanitized
  - `scan-binary` with `extra_paths` containing null bytes → 400
  - `save-config` with path to read-only location → 403
  - `save-config` with path outside allowed directories → 403
  - `build` with `jobs` = 0, negative, or absurdly large → 400
  - `build` with `install_dir` that doesn't exist → 400 or auto-create
  - `recommend` with empty `models` array → 400
  - `recommend` with `hardware` missing required fields → 400
  - `check-path` with empty path → 400

- **Happy-path response contract tests:** Each endpoint returns the documented JSON shape.
- **Error response contract tests:** Each endpoint returns structured errors (not 500 with stack traces).
```

---

### [RULE 22 — Validate at Boundaries, Trust Nothing] — Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4 — Build llama.cpp)

**What's wrong:** The `POST /api/setup/build` endpoint accepts a `backend` string, an `install_dir` path, and a `jobs` integer. The design specifies that the backend runs `git clone`, `cmake`, and `cmake --build` with these parameters. There is no mention of validating `backend` against a known enum (`cuda | metal | vulkan | cpu`), no mention of sanitizing `install_dir` before using it as a filesystem target (the binary is copied there after build), and no mention of range-checking `jobs`. The testing strategy does not test any of these boundaries.

**Why it matters:** The build endpoint is the most dangerous endpoint in the wizard — it executes shell commands on the host. If `install_dir` is not validated, a crafted request could write the compiled binary to an arbitrary location. If `backend` is not validated against the known enum, the cmake flag construction could produce unexpected behavior. If `jobs` is not bounded, a value like `jobs: 999999` could fork-bomb the host.

**Suggested fix:** The design should specify (and tests should verify):

```
- `backend` must be one of: "cuda", "metal", "vulkan", "cpu". Any other value → 400.
- `install_dir` must be canonicalized and within an allowed set of directories. 
  Tests: relative path → rejected; path outside home dir → rejected; symlink to outside → rejected.
- `jobs` must be 1..=N where N is detected logical cores. 0 or negative → 400. > cores → clamped with warning.
```

---

### [RULE 24 — Test Interactions, Not Just Units] — Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 10 — Testing Strategy)

**What's wrong:** The testing strategy has a complete gap between frontend and backend. Section 10.1 tests frontend components. Section 10.2 tests frontend integration flows with mocked API responses. There are no backend integration tests, no tests that verify the real backend endpoints return the documented contract, and no tests that drive the full stack (frontend -> API -> backend logic). The wizard introduces seven new API endpoints but the only tests that exercise them are frontend tests with mocked responses — which by definition cannot catch contract drift between the real backend and the mocked responses.

**Why it matters:** This is exactly the failure mode Rule 24 warns about. If the frontend tests mock the API to return `{ "found": true, "path": "..." }` but the real backend returns `{ "found": true, "binary_path": "..." }` (note: `path` vs `binary_path`), the frontend tests pass, the backend tests (if they existed) would pass, and the integration breaks in production. The design defines the contract in Section 11, but nothing tests that the backend actually conforms to it. The project convention uses `mock-llama` for backend integration tests — the wizard backend logic (hardware detection, model scanning, build orchestration) is equally testable with mocked shell commands and filesystem fixtures.

**Suggested fix:** Add backend integration tests to `10.6`:

```markdown
- **Backend integration tests** (in `llamaforge` or `llamaforge-api` crate):
  - Hardware detection with mocked `nvidia-smi` / `system_profiler` output → verify response shape
  - Binary scan with a temp directory containing a mock binary → verify found/not-found responses
  - Model scan with a temp directory containing .gguf files → verify discovery and metadata extraction
  - Build orchestration with a mock git/cmake that streams predictable output → verify SSE events
  - Config recommendation with known hardware + model inputs → verify deterministic output
  - Save config to temp directory → verify TOML file written correctly
  - Save config to read-only directory → verify 403 response
```

---

### [RULE 25 — Test Recovery, Not Just Failure] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 10.2 — Integration Tests)

**What's wrong:** The integration tests describe testing build failure (error UI appears, retry works, skip works) and hardware detection failure (error banner with fallback). But they do not test recovery after failure. Specifically: after a build failure, does clicking "Retry" actually work — does the SSE connection re-establish, does the build log reset, does a subsequent success correctly transition the UI? After hardware detection fails and the user enters manual values, do subsequent steps (config recommendations) correctly use the manually-entered hardware data instead of the failed detection data? After an SSE connection loss and reconnect (Section 6.2 specifies 3 reconnect attempts with exponential backoff), does the build log correctly resume — or do log lines get duplicated or lost?

**Why it matters:** The wizard's error handling design is thoughtful — every error state has a recovery path (retry, skip to manual, reconnect). But the testing strategy only tests that the error state appears, not that the recovery path actually works end-to-end. Users will hit these paths regularly (build failures are common, GPU detection fails on many setups). If the recovery paths are untested, they will be the source of the most frustrating bugs — the kind where the user sees the error, tries the suggested fix, and lands in an even worse state.

**Suggested fix:**

```markdown
### 10.2 Integration Tests (additions)

- **Build failure → retry → success:** Mock SSE to emit failure, verify error UI, 
  simulate retry click, mock SSE to emit success on second attempt, verify transition 
  to binary-found state with correct path.
- **Build failure → skip → manual entry → next step:** Mock failure, skip to manual 
  entry, enter a path, advance to Models step, verify the manually-entered binary path 
  is used in the final config.
- **Hardware detection failure → manual entry → config recommendations:** Mock 500, 
  enter hardware manually, advance to Config step, verify recommendations use the 
  manually-entered GPU/RAM/CPU values (not null/default).
- **SSE reconnect during build:** Mock SSE disconnect after N log lines, verify reconnect 
  attempt, mock successful reconnect, verify log continues without duplication or gaps.
- **SSE reconnect exhaustion:** Mock 3 failed reconnects, verify "Connection lost" UI 
  with retry button, verify manual retry re-establishes the connection.
```

---

### [RULE 24 — Test Interactions, Not Just Units] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 10 — Testing Strategy)

**What's wrong:** The SSE streaming behavior for the build endpoint is not tested in the described test plan. Section 10.2 mentions "Mock build SSE to emit lines then an error event" but does not describe how the SSE stream is tested. There is no mention of testing: EventSource connection establishment, SSE event type parsing (`phase`, `progress`, `log`, `complete`, `failed`), reconnection behavior on disconnect, behavior when the server sends malformed SSE events, behavior when the SSE stream ends unexpectedly (no `complete` or `failed` event), or memory management for long build logs (the log container is 400px fixed-height, but what happens when 10,000 log lines accumulate in the `logLines` array?).

**Why it matters:** SSE is the most complex network interaction in the wizard and the most likely to have edge-case bugs. The frontend must correctly parse multiple event types, handle reconnection, throttle screen reader announcements (Section 8.4), and manage a potentially unbounded log buffer. Testing only "emit lines then error" covers one path through a complex state machine.

**Suggested fix:**

```markdown
### 10.2 Integration Tests (additions)

- **SSE event parsing:** Mock SSE stream emitting each event type (phase, progress, log, 
  complete, failed) and verify the UI updates correctly for each.
- **SSE unexpected disconnect:** Mock SSE stream that closes without a complete/failed 
  event. Verify reconnect behavior (exponential backoff, 3 attempts).
- **SSE malformed event:** Mock SSE stream with an event whose `data` is not valid JSON. 
  Verify graceful handling (log the error, continue listening for next event).
- **Long build log:** Mock SSE stream emitting 5000+ log lines. Verify the LogViewer 
  auto-scrolls, does not cause excessive memory growth, and remains responsive.
- **SSE during navigation away:** Start a build, navigate away from the build step, 
  verify EventSource is closed (cleanup in $effect). Navigate back, verify reconnect 
  or fresh status check.
```

---

### [RULE 22 — Validate at Boundaries, Trust Nothing] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 5.6 — Form Design)

**What's wrong:** The form validation specifies `gpu_layers: integer, -1 or >= 0` and `ctx_size: integer >= 0`, but does not specify upper bounds. A `gpu_layers` value of 999999 would be accepted by the described validation but would cause the server to fail at startup. Similarly, `ctx_size` of 2^31 would pass validation but is meaningless. The testing strategy does not include tests for these boundary values on the `ServerConfigForm` component — it mentions "validates input ranges" but does not specify what ranges.

**Why it matters:** The wizard's purpose is to produce a valid config that passes `llamaforge validate` on first try (Success Criteria, Section 1.3). If the form validation allows values that the downstream validator rejects, the wizard fails its primary success criterion. Worse, if the form validation is looser than the backend validation in `/api/setup/recommend`, the user can enter values that the recommendation engine cannot process.

**Suggested fix:** Specify concrete upper bounds in the validation rules and test them:

```markdown
- `gpu_layers`: integer, -1 (auto) or 0–512. Test: -2 → rejected, 513 → rejected, -1 → accepted.
- `ctx_size`: integer, 0 (auto) or 128–1048576 (1M). Test: 64 → rejected, 2000000 → rejected.
- `parallel_slots`: integer, 1–64. Test: 0 → rejected, 65 → rejected.
- `port`: integer, 1024–65535. Test: 1023 → rejected, 65536 → rejected, 8080 → accepted.
```

---

### [RULE 25 — Test Recovery, Not Just Failure] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 10 — Testing Strategy / Section 12 — Standalone Wizard Mode)

**What's wrong:** The standalone wizard mode (Section 12) has a specific lifecycle: ephemeral port bind, browser open, wait for save, then exit. The testing strategy does not include any tests for this mode. Specifically: what happens if the save endpoint fails and the user retries? Does the server stay alive? What happens if the user closes the browser tab without saving — does the server hang forever (it "waits for the save-config endpoint to be called")? What happens if two browser tabs are opened to the same wizard URL — is there a race condition on save? What if the user navigates away from the wizard and back — does the wizard state reset correctly?

**Why it matters:** The standalone mode is the primary first-run experience (`llamaforge init --interactive`). It is the first thing new users encounter. If save fails and the server exits anyway, the user has a broken first experience with no config file. If the server hangs after the browser closes, the user has a zombie process. These are recovery-path bugs that only surface in production use.

**Suggested fix:**

```markdown
### 10.6 Backend API Tests (additions)

- **Standalone mode save failure → retry:** Start wizard server, POST save with invalid 
  path → 403. Verify server stays alive. POST save with valid path → 200. Verify server 
  shuts down cleanly.
- **Standalone mode Ctrl+C:** Start wizard server, send SIGINT. Verify clean shutdown 
  with exit code 130. No zombie process.
- **Standalone mode browser close:** Start wizard server, verify it does NOT exit when 
  no HTTP requests arrive for 30 seconds (it should wait indefinitely for save).
- **Standalone mode concurrent tabs:** Two tabs open the wizard, both submit save. Verify 
  no race condition — first save wins, second gets 409 or success (both are acceptable).
```

---

### [RULE 23 — Fail Fast and Loud] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.1 — Hardware Detection / Section 10)

**What's wrong:** The hardware detection endpoint runs shell commands (`nvidia-smi`, `system_profiler`, `rocm-smi`, `vulkaninfo`) and parses their output. The design does not specify what happens when these commands produce unexpected output (new `nvidia-smi` format, localized output, partial output from a killed process). The testing strategy does not include tests for parsing edge cases. The error handling in Section 6.2 only covers the case where the endpoint returns a 500 or times out — not the case where the endpoint returns 200 with degraded/partial data (e.g., GPU detected but VRAM is 0 because parsing failed silently).

**Why it matters:** Shell command output parsing is one of the most brittle boundaries in the system. `nvidia-smi` has changed its output format before. `system_profiler` output varies between macOS versions. If the parser encounters unexpected output and silently returns `vram_mb: 0` instead of failing, the recommendation engine will produce a CPU-only config for a system with a 24 GB GPU. The user will wonder why their GPU-equipped machine is running CPU inference. This is exactly the "default-on-error masking the failure" anti-pattern from Rule 23.

**Suggested fix:**

```markdown
### 10.6 Backend API Tests (additions)

- **Hardware detection parser tests** (unit-level, in the Rust crate):
  - `nvidia-smi` output from CUDA 12.x format → correct GPU info
  - `nvidia-smi` output with unexpected extra fields → still parses correctly
  - `nvidia-smi` empty output (command ran but no GPUs) → empty gpus array, not error
  - `nvidia-smi` gibberish output → error response (not 200 with vram_mb: 0)
  - `system_profiler` output with Apple Silicon unified memory → correct vram_mb
  - `system_profiler` output with Intel Mac discrete GPU → correct vram_mb
  - Multi-GPU nvidia-smi output → all GPUs in array
  - rocm-smi output → correct AMD GPU info
  - No GPU commands available → 200 with empty gpus array + recommended_backend: "cpu"
```

---

### [RULE 24 — Test Interactions, Not Just Units] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 10 — Testing Strategy)

**What's wrong:** The testing strategy does not describe contract tests between the frontend's TypeScript types (Appendix: New TypeScript Types) and the backend's Rust API response types. The design carefully defines both the TypeScript interfaces and the JSON response shapes in Section 11, but there is no mechanism described to verify they stay in sync. If a backend developer adds a field to the Rust response struct, the TypeScript types will be stale until someone manually notices.

**Why it matters:** Contract drift between frontend and backend is a classic integration bug. The design has both sides of the contract documented in the same document, which helps during initial implementation. But over time, changes to the backend API will need to be reflected in the TypeScript types, and without a contract test (or generated types from a shared schema), drift is inevitable.

**Suggested fix:**

```markdown
### 10.7 Contract Tests

- JSON schema snapshots for each `/api/setup/*` endpoint response, generated from backend 
  test output and validated against the TypeScript type definitions. If the backend changes 
  a response shape, the snapshot test fails and forces a TypeScript type update.
- Alternatively, generate TypeScript types from the Rust types using a build-time codegen 
  step (e.g., `ts-rs` crate) so they cannot drift.
```

---

### [RULE 22 — Validate at Boundaries, Trust Nothing] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.3 — Model Scanner)

**What's wrong:** The model scanner accepts a `directories` array and performs recursive directory walks. The design does not specify limits on: the number of directories (can a request send 10,000 directories?), the depth of recursion (symlink cycles?), or the total scan time per request. The testing strategy does not include tests for these boundary conditions. The success criteria specify "< 5 seconds for < 10,000 files" but the API contract does not describe behavior at or above this limit.

**Why it matters:** Without a max-directories limit, a single API call could trigger 10,000 concurrent directory walks. Without symlink cycle detection, a recursive walk could loop forever. Without a scan timeout at the API level, a scan of `/` could tie up the server thread indefinitely. These are boundary conditions that should be validated at the API entry point.

**Suggested fix:**

```markdown
- `scan-models` with > 10 directories → 400 with message "Maximum 10 directories per scan"
- `scan-models` with directory containing symlink cycle → scan completes (cycle detected, skipped)
- `scan-models` with `/` as directory → 400 ("Cannot scan root directory") or timeout after 30s with partial results
- `scan-models` response always includes `scan_duration_ms` and `files_scanned` for observability
```

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 3     |
| MEDIUM   | 5     |
| LOW      | 2     |

## Commendations

1. **Thorough frontend component test plan.** The decision to test all five states (initial, loading, loaded, empty, error) per component is excellent and goes beyond what most wizard designs specify. This will catch a large class of UI bugs.

2. **Thoughtful error UX design.** Every error state in Section 6.2 has a concrete recovery action — retry, manual entry fallback, or partial results. This is a user-respecting design that avoids dead ends. The testing strategy just needs to verify these recovery paths work.

3. **Keyboard-only flow test.** Including a full wizard completion test using only keyboard events (Section 10.2 and 10.4) is commendable. Many wizard designs ignore keyboard navigation entirely.

4. **Performance budget with CI enforcement.** The 40 KB gzipped budget with `bundle-baseline.json` CI enforcement (Section 10.5) is a concrete, measurable, and automatically enforced standard. Well done.

5. **No optimistic updates.** The explicit decision to avoid optimistic updates for the save operation (Section 5.5) is the right call for an irreversible configuration write, and the rationale is clearly stated.

6. **SSE reconnect strategy.** The exponential backoff with 3 attempts and a visible "Connection lost" fallback (Section 6.2) is a solid pattern for the build streaming use case.
