# Design Review — LlamaForge Setup Wizard

**Project:** LlamaForge Setup Wizard
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` (full design document, Sections 1–15 + appendices)
**Overall Verdict:** FAIL (9 HIGH findings)

---

## Executive Summary

The setup wizard design is ambitious and well-structured — the user flows are thoughtful, the error-handling UX is exemplary (every failure has a recovery path), and the component architecture follows established project patterns cleanly. However, the design has critical gaps in three areas: **backend observability is entirely absent** (zero mention of structured logging for seven new endpoints that execute shell commands and compile C++), **the first-run mode transition from setup to running has no failure path or state machine** (the most critical transition in the feature is under-specified), and **the testing strategy covers only the frontend** (no backend tests for endpoints that accept filesystem paths and execute subprocesses). These gaps must be addressed before implementation.

---

## Critical Findings (HIGH)

### 1. First-run mode transition is under-specified — R1 + R11 + R26

**Source:** Resilience, Data, Scaling (merged — three sub-agents flagged different aspects of the same problem)

**File:** Section 13 (First-Run Detection in Main UI)

The `"setup" → "running"` transition is described narratively but has no explicit state machine, no failure path, and leaks blast radius into the inference API.

**Three sub-problems:**

**(a) No failure recovery (Resilience R1).** If the hot-reload fails after config is saved (e.g., port conflict, bad binary path), the wizard shows "success" but the server is in an indeterminate state. The user just completed multi-step guided setup and thinks the system is working.

**(b) No state machine (Data R11).** The `mode` field has no enumerated transitions, no `Transitioning` intermediate state, and no definition of which `ConfigLoadError` variants trigger setup mode. `ParseError` (corrupted config) is unmentioned — does it silently enter setup mode, hiding corruption?

**(c) Inference endpoints live during setup (Scaling R26).** Full transport layer boots with zero backends, so existing clients get confusing errors instead of a clear 503.

**Fix:** Define an explicit `Mode::Setup → Mode::Transitioning → Mode::Running` state machine. Pre-validate config before writing. Return two-phase result from save. Return `503 Service Unavailable` from inference endpoints during setup. Only `NotFound` and `EmptyFile` trigger setup mode — `ParseError` is a hard exit.

---

### 2. No backend logging specified — R19

**Source:** Observability

**File:** Section 11 (Backend API Contract, all endpoints)

Seven new endpoints shell out to `nvidia-smi`, walk filesystems, run `git clone`, `cmake`, and `make` — with zero mention of `tracing`, `#[instrument]`, log levels, or any diagnostic output. When a user reports "GPU not detected but I have an RTX 4090," the developer has no data: no raw command output, no exit code, no PATH, no parse result.

**Fix:** Add a "Backend Observability" subsection. Every handler gets a `tracing::info_span!` with a session correlation ID. Log raw command outputs, exit codes, and parse results. Hardware detection at INFO/WARN; build at INFO with file-persisted logs; model scan with per-directory stats and permission error warnings.

---

### 3. Build logs not persisted to disk — R19

**Source:** Observability

**File:** Section 11.4 (Build llama.cpp)

Build output streams via SSE for real-time display but is ephemeral — if the browser tab closes or the user navigates away, the complete log (including cmake configuration output needed to diagnose most failures) is gone forever.

**Fix:** Tee all stdout/stderr to `$XDG_DATA_HOME/llamaforge/build-logs/build-<timestamp>.log`. Include `log_path` in the SSE `complete`/`failed` events. Show the path in the failure UI with "include this in bug reports."

---

### 4. Standalone wizard has no idle timeout — R1

**Source:** Resilience

**File:** Section 12 (Standalone Wizard Mode)

If the user opens the browser, walks away, or closes the tab without saving, the process hangs indefinitely. No idle timeout, no heartbeat, no self-cleanup. On non-expert users' machines, this becomes a zombie process.

**Fix:** 30-minute idle timeout (configurable via `--timeout`). Periodic stdout reminder. Clean exit on timeout.

---

### 5. Hardware detection has no timeout — R2

**Source:** Resilience

**File:** Section 6.2 / Section 11.1

The design explicitly says "keep waiting (no timeout kill)." But `nvidia-smi` hangs on bad GPU states, `vulkaninfo` hangs on misconfigured ICDs, `system_profiler` can hang waiting for hardware. The first user-facing step becomes a stuck spinner.

**Fix:** 30-second overall timeout with per-command sub-timeouts (10s each). Report timed-out subsystems as "detection timed out" (distinct from "not detected"). Return partial results.

---

### 6. `save-config` leaks `AppConfig` internal schema — R14

**Source:** Interfaces (also flagged by Data R10)

**File:** Section 11.6

The endpoint accepts the full `AppConfig` structure, coupling the frontend to every internal config field. The wizard only configures `[servers.*]` entries but must construct `priority_rules`, `routing`, `starvation`, etc. In the reconfigure flow, this risks overwriting hand-tuned sections.

