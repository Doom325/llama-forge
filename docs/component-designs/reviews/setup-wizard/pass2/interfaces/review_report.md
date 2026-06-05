# Interface Design & Dependency Review (Pass 2) — LlamaForge Setup Wizard

**Reviewer:** design-review-interfaces
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` — Sections 4.2 (Component Contracts), 5 (State Management), 11 (Backend API Contract), 12 (Standalone Wizard Mode), 13 (First-Run Detection), Appendix (TypeScript Types)
**Verdict:** PASS

## Prior Finding Resolutions

### Finding 1 — HIGH R14: save-config leaks AppConfig — RESOLVED

Section 11.6 now specifies a narrow request body with `servers[]` and `global_overrides` only. The explicit statement "The frontend never sees or constructs `AppConfig`. The backend is the sole owner of config assembly" (lines 870-871) directly addresses the leaky abstraction. The `SaveConfigRequest` TypeScript type in the appendix matches: it contains `servers: ServerConfigDraft[]` and `global_overrides?: { bind_address?: string; http_port?: number }`. No `AppConfig` reference anywhere in the frontend types. Fully resolved.

### Finding 2 — MEDIUM R16: Build SSE non-reconnectable — RESOLVED

Section 11.4 now implements the job pattern exactly as recommended:

- `POST /api/setup/build` returns `202 Accepted { "build_id": "uuid-here" }` (creation)
- `GET /api/setup/build/{id}` returns snapshot status (observation)
- `GET /api/setup/build/{id}/stream` returns reconnectable SSE (live stream)
- `DELETE /api/setup/build/{id}` cancels the build

POST is documented as idempotent (returns existing `build_id` if a build is already running for the same backend). The separation of mutation from observation is clean. The shared `setup_router()` in Section 12.1 registers all four routes. Fully resolved.

### Finding 3 — MEDIUM R14: Recommend echoes hardware back — RESOLVED

Section 11.5 now shows the recommendation request containing only `models[]` with `path` and `size_bytes`. The statement "The backend caches hardware detection results in memory. The recommendation endpoint reads cached hardware internally." directly addresses the round-trip leak. The `RecommendationRequest` TypeScript type confirms: `models: Array<{ path: string; size_bytes: number }>` with no `hardware` field. Fully resolved.

### Finding 4 — MEDIUM R15: Build SSE no behavioral guarantees — RESOLVED

Section 11.4 now contains a dedicated "Stream Contract" subsection specifying:

- Event ordering: `phase` always precedes `progress`/`log` for that phase. First event is always `phase`. Last is always `complete` or `failed`.
- `progress.percent` is monotonically non-decreasing within a phase.
- `log` events contain complete lines (backend buffers partial lines).
- Log throttling: max 10/s with batching to `log.lines: string[]`.
- Terminal event replay on reconnection.

All five behavioral guarantees from the original recommendation are present. Fully resolved.

### Finding 5 — MEDIUM R16: Wizard store flat nullable bag — RESOLVED

Section 5.3 now defines a lean `SetupWizardStore` with only three client-owned fields: `selectedModelIds`, `userOverrides`, and `savePath`. The key statement: "TanStack Query is the sole source of truth for server-fetched data (hardware, binary scan, models, recommendations). The store holds only client-owned state that does not exist in any API response."

Additionally, Section 5.3 introduces a `STEP_TRANSITIONS` state machine with explicit `canAdvance` guards and `invalidatesOnReentry` rules. The `completedSteps` set is derived from the transition table, not stored independently. This addresses the narrow waist concern differently than the discriminated union approach suggested — it uses URL-based step routing with transition guards rather than a single typed state object. The approach is valid: the advancement guards enforce preconditions, and TanStack Query's cache ensures server data exists before a step can complete. The tradeoff is that steps still need to handle the "loading" state before TanStack Query populates, but that is inherent to any async data fetching pattern.

**Remaining minor gap:** The `canAdvance` predicates reference `s.hardware`, `s.binaryPath`, `s.selectedModels`, and `s.serverConfigs` which are not fields on the lean store (the store only has `selectedModelIds`, `userOverrides`, `savePath`). These must be reading from TanStack Query cache. The pseudocode should clarify that these predicates check query cache status rather than store fields to avoid confusion during implementation. This is a documentation clarity issue, not an interface design issue. Not re-raising as a finding.

Resolved.

### Finding 6 — MEDIUM R14: Hardware response leaks driver_version/arch — RESOLVED

Section 11.1 now shows a narrow hardware response with only the fields the wizard needs: `gpus[].index`, `gpus[].name`, `gpus[].vram_mb`, `cpu.name`, `cpu.cores_physical`, `cpu.cores_logical`, `ram_mb`, and `recommended_backend`. No `driver_version`, no `arch`, no `os`. The design adds: "Diagnostic fields (driver versions, architecture details) are available via the optional `?diagnostics=true` query parameter for bug reports." This is even cleaner than a separate endpoint — it keeps the primary response narrow while making diagnostics accessible without a new route. The `GpuInfo` and `CpuInfo` TypeScript types in the appendix confirm the narrow shape. Fully resolved.

### Finding 7 — LOW R15: Binary scan edge cases undocumented — RESOLVED

Section 11.2 now documents the edge case: "A timed-out or hung binary is reported as `found: true` with `version: 'found but unresponsive to --version'` so the user can confirm or reject it in the UI." The 5-second timeout on `--version` is explicit. The `BinaryScanResult` TypeScript type uses a discriminated union (`found: true | false`), and the `version` field on the found variant is a plain `string` which accommodates both the parsed version and the fallback message.

**Remaining minor gap:** The design does not specify what `backends` contains when version detection fails. Is it an empty array? `["unknown"]`? The frontend's `BinaryStep` likely needs to know whether to show the backend selector when backends are unknown. This is minor enough that it can be resolved during implementation. Not re-raising as a finding.

Resolved.

### Finding 8 — LOW R17: Implementation details in API contract — RESOLVED

The document now has a dedicated "Appendix: Implementation Notes" section (lines 1493-1516) containing the shell command details, search paths, directory walk strategy, build commands, and config recommendation internals. These are cleanly separated from the API contract sections (11.1-11.7), which now contain only request/response schemas and behavioral guarantees. Fully resolved.

### Finding 9 — LOW R14: Mode field on status endpoint — RESOLVED

Section 13.6 now uses `setup_required: boolean` on the status response, exactly as recommended. The design explicitly states: "The field is a simple boolean rather than an enum to keep the API surface minimal. Clients do not need to distinguish `Setup` from `Transitioning` — the internal mode state machine is a server-side implementation detail." This is the narrowest possible contract for first-run detection. Fully resolved.

---

## New Interface Evaluation

### preview-config endpoint — Well-Designed

Section 11.6 introduces `POST /api/setup/preview-config` with the same request body as `save-config` but returning `{ "version": 1, "toml": "..." }` without writing to disk. This is a clean read-only projection of the save operation. The shared request body avoids type duplication. The TOML string response is the right abstraction — the frontend renders it without parsing, so the backend owns the serialization format. No findings.

### Build job pattern (POST/GET/stream/DELETE) — Well-Designed

The four-endpoint build pattern is a textbook job abstraction:

- POST creates (idempotent for same backend)
- GET observes (snapshot for reconnection)
- GET /stream observes live (reconnectable SSE with terminal replay)
- DELETE cancels

The `202 Accepted` response with `build_id` follows HTTP semantics correctly. The SSE stream contract subsection provides the behavioral guarantees. Resource controls (jobs cap, nice priority, disk space check) are documented with specific values. Cleanup semantics for every termination path (success, failure, skip, disconnect, timeout) are specified. The 2-minute grace period on SSE disconnect before killing the build is a pragmatic choice that avoids premature termination from tab switches. No findings.

### acquire-binary evolution seam — Well-Designed

Section 11.4 includes: "Future: a separate `POST /api/setup/acquire-binary` with `{ "method": "download", "release": "..." }` for prebuilt binary downloads (currently NG2)." This is the right pattern for documenting a future extension point: a separate endpoint name that won't collide with the existing build endpoint, with a discriminator field (`method`) that makes the intent clear. It signals to implementors that the binary acquisition path will fork without disrupting the build path. No findings.

### source discriminator on scan-models — Well-Designed

Section 11.3 includes `"source": "local"` in the request body with a future note: `{ "source": "huggingface", "query": "...", "quantization": "..." }`. This discriminated request pattern is the right way to design an extensible scan endpoint. The `source` field acts as a type tag that determines which additional fields are valid. When HuggingFace support is added, the endpoint URL stays the same and the response shape (`models[]`) can remain consistent. The `truncated` / `truncation_reason` mechanism also future-proofs for remote sources that may hit rate limits or pagination.

One observation: the `source` field is currently always `"local"` and is mandatory in the request. This is slightly over-specified for v1 — if the only valid value is `"local"`, the field adds ceremony without information. However, establishing the discriminator now prevents a breaking change later when a second source is added. This is a reasonable tradeoff for a pre-1.0 internal API. No findings.

---

## New Findings

### [RULE 15 — Be Explicit About Guarantees] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.3, lines ~680-698)

**What's wrong:** The model scan response documents `truncation_reason` as `"timeout" | "max_results" | "max_depth"` when `truncated` is `true`, and `null` when `truncated` is `false`. However, the `models` array ordering is not specified. The filesystem walk order is implementation-dependent (platform-specific, filesystem-specific), and the `max_results` truncation makes ordering semantically significant: if only 100 of 500 models are returned, which 100? First-found? Alphabetical? Most recently modified?

**Why it matters:** When `truncated` is `true`, the user sees a subset of their models. Without a defined ordering, the subset is arbitrary and non-reproducible. A user who re-runs the scan might get different models in the truncated set. More practically, the frontend's `ModelTable` component likely wants to display models in a useful order (by name, by size, by date). If the backend returns them in walk order, the frontend must sort — but if the backend pre-sorts, the frontend can rely on it.

**Suggested fix:** Add to Section 11.3: "When `truncated` is `true` due to `max_results`, the returned models are those discovered first during the directory walk (depth-first, lexicographic within each directory). The order is deterministic for the same filesystem state but not guaranteed across platforms. The frontend should sort the `models` array client-side for display purposes. The backend does not guarantee any particular ordering."

---

### [RULE 14 — Make Interfaces Narrow] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4, lines ~747-757, and Appendix TypeScript types)

**What's wrong:** The SSE `complete` event includes `log_path` (from Section 11.7's build log persistence), `replaced_existing`, and `backup_path`. The SSE `failed` event also includes `log_path`. These fields tie the SSE event shape to two implementation concerns: file persistence of build logs and binary backup strategy. If build log storage moves from flat files to a database, or if the backup strategy changes to versioned directories, the SSE event shape would need to change.

**Why it matters:** SSE events are consumed by the `BuildProgress` component. Currently the component needs the `binary_path` to advance the wizard (it becomes the binary path for subsequent steps) and the status to update the UI. The `log_path`, `replaced_existing`, and `backup_path` are informational — displayed in the `CompletionScreen` but not used for wizard logic. Bundling informational display fields into the SSE event shape couples the stream contract to the backup/logging implementation.

**Suggested fix:** This is a minor concern for a pre-1.0 internal API. The pragmatic fix is to document that `log_path`, `replaced_existing`, and `backup_path` on SSE events are informational and may change shape in future versions. The `binary_path` and `duration_secs` on `complete`, and `phase`, `error`, `last_lines` on `failed`, are the stable contract fields. This costs zero implementation effort and protects against accidental coupling.

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 0     |
| MEDIUM   | 0     |
| LOW      | 2     |

All 9 prior findings are resolved. The two new LOW findings are documentation-level improvements, not design flaws.

## Commendations

1. **Every prior finding addressed substantively, not cosmetically.** The save-config narrowing, build job pattern, store simplification, and hardware response trimming are all structural changes that genuinely improve the interface design. None were superficially patched.

2. **preview-config as a read-only projection of save-config.** Sharing the request body between preview and save is elegant — it guarantees the preview matches what will be saved, and the frontend uses one type for both calls. This is the right application of the DRY principle at the API boundary.

3. **`?diagnostics=true` query parameter for hardware.** This is a more elegant solution than the separate `/hardware/diagnostics` endpoint that was suggested. It keeps the route surface minimal while making diagnostic data accessible. The default-narrow, opt-in-wide pattern is excellent API design.

4. **Stream Contract subsection.** The five explicit behavioral guarantees (ordering, monotonicity, line completeness, throttling, terminal replay) transform the build SSE from an undefined stream into a contractual interface. This is exactly what Rule 15 asks for.

5. **Downstream invalidation on step re-entry.** The `invalidatesOnReentry` mechanism in the step transition table is a thoughtful addition. It solves a real problem (stale derived data when prerequisite steps change) at the right abstraction level (declarative per-step configuration rather than imperative cache management).

6. **Consistent `version: 1` field across all responses.** This enables backward-compatible API evolution without URL versioning. When the API shape changes, clients can negotiate based on the version field rather than requiring a `/v2/` prefix.

7. **Appendix separation of implementation notes.** Moving shell commands, search paths, and build recipes to a dedicated appendix is exactly the right editorial choice. The API contract sections now read as pure interface specifications.
