# Scaling, Blast Radius & Evolution Review (Pass 2) — LlamaForge Setup Wizard

**Reviewer:** design-review-scaling
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` — Full document (Sections 1-15, all appendices). Second pass verifying resolution of 7 findings from the first scaling review, plus assessment of new material (Mode state machine, validation-launch step, resource controls).
**Verdict:** PASS

## Re-Review: First-Pass Findings

### Finding 1 — [RULE 26] HIGH: Setup mode blast radius leaks into inference API

**Status: RESOLVED**

The design now explicitly addresses this in three places:

1. **Section 13.1 (Config Load Triage):** Enumerates all `ConfigLoadError` variants with explicit behavior. `NotFound` and `EmptyFile` enter setup mode. `ParseError` is a **hard exit** with `process::exit(78)` (EX_CONFIG) and a clear error message to stderr. The original finding asked for this exact disambiguation — it is now specified.

2. **Section 13.3 (Inference Endpoint Behavior During Setup):** Both gRPC and HTTP inference endpoints now return well-defined error responses during `Setup` and `Transitioning` modes. gRPC returns status 14 (UNAVAILABLE) with `setup-url` metadata. HTTP returns 503 with `Retry-After: 60` and a JSON body containing `setup_url`. This is exactly the suggested fix from the first pass.

3. **Section 13.3 implementation:** The `SetupModeGuard` tower middleware is applied to the inference router only, not the setup or management routers. This correctly scopes the blast radius — setup endpoints, the status endpoint, and the UI remain accessible during setup mode.

No remaining gaps.

---

### Finding 2 — [RULE 26] MEDIUM: Build resource consumption unbounded

**Status: RESOLVED**

Section 11.4 ("Resource controls") now specifies all four mitigations from the original suggestion:

1. **Jobs cap:** `jobs = min(requested_jobs, num_cpus / 2)`. The effective job count is reported in the first `phase` SSE event so the user knows what was applied.
2. **Process priority:** `nice 10` on the build subprocess tree (Linux/macOS).
3. **Disk space check:** 5 GB minimum free disk at the temp directory mount point, checked before clone. Returns `409 Conflict` with structured error if insufficient.
4. **First-run warning:** The frontend shows a confirmation dialog before starting the build with a message about resource impact and estimated duration.

Additionally, Section 10.6 includes backend API tests for resource control boundaries: `jobs = 0` and negative values are rejected (400); `jobs > num_cpus` is clamped with a warning.

No remaining gaps.

---

### Finding 3 — [RULE 28] MEDIUM: No evolution seams for NG1/NG2

**Status: RESOLVED**

The design now includes clean extension points for both model downloading and prebuilt binary downloads:

1. **Model scanner (Section 11.3):** The request body includes a `"source": "local"` discriminator field. The response contract explicitly documents `Future: { "source": "huggingface", "query": "...", "quantization": "..." }` as the intended v2 extension path. This is exactly the suggested fix.

2. **Binary acquisition (Section 11.4):** The "Future" note documents `POST /api/setup/acquire-binary` with `{ "method": "download", "release": "..." }` as the intended v2 endpoint for prebuilt binary downloads. This keeps the build SSE schema separate from a future download SSE schema.

3. **TypeScript types (Appendix):** `ModelScanRequest` includes the `source: 'local'` field, making the discriminator visible to the frontend from day one.

The design chose to document the future endpoints as notes rather than defining stub endpoints — this is the right call for a v1. The seams exist; the plumbing does not need to.

No remaining gaps.

---

### Finding 4 — [RULE 27] MEDIUM: Model scanner no bounds

**Status: RESOLVED**

Section 11.3 now specifies all three bounding mechanisms:

1. **`max_depth`:** Optional parameter, default 5. Limits recursive directory walk depth.
2. **`max_results`:** Optional parameter, default 100. Limits the number of returned models.
3. **30-second scan timeout:** Enforced at the backend level.

The response contract now includes:
- `truncated: boolean` — indicates whether any limit was hit.
- `truncation_reason: "timeout" | "max_results" | "max_depth" | null` — identifies which limit fired first.
- `warnings: string[]` — surfaces permission errors and symlink cycles.

Section 10.6 includes boundary tests: empty directories array returns 400, more than 10 directories returns 400, non-existent directory returns partial success, path traversal is rejected.

The `timeout_ms` parameter from the original suggestion was not added as a client-tunable parameter — the backend enforces a fixed 30-second timeout instead. This is acceptable: the backend-enforced timeout provides the safety guarantee, and letting clients tune it adds API surface without meaningful user benefit in a wizard context.

No remaining gaps.

---

### Finding 5 — [RULE 28] MEDIUM: No rollback path for setup-to-running

**Status: RESOLVED**

This was the most substantive finding, and the design has addressed it thoroughly with the validation-launch step in Section 13.5:

1. **Pre-validation (Step 1):** Before writing to disk, the handler performs round-trip TOML validation, path resolution and existence checks, and port availability verification. Failures return 422 with per-field error details.

2. **Validation-launch (Step 3):** After writing the config, the handler spawns the first configured server and waits for a health check (30-second timeout). On failure, the response is 200 (config was written) with `validation_launch.success: false`, stderr output, and a corrective hint. The mode stays `Setup` — the user remains in the wizard.

3. **Hot-reload failure recovery (Step 4):** If the hot-reload itself fails after validation-launch succeeds, the mode reverts to `Setup` with the `reload.error` surfaced to the UI.

4. **Mode state machine (Section 13.2):** The `Transitioning` state provides an explicit intermediate mode during hot-reload, with a defined fallback path back to `Setup` on failure.

5. **CompletionScreen verification (Section 13.7):** The frontend does not navigate to the dashboard on save-config response alone. It verifies `validation_launch.success`, then `reload.success`, then polls `/api/status` to confirm `setup_required == false`. Each failure case has a specific UI: "Go Back" to adjust settings, "Retry" to re-attempt, and even a "Go to Dashboard Anyway" escape hatch after 10 seconds of polling.

**One minor observation (not a finding):** The validation-launch only tests the *first* configured server. If the user configures 3 models and the first works but the third OOMs, the wizard will show success. This is an acceptable trade-off for v1 — testing all servers would significantly increase save latency, and the single-server validation catches the most common failure mode (bad binary path, insufficient VRAM for any offload at all). The design should document this limitation, but it does not rise to finding level.

No remaining gaps.

---

### Finding 6 — [RULE 27] LOW: Dual-boot mode complexity

**Status: RESOLVED**

Section 12.1 now defines a shared `setup_router()` function that is used identically in both modes:

1. **Shared router:** Both standalone mode and first-run mode mount the same `setup_router()`. The function is defined once with all 9 setup endpoints. Standalone composes `ui_router().merge(setup_router())`; first-run composes `full_app_router().merge(setup_router().layer(setup_mode_guard))`.

2. **Mode guard in running mode (Section 13.8):** Setup endpoints remain mounted in `Running` mode for the reconfigure flow, but are guarded by a middleware requiring `X-Setup-Reconfigure: true` header. Without it, setup endpoints return 403. This prevents accidental invocation on a running instance.

The original finding suggested `404` in running mode; the design chose `403` with a required header for the reconfigure flow, which is arguably better — it allows Flow 4 (reconfigure) without needing a mode transition, while still preventing accidental calls.

No remaining gaps.

---

### Finding 7 — [RULE 26] LOW: Orphaned build subprocesses

**Status: RESOLVED**

Section 11.4 ("Orphaned subprocess cleanup") now specifies the full cleanup protocol:

1. **Process group:** Build process is spawned in its own process group (`setsid` on Linux, `setpgid` on macOS).
2. **Graceful shutdown:** On wizard shutdown or SSE disconnect grace period expiry, send `SIGTERM` to the process group, wait 5 seconds, then `SIGKILL` if still alive.
3. **SSE disconnect grace period:** 2-minute grace period for client reconnection. If no reconnect, the build is killed and temp directory cleaned up.
4. **Ctrl+C handling:** Section 10.8 includes a test that verifies SIGINT kills the wizard server with exit code 130 and leaves no child processes running.

No remaining gaps.

---

## New Material Assessment

### Mode State Machine (Section 13.2) — New Blast Radius Concerns

The three-state machine (`Setup -> Transitioning -> Running`, with `Transitioning -> Setup` fallback) is clean and well-bounded:

- **No `Running -> Setup` transition:** This is the correct design choice. Once running, the mode never regresses. The reconfigure flow (Section 13.8) operates within `Running` mode, avoiding a blast radius where an operator's reconfigure action takes down inference.

- **`Transitioning` state duration is bounded:** The hot-reload is a synchronous config swap via `ArcSwap`. The `Transitioning` window is on the order of milliseconds. The 503 guard covers this window, so inference clients see a brief `Retry-After: 60` response rather than a mysterious failure.

- **No mode state persistence:** The mode is in-memory only (`Arc<ArcSwap<Mode>>`). If the process crashes during `Transitioning`, it restarts and re-evaluates the config from disk — entering either `Running` (config exists and is valid) or `Setup` (config missing/empty). There is no stuck-in-transitioning failure mode.

No new blast radius concerns from the Mode state machine.

### Validation-Launch Failure Modes

The validation-launch step (Section 13.5, Step 3) introduces its own potential issues, which the design handles adequately:

1. **Port conflict during validation:** The validation launch spawns `llama-server` on the configured port. If another process took the port between pre-validation (Step 1) and validation-launch (Step 3), the health check will fail. The design handles this: the failure is surfaced to the user with stderr output and a hint. The TOCTOU window is inherently small (seconds).

2. **Validation server not cleaned up:** If the validation server passes its health check, the design says "kill the validation server, proceed to Step 4." If the wizard process crashes between health check success and the kill, the validation server becomes an orphan. However, the real servers will be spawned by the hot-reload in Step 4, and any port conflict with the orphaned validation server will surface as a reload failure — which reverts to `Setup` mode. The user retries, the orphan has its port, the retry's pre-validation catches the conflict. This is a self-healing path, not a stuck state.

3. **30-second health check timeout:** Reasonable for a single server startup. Large models on CPU-only hardware might take longer, but this is a validation check (proving the binary works with the config), not a production readiness check.

No new findings from the validation-launch step.

### `Arc<ArcSwap<Mode>>` — Double Indirection

The mode is stored as `Arc<ArcSwap<Mode>>`. The `Arc` is for sharing across handlers; the `ArcSwap` is for lock-free atomic updates. This is the same pattern the project uses for config (documented in `01_CONFIG_MANAGER.md`). The double indirection is a standard Rust pattern for shared mutable state in async contexts. Not a concern.

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 0     |
| MEDIUM   | 0     |
| LOW      | 0     |

All 7 findings from the first pass are resolved. No new scaling, blast radius, or evolution concerns introduced by the updated material.

## Commendations

1. **Thorough validation pipeline.** The four-step save-config flow (pre-validation, write, validation-launch, hot-reload) is exactly the right level of rigor for a setup-to-running transition. The pre-validation catches static errors before touching disk; the validation-launch catches runtime errors before committing the mode transition. Each step has a defined failure response and recovery path. This is what "design for the failure case" looks like.

2. **`Transitioning` state is a genuine improvement.** The first pass asked for a rollback path. The design delivered a proper state machine with an explicit intermediate state and a defined fallback. The `Transitioning -> Setup` revert on hot-reload failure is cleaner than the "delete config and restart" path the first pass was concerned about.

3. **Resource controls are concrete and testable.** The jobs cap, nice value, disk space check, and first-run warning are not just described — they have specific values, specific error codes, and specific test cases in Section 10.6. This makes them verifiable, not aspirational.

4. **Process group cleanup is production-grade.** The `setsid`/`setpgid` + `SIGTERM` + 5s wait + `SIGKILL` sequence with SSE disconnect grace period is the standard Unix process cleanup pattern. Combined with the test in Section 10.8 that verifies no orphaned processes after SIGINT, this is fully specified.

5. **Reconfigure guard is well-designed.** Using a required header (`X-Setup-Reconfigure: true`) in `Running` mode rather than a mode transition is a smart design choice. It allows the reconfigure flow without creating a window where inference is degraded — the blast radius of reconfiguration is zero on the inference path.