**Fix:** Accept only the wizard's logical outputs (selected servers + per-model overrides). Backend assembles full `AppConfig` with defaults and merges into existing config. Add `POST /api/setup/preview-config` to return rendered TOML without writing.

---

### 7. No backend API tests — R22 + R24

**Source:** Testing

**File:** Section 10 (Testing Strategy)

The testing strategy contains zero backend tests. Seven endpoints accept filesystem paths, numeric parameters, and JSON payloads — but only frontend tests (with mocked responses) exist. No boundary validation tests, no response contract tests, no backend integration tests. The `POST /api/setup/build` endpoint's `backend`, `install_dir`, and `jobs` parameters have no validation specified at all.

**Fix:** Add Section 10.6 (Backend API Tests): input validation boundary tests for every endpoint (empty arrays, path traversal, null bytes, out-of-range numbers), happy-path response contract tests, error response tests. Backend integration tests with mocked shell commands and filesystem fixtures. Validate `backend` against known enum, sanitize `install_dir`, cap `jobs`.

---

### 8. Frontend-backend contract gap — R24

**Source:** Testing

**File:** Section 10

Frontend integration tests mock all API responses. Backend integration tests don't exist. Nothing verifies the real backend conforms to the contract the frontend mocks against. A field name mismatch (`path` vs `binary_path`) goes undetected until runtime.

**Fix:** JSON schema snapshot tests generated from backend test output and validated against TypeScript types. Or use `ts-rs` crate for build-time type generation.

---

### 9. Setup mode blast radius on inference API — R26

**Source:** Scaling

**File:** Section 13

First-run setup mode boots the full transport layer, leaving gRPC/HTTP inference endpoints live with zero backends. Existing clients get confusing errors.

**Fix:** (Addressed in Finding 1c above.) Inference endpoints return `503 {"error": "llamaforge is in setup mode", "setup_url": "..."}` during setup.

---

## Warnings (MEDIUM) — 18 findings

| # | Rule | Domain | Summary |
|---|------|--------|---------|
| 10 | R1 | Resilience | Build failure leaves temp directories uncleaned. Repeated retries accumulate GB of artifacts. |
| 11 | R2 | Resilience | Build process has no backend-enforced timeout. Frontend's 60s SSE silence check is UX, not enforcement. Per-phase timeouts needed (clone 5m, configure 5m, build 20m). |
| 12 | R2 | Resilience | Binary scan runs `llama-server --version` with no timeout. Discovered binary could hang. 5-second subprocess timeout needed. |
| 13 | R3 | Resilience | SSE reconnect specifies backoff without jitter. Sets wrong precedent for codebase. |
| 14 | R6 | Simplicity | Over-decomposed config form: 4 single-field components (`GpuLayersInput`, etc.) each wrap one `<input>`. Collapse into `ServerConfigForm`. |
| 15 | R6 | Simplicity | Dual-mode boot path complexity: standalone + first-run create two server assembly paths. Collapse into one boot path with `exit_on_save` flag. |
| 16 | R11 | Data | Wizard step progression is an implicit state machine with no transition table, no per-step completion predicates, and no invalidation rules for downstream steps on back-navigation. |
| 17 | R12 | Data | Config overwrite with `force=true` has no backup. Backup-before-overwrite to `<path>.backup.<timestamp>`. |
| 18 | R12 | Data | Build success overwrites existing binary with no backup or warning. |
| 19 | R13 | Data | All 7 API endpoints are unversioned. Add `"version": 1` field to responses. |
| 20 | R10 | Data | `SetupWizardStore` duplicates server-fetched data already in TanStack Query cache. Store should hold only client-owned state. |
| 21 | R16 | Interfaces | Build SSE endpoint merges initiation and streaming into one POST. Non-reconnectable, non-observable. Split into POST (create) + GET (snapshot) + GET /stream (SSE) + DELETE (cancel). |
| 22 | R14 | Interfaces | `recommend` endpoint requires client to echo back hardware data the backend produced. Backend should cache its own detection results. |
| 23 | R15 | Interfaces | Build SSE stream lacks behavioral guarantees: no event ordering, no monotonicity, no reconnection semantics, no rate limits. |
| 24 | R20 | Observability | No session correlation across wizard API calls. Add `X-Setup-Session-Id` header. |
| 25 | R19 | Observability | Model scan silently skips permission errors and symlink issues. Add `warnings` field to response. |
| 26 | R26 | Scaling | Build endpoint accepts client-specified `jobs` with no cap. Cap to `num_cpus / 2`, set `nice` on subprocess. |
| 27 | R28 | Scaling | No evolution seams for model downloading (NG1) or prebuilt binary downloads (NG2). Add `source` discriminator to scan-models, separate `acquire-binary` endpoint. |

---

## Suggestions (LOW) — 12 findings

