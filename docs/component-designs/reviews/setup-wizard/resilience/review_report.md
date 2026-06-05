# Resilience & Failure Mode Review -- LlamaForge Setup Wizard

**Reviewer:** design-review-resilience
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` -- Sections 6, 11, 12, 13 (network resilience, API contract, standalone mode, first-run detection), plus error/failure handling throughout.
**Verdict:** PASS WITH WARNINGS

## Violations Found

### [RULE 1 -- Graceful Degradation & Recoverability] -- Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 12, lines 704-726)

**What's wrong:** The standalone wizard server lifecycle has no timeout or watchdog. Step 5 says "Wait for the save-config endpoint to be called. On success, shut down the server and exit 0." If the user opens the browser, walks away, or closes the tab without saving, the server process hangs indefinitely. There is no idle timeout, no heartbeat from the browser, and no mechanism to detect an abandoned session.

**Why it matters:** A `llamaforge init --interactive` invocation that the user forgets about becomes a zombie process holding an open port. On machines where port exhaustion or stale processes cause confusion (the target audience is non-expert users), this is a real usability and operational problem. The process will survive terminal close if backgrounded and has no self-cleanup path.

**Suggested fix:**
```
Standalone mode should:
1. Set a configurable idle timeout (e.g., 30 minutes with no API calls).
2. Periodically log a reminder to stdout: "Wizard still running at http://127.0.0.1:54321/ui/setup"
3. On idle timeout expiry, print "No activity detected. Shutting down." and exit 0.
4. Optionally accept --timeout=<duration> to override the default.
```

---

### [RULE 1 -- Graceful Degradation & Recoverability] -- Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 13, lines 729-738)

**What's wrong:** The first-run detection transition from "setup mode" to "running mode" has no recovery path if hot-reload fails. Section 13 step 6 states: "On config save, the server hot-reloads the new config and transitions to 'running' mode." But there is no specification of what happens if the hot-reload fails -- for example, if the newly written config references a binary path that doesn't exist, a model file that was deleted between scan and save, or a port that another process grabbed in the interim. The wizard has already shown the "success" screen, the user thinks setup is done, and the server is in an indeterminate state between "setup" and "running."

**Why it matters:** This is the single most critical transition in the entire wizard. The user just completed a multi-step guided setup and expects the system to be working. A silent failure here -- or worse, a partial transition where the UI switches to the dashboard but no llama-server processes are healthy -- destroys the zero-knowledge onboarding promise (G1). Recovery requires understanding what went wrong in a system the user just learned exists.

**Suggested fix:**
```
1. The save-config endpoint should validate the config can be loaded
   (parse + resolve paths + check port availability) BEFORE writing it.
