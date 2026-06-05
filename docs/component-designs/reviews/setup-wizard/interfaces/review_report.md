# Interface Design & Dependency Review — LlamaForge Setup Wizard

**Reviewer:** design-review-interfaces
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` — Sections 4.2 (Component Contracts), 11 (Backend API Contract), 12 (Standalone Wizard Mode), 13 (First-Run Detection), Appendix (TypeScript Types)
**Verdict:** PASS WITH WARNINGS

## Violations Found

### [RULE 14 — Make Interfaces Narrow] — Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.6, lines 680–689)

**What's wrong:** `POST /api/setup/save-config` accepts the full `AppConfig` structure in its request body (`"config": { /* full AppConfig structure */ }`). This means the frontend must construct and send the entire internal config schema, coupling the UI to every field in `AppConfig`. If `AppConfig` gains a new field, the wizard's save contract silently changes. Worse, it inverts responsibility: the frontend is assembling a config object that the backend should own.

**Why it matters:** The wizard already has all the data the backend needs to generate a config: the hardware info, selected models, and per-model config overrides. Sending the full `AppConfig` means the frontend duplicates the config assembly logic that lives in the backend's config module. This is the classic "database schema in the API" pattern — the internal persistence format (`AppConfig`/`llamaforge.toml`) is leaking through the API boundary. Any structural change to `AppConfig` (new top-level fields, renamed sections, nested config groups) breaks the frontend contract even if the wizard's conceptual inputs haven't changed.

**Suggested fix:**
The save endpoint should accept the wizard's *logical outputs*, not the serialized config:

```
POST /api/setup/save-config
{
  "path": "/home/user/.config/llamaforge/llamaforge.toml",
  "force": false,
  "hardware": { /* from step 1 */ },
  "servers": [
    {
      "model_path": "/home/user/models/llama-3-8b-Q4_K_M.gguf",
      "alias": "llama-3-8b",
      "binary_path": "/usr/local/bin/llama-server",
      "port": 8080,
      "gpu_layers": -1,
      "ctx_size": 8192,
      "parallel_slots": 2,
      "cache_type_k": "q8_0",
      "cache_type_v": "q8_0",
      "flash_attn": "auto"
    }
  ]
}
```

The backend assembles the full `AppConfig` (filling in global defaults, listen addresses, priority rules, etc.) and writes the TOML. The frontend never sees or constructs `AppConfig`. This also makes the `ReviewStep`'s TOML preview a backend concern — add a `POST /api/setup/preview-config` that returns the rendered TOML string from the same logical inputs, without writing to disk.

---

### [RULE 14 — Make Interfaces Narrow] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.1, lines 524–549)

**What's wrong:** The hardware detection response includes `driver_version` and `compute_backend` per GPU, plus `os` and `arch` at the top level. The wizard UI displays none of these except `compute_backend` (indirectly, via `recommended_backend`). The `driver_version` is an internal implementation detail — it's useful for debugging but not for wizard decisions. Including it in the API contract means consumers may start depending on its format (e.g., parsing "550.54.14" to check CUDA compatibility), which couples the API to NVIDIA's versioning scheme.

**Why it matters:** Every field in a response is a maintenance commitment. `driver_version` has no defined use in the wizard flow. If a future consumer starts parsing it, changing the detection backend (e.g., from `nvidia-smi` to NVML) could break them if the version string format differs. `os` and `arch` are also questionable — the wizard runs on the same machine, so the frontend already knows the OS.

**Suggested fix:** Move `driver_version` and `arch` out of the primary response. If they're needed for a future "system info" diagnostic panel, add a separate `GET /api/setup/hardware/diagnostics` endpoint. Keep the primary response narrow:

```json
{
  "gpus": [{ "index": 0, "name": "NVIDIA RTX 4090", "vram_mb": 24564 }],
  "cpu": { "name": "AMD Ryzen 9 7950X", "cores_physical": 16, "cores_logical": 32 },
  "ram_mb": 65536,
  "recommended_backend": "cuda"
}
```

If `driver_version` is genuinely needed for the build step's backend selection (e.g., to warn about old CUDA drivers), document that dependency explicitly.

---

### [RULE 15 — Be Explicit About Guarantees] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.3, lines 574–596)

**What's wrong:** The model scanner response includes `scan_duration_ms` and `files_scanned` as top-level fields. These are diagnostic/observability values, not part of the functional contract. The design does not document whether these fields are guaranteed to be present, whether `scan_duration_ms` is wall-clock or CPU time, or whether `files_scanned` counts only `.gguf` files or all files traversed. The `quantization_hint` and `architecture_hint` fields use `_hint` suffixes (good — signals best-effort), but the doc doesn't state what value they take when inference fails (empty string? `"unknown"`? `null`?).

**Why it matters:** Without explicit guarantees, a future frontend developer might use `scan_duration_ms` to show a "scan was fast/slow" indicator, then break when the backend changes its measurement. The `_hint` fields need a defined fallback value so the frontend can render a consistent "Unknown" state without null-checking.

**Suggested fix:** Add a contract note to Section 11.3:
- `scan_duration_ms` and `files_scanned` are diagnostic; they may be removed in future versions. Do not build UI logic on them.
- `quantization_hint` and `architecture_hint` are best-effort inferences from the filename. When inference fails, both are the empty string `""`. The frontend should display "Unknown" for empty hints.
- The `models` array order is not guaranteed (filesystem walk order). If the UI needs sorted display, sort client-side.

---

### [RULE 15 — Be Explicit About Guarantees] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4, lines 599–636)

**What's wrong:** The build SSE stream defines five event types (`phase`, `progress`, `log`, `complete`, `failed`) but does not define: (a) whether events are guaranteed to arrive in a specific order (must `phase` always precede `progress` for that phase?), (b) whether `progress.percent` is monotonically increasing, (c) what happens if the SSE connection drops mid-build (does the build continue? can the client reconnect and resume the stream?), (d) whether `log` events are guaranteed to contain complete lines or can contain partial lines, (e) maximum frequency of `log` events (a fast build can emit thousands of lines per second).

**Why it matters:** The `BuildProgress` component depends on these SSE events to render a coherent UI. Without ordering guarantees, a `progress` event arriving before its `phase` event would show a percentage for an unknown phase. Without reconnection semantics, the frontend cannot recover from a transient network hiccup during a 5-minute build. Without rate guarantees, a fast build could overwhelm the browser's DOM update pipeline if every `log` event triggers a re-render.

**Suggested fix:** Add a "Stream Contract" subsection to 11.4:
- Event ordering: `phase` always precedes any `progress` or `log` for that phase. The first event is always `phase`. The last event is always `complete` or `failed`.
- `progress.percent` is monotonically non-decreasing within a phase. It may jump (e.g., 0 to 45 if git reports progress in bursts).
- `log` events contain complete lines (the backend buffers partial lines before emitting).
- The backend SHOULD throttle `log` events to at most 10 per second by batching lines. The `log.line` field becomes `log.lines: string[]` when batched.
- If the SSE connection drops, the build continues server-side. The client can issue `GET /api/setup/build/status` to get the current build state (phase, last N log lines, percent). If the build is still running, the client can reconnect to a new SSE stream on `GET /api/setup/build/stream` (not POST — the build is already running). This separates "start a build" (POST, idempotent with a build ID) from "watch a build" (GET, reconnectable).
- The terminal event (`complete` or `failed`) is replayed on reconnection if the build finished while disconnected.

---

### [RULE 16 — Find the Narrow Waist] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Sections 4.2, 5.3, 11)

**What's wrong:** The wizard has no single, minimal abstraction that decouples the step UI from the backend API. Each step directly knows its specific API endpoint and response shape. The `SetupWizardStore` (Section 5.3) accumulates raw API responses as properties (`hardware: HardwareInfo | null`, `binaryPath: string | null`, etc.), making it a bag of nullable fields rather than a typed state machine. There is no "wizard session" abstraction — the accumulated state is just the union of what each step wrote into the store.

**Why it matters:** The wizard is a linear state machine with well-defined transitions (welcome -> hardware -> binary -> models -> configure -> review). Each state has preconditions (e.g., "configure" requires hardware + binary + selected models). Without a narrow waist that enforces these transitions, nothing prevents a future developer from navigating directly to `/ui/setup/configure` with an empty store, hitting null reference errors. The store's flat nullable structure forces every step to defensively check whether prerequisite data exists.

**Suggested fix:** Define the wizard state as a discriminated union (the narrow waist):

```typescript
type WizardState =
  | { step: 'welcome' }
  | { step: 'hardware'; hardware: HardwareInfo }
  | { step: 'binary'; hardware: HardwareInfo; binary: BinaryInfo }
  | { step: 'models'; hardware: HardwareInfo; binary: BinaryInfo; models: DiscoveredModel[] }
  | { step: 'configure'; hardware: HardwareInfo; binary: BinaryInfo; models: DiscoveredModel[]; configs: Map<string, ServerConfigDraft> }
  | { step: 'review'; hardware: HardwareInfo; binary: BinaryInfo; models: DiscoveredModel[]; configs: Map<string, ServerConfigDraft> }
