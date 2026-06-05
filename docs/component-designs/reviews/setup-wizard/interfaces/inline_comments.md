<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 680-689 RULE: 14 SEVERITY: HIGH -->
**Rule 14 — Make Interfaces Narrow**
`POST /api/setup/save-config` accepts `"config": { /* full AppConfig structure */ }`. This leaks the internal config schema through the API boundary. The frontend should not construct `AppConfig` — it should send the wizard's logical outputs (selected models, per-model config overrides, save path) and let the backend assemble the full config.

Suggested: Replace the request body with the wizard's domain-level outputs:
```json
{
  "path": "/home/user/.config/llamaforge/llamaforge.toml",
  "force": false,
  "servers": [
    {
      "model_path": "...", "alias": "...", "binary_path": "...",
      "port": 8080, "gpu_layers": -1, "ctx_size": 8192,
      "parallel_slots": 2, "cache_type_k": "q8_0", "cache_type_v": "q8_0",
      "flash_attn": "auto"
    }
  ]
}
```
Add a `POST /api/setup/preview-config` that returns the rendered TOML from the same inputs (for the ReviewStep's preview) without writing to disk.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 524-549 RULE: 14 SEVERITY: MEDIUM -->
**Rule 14 — Make Interfaces Narrow**
The hardware detection response includes `driver_version`, `arch`, and `os` — fields the wizard UI never displays or acts on. `driver_version` especially is an implementation detail whose format is coupled to the detection backend (`nvidia-smi` vs. NVML vs. `rocm-smi`). Every field in the response is a maintenance commitment.

Suggested: Remove `driver_version`, `arch`, and `os` from the primary response. If needed for diagnostics, serve them from a separate `GET /api/setup/hardware/diagnostics` endpoint. Keep the primary response to: gpus (index, name, vram_mb), cpu (name, cores_physical, cores_logical), ram_mb, recommended_backend.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 574-596 RULE: 15 SEVERITY: MEDIUM -->
**Rule 15 — Be Explicit About Guarantees**
The model scanner response includes `scan_duration_ms` and `files_scanned` without documenting whether they are part of the stable contract. The `quantization_hint` and `architecture_hint` fields have no defined fallback value when filename inference fails. The `models` array has no documented sort order.

Suggested: Add contract notes:
- `scan_duration_ms` and `files_scanned` are diagnostic and may be removed in future versions.
- `quantization_hint` and `architecture_hint` are `""` (empty string) when inference fails.
- `models` array order is unspecified; sort client-side if needed.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 599-636 RULE: 15 SEVERITY: MEDIUM -->
**Rule 15 — Be Explicit About Guarantees**
The build SSE stream defines five event types but omits critical behavioral guarantees: event ordering (must `phase` precede `progress`?), monotonicity of `progress.percent`, reconnection semantics (does the build survive a dropped SSE connection?), whether `log` lines are complete or partial, and maximum event frequency.

Suggested: Add a "Stream Contract" subsection specifying:
1. `phase` always precedes `progress`/`log` for that phase; first event is `phase`, last is `complete` or `failed`.
2. `progress.percent` is monotonically non-decreasing within a phase.
3. `log` events contain complete lines (backend buffers partials).
4. Backend throttles `log` to max 10/sec by batching into `lines: string[]`.
5. Build continues server-side if SSE drops. Provide `GET /api/setup/build/{id}` for snapshot and `GET /api/setup/build/{id}/stream` for reconnectable SSE.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 599-636 RULE: 16 SEVERITY: MEDIUM -->
**Rule 16 — Find the Narrow Waist**
`POST /api/setup/build` conflates build initiation and output streaming in a single SSE POST. This makes the build non-reconnectable, non-observable from another tab, and fragile to browser lifecycle events. A 10-minute build that loses its SSE connection is unrecoverable.

Suggested: Split into the standard long-running job pattern:
```
POST   /api/setup/build              -> 202 { "build_id": "..." }
GET    /api/setup/build/{id}         -> 200 { status, phase, percent, last_lines }
GET    /api/setup/build/{id}/stream  -> SSE (reconnectable)
DELETE /api/setup/build/{id}         -> 200 (cancel)
```
POST is idempotent if a build is already running. GET provides snapshot for reconnection. This matches the project's existing pattern of separating mutations from observations.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 289-303 RULE: 16 SEVERITY: MEDIUM -->
**Rule 16 — Find the Narrow Waist**
`SetupWizardStore` is a flat bag of nullable fields. Each step must defensively check whether prerequisite data exists. There is no typed state machine enforcing that "configure" can only be reached with hardware + binary + models populated.

Suggested: Model the wizard state as a discriminated union where each step variant carries exactly the data it and all subsequent steps need:
```typescript
type WizardState =
  | { step: 'welcome' }
  | { step: 'hardware'; hardware: HardwareInfo }
  | { step: 'binary'; hardware: HardwareInfo; binary: BinaryInfo }
  | { step: 'models'; hardware: HardwareInfo; binary: BinaryInfo; models: DiscoveredModel[] }
  | { step: 'configure'; hardware: HardwareInfo; binary: BinaryInfo; models: DiscoveredModel[]; configs: Map<string, ServerConfigDraft> }
  | { step: 'review'; /* same as configure */ }
```
No nulls, no optional fields. The layout constructs the next variant only when the current step completes.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 639-676 RULE: 14 SEVERITY: MEDIUM -->
**Rule 14 — Make Interfaces Narrow**
`POST /api/setup/recommend` requires the client to echo back the full `hardware` object that the backend itself produced. The backend is the authority on hardware detection — it should not delegate custody of that data to the client. A modified or stale hardware object would produce incorrect recommendations silently.

Suggested: The backend caches its own hardware detection result (single-user, single-machine — a simple `Arc<Mutex<Option<HardwareInfo>>>` suffices). The recommend endpoint reads cached hardware internally:
```json
POST /api/setup/recommend
{ "models": [{ "path": "...", "size_bytes": 4370000000 }] }
```
If the user re-detects hardware, `GET /api/setup/hardware` refreshes the cache.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 551-568 RULE: 15 SEVERITY: LOW -->
**Rule 15 — Be Explicit About Guarantees**
`BinaryScanResult` with `found: true` does not define the value of `version` when `llama-server --version` fails or returns unexpected output. Does `backends` default to `["unknown"]` or `[]`?

Suggested: Document edge cases:
- Version detection failure: `version: "unknown"`, `backends: []`.
- Binary exists but not executable: `found: false`, path included in `searched_paths`.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 548-549 RULE: 17 SEVERITY: LOW -->
**Rule 17 — Wrap Dependencies, Don't Leak Them**
Implementation details (`nvidia-smi --query-gpu=...`, `rocm-smi`, `vulkaninfo`, `system_profiler`, `sysctl`) appear in the same section as the API response schema. A reader might interpret shell command specifics as part of the interface contract.

Suggested: Move the "Backend implementation" paragraphs to a separate "Implementation Notes" appendix or subsection, clearly separated from the request/response schema.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 729-738 RULE: 14 SEVERITY: LOW -->
**Rule 14 — Make Interfaces Narrow**
Adding `"mode": "setup" | "running"` to `GET /api/status` puts wizard-specific state on a general-purpose endpoint. Future modes (`"maintenance"`, `"draining"`) would turn this into a stringly-typed state machine that every `/api/status` consumer must understand.

Suggested: Either use a boolean (`"setup_required": true`) for a narrower contract, or document the enum as extensible with the invariant: `"setup"` = no valid config exists, `"running"` = at least one server configured. Consumers must handle unknown values by treating them as `"running"`.
