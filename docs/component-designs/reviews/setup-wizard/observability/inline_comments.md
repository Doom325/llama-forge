<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 519-700 RULE: 19 SEVERITY: HIGH -->
**Rule 19 — Structured Logging with Context**
Section 11 (Backend API Contract) defines seven new endpoints that execute shell commands, walk filesystems, and run multi-minute C++ builds — but specifies zero logging for any of them. There is no mention of `tracing`, `#[instrument]`, structured log fields, or log levels anywhere in the backend contract.

These endpoints are the highest-failure-surface code in the wizard. Without structured logging, a bug report of "hardware detection says Not detected but I have an RTX 4090" gives the developer nothing to work with — no command executed, no raw output captured, no exit code, no PATH searched.

Suggested: Add a Section 11.8 "Backend Observability" specifying:
- A `setup_session_id` (UUID) generated per wizard session, threaded through all requests as a correlation ID
- Every handler gets a `tracing::info_span!` at the adapter boundary with the session ID
- Hardware detection: log exact commands, raw stdout/stderr, exit codes, parse results
- Binary scan: log paths searched, existence checks, version command output
- Model scan: log directories entered, permission errors, symlinks skipped, per-directory file counts
- Build: log full command lines, exit codes, and persist complete output to a log file on disk
- Recommendation: log input hardware summary, per-model VRAM calculation steps, final values
- Save config: log resolved path, overwrite status, any I/O errors with full error chains

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 599-636 RULE: 19 SEVERITY: HIGH -->
**Rule 19 — Structured Logging with Context**
The build endpoint (Section 11.4) streams stdout/stderr to SSE for real-time display, but the design does not specify persisting the build log to a file on disk. The SSE stream is ephemeral — if the browser tab closes, the connection drops, or the user navigates away, the build log is gone. Build failures from C++ compilation with CUDA/Metal/Vulkan backends are the wizard's highest-friction failure mode and often require the complete cmake configuration output (which scrolled off screen) plus the actual error to diagnose.

Suggested: Specify that the build handler tees all subprocess stdout/stderr to `$XDG_DATA_HOME/llamaforge/build-logs/build-<timestamp>.log`. Include a `log_path` field in both the `complete` and `failed` SSE events. The UI failure screen should display this path and suggest including it in bug reports. Retain logs for 30 days or the last 5 builds.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 524-549 RULE: 19 SEVERITY: MEDIUM -->
**Rule 19 — Structured Logging with Context**
The hardware detection endpoint (Section 11.1) returns a clean, parsed response, but the design does not specify what happens to the raw output of `nvidia-smi`, `system_profiler`, `rocm-smi`, `vulkaninfo`, `/proc/cpuinfo`, etc. When the parsed result is wrong (VRAM reported as 0, wrong backend recommended), the developer needs the raw command output to determine if parsing or the command itself is at fault.

Suggested: (1) Log raw command outputs at DEBUG level via tracing. (2) Add an optional `diagnostics` field to the response (populated when `?diagnostics=true` is passed) containing raw command outputs. (3) The UI hardware screen includes a "Copy diagnostic info" button that fetches with diagnostics=true and copies to clipboard for bug reports.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 519-523 RULE: 20 SEVERITY: MEDIUM -->
**Rule 20 — Distributed Tracing / Session Correlation**
The seven wizard API endpoints are defined independently with no shared identifier linking them. From the server's perspective, the hardware detection call, binary scan, model scan, recommendation request, and config save are all unrelated HTTP requests. Correlating them in logs requires timestamp matching, which is fragile.

Suggested: Define a `X-Setup-Session-Id` header. The frontend generates a UUID when the wizard starts (in SetupLayout initialization) and attaches it to every API request. Every backend handler includes this ID as a structured field in its tracing span. This enables reconstructing the full setup flow from server logs by session ID.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 574-596 RULE: 19 SEVERITY: MEDIUM -->
**Rule 19 — Structured Logging with Context**
The model scan endpoint (Section 11.3) returns `scan_duration_ms` and `files_scanned` but the design does not specify logging for the scan walk itself. The implementation note says "recursive directory walk (respecting symlinks, skipping hidden directories)" but does not mention logging permission errors, skipped directories, or symlink resolution decisions. When a user reports "my models aren't found," there is no way to tell if the scanner didn't look, looked and found nothing, or hit a silent permission error.

Suggested: Log each top-level directory entered (INFO), permission errors during walk (WARN with path and error), symlink decisions (DEBUG), hidden directories skipped (DEBUG), and per-directory file counts at completion (INFO). Add a `warnings` field to the API response for user-visible issues like permission denied errors so the UI can surface them.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 702-726 RULE: 19 SEVERITY: LOW -->
**Rule 19 — Structured Logging with Context**
Section 12 (Standalone Wizard Mode) specifies printing the URL to stdout but is silent on logging configuration. There is no mention of whether a tracing subscriber is configured, what the default log level is, or how a user enables verbose logging. If no subscriber is configured, all structured logging from endpoints is silently discarded.

Suggested: Specify that standalone mode configures a `tracing_subscriber` with `RUST_LOG` support, defaulting to WARN level on stderr. The startup message should include: `"For verbose output, run with RUST_LOG=debug"`. Build subprocess output should always be persisted to a log file regardless of the configured log level.
