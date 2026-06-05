<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 729-738 RULE: 26 SEVERITY: HIGH -->
**Rule 26 — Control the Blast Radius**
Setup mode boots the full transport layer, which means gRPC inference endpoints and `/v1/*` HTTP shims are live with zero backends. Any client connecting during setup gets confusing errors from the inference path rather than a clear "not ready" signal.

Additionally, the design triggers setup mode on `ConfigLoadError::NotFound` and `ConfigLoadError::EmptyFile` but does not address `ConfigLoadError::ParseError`. A corrupted config silently entering setup mode hides the corruption from the operator.

Suggested:
1. In setup mode, inference endpoints should return `503 Service Unavailable` with `{"error": "llamaforge is in setup mode", "setup_url": "..."}`.
2. Explicitly enumerate which `ConfigLoadError` variants trigger setup mode. Parse errors should hard-exit with an actionable error message, not silently enter setup mode.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 599-636 RULE: 26 SEVERITY: MEDIUM -->
**Rule 26 — Control the Blast Radius**
The build endpoint accepts a client-specified `jobs` parameter with no server-side cap. In first-run mode (inside `llamaforge serve`), the build competes with the server process for CPU. A build requesting 64 jobs on an 8-core machine will oversubscribe the system and degrade everything running on it.

Suggested:
1. Cap `jobs` to `min(requested, num_cpus / 2)` server-side.
2. Run the build subprocess with reduced priority (`nice`).
3. Warn users in first-run mode that builds are resource-intensive; recommend standalone wizard for the build step.
4. Pre-check available disk space (llama.cpp clone + build artifacts need several GB).

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 519-699 RULE: 28 SEVERITY: MEDIUM -->
**Rule 28 — Design for Evolution, Not Permanence**
Model downloading (NG1) and prebuilt binary downloads (NG2) are the two most likely v2 wizard features. The current API surface does not leave clean extension seams for them.

`POST /api/setup/scan-models` only accepts `directories` — adding a HuggingFace search will need a different request shape. `POST /api/setup/build` is build-from-source only — adding prebuilt downloads will need a different SSE event schema or a separate endpoint.

Suggested:
1. Add a `"source": "local"` discriminator to `scan-models` from day one (only `"local"` implemented, but the field exists for future `"huggingface"` etc.).
2. Define a separate `POST /api/setup/acquire-binary` with a `"method": "build" | "download"` discriminator, or document that `POST /api/setup/download-binary` will be the v2 endpoint.
3. Note in the design that `BinaryStep` and `ModelStep` should be structured as sub-flow selectors (build vs. download, local-scan vs. registry-search) so the v2 addition is a new branch, not a rewrite.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 574-595 RULE: 27 SEVERITY: MEDIUM -->
**Rule 27 — Pressure Test (10x)**
The model scanner is a one-shot `POST` that recursively walks directories. A user pointing it at `~/` (500K+ files) will block the wizard for minutes. The 30-second frontend timeout (Section 6.2) promises "partial results" but the API contract returns a single JSON response with no streaming or truncation mechanism.

Suggested:
1. Add `max_depth` (default 5), `max_results` (default 100), and `timeout_ms` (default 10000) parameters to the scan endpoint.
2. Add `truncated: boolean` and `truncation_reason: "timeout" | "max_results" | "max_depth"` fields to the response.
3. Consider making the scan cancelable (SSE stream or a `DELETE` endpoint to abort in-progress scans).

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 729-738 RULE: 28 SEVERITY: MEDIUM -->
**Rule 28 — Design for Evolution, Not Permanence**
The setup-to-running transition has no rollback path. If the saved config is valid TOML but operationally broken (e.g., `gpu_layers` too high, causing llama-server to OOM), the wizard shows "Ready!" and navigates to the dashboard — but the system is non-functional. The user must manually edit or delete the config to re-enter setup.

Suggested:
1. After saving, perform a **validation launch** — start the first server and wait for a health check (30s timeout). If it fails, keep the user in the wizard with the error and an option to adjust config.
2. Add `POST /api/setup/validate-launch` to attempt a trial spawn before committing the mode transition.
3. If the system boots with an existing config but all server health checks fail, show a "reconfigure" banner on the dashboard.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 703-725 RULE: 27 SEVERITY: LOW -->
**Rule 27 — Pressure Test (New-Hire)**
Two boot modes (standalone wizard vs. first-run within serve) with overlapping-but-different endpoint surfaces are a complexity hotspot. Can `/api/setup/build` be called on a running production instance? What happens?

Suggested:
1. Guard all `/api/setup/*` endpoints behind a mode check — return `404` in running mode (unless explicitly in reconfigure flow).
2. Extract a shared `setup_router()` function mounted identically in both modes to eliminate behavioral divergence.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 599-636 RULE: 26 SEVERITY: LOW -->
**Rule 26 — Control the Blast Radius**
The build endpoint spawns `git clone` and `cmake --build` as child processes. If the wizard is killed (Ctrl+C, browser close, crash), the design does not specify cleanup of orphaned build subprocesses. A 16-thread `cmake` build will continue running until it finishes or errors.

Suggested:
Spawn the build subprocess in a process group. On wizard shutdown or SSE disconnect, send SIGTERM to the process group, wait briefly, then SIGKILL. Document this in the build endpoint's implementation notes.

---
