# Scaling, Blast Radius & Evolution Review — LlamaForge Setup Wizard

**Reviewer:** design-review-scaling
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` — Sections 1-15, all appendices. Focus on dual-mode architecture (standalone + first-run), API surface evolution, and setup-to-running transition.
**Verdict:** PASS WITH WARNINGS

## Violations Found

### [RULE 26 — Control the Blast Radius] — Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 13, lines 729-738)

**What's wrong:** The first-run "setup mode" boots the **full transport layer** (HTTP + gRPC) with an empty config, meaning the gRPC inference endpoints (`/v1/chat/completions`, streaming RPCs) are live and accepting connections — but there are zero llama-server backends to serve them. Any client that connects during setup mode (a cron job, an existing integration, a script left running) will get confusing errors from the inference path rather than a clear "server is not ready" rejection.

**Why it matters:** The blast radius of setup mode leaks beyond the wizard into the inference API surface. A user who runs `llamaforge serve` expecting it to resume service but hits a config issue (corrupted file, permission error) will silently enter setup mode instead, leaving their inference clients broken with no clear signal. The design specifies `ConfigLoadError::NotFound` and `ConfigLoadError::EmptyFile` as triggers, but `ConfigLoadError::ParseError` (a corrupted config) is unmentioned — is that a hard exit or also setup mode? The ambiguity increases blast radius.

**Suggested fix:**

In setup mode, the transport layer should boot but the inference endpoints (gRPC completion RPCs and `/v1/*` HTTP shims) should return a well-defined `503 Service Unavailable` with a body like `{"error": "llamaforge is in setup mode", "setup_url": "http://127.0.0.1:8000/ui/setup"}` rather than routing requests to a nonexistent server pool. Additionally, explicitly enumerate which `ConfigLoadError` variants trigger setup mode and which trigger a hard exit with an error message. Parse errors should **not** silently enter setup mode — that hides corruption.

```rust
// In the transport layer's request handler during setup mode:
if app_state.mode() == AppMode::Setup {
    return StatusCode::SERVICE_UNAVAILABLE.into_response()
        .with_body(json!({"error": "llamaforge is in setup mode", "setup_url": setup_url}));
}

// In main.rs — be explicit about which errors trigger setup mode:
match config::load() {
    Err(ConfigLoadError::NotFound) | Err(ConfigLoadError::EmptyFile) => enter_setup_mode(),
    Err(ConfigLoadError::ParseError(e)) => {
        eprintln!("Config parse error: {e}. Fix the config file or delete it to enter setup mode.");
        process::exit(78);
    }
    // ...
}
```

---

### [RULE 26 — Control the Blast Radius] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4, lines 599-636)

**What's wrong:** The build endpoint (`POST /api/setup/build`) clones a repository from GitHub and runs `cmake` + `make` with the number of parallel jobs specified by the client (`"jobs": 16`). There is no resource capping. On a machine with 8 cores, a client requesting `"jobs": 64` will oversubscribe the machine. More critically, the build process can consume arbitrary disk space (the llama.cpp repository plus build artifacts is several GB) and saturate all CPU cores for the entire build duration. During a first-run setup inside `llamaforge serve`, this means the build competes with the running server process for CPU and disk I/O.

**Why it matters:** The build process has unbounded blast radius on system resources. In standalone wizard mode this is acceptable (the wizard is the only thing running). In first-run mode within `llamaforge serve`, a build that saturates CPU will degrade any other work on the machine — including the LlamaForge server itself once the user finishes setup and the first inference requests arrive. If the build is still running when the user completes configuration, the transition to running mode could fail or be degraded.

**Suggested fix:**

1. Cap `jobs` server-side to `min(requested_jobs, num_cpus / 2)` — never let the build consume more than half the available cores.
2. Set a `nice` value (lower priority) on the build subprocess so it yields to the inference server and other processes.
3. In first-run mode, clearly warn the user that builds are resource-intensive and recommend using the standalone wizard (`llamaforge init --interactive`) for the build step if the server needs to remain responsive.
4. Add a maximum disk space check before starting the clone (ensure at least 5 GB free in the target directory).

---

### [RULE 28 — Design for Evolution, Not Permanence] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11, lines 519-699; Non-Goals NG1 and NG2, lines 29-30)

**What's wrong:** Model downloading (NG1) and prebuilt binary downloads (NG2) are explicitly out of scope for v1 — that is the right call. But the API surface as designed does not leave a clean seam for adding them. The model scanner (`POST /api/setup/scan-models`) only accepts `directories` to scan on disk. When model downloading is added, it will need a fundamentally different flow: search a registry, show results, initiate a download, track progress. If this gets bolted onto the existing `/api/setup/scan-models` endpoint, the contract becomes messy. Similarly, the binary acquisition path (`POST /api/setup/build`) is a build-from-source flow. Adding a "download prebuilt" path requires either overloading this endpoint (SSE with a completely different event schema) or adding a new endpoint — which is fine, but the frontend routing assumes a single `BinaryStep` orchestrator that manages one acquisition flow.

**Why it matters:** These are the two most likely v2 features for the wizard (the design itself says prebuilt downloads are "a natural future enhancement"). If the v1 API and component structure don't leave room for them, adding them later will require reworking the `BinaryStep` and `ModelStep` components and their associated API contracts — a painful mid-lifecycle refactor of a user-facing flow.

**Suggested fix:**

For the API, design the scan-models endpoint to accept a `source` discriminator from day one, even if only `"local"` is implemented:

```typescript
POST /api/setup/scan-models
{ "source": "local", "directories": [...] }

// Future:
POST /api/setup/scan-models
{ "source": "huggingface", "query": "llama-3", "quantization": "Q4_K_M" }
```

For the binary step, add a `method` discriminator to the build endpoint or (better) define a separate `POST /api/setup/acquire-binary` endpoint that dispatches to build-from-source or download-prebuilt based on a `method` field. This keeps the SSE event schemas cleanly separated.

For the frontend, structure `BinaryStep` and `ModelStep` as orchestrators that select between sub-flows (build vs. download, local-scan vs. registry-search) rather than hardcoding a single flow. This is a design-level note, not a code change — the current component hierarchy is close (BinaryStep already has BinaryFound/BinaryNotFound/BuildConfigurator branches), but the design should explicitly call out the extension point.

---

### [RULE 27 — Pressure Test: 10x, Chaos, New-Hire] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.3, lines 574-595)

**What's wrong:** The model scanner performs a recursive directory walk and the design says it should complete in under 5 seconds for fewer than 10,000 files. But the design does not specify a maximum scan depth, maximum number of results, or a cancellation mechanism. The 10x test: a user points the scanner at their home directory (`~/`) containing 500,000 files. The scan will take minutes, consume significant I/O, and the `POST /api/setup/scan-models` request will block (or time out on the frontend at 30 seconds, showing partial results — but only if the backend supports streaming partial results, which is not specified for this endpoint).

**Why it matters:** The model scanner is one-shot (not SSE-streamed), so there is no way to stream partial results or cancel it. A user who scans the wrong directory blocks the wizard for an unpredictable duration. The frontend's 30-second timeout (Section 6.2) is the only safety valve, but the design says "show partial results" without specifying how — the API contract returns a single JSON response, not a stream.

**Suggested fix:**

1. Add a `max_depth` parameter (default 5) and a `max_results` parameter (default 100) to the scan-models endpoint. Document that exhaustive deep scans are not the intent — the user should point at a models directory, not their entire filesystem.
2. Add a `timeout_ms` parameter (default 10000) so the backend can return partial results within a bounded time. The response already has `files_scanned` — add a `truncated: boolean` field and a `truncation_reason: "timeout" | "max_results" | "max_depth"` field so the frontend can tell the user why the scan stopped early.
3. Consider making the model scan cancelable: either via an SSE stream (like the build) or via a `DELETE /api/setup/scan-models` endpoint that aborts the in-progress scan.

```json
POST /api/setup/scan-models
{
  "directories": ["/home/user/models"],
  "max_depth": 5,
  "max_results": 100,
  "timeout_ms": 10000
}
-> 200 OK
{
  "models": [...],
  "scan_duration_ms": 1234,
  "files_scanned": 5678,
  "truncated": false
}
```

---

### [RULE 28 — Design for Evolution, Not Permanence] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 13, lines 729-738)

**What's wrong:** The setup-to-running transition is a one-shot state change: the server writes the config, hot-reloads it, and flips `mode` from `"setup"` to `"running"`. The design does not describe a rollback path. If the hot-reload partially succeeds (config is written, but spawning the first llama-server process fails because the binary path is wrong, or the model file is corrupted), the system is in a liminal state — neither in setup mode (config exists) nor fully running (no healthy servers). The user's only option is to manually delete or edit the config and restart.

**Why it matters:** The transition from setup to running is a deployment-like event that needs a rollback story. Without one, a wizard that generates a valid-looking but operationally broken config (e.g., GPU layer count too high for available VRAM, causing llama-server to OOM on startup) leaves the user stuck. They completed the wizard, saw "Ready!", but the system doesn't work — and re-running the wizard requires either manually entering setup mode again or deleting the config.

**Suggested fix:**

1. After saving the config, do a **validation launch** — attempt to start the first server instance and wait for its first health check. If the health check fails within a timeout (e.g., 30 seconds), present the failure to the user in the wizard (do not navigate to the dashboard) and offer to adjust the config. This keeps the user in the wizard's guided environment instead of dumping them into a broken dashboard.
2. Add a `POST /api/setup/validate-launch` endpoint that attempts to spawn one server and reports success/failure before committing the transition.
3. If the system boots with an existing config but all server health checks fail, offer a "reconfigure" banner on the dashboard rather than silently showing an empty server list.

---

### [RULE 27 — Pressure Test: 10x, Chaos, New-Hire] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 12, lines 703-725)

**What's wrong (new-hire test):** The dual-mode architecture (standalone wizard via `llamaforge init --interactive` vs. first-run wizard within `llamaforge serve`) means there are two separate code paths that must produce identical behavior. The standalone mode boots a minimal axum server with only UI + setup endpoints; the first-run mode boots the full transport layer and adds setup endpoints alongside everything else. This dual-boot path creates a surface area that a new contributor must understand: which endpoints exist in which mode? Can setup APIs be called when not in setup mode? What happens if someone hits `/api/setup/build` on a running production instance?

**Why it matters:** The new-hire test asks whether someone can understand the system in a week. Two boot modes with overlapping-but-different endpoint surfaces is a complexity multiplier. It is manageable in a solo project, but should be documented as a known complexity hotspot.

**Suggested fix:**

1. Guard all `/api/setup/*` endpoints behind the mode check — they should return `404 Not Found` (or `403 Forbidden`) when the server is in `"running"` mode, unless explicitly entered via the reconfigure flow (Flow 4). This prevents accidental invocation of setup APIs on a production server.
2. Consider extracting the setup API endpoints into a reusable `setup_router()` function that is mounted identically in both standalone and first-run mode. This ensures the two modes serve exactly the same setup endpoints, eliminating behavioral divergence.

```rust
fn setup_router() -> Router {
    Router::new()
        .route("/api/setup/hardware", get(hardware_handler))
        .route("/api/setup/scan-binary", post(scan_binary_handler))
        .route("/api/setup/scan-models", post(scan_models_handler))
        .route("/api/setup/build", post(build_handler))
        .route("/api/setup/recommend", post(recommend_handler))
        .route("/api/setup/save-config", post(save_config_handler))
        .route("/api/setup/check-path", post(check_path_handler))
}

// Standalone mode:
let app = Router::new()
    .merge(ui_router())
    .merge(setup_router());

// First-run mode (inside full serve):
let app = full_app_router()
    .merge(setup_router().layer(require_setup_mode_middleware));
```

---

### [RULE 26 — Control the Blast Radius] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4, lines 599-636)

**What's wrong:** The build endpoint runs `git clone` and `cmake --build` as child processes. If the wizard process is killed (Ctrl+C, crash, browser tab closed), the design does not specify cleanup for orphaned build subprocesses. A `cmake --build -j16` that outlives its parent will continue consuming CPU and disk until it completes or errors.

**Why it matters:** Orphaned build processes are a minor blast radius leak. The user expects that closing the wizard stops all wizard-initiated work.

**Suggested fix:**

Specify that the build subprocess is spawned in a process group. On wizard shutdown (or SSE disconnect), send SIGTERM to the process group and wait briefly before SIGKILL. Document this in the build endpoint's backend implementation notes.

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 1     |
| MEDIUM   | 4     |
| LOW      | 2     |

## Commendations

1. **Right-sized dual-mode architecture.** The decision to have the standalone wizard be a minimal axum server (not a separate binary) and the first-run wizard share the same UI routes is well-calibrated. It avoids the over-engineering of a separate wizard application while providing genuine operational convenience for both fresh installs and running systems. The "not a separate binary" decision is particularly good — it means zero additional build/release/distribution complexity.

2. **Shell commands over Rust-native GPU detection.** The decision to use `nvidia-smi`, `system_profiler`, etc. instead of Rust FFI bindings to CUDA/Metal/Vulkan is the right call for a one-time setup feature. It avoids pulling heavy native dependencies into the build for a feature that runs once. This is a textbook example of right-sizing — the operational cost of those dependencies (build complexity, binary size, platform-specific CI) would exceed their value at current scale.

3. **Ephemeral port for standalone mode.** Binding to `127.0.0.1:0` avoids port conflicts without configuration. Simple, correct, and eliminates an entire class of "port already in use" support issues.

4. **Clean separation of wizard state from server state.** The wizard stores its accumulated state in a client-side Svelte store (not the backend) and only touches the backend at save time. This means a crashed wizard or a browser refresh has zero effect on the running server — the blast radius of wizard failures is contained to the wizard session.

5. **TanStack Query with `staleTime: Infinity` for wizard data.** Correctly identifies that wizard scan results are session-scoped and should not be refetched. Avoids unnecessary hardware re-detection when the user navigates back to a previous step.

6. **Explicit non-goals for v1.** Listing model downloading and prebuilt binary downloads as non-goals (NG1, NG2) with clear rationale is better than silently omitting them. It creates a documented decision that can be revisited rather than a gap that might be accidentally filled with an ad-hoc implementation.