| # | Rule | Domain | Summary |
|---|------|--------|---------|
| 28 | R1 | Resilience | Backend behavior unspecified on SSE client disconnect during build. Could orphan builds or cause parallel builds. |
| 29 | R4 | Resilience | Save-config has no server-side idempotency guard. Add mutex to prevent concurrent writes. |
| 30 | R2 | Resilience | Model scan has frontend timeout but no backend timeout. Backend walk continues after client abort. |
| 31 | R6 | Simplicity | `check-path` endpoint duplicates validation in `save-config`'s 403. Drop for v1. |
| 32 | R8 | Simplicity | Prefetching next-step data is premature for a run-once wizard. Remove for v1. |
| 33 | R9 | Simplicity | Build time estimation from log heuristics is unreliable. Show factual progress only. |
| 34 | R7 | Simplicity | 3 of 5 `BinaryStep` children are trivial single-state views. Inline as `{#if}` blocks. |
| 35 | R10 | Data | Save-config accepts full `AppConfig` coupling wizard to sections it doesn't expose. (Subsumed by HIGH finding #6.) |
| 36 | R14 | Interfaces | Hardware response exposes unnecessary `driver_version`, `arch`. Narrow the response. |
| 37 | R14 | Interfaces | `mode` field on `/api/status` is wizard-specific. Use `"setup_required": boolean` instead. |
| 38 | R17 | Interfaces | Implementation details (`nvidia-smi`, `system_profiler`) in API contract section. Separate into appendix. |
| 39 | R19 | Observability | Standalone mode logging not specified (tracing subscriber, default level, RUST_LOG support). |

---

## Commendations

1. **Exemplary error-handling UX.** Every screen has five explicit states (initial, loading, loaded, empty, error). Every failure has a recovery action (retry, skip to manual, partial results). The "no blank screens, no stuck spinners" principle is stated and (mostly) enforced.

2. **Strong component contracts.** Section 4.2 explicitly lists what each component does NOT know — negative contracts are harder to write and far more valuable than positive ones.

3. **Discriminated union types throughout.** `BinaryScanResult`, `BuildStatus`, `StreamingState` use proper tagged unions, making impossible states unrepresentable.

4. **Right-sized architecture decisions.** Shell commands over Rust FFI for GPU detection, ephemeral port for standalone mode, SvelteKit routes over a separate app, no optimistic updates for an irreversible save — every "Alternatives Considered" entry shows mature trade-off analysis.

5. **`staleTime: Infinity` for wizard queries.** Correctly identifies that one-shot scan results don't go stale within a session.

6. **Clean intelligence distribution.** Steps are orchestrators that own API calls; children are pure presentational. Matches the project's established pattern.

7. **Comprehensive "Alternatives Considered" section.** Prevents future re-litigation of settled decisions.

---

## Review Coverage

| Domain | Sub-agent | Violations | Status |
|--------|-----------|------------|--------|
| Resilience | design-review-resilience | 10 (3H, 4M, 3L) | Reviewed |
| Simplicity | design-review-simplicity | 6 (0H, 2M, 4L) | Reviewed |
| Data & State | design-review-data | 8 (1H, 5M, 2L) | Reviewed |
| Interfaces | design-review-interfaces | 9 (1H, 5M, 3L) | Reviewed |
| Observability | design-review-observability | 6 (2H, 3M, 1L) | Reviewed |
| Testing | design-review-testing | 10 (3H, 5M, 2L) | Reviewed |
| Scaling & Evolution | design-review-scaling | 7 (1H, 4M, 2L) | Reviewed |

## Summary (after deduplication)

| Severity | Count |
|----------|-------|
| HIGH     | 9     |
| MEDIUM   | 18    |
| LOW      | 12    |
| **Total**| **39** |

---

## Top 5 Actionable Changes (ordered by impact)

These five changes address the majority of HIGH findings and several MEDIUMs:

1. **Add "Backend Observability" section (§11.8).** Session correlation ID, structured logging for all handlers, build log persistence to disk. Fixes findings #2, #3, #24, #25, #39.

2. **Define first-run mode as explicit state machine.** `Mode::Setup → Transitioning → Running` with guarded transitions, 503 on inference endpoints during setup, explicit `ConfigLoadError` variant handling. Fixes findings #1, #9.

3. **Add "Backend API Tests" section (§10.6).** Boundary validation, response contract, backend integration tests with mocked shell commands. JSON schema snapshot tests for frontend-backend contract. Fixes findings #7, #8.

4. **Narrow the save-config endpoint.** Accept logical wizard outputs, not `AppConfig`. Backend assembles and merges config. Add `preview-config` endpoint. Fixes finding #6 and Data findings #20, #35.

5. **Add timeouts to all backend operations.** Hardware detection 30s, binary version check 5s, build per-phase timeouts, model scan backend timeout. Fixes findings #4, #5, #11, #12, #30.
