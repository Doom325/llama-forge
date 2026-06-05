# Observability & Operational Visibility Review — LlamaForge Setup Wizard

**Reviewer:** design-review-observability
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` (full design document, Sections 1-15 + appendices)
**Verdict:** FAIL

---

## Context Note

The setup wizard is a one-time (or rare) setup flow, not a long-running service. It does not need production-grade dashboards, SLO alerting, or distributed tracing in the traditional sense. However, it executes high-failure-rate operations (hardware detection via shell commands, filesystem scanning, and compiling C++ from source) on diverse user machines. When these fail, the user files a bug report. The question is: can the developer diagnose the failure from what was captured? That is the observability standard for a setup tool.

Rules 18 (Metrics/Percentiles) and 21 (Actionable Alerts) are not applicable to a setup wizard and are not evaluated. Rule 20 (Distributed Tracing) is evaluated in a reduced form: correlation of operations within a single setup session. The review focuses primarily on Rule 19 (Structured Logging with Context) and on whether the design captures enough diagnostic information to make failures debuggable without reproducing them.

---

## Violations Found

### [RULE 19 — Structured Logging with Context] — Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11 — Backend API Contract, entire section)

**What's wrong:** The design specifies seven new backend endpoints (`/api/setup/hardware`, `/api/setup/scan-binary`, `/api/setup/scan-models`, `/api/setup/build`, `/api/setup/recommend`, `/api/setup/save-config`, `/api/setup/check-path`) but never mentions structured logging, tracing spans, or any form of server-side diagnostic output for any of them. There is no mention of `tracing`, `#[instrument]`, log levels, or any logging strategy for the backend handlers.

**Why it matters:** These endpoints shell out to `nvidia-smi`, `system_profiler`, `rocm-smi`, `vulkaninfo`, walk the filesystem recursively, run `git clone`, run `cmake`, and run `make`. Every one of these operations has a high variance failure surface across user environments (missing drivers, broken PATH, permission issues, filesystem quirks, compiler version mismatches). Without structured logging on the backend:

- When a user reports "Hardware detection says 'Not detected' but I have an RTX 4090," the developer has zero diagnostic data. What command was run? What was the raw stdout/stderr? What exit code? What PATH was searched? What OS version?
- When a model scan times out, there is no record of which directory was being scanned, how many files were visited, where the walk stalled, or whether a symlink loop was detected.
- When the recommendation engine gives a bad suggestion, there is no trace of the inputs (hardware info, model size, VRAM calculation steps) that produced it.

**Suggested fix:** Add a "Backend Observability" subsection to Section 11 (or a new Section 11.8) specifying:

1. Every `/api/setup/*` handler gets a `tracing::info_span!` at the adapter boundary with a `setup_session_id` (a UUID generated when the wizard starts, returned in the first API call, and passed as a header on subsequent calls). This is the correlation ID for the entire setup session.
2. Hardware detection logs: the exact commands executed, their raw stdout/stderr (truncated to a reasonable size), their exit codes, and the parse result. Log level: `INFO` for successful detection, `WARN` for partial detection (e.g., GPU not found), `ERROR` for command execution failures.
3. Binary scan logs: the list of paths searched, whether each path existed, the result of `llama-server --version` (raw output + parsed version), and any errors encountered.
4. Model scan logs: directories entered, file count per directory, any permission errors or symlink issues skipped, total scan duration.
5. Build process logs: the full command lines executed (`git clone ...`, `cmake ...`, `cmake --build ...`), their exit codes, and — critically — the complete stderr/stdout captured to a file on disk (not just streamed to SSE). See the next finding for detail.
6. Config recommendation logs: input hardware summary, per-model VRAM calculation steps, final recommended values and the rationale that produced them.
7. Save config logs: the resolved path, whether a file was overwritten, any permission or I/O errors with full error chains.

All logging should use the project's existing `tracing` infrastructure with structured key-value fields (not string interpolation).

---