2. After writing, the hot-reload response should be a two-phase commit:
   a. Write config to disk.
   b. Attempt hot-reload.
   c. If hot-reload succeeds, return 200 with { "mode": "running" }.
   d. If hot-reload fails, return 200 with { "mode": "setup", "reload_error": "..." }.
      Config is on disk (good -- user doesn't lose work) but the UI stays in
      setup mode and shows the error with actionable guidance.
3. The CompletionScreen should poll /api/status once after save and verify
   mode == "running" before showing the success message. If mode is still
   "setup", show "Config saved but server startup encountered an issue: [error]"
   with a "Retry" button that triggers hot-reload again.
```

---

### [RULE 1 -- Graceful Degradation & Recoverability] -- Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4, lines 599-636)

**What's wrong:** The build process (clone + cmake + make) has no cleanup path on failure. The design specifies cloning into a temp directory and building there, but does not specify what happens to the temp directory if the build fails, the SSE connection drops, or the user clicks "Skip." Open Question 2 (Section 14) acknowledges the cleanup question for *successful* builds but ignores the failed-build case entirely. Repeated "Retry" attempts could accumulate multiple partial clone/build directories.

**Why it matters:** Build artifacts for llama.cpp are large (hundreds of MB for the repo, potentially gigabytes for build artifacts with CUDA). On machines with constrained disk space -- which is common for the exact user profile this wizard targets (someone exploring local LLM inference) -- abandoned build directories are a real problem. The user may not know where the temp directory is or that it exists.

**Suggested fix:**
```
1. On build failure, the backend should clean up the build directory
   (rm -rf the temp dir) unless the user explicitly chooses "Retry"
   (in which case, reuse the existing clone if present -- skip re-clone).
2. On SSE disconnect without a "retry" or "skip" signal, clean up after
   a grace period (e.g., 5 minutes).
3. On "Skip," clean up immediately.
4. Document the temp directory location in the build UI
   (e.g., "Building in /tmp/llamaforge-build-xxxx").
```

---

### [RULE 2 -- Timeouts Everywhere] -- Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 6.2, lines 352-358)

**What's wrong:** Hardware detection has no timeout. The design explicitly says: "Keep waiting (no timeout kill -- the scan may need 10+ seconds on complex systems)." While the rationale (complex multi-GPU systems are slow) is valid, "keep waiting" with no upper bound is a missing-timeout violation. The scan runs shell commands (`nvidia-smi`, `system_profiler`, `vulkaninfo`); any of these can hang indefinitely -- `nvidia-smi` hangs when the NVIDIA kernel module is loaded but the GPU is in a bad state; `vulkaninfo` hangs on misconfigured Vulkan ICDs; `system_profiler` can hang waiting for Bluetooth/network hardware responses on macOS.

**Why it matters:** The wizard's first user-facing step after "Start Setup" is hardware detection. If it hangs, the user's very first experience is a stuck spinner with no way out except killing the process. This directly undermines G1 (zero-knowledge onboarding) and the "no stuck spinners" promise from Section 6.2.

**Suggested fix:**
```
1. Set a 30-second overall timeout on the hardware detection endpoint.
2. Internally, set per-command timeouts: nvidia-smi (10s), rocm-smi (10s),
   vulkaninfo (10s), system_profiler (15s), /proc reads (5s), sysctl (5s).
3. If a specific probe times out, report that subsystem as "detection timed out"
   (not "not detected" -- the user needs to know the difference) and continue
   with the probes that did complete.
4. If the overall timeout fires, return partial results with a warning:
   "Hardware detection timed out after 30 seconds. Showing partial results."
5. The frontend shows the "taking longer than expected" message after 10 seconds
   (as currently designed) but also shows a "Cancel and enter manually" button.
```

---

### [RULE 2 -- Timeouts Everywhere] -- Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4, lines 599-636)

**What's wrong:** The build process (git clone + cmake + make) has no overall timeout. The only timeout-adjacent behavior is the frontend's "If no SSE events for 60 seconds, show 'Build appears stalled' warning." But this is a frontend UX hint, not a backend enforcement. The backend process continues indefinitely. A cmake configure step that hangs (e.g., trying to detect CUDA on a misconfigured system, or a network fetch for a cmake dependency), or a make step that enters an infinite compile loop (rare but possible with pathological template instantiations), will run forever. The 60-second SSE silence detection also doesn't apply if the build is producing output slowly -- a build that emits one line every 59 seconds will never trigger the warning but may take hours.

**Why it matters:** The build step executes arbitrary external processes (git, cmake, make/ninja) that the wizard does not control. These processes can fail in ways that are not "failure" in the exit-code sense -- they can hang. Without a backend-enforced timeout, the wizard process becomes a zombie orchestrating a zombie build.

**Suggested fix:**
```
1. Backend overall build timeout: 30 minutes (configurable via
   --build-timeout). This covers clone + configure + compile.
2. Backend per-phase timeouts: clone (5 min), configure (5 min),
   build (20 min). If any phase exceeds its timeout, kill the
   child process tree (SIGKILL the process group), send an SSE
   "failed" event with error "Build timed out during [phase]",
   and clean up.
3. The frontend's 60-second SSE silence warning is a good UX
   addition but should not be the only timeout mechanism.
```

---

### [RULE 2 -- Timeouts Everywhere] -- Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.2, lines 552-568)

**What's wrong:** The binary scan endpoint runs `llama-server --version` on a discovered binary with no specified timeout. If the discovered file is a binary that hangs on `--version` (e.g., an older llama-server build that doesn't support the flag and starts a server instead, or a similarly-named binary that isn't llama-server at all), the scan will hang.

**Why it matters:** The binary scanner searches `$PATH` and common locations, then executes whatever it finds. Executing an unknown binary -- even with `--version` -- is inherently risky from a timeout perspective. The wizard targets users who may have partial or broken llama.cpp installations.

**Suggested fix:**
```
1. Set a 5-second timeout on the `llama-server --version` subprocess.
2. If it times out, report the binary as "found but unresponsive to --version"
   and let the user confirm or reject it.
3. If the process produces output but doesn't exit within 5 seconds,
   kill it (it may have started serving instead of printing version).
```

---

### [RULE 1 -- Graceful Degradation & Recoverability] -- Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.6, lines 679-689)

**What's wrong:** The save-config endpoint returns 409 Conflict if the file exists and `force=false`, and 403 for permission errors, but there is no handling of partial write failures. If the write starts but is interrupted (disk full mid-write, process killed, power loss), the config file may be left in a corrupt or truncated state. The next `llamaforge serve` invocation would find a file that exists but isn't valid TOML, and the behavior in that scenario is unspecified -- it's neither `NotFound` (which triggers setup mode) nor a valid config (which starts normally).

**Why it matters:** The user just completed the entire wizard flow. If the save produces a corrupt file, they have to either manually fix the TOML (defeating the wizard's purpose) or delete it and start over. For the target user (no llama.cpp expertise), a corrupt config file is a dead end.

**Suggested fix:**
```
1. Write to a temporary file in the same directory first (e.g., .llamaforge.toml.tmp).
2. Validate the written file can be parsed back as valid TOML.
3. Rename (atomic on POSIX) the temp file to the final path.
4. If rename fails, report the error and clean up the temp file.
5. Add ConfigLoadError::InvalidToml to the first-run detection logic in
   Section 13 so that a corrupt config also triggers setup mode rather
   than a cryptic parse error.
```

---

### [RULE 3 -- Retry with Backoff, Jitter & Circuit Breakers] -- Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 6.2, lines 356)

**What's wrong:** The SSE reconnect strategy for the build stream specifies "exponential backoff (3 attempts)" but does not mention jitter. While the wizard is a single-client system (one browser tab), the design document is also a specification for implementation. Specifying backoff without jitter teaches the wrong pattern and, if the reconnect logic is copied to other SSE consumers in the codebase (the main UI has SSE for live data), the missing jitter could matter in multi-client scenarios.

**Why it matters:** This is a low-impact finding for the wizard specifically (single client), but the design document serves as a reference for implementation patterns. Specifying jitter costs nothing and sets the right precedent.

**Suggested fix:**
```
Change "exponential backoff (3 attempts)" to:
"exponential backoff with jitter (base 1s, max 8s, 3 attempts max),
then show 'Connection lost' with a manual retry button."
```

---

### [RULE 1 -- Graceful Degradation & Recoverability] -- Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 5.4, lines 242-248)

**What's wrong:** The SSE connection for the build step has a cleanup mechanism (`$effect` cleanup closes EventSource), but there is no specification of what the *backend* does when the SSE client disconnects mid-build. The build process (git clone + cmake + make) is a subprocess tree. If the client disconnects (user navigates away, closes the tab, closes the browser), does the backend cancel the build? Or does it continue building to completion with no consumer for the output?

**Why it matters:** A disconnected build that continues wastes CPU and disk resources. If the user navigates back to the build step and clicks "Build" again, they could end up with two concurrent build processes. The frontend tracks build status via SSE, so a build that completed while the client was disconnected would need to be discoverable somehow.

**Suggested fix:**
```
Specify backend behavior on SSE client disconnect:
Option A (recommended): Continue the build for a grace period (e.g., 2 minutes).
  If the client reconnects within that window, resume streaming from where it
  left off (buffer recent log lines). If not, kill the build and clean up.
Option B: Kill the build immediately on disconnect. The user retries from scratch.
  Simpler but worse UX if the disconnect was transient (e.g., browser tab froze
  momentarily).

Either way, the build endpoint should be idempotent: if a build is already
in progress, a new POST /api/setup/build should either join the existing
stream or return a 409 with the current build status, not start a second build.
```

---

### [RULE 4 -- Idempotency as Default] -- Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.6, lines 679-689)

**What's wrong:** The save-config endpoint (`POST /api/setup/save-config`) has no idempotency guard beyond the `force` flag for existing files. If the user double-clicks "Save & Finish" or the network is slow and the frontend retries, two concurrent save requests could race. The design mentions "double-submit protection" on the frontend (Section 5.6, line 331: button disabled during request), but this is a UI-only guard -- it does not protect against network retries, browser extensions that replay requests, or a frontend bug.

**Why it matters:** For config file writes, the consequence of a double-submit is likely benign (writing the same content twice). But if the second request arrives while the first is mid-write, it could interfere with the atomic-write pattern. For a single-user localhost tool this is low severity, but the design should acknowledge the concurrency model.

**Suggested fix:**
```
Add a simple server-side guard: the save-config handler acquires a
mutex/lock before writing. If a write is already in progress, return
200 with the result of the in-progress write (wait for it) rather than
starting a second write. This is trivial to implement and makes the
endpoint safe against any double-submit scenario.
```

---

### [RULE 2 -- Timeouts Everywhere] -- Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.3, lines 574-595)

**What's wrong:** The model scan endpoint specifies a 30-second frontend timeout (Section 6.2) with partial results on timeout, but the *backend* timeout behavior is unspecified. The frontend timeout is a client-side abort -- the backend scan continues. For a recursive directory walk across network-mounted filesystems (NFS, SMB), the walk itself can hang on I/O. The backend should have its own timeout to prevent resource exhaustion.

**Why it matters:** Users who point the scanner at a network share or a very deep directory tree could cause the backend to spend minutes or hours walking the filesystem. The frontend would show "timed out, showing partial results" but the backend thread remains blocked.

**Suggested fix:**
```
1. Backend scan timeout: 30 seconds (matching the frontend expectation).
   On timeout, return whatever has been found so far with a
   "scan_truncated: true" field.
2. Consider a streaming approach: return results incrementally via SSE
   so the user sees models appearing in real time, with a "Stop Scan"
   button. This also solves the progress feedback problem.
```

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 3     |
| MEDIUM   | 4     |
| LOW      | 3     |

## Commendations

1. **Thorough error state coverage in the screen state table (Section 2.2).** Every screen has explicit Initial, Loading, Loaded, Empty, and Error columns. This is exactly the right way to think about UI failure modes -- most designs omit one or two of these states and the implementation discovers them in production.

2. **Fallback-to-manual pattern throughout.** Hardware detection, binary scanning, and model scanning all degrade to manual entry forms when automation fails. This is textbook graceful degradation (Rule 1) and is well-specified.

3. **"No blank screens, no stuck spinners" as an explicit design principle (Section 6.2).** Stating this as a principle rather than hoping it emerges from implementation is the right approach. The violations above are about making this principle fully enforceable by adding the backend-side timeout enforcement that the frontend-side UX assumes.

4. **SSE for build streaming (Section 11.4).** Using server-sent events instead of polling for build output is the correct architectural choice. The event schema (phase, progress, log, complete, failed) is well-structured and covers the important state transitions.

5. **Discriminated union for BinaryScanResult and BuildStatus types.** The TypeScript types in the appendix use proper tagged unions (`found: true | found: false`, `status: 'idle' | 'running' | 'complete' | 'failed'`) rather than nullable fields or boolean flags. This makes impossible states unrepresentable on the client side.

6. **Explicit "no optimistic updates" decision (Section 5.5).** For a sequential wizard with one mutation (save), acknowledging that optimistic updates are inappropriate and explaining why is better than silently omitting them. This shows the author considered and rejected the pattern rather than forgetting it.
