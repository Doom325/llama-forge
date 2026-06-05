<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 704-709 RULE: 1 SEVERITY: HIGH -->
**Rule 1 -- Graceful Degradation & Recoverability**
The standalone wizard server (Section 12) "waits for the save-config endpoint to be called" with no idle timeout. If the user opens the browser and walks away, closes the tab, or the browser open fails silently, the process hangs indefinitely. Add an idle timeout (e.g., 30 minutes with no API calls), print periodic reminders to stdout, and exit cleanly on expiry. Accept `--timeout=<duration>` to override.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 736-738 RULE: 1 SEVERITY: HIGH -->
**Rule 1 -- Graceful Degradation & Recoverability**
Section 13 step 6: "On config save, the server hot-reloads the new config and transitions to 'running' mode" has no failure path. If hot-reload fails (binary path gone, model deleted, port grabbed by another process), the wizard has already shown success and the server is in an indeterminate state. The save-config response in first-run mode should include the reload result. If reload fails, the UI should stay in setup mode and show the error rather than navigating to a broken dashboard. Pre-validate the config (resolve paths, check port availability) before writing.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 352-354 RULE: 2 SEVERITY: HIGH -->
**Rule 2 -- Timeouts Everywhere**
Hardware detection is explicitly designed with no timeout: "Keep waiting (no timeout kill)." This violates the "no stuck spinners" promise from the same section. Shell commands like `nvidia-smi`, `vulkaninfo`, and `system_profiler` can hang indefinitely on misconfigured systems. Add a 30-second overall timeout with per-command timeouts (10-15 seconds each). On timeout, return partial results (what completed) with a "detection timed out" status distinct from "not detected." Add a "Cancel and enter manually" button to the frontend after 10 seconds.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 599-636 RULE: 1 SEVERITY: MEDIUM -->
**Rule 1 -- Graceful Degradation & Recoverability**
The build process clones llama.cpp into a temp directory but specifies no cleanup on failure. Open Question 2 considers cleanup for successful builds but ignores the failed/abandoned case. Repeated "Retry" clicks could accumulate multiple partial clone+build directories (hundreds of MB each). Specify: on build failure, clean up unless user retries (reuse existing clone to skip re-clone); on "Skip," clean up immediately; on SSE disconnect with no reconnect within a grace period, clean up.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 599-636 RULE: 2 SEVERITY: MEDIUM -->
**Rule 2 -- Timeouts Everywhere**
The build process (git clone + cmake + make) has no backend-enforced timeout. The frontend's 60-second SSE silence detection is a UX hint, not a timeout -- the backend process continues regardless, and a slow-but-not-silent build (one log line every 59 seconds) will never trigger it. Add per-phase backend timeouts: clone (5 min), configure (5 min), build (20 min), with an overall 30-minute cap. On timeout, SIGKILL the process group, emit an SSE "failed" event, and clean up.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 552-568 RULE: 2 SEVERITY: MEDIUM -->
**Rule 2 -- Timeouts Everywhere**
The binary scan runs `llama-server --version` on discovered binaries with no specified timeout. An older llama-server build without `--version` support may start a server instead of printing version info and exiting. A non-llama-server binary with the same name could do anything. Set a 5-second timeout on the version-check subprocess. If it times out, report the binary as "found but unresponsive" and let the user confirm manually.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 679-689 RULE: 1 SEVERITY: MEDIUM -->
**Rule 1 -- Graceful Degradation & Recoverability**
The save-config endpoint has no atomic write guarantee. If the process is killed or disk fills mid-write, the config file may be left truncated/corrupt. The next `llamaforge serve` would find a file that exists but isn't valid TOML -- and the design doesn't specify whether `ConfigLoadError::InvalidToml` triggers setup mode or an error exit. Write to a temp file first, validate, then atomic-rename. Also add `InvalidToml` to the first-run detection triggers in Section 13.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 356 RULE: 3 SEVERITY: MEDIUM -->
**Rule 3 -- Retry with Backoff, Jitter & Circuit Breakers**
SSE reconnect for the build stream specifies "exponential backoff (3 attempts)" but no jitter. While single-client for the wizard, this design doc serves as an implementation reference. If the reconnect pattern is reused for the main UI's SSE connections (multi-client), synchronized retry waves become a real concern. Change to "exponential backoff with jitter (base 1s, max 8s, 3 attempts)."

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 242-248 RULE: 1 SEVERITY: LOW -->
**Rule 1 -- Graceful Degradation & Recoverability**
The SSE cleanup on client disconnect is specified for the frontend (`$effect` cleanup closes EventSource) but the backend behavior is unspecified. When the SSE client disconnects mid-build, does the backend kill the build subprocess, continue it, or continue with a grace period? Also: if the user navigates back and clicks "Build" again while a previous build is still running server-side, the endpoint should either join the existing build's stream or reject with 409 -- not start a parallel build.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 679-689 RULE: 4 SEVERITY: LOW -->
**Rule 4 -- Idempotency as Default**
The save-config endpoint has frontend double-submit protection (disabled button) but no server-side guard. If the network is slow and the client retries, two concurrent writes could race. Add a server-side mutex on the write path: if a write is in progress, wait for it and return its result rather than starting a second concurrent write.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 574-595 RULE: 2 SEVERITY: LOW -->
**Rule 2 -- Timeouts Everywhere**
The model scan has a 30-second *frontend* timeout but no specified *backend* timeout. The frontend abort does not cancel the backend's recursive directory walk. On network-mounted filesystems (NFS, SMB), individual `stat()` calls can block for minutes. Add a backend-side scan timeout (30 seconds) that returns partial results with `scan_truncated: true`. Consider making the scan streaming (SSE) so results appear incrementally and the user can stop early.
