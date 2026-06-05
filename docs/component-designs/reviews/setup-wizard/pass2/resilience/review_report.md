# Resilience & Failure Mode Review (Pass 2) -- LlamaForge Setup Wizard

**Reviewer:** design-review-resilience
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` -- full document, with focus on Sections 6, 11, 12, 13 (network resilience, API contract, standalone mode, first-run detection and mode state machine). Second pass verifying resolution of 10 first-pass findings plus scanning for new issues introduced by the redesign.
**Verdict:** PASS

---

## First-Pass Finding Verification

### Finding 1 -- HIGH R1: Standalone wizard no idle timeout
**Status: RESOLVED**

Section 12.5 now specifies a complete idle timeout system:
- Default 30-minute timeout, overridable via `--timeout=<duration>` (Section 12.4 CLI interface).
- Idle timer resets on any `/api/setup/*` API call via middleware touching an `Arc<AtomicU64>` last-activity timestamp.
- Periodic 5-minute stdout reminders with the wizard URL.
- Clean shutdown on expiry with exit code 0 and an informative message.
- Timer check frequency: every 60 seconds (avoids per-second wakeups while keeping granularity within one minute).

This is thorough and well-specified. The `AtomicU64` + monotonic `Instant` approach is correct for avoiding spurious resets from clock skew. No remaining gaps.

---

### Finding 2 -- HIGH R1: First-run transition no failure path
**Status: RESOLVED**

Section 13.2 introduces an explicit `Mode` state machine (`Setup`, `Transitioning`, `Running`) with `Arc<ArcSwap<Mode>>`. The transition diagram and failure paths are fully specified:

- **Setup -> Transitioning**: Triggered only after save-config succeeds AND validation-launch passes (Section 13.5).
- **Transitioning -> Running**: On successful `config_handle.reload()`.
- **Transitioning -> Setup**: On hot-reload failure, with `reload_error` in the response.
- **No Running -> Setup transition**: Correct -- reconfigure stays in Running mode (Section 13.8).

Section 13.5 adds a four-step save-config flow with pre-validation (round-trip TOML serialization, path resolution, port availability check) and validation-launch (spawn a test server, wait for health check). Section 13.7 adds CompletionScreen verification polling (`GET /api/status` every 2s, up to 5 attempts) with explicit handling for the case where `setup_required` remains `true` after polling.

This is a comprehensive fix. The multi-step validation pipeline catches failures early (before writing), at write time, at validation-launch, and at hot-reload -- four defense layers.

---

### Finding 3 -- HIGH R2: Hardware detection no timeout
**Status: RESOLVED**

Section 6.2 (table row for hardware detection) and Section 11.1 now specify:
- 30-second overall hard timeout on the backend endpoint.
- Per-command sub-timeouts: `nvidia-smi` 10s, `rocm-smi` 10s, `vulkaninfo` 10s, `system_profiler` 15s, `/proc` reads 5s, `sysctl` 5s.
- Timed-out probes reported as "detection timed out" (distinct from "not detected") via a `warnings` field.
- Timed-out processes killed via `SIGKILL`.
- Frontend shows "Detection is taking longer than expected" after 10s with a "Cancel and enter manually" button.
- On overall timeout, partial results returned with a banner explaining the situation.

Matches the suggested fix exactly. Per-command timeouts are reiterated in the Implementation Notes appendix (Section 15) for implementer reference. No gaps.

---

### Finding 4 -- MEDIUM R1: Build failure leaves temp dirs uncleaned
**Status: RESOLVED**

Section 11.4 (Cleanup subsection) now provides a comprehensive cleanup specification:
- **On build failure:** Clean up temp directory UNLESS user clicks "Retry" (existing clone reused to skip re-clone).
- **On "Skip":** Clean up immediately.
- **On SSE disconnect:** Build continues for a 2-minute grace period. If no reconnect, kill the build and clean up.
- **On build success:** Temp directory cleaned after binary copied to `install_dir`.
- **Orphaned subprocess cleanup:** Build process spawned in its own process group (`setsid`/`setpgid`). On shutdown, `SIGTERM` -> 5s wait -> `SIGKILL` on the process group.
- **Temp directory path displayed in UI:** "Building in /tmp/llamaforge-build-xxxx" shown to user.

This covers all the failure/abandonment scenarios. The process-group cleanup for orphaned subprocesses is a good addition beyond what was suggested.

---

### Finding 5 -- MEDIUM R2: Build process no backend timeout
**Status: RESOLVED**

Section 11.4 (Timeouts subsection) now specifies:
- Overall build timeout: 30 minutes, configurable via `--build-timeout` CLI flag (minimum 5 minutes).
- Per-phase timeouts: clone 5 min, configure 5 min, build 20 min.
- On timeout: kill entire child process tree via `kill(-pgid, SIGKILL)`, send SSE `failed` event with phase-specific error message, clean up temp directory.

Matches the suggested fix. The configurable `--build-timeout` with a 5-minute minimum floor is a sensible addition.

---

### Finding 6 -- MEDIUM R2: Binary scan version check no timeout
**Status: RESOLVED**

Section 11.2 now specifies: "If found, runs `llama-server --version` (5-second timeout) to extract version and supported backends. A timed-out or hung binary is reported as `"found": true` with `"version": "found but unresponsive to --version"` so the user can confirm or reject it in the UI."

This matches the suggested fix exactly. The distinction between "found and responsive" vs. "found but unresponsive" is correctly preserved in the API response contract.

---

### Finding 7 -- MEDIUM R3: SSE backoff without jitter
**Status: RESOLVED**

Section 6.2 (Build SSE row) now reads: "SSE reconnect with exponential backoff and jitter (base 1s, max 8s, 3 attempts), then show 'Connection lost' with manual retry button."

Jitter is explicitly specified alongside the backoff parameters. No remaining gap.

---

### Finding 8 -- LOW R1: Backend behavior on SSE disconnect
**Status: RESOLVED**

Section 11.4 (Cleanup subsection) now specifies: "On SSE disconnect: If the SSE client disconnects mid-build, the build continues server-side for a 2-minute grace period. If the client reconnects within that window (via `GET /api/setup/build/{id}/stream`), it resumes from the current position in the log stream. If no reconnect occurs within the grace period, the build is killed and the temp directory is cleaned up."

Additionally, Section 11.4 specifies the job pattern with `POST /api/setup/build` being idempotent: "POST is idempotent: if a build is already running for the same backend, return the existing `build_id`." This prevents parallel builds. The `GET /api/setup/build/{id}/stream` endpoint with terminal event replay handles the reconnection case.

This is a thorough resolution covering grace period, reconnection, stream replay, and idempotency.

---

### Finding 9 -- LOW R4: Save-config no idempotency
**Status: RESOLVED**

Section 11.6 now specifies: "The save-config handler acquires a `tokio::sync::Mutex` before performing the backup + write sequence. If a second save-config request arrives while a write is already in progress, it waits for the in-progress write to complete and then returns the result of its own write (not the in-progress one's result, since the configs may differ)."

The mutex serializes concurrent writes. The design correctly notes that the second write returns its own result (not the first's), which is the right behavior since the configs may differ. No remaining gap.

---

### Finding 10 -- LOW R2: Model scan no backend timeout
**Status: RESOLVED**

Section 11.3 now specifies: "The backend enforces a 30-second scan timeout. If any limit is hit, the response returns the models found so far with `"truncated": true` and `truncation_reason` indicating which limit fired first." The `truncation_reason` field distinguishes between `"timeout"`, `"max_results"`, and `"max_depth"` -- allowing the frontend to show an appropriate message.

The response contract includes `truncated: boolean` and `truncation_reason` with a union type. This is well-specified. The streaming/incremental approach was not adopted (noted as a "consider" in the original suggestion), which is fine for v1 -- the 30-second timeout with partial results is sufficient.

---

## New Issues Scan

With the redesign introducing significant new machinery (the Mode state machine, the validation-launch step, the job pattern for builds, process group management, the shared setup router, and the CompletionScreen verification sequence), I examined each for new resilience concerns.

### [RULE 2 -- Timeouts Everywhere] -- Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 13.5, lines 1179-1183)

**What's wrong:** The validation-launch step in Section 13.5 spawns a `llama-server` process and waits for its health check with a 30-second timeout. This is good. However, after the health check succeeds or fails, the spec says "kill the validation server." There is no timeout specified for the kill itself. If the llama-server process ignores `SIGTERM` (which is rare but possible -- especially if it is in a CUDA driver call), the save-config handler could hang waiting for the child process to exit.

**Why it matters:** This is on the critical path of the wizard completion flow. A hang here blocks the save-config response and leaves the user staring at a loading spinner on the CompletionScreen. Given that the build process cleanup already specifies the SIGTERM -> 5s wait -> SIGKILL pattern (Section 11.4), the omission here is likely an oversight rather than an intentional design choice.

**Suggested fix:**
```
After the health check completes (success or failure), use the same
process cleanup pattern as the build process:
1. Send SIGTERM to the validation server process.
2. Wait up to 5 seconds for it to exit.
3. If still alive, send SIGKILL.
This mirrors the orphaned-subprocess cleanup pattern already specified
in Section 11.4 and keeps the behavior consistent.
```

---

### [RULE 1 -- Graceful Degradation & Recoverability] -- Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 13.5, lines 1155-1158)

**What's wrong:** The pre-validation step in Section 13.5 checks port availability by attempting a non-blocking `TcpListener::bind` and releasing immediately. There is a TOCTOU (time-of-check-to-time-of-use) gap between this check and the actual server startup during validation-launch (Step 3) or normal operation after hot-reload. Another process could bind the port in the interval.

**Why it matters:** This is inherent to port availability checks and acknowledged in most systems by treating the check as advisory. The validation-launch step (Step 3) will catch the conflict when the real `llama-server` fails to bind. However, the design does not explicitly note that the pre-validation port check is advisory and that the validation-launch is the authoritative check. Without this note, an implementer might skip the validation-launch assuming the pre-validation is sufficient.

**Suggested fix:**
```
No code change needed. Add a one-line note to the pre-validation
port check: "This check is advisory (TOCTOU gap is inherent);
the validation-launch step is the authoritative verification that
the server can bind its configured port."
```

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 0     |
| MEDIUM   | 0     |
| LOW      | 2     |

All 10 first-pass findings: **RESOLVED** (10/10).

2 new LOW-severity findings from the redesign -- neither is blocking.

## Commendations

1. **Comprehensive mode state machine (Section 13.2).** The `Setup -> Transitioning -> Running` state machine with explicit transitions, failure rollback (`Transitioning -> Setup`), and the decision to forbid `Running -> Setup` is exactly the right design. The use of `Arc<ArcSwap<Mode>>` for lock-free reads with atomic swaps is well-suited to a hot-path middleware check.

2. **Four-layer validation pipeline (Section 13.5).** Pre-validation (parse + paths + ports), write, validation-launch (spawn + health check), and hot-reload form a defense-in-depth chain. Each layer catches a different class of failure: pre-validation catches obviously wrong configs, validation-launch catches runtime issues (CUDA OOM, binary incompatibility), and hot-reload catches server-level integration issues. This is thorough without being excessive for a local tool.

3. **Build job pattern with idempotent POST and terminal event replay (Section 11.4).** The redesign from a single endpoint to a proper job pattern (POST creates, GET polls, GET /stream reconnects, DELETE cancels) with idempotent POST and terminal event replay on reconnect is a significant resilience upgrade. This handles every SSE lifecycle scenario: fresh connection, reconnection after transient disconnect, reconnection after build completion, and cancellation.

4. **Process group cleanup (Section 11.4).** Spawning build subprocesses in their own process group with `setsid`/`setpgid` and the SIGTERM -> 5s -> SIGKILL escalation pattern ensures that cmake/make child processes are cleaned up even when the parent build process is killed. This is the correct approach and prevents orphaned compiler processes.

5. **Orphan-resistant grace period design.** The 2-minute SSE disconnect grace period strikes a good balance between allowing transient reconnection and preventing indefinite resource consumption. Combined with the build job pattern (where the client can reconnect via `GET /build/{id}/stream`), this handles the common case of browser tab freezes or brief network interruptions without punishing the user.

6. **Atomic write with backup rotation (Section 11.6).** The backup-before-overwrite pattern with 5-backup retention and best-effort pruning addresses the original finding about partial write corruption risk. The write mutex serializes concurrent writes, and the backup provides a recovery path if the new config is problematic.