```

Each step variant carries exactly the data it and all subsequent steps need — no nulls, no optional fields. The `SetupLayout` controls transitions by constructing the next variant only when the current step's data is complete. Steps receive their slice of the state as non-nullable props. This is the narrow waist: every step below "configure" knows it has hardware info because the type guarantees it, and every step above "hardware" doesn't need to care about the hardware API shape.

---

### [RULE 16 — Find the Narrow Waist] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4, lines 599–636)

**What's wrong:** The build endpoint (`POST /api/setup/build`) conflates two concerns in a single SSE POST: (1) initiating a build, and (2) streaming its output. This makes the endpoint non-idempotent, non-reconnectable, and non-observable. If the browser tab crashes mid-build, the build process either orphans or the user must re-initiate it. There is no way to query "is a build already running?" before starting a new one.

**Why it matters:** Builds are long-running (up to 10+ minutes) and have real side effects (disk writes, CPU saturation). A narrow waist for long-running operations is the "job" abstraction: POST to create a job, GET to observe it, DELETE to cancel it. The current design merges creation and observation into one connection, making the build fragile to network interruptions and browser lifecycle events.

**Suggested fix:** Split into three endpoints:
```
POST /api/setup/build          -> 202 Accepted { "build_id": "..." }
GET  /api/setup/build/{id}     -> 200 { status, phase, percent, last_lines }
GET  /api/setup/build/{id}/stream -> SSE (reconnectable, replays terminal event)
DELETE /api/setup/build/{id}   -> 200 (cancels the build, kills the process)
```

The POST is idempotent if a build is already running for the same backend (return the existing `build_id`). The GET provides a snapshot for reconnection. The SSE GET is the live stream. This pattern matches the existing project convention of separating mutations from observations (e.g., `POST /api/servers/{id}/start` is separate from `GET /api/servers`).

---

### [RULE 14 — Make Interfaces Narrow] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.5, lines 639–676)

**What's wrong:** The recommendation endpoint accepts the full `hardware` object in the request body, even though the backend already computed and returned this data in `GET /api/setup/hardware`. This means the frontend sends hardware data back to the backend that the backend itself produced. This is a round-trip leak — the backend should remember its own detection results rather than requiring the client to echo them back.

**Why it matters:** If the client modifies the hardware object (accidentally or intentionally), the recommendations would be based on incorrect hardware data. The backend is the authority on hardware detection — it should not delegate custody of that data to the frontend. This also increases the request payload size unnecessarily and creates a subtle coupling: the request body shape of `/api/setup/recommend` is coupled to the response shape of `/api/setup/hardware`.

**Suggested fix:** The backend should cache hardware detection results (keyed by a session or simply in memory, since the wizard is single-user per machine). The recommendation endpoint should reference cached hardware:

```
POST /api/setup/recommend
{
  "models": [
    { "path": "/home/user/models/llama-3-8b-Q4_K_M.gguf", "size_bytes": 4370000000 }
  ]
}
```

If the backend needs a hardware refresh (e.g., user plugged in a new GPU), the client calls `GET /api/setup/hardware` again to update the cache, then calls `/api/setup/recommend`. The recommendation endpoint internally reads the cached hardware data. This also simplifies the TypeScript types — `RecommendationRequest` no longer needs to embed `HardwareInfo`.

---

### [RULE 15 — Be Explicit About Guarantees] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.2, lines 551–568)

**What's wrong:** The binary scanner response has two shapes discriminated by the `found` boolean, but the TypeScript type (`BinaryScanResult`) does not document what happens when `found: true` but the version check fails (e.g., the binary exists but `llama-server --version` returns an error or unexpected output). Is `version` an empty string? Is it `"unknown"`? Can `backends` be empty?

**Why it matters:** The `BinaryFound` presentational component displays the version string. If it receives an empty string, the UI would show "Version: " with trailing empty space, which is a cosmetic bug. Defining the edge cases in the contract prevents this.

**Suggested fix:** Document in Section 11.2:
- If the binary exists but version detection fails, `version` is `"unknown"` and `backends` is `["unknown"]`.
- If the binary exists but is not executable, the result is `found: false` with the path included in `searched_paths`.

---

### [RULE 17 — Wrap Dependencies, Don't Leak Them] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.1, lines 548–549)

**What's wrong:** The hardware detection implementation section mentions specific shell commands (`nvidia-smi --query-gpu=...`, `rocm-smi`, `vulkaninfo`, `system_profiler SPDisplaysDataType`, `sysctl`) in the API contract documentation. While these are described as "backend implementation," they appear in the same section as the API response schema, blurring the boundary between contract and implementation. A consumer reading Section 11.1 might assume these commands are part of the contract (e.g., expecting `driver_version` format to match `nvidia-smi` output).

**Why it matters:** Implementation details in API documentation become implicit contracts. The backend should be free to switch from `nvidia-smi` parsing to NVML FFI without consumers expecting a format change.

**Suggested fix:** Move the implementation paragraphs to a separate "Implementation Notes" subsection or an appendix, clearly separated from the API contract. The API contract section should only contain the request/response schema and behavioral guarantees.

---

### [RULE 14 — Make Interfaces Narrow] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 13, lines 729–738)

**What's wrong:** The first-run detection mechanism adds a `mode` field to the existing `GET /api/status` response. This is a new field on an existing endpoint, and its value space (`"setup" | "running"`) is tightly coupled to the wizard feature. If future features add more modes (e.g., `"maintenance"`, `"draining"`), this becomes a stringly-typed state machine on a general-purpose endpoint.

**Why it matters:** Adding feature-specific state to a general endpoint means every consumer of `/api/status` must now understand wizard-related modes. The mode field is really answering the question "is initial setup complete?" which is a boolean, not an extensible enum.

**Suggested fix:** Either:
(a) Use a boolean: `"setup_required": true` in the status response. This is a narrower contract — consumers check one flag, not an enum.
(b) Keep the current approach but document the enum as extensible (consumers must handle unknown values gracefully) and define the invariant: `"setup"` means no valid config exists; `"running"` means at least one server is configured.

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 1     |
| MEDIUM   | 5     |
| LOW      | 3     |

## Commendations

1. **Component contracts are well-defined.** Section 4.2 explicitly lists what each presentational component does NOT know. This is a strong pattern — negative contracts ("does NOT know about SSE connections, abort controllers") are harder to write than positive ones and far more valuable for preventing coupling creep.

2. **Discriminated union for `BinaryScanResult`.** The TypeScript type uses a proper tagged union (`found: true | false`) rather than a bag of optional fields. This is exactly the right pattern for API responses with variant shapes.

3. **`StatusBadge` reuse for hardware status.** The design consistently uses the existing triple-redundancy status pattern (icon + text + color) rather than inventing a new status display for the wizard. This reinforces the project's accessibility convention.

4. **No `{@html}` anywhere.** The TOML preview uses `renderMarkdown()` with a code block, and the build log uses `<pre>` text content. This is rigorous adherence to the existing security convention.

5. **TanStack Query with `staleTime: Infinity`.** The decision to treat wizard API responses as session-stable (never refetch in background) is correct for a linear wizard flow and avoids unnecessary network traffic.

6. **Separation of orchestrator and presentational components.** The hierarchy in Section 4.1 draws a clean line: steps are orchestrators that own API calls, children are pure presentational. This matches the project's established intelligence distribution pattern from `FRONTEND_ARCHITECTURE.md`.

7. **`POST` for scan endpoints.** Using POST for model scanning and binary scanning (which accept search directories in the body) is correct — these are not idempotent GETs, and the request body carries meaningful parameters. This avoids the anti-pattern of encoding complex query structures in URL parameters.