### [RULE 19 — Structured Logging with Context] — Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4 — Build llama.cpp)

**What's wrong:** The build process streams stdout/stderr to the SSE connection for real-time display, but the design does not specify that build output is also captured to a persistent log file on disk. If the SSE connection drops, the browser tab is closed, or the user does not copy the error output before navigating away, the build log is gone forever.

**Why it matters:** Build failures are the single highest-friction failure mode in the wizard. A C++ build of llama.cpp with CUDA/Metal/Vulkan backends can fail for dozens of environment-specific reasons (wrong CUDA toolkit version, missing headers, incompatible compiler, linker errors). The streaming log displayed in the UI is ephemeral — it exists only in the browser's memory during the SSE connection. If the user reports "the build failed" and provides a screenshot of the last few lines, that is often insufficient to diagnose the root cause. The developer needs the complete build log including cmake's configuration output (which scrolled off the screen long before the link error appeared).

**Suggested fix:** The design should specify that:

1. The build handler writes all stdout and stderr from every subprocess (git, cmake configure, cmake build) to a timestamped log file: `$XDG_DATA_HOME/llamaforge/build-logs/build-<timestamp>.log`.
2. The SSE `complete` and `failed` events include a `log_path` field pointing to this file.
3. The UI's build failure screen shows the log file path and suggests including it in bug reports.
4. Build logs are retained for at least 30 days or the last 5 builds (whichever is more), with older logs cleaned up automatically.

This is cheap to implement (tee stdout/stderr to both SSE and a file) and transforms build failures from "screenshot the last 10 lines" to "attach the complete log."

---

### [RULE 19 — Structured Logging with Context] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.1 — Hardware Detection)

**What's wrong:** The hardware detection endpoint response includes structured data (GPU name, VRAM, driver version, etc.) but the design does not specify what happens to the raw output of the detection commands. The response is a parsed, cleaned-up summary. The raw `nvidia-smi` output, the raw `system_profiler` output, and the raw `/proc/cpuinfo` contents are not surfaced or preserved.

**Why it matters:** When the parsed result is wrong (e.g., VRAM reported as 0, GPU name garbled, wrong backend recommended), the developer needs the raw command output to determine whether the parsing logic is wrong or the command itself returned unexpected data. Without the raw output, the developer must ask the user to manually run `nvidia-smi --query-gpu=...` and paste the result — adding a round-trip to every hardware detection bug report.

**Suggested fix:** Two complementary approaches:

1. The hardware detection handler logs the raw command outputs at `DEBUG` level via `tracing`. In production this is off by default but can be enabled by setting `RUST_LOG=llamaforge_api::setup::hardware=debug` for diagnostic runs.
2. The API response includes an optional `diagnostics` field (populated when a query parameter like `?diagnostics=true` is passed) containing the raw command outputs. The UI does not display this by default but includes it in a "Copy diagnostic info" button on the hardware screen. This gives users a one-click way to attach hardware detection details to bug reports.

---

### [RULE 20 — Distributed Tracing / Session Correlation] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11 — Backend API Contract, all endpoints)

**What's wrong:** There is no session correlation mechanism across the wizard's API calls. Each endpoint is defined independently with no shared identifier linking the hardware detection call to the binary scan call to the model scan call to the config save call. From the server's perspective, these are unrelated HTTP requests.

**Why it matters:** When debugging a reported issue ("the wizard recommended wrong settings"), the developer needs to reconstruct the full sequence: what hardware was detected, what binary was found, what models were scanned, what recommendations were generated, and what was actually saved. Without a session ID threading through all calls, correlating log entries across these endpoints requires matching by timestamp proximity — which is fragile and breaks if there is any log interleaving from concurrent requests (possible if multiple browser tabs run the wizard, or in Flow 4 where the main UI is also serving requests).

**Suggested fix:** Define a `X-Setup-Session-Id` header (or a `session_id` field in POST bodies). The session ID is a UUID generated by the frontend when the wizard starts (in `SetupLayout`'s initialization) and attached to every API request. Every backend handler includes this ID as a field in its tracing span. This enables `grep`-ing server logs by session ID to reconstruct the full setup flow. Add this to Section 11 as a cross-cutting concern before the individual endpoint definitions.

---

### [RULE 19 — Structured Logging with Context] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.3 — Model Scanner)

**What's wrong:** The model scan endpoint response includes `scan_duration_ms` and `files_scanned`, which is good summary data. But the design does not specify logging for the scan process itself — particularly for the error cases that are silently skipped. The "Backend implementation" note says "recursive directory walk (respecting symlinks, skipping hidden directories)" but does not mention logging skipped directories, permission errors, or symlink resolution failures.

**Why it matters:** Model scan problems are subtle. A user says "I know I have models in `/data/models` but the wizard doesn't find them." The models might be behind a permission boundary, in a hidden directory (`.cache/huggingface/`), behind a symlink that the scanner does not follow, or have a non-standard extension. Without logging what was skipped and why, the developer cannot distinguish "the scanner didn't look there" from "the scanner looked and found nothing" from "the scanner hit a permission error and silently continued."

**Suggested fix:** The model scan handler should log:

- Each top-level directory entered (INFO level)
- Permission errors encountered during the walk (WARN level, with the path and error)
- Symlink resolution decisions (DEBUG level — followed vs. skipped, with reason)
- Hidden directories skipped (DEBUG level)
- Total files examined vs. `.gguf` files found per directory (INFO level at scan completion)

The API response should include a `warnings` field for user-visible issues (e.g., `"Permission denied: /data/models/private/"`) so the UI can surface them rather than silently showing an incomplete result.

---

### [RULE 19 — Structured Logging with Context] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 12 — Standalone Wizard Mode)

**What's wrong:** The standalone wizard mode prints the URL to stdout and then... the design is silent on what else goes to stdout/stderr during the wizard's operation. There is no mention of log output configuration for the standalone mode — whether `tracing` output goes to stderr, to a file, what the default log level is, or how a user would enable verbose logging for diagnostic purposes.

**Why it matters:** In standalone mode (`llamaforge init --interactive`), the terminal is the only place server-side logs can go (there is no persistent daemon, no log file configuration). If the default is silent (no tracing subscriber configured), all the structured logging from the endpoints is lost. If the default is verbose, the terminal fills with noise while the user is trying to use the wizard in the browser. The design needs to specify the logging posture for this mode.

**Suggested fix:** Specify that standalone wizard mode:

1. Configures a `tracing_subscriber` with `RUST_LOG` support (consistent with the main `llamaforge serve` mode).
2. Default log level is `WARN` to stderr (quiet terminal — the user is in the browser).
3. The startup message includes a hint: `"For verbose output, run with RUST_LOG=debug"`.
4. Build subprocess output is always written to a log file regardless of the log level (per the build log finding above).

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 2     |
| MEDIUM   | 3     |
| LOW      | 1     |

---

## Commendations

1. **Build log streaming via SSE is well-designed.** The phase/progress/log event structure (Section 11.4) gives real-time visibility into the build process. The build failure response includes `last_lines`, which shows awareness that users need context when things go wrong. The gap is persistence, not real-time visibility.

2. **Error states are thoroughly specified for the frontend.** Section 6.2 (Error Handling Strategy) defines specific behavior for every API call's timeout, error, and network failure cases. The "No blank screens. No stuck spinners." principle is excellent. The gap is that corresponding backend logging is not specified to match.

3. **Model scan response includes `scan_duration_ms` and `files_scanned`.** These are proto-metrics — they give the developer (and the user) a sense of whether the scan was thorough. This is a good instinct; extending it with per-directory stats and a warnings field would complete the picture.

4. **Hardware detection fallback to manual entry is well-thought-out.** The design acknowledges that detection can fail and provides a clean fallback path (Section 2.1, unhappy path). The missing piece is capturing why detection failed so the developer can improve it.
