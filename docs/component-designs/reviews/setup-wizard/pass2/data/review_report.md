# Data, State & Schema Review (Pass 2) — LlamaForge Setup Wizard

**Reviewer:** design-review-data
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` — full document (Sections 1-15 + Appendices), with focus on Sections 5 (State Management), 11 (Backend API Contract), 12 (Standalone Wizard Mode), 13 (First-Run Detection and Mode State Machine)
**Verdict:** PASS

## Prior Finding Disposition

### Finding 1 — HIGH R11: First-run mode transition no state machine
**Status: RESOLVED**

Section 13.2 now defines an explicit `Mode` enum with three states (`Setup`, `Transitioning`, `Running`) stored as `Arc<ArcSwap<Mode>>`. The state transition diagram in Section 13.2 is clear:

- `Setup -> Transitioning`: on save-config success + validation-launch pass
- `Transitioning -> Running`: on hot-reload success
- `Transitioning -> Setup`: on hot-reload failure

The design correctly specifies that there is no `Running -> Setup` transition (Section 13.2, final paragraph). The `Transitioning` state is properly gated — inference endpoints return 503 during both `Setup` and `Transitioning` (Section 13.3). The `GET /api/status` endpoint exposes `setup_required: boolean` to abstract the internal state machine from clients (Section 13.6), which is clean — clients do not need to know `Setup` vs `Transitioning`.

The `SetupModeGuard` middleware in Section 13.3 is correctly applied to the inference router only, not the setup or management routers. The reconfigure guard in Section 13.8 requires `X-Setup-Reconfigure: true` header in `Running` mode, which prevents stale wizard tabs from accidentally calling setup endpoints.

No remaining gaps.

---

### Finding 2 — MEDIUM R11: Wizard step progression implicit
**Status: RESOLVED**

Section 5.3 now contains an explicit `STEP_TRANSITIONS` table defining:
- `canAdvance` predicates for each step (e.g., `hardware: (s) => s.hardware !== null`, `models: (s) => s.selectedModels.length > 0`)
- `invalidatesOnReentry` lists specifying which downstream steps are invalidated when a user navigates back

The invalidation mechanism is well-defined: `SetupLayout` calls `queryClient.removeQueries()` for invalidated steps' TanStack Query data and clears associated client state. The `completedSteps` set is derived by iterating the transition table (not stored independently), eliminating the drift risk from the original finding.

The `StepNavigation` component's Next button is disabled unless `STEP_TRANSITIONS[currentStep].canAdvance(store)` returns true, making the transition table the single source of truth for step completion.

No remaining gaps.

---

### Finding 3 — MEDIUM R12: Config overwrite no backup
**Status: RESOLVED**

Section 11.6 now specifies backup-before-overwrite:
- When `force=true` and the target file exists, the handler copies the existing file to `<path>.backup.<ISO-timestamp>`
- The response includes `backup_path` when a backup was created (`null` otherwise)
- The `CompletionScreen` displays the backup path to the user
- Retention policy: maximum 5 backup files per config path, oldest pruned on each save
- Pruning is best-effort (failure logged at WARN, does not block save)
- A `tokio::sync::Mutex` protects the backup + write sequence against concurrent writes (Section 11.6, Write mutex)

The response schema shows both cases: `"backup_path": null` (no existing file) and `"backup_path": "/home/user/.config/llamaforge/llamaforge.toml.backup.2026-06-03T14-30-00"` (backup created).

No remaining gaps.

---

### Finding 4 — MEDIUM R12: Build overwrites binary no backup
**Status: RESOLVED**

Section 11.4 now specifies binary backup before overwrite:
- Before copying the built binary to `install_dir`, the handler checks if a file already exists at the target path
- If it does, the existing binary is renamed to `llama-server.backup.<ISO-timestamp>`
- The SSE `complete` event includes `replaced_existing: true/false` and `backup_path`
- The `CompletionScreen` displays "Previous binary backed up to `<backup_path>`" when `replaced_existing` is `true`

The SSE `complete` event schema is explicitly documented with both the replacement and non-replacement cases.

No remaining gaps.

---

### Finding 5 — MEDIUM R13: API endpoints unversioned
**Status: RESOLVED**

Section 11 now opens with: "All `/api/setup/*` responses include a top-level `"version": 1` field." Every response example in Sections 11.1-11.6 includes `"version": 1` at the top level. The design also documents: "The wizard API contract is internal and unstable until v1.0. When the main API moves to `/api/v1/`, wizard endpoints move with it."

The TypeScript types in the Appendix do not include the `version` field on the response types (e.g., `HardwareInfo`, `ModelScanResult`, `RecommendationResult`). This is a minor omission — the response wrapper likely strips the version field before constructing domain types. However, if `ts-rs` is used (Section 10.7), the Rust response structs will include `version` and the generated TypeScript will too, creating a mismatch with the Appendix types. This is noted as a new LOW finding below.

---

### Finding 6 — MEDIUM R10: SetupWizardStore duplicates TanStack cache
**Status: RESOLVED**

Section 5.3 now explicitly states: "TanStack Query is the sole source of truth for server-fetched data (hardware, binary scan, models, recommendations). The store holds only client-owned state that does not exist in any API response."

The `SetupWizardStore` is pared down to three fields:
- `selectedModelIds` (client-only selection)
- `userOverrides` (client-only form edits)
- `savePath` (client-only path input)

The `ConfigStep` merges server recommendations with `userOverrides` at render time. The `completedSteps` is explicitly called out as derived ("computed, not stored"). No server-fetched data is duplicated in the store.

No remaining gaps.

---

### Finding 7 — LOW R10: save-config accepts full AppConfig
**Status: RESOLVED**

Section 11.6 now shows the save-config request body accepts only `servers` and `global_overrides`:

```json
{
  "path": "...",
  "force": false,
  "servers": [ ... ],
  "global_overrides": { "bind_address": "...", "http_port": ... }
}
```

The design explicitly states: "The frontend never sees or constructs `AppConfig`. The backend is the sole owner of config assembly -- it takes the wizard's server list and global overrides, fills defaults for priority_rules, routing, and other advanced settings, validates through the existing pipeline, and writes."

The `SaveConfigRequest` TypeScript type in the Appendix matches: it has `servers: ServerConfigDraft[]` and `global_overrides?: { bind_address?: string; http_port?: number }` — no `AppConfig` anywhere.

No remaining gaps.

---

### Finding 8 — LOW R11: Build SSE no reconnection/cancellation
**Status: RESOLVED**

Section 11.4 now specifies the full job pattern:
- `POST /api/setup/build` creates the build, returns `build_id` (idempotent — returns existing ID if build already running)
- `GET /api/setup/build/{id}` polls current status (phase, percent, last_lines)
- `GET /api/setup/build/{id}/stream` reconnects to the SSE stream (replays terminal event if build finished)
- `DELETE /api/setup/build/{id}` cancels the build

The `BuildStatus` TypeScript type in the Appendix now includes `'cancelled'` as a status variant. The SSE stream contract is specified in detail (event ordering, monotonic progress, log batching, terminal event replay on reconnect). Section 6.2 specifies SSE reconnect with exponential backoff and jitter (base 1s, max 8s, 3 attempts), then fallback to manual retry.

The grace period for disconnected clients is specified (2 minutes server-side before killing the build). Orphaned subprocess cleanup uses process groups with SIGTERM then SIGKILL.

No remaining gaps.

---

## New Violations Found

### [RULE 13 — Version Everything] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Appendix: New TypeScript Types)

**What's wrong:** The Appendix TypeScript types for API responses (`HardwareInfo`, `ModelScanResult`, `RecommendationResult`, `BinaryScanResult`) do not include the `"version": 1` field that Section 11 mandates on all responses. The `SaveConfigRequest` type also does not show a version field in the response, though the response examples in Section 11.6 include it.

This creates an ambiguity: is `version` stripped by the `apiFetch` wrapper before constructing domain types, or is it part of the domain type and simply omitted from the Appendix? If `ts-rs` is adopted (Section 10.7), the generated TypeScript types will include `version` on the Rust response structs, potentially conflicting with these hand-written Appendix types.

**Why it matters:** The Appendix types serve as the contract between the design doc and the implementation. If they are incomplete, implementers may omit the version field from the TypeScript types entirely, undermining the versioning strategy.

**Suggested fix:** Either:
1. Add `version: number` to each response type in the Appendix, or
2. Define a generic `ApiResponse<T> = { version: number } & T` wrapper and document that all response types are wrapped, or
3. Explicitly note in the Appendix that these are the domain payload shapes and `version` is handled by the `apiFetch` wrapper (stripped on read, not part of domain types). Option 3 is cleanest — it keeps domain types focused and the version check is a cross-cutting concern.

---

### [RULE 11 — Make State Transitions Explicit] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 5.3, STEP_TRANSITIONS table)

**What's wrong:** The `STEP_TRANSITIONS` table's `canAdvance` predicates reference `store` properties (`s.hardware`, `s.binaryPath`, `s.selectedModels`, `s.serverConfigs`) that do not exist on the redesigned `SetupWizardStore`. The store now holds only `selectedModelIds`, `userOverrides`, and `savePath`. The hardware, binary, and model data live in TanStack Query cache.

For example, `hardware: { canAdvance: (s) => s.hardware !== null }` references `s.hardware`, but hardware data is in the TanStack Query cache under `['setup', 'hardware']`, not in the store. The `canAdvance` predicates would need to check query cache state (e.g., `queryClient.getQueryData(['setup', 'hardware']) !== undefined`) or accept additional parameters beyond the store.

**Why it matters:** The transition table is the single source of truth for step completion. If its predicates reference non-existent store properties, the implementation will either (a) re-add those properties to the store (re-introducing the duplication from Finding 6), or (b) change the predicate signatures (breaking the documented contract). This is a minor inconsistency between two parts of the same section.

**Suggested fix:** Update the `canAdvance` predicates to reflect the actual state locations:

```typescript
const STEP_TRANSITIONS: Record<WizardStep, {
  canAdvance: (store: SetupWizardStore, queryClient: QueryClient) => boolean;
  invalidatesOnReentry: WizardStep[];
}> = {
  welcome:   { canAdvance: () => true, invalidatesOnReentry: [] },
  hardware:  { canAdvance: (_, qc) => qc.getQueryData(['setup', 'hardware']) != null, invalidatesOnReentry: ['configure', 'review'] },
  binary:    { canAdvance: (_, qc) => qc.getQueryData(['setup', 'scan-binary'])?.found === true, invalidatesOnReentry: ['configure', 'review'] },
  models:    { canAdvance: (s) => s.selectedModelIds.size > 0, invalidatesOnReentry: ['configure', 'review'] },
  configure: { canAdvance: (s) => s.userOverrides.size > 0 && allFieldsValid(s), invalidatesOnReentry: ['review'] },
  review:    { canAdvance: () => true, invalidatesOnReentry: [] },
};
```

Or add a note that the predicates are pseudocode showing intent, not literal implementation.

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 0     |
| MEDIUM   | 0     |
| LOW      | 2     |

All 8 prior findings are fully resolved. The two new LOW findings are minor inconsistencies in the Appendix and a predicate/store mismatch within Section 5.3 — neither blocks implementation.

## Commendations

1. **Mode state machine is excellent (Section 13.2).** The three-state `Mode` enum with `Arc<ArcSwap<Mode>>` is exactly the right pattern. The transition diagram is clear, the failure path (`Transitioning -> Setup` on hot-reload failure) is specified, and the decision to keep `Running` permanent (no `Running -> Setup` transition) simplifies reasoning about the system.

2. **Build job pattern is thorough (Section 11.4).** The POST/GET/GET-stream/DELETE pattern with idempotent POST, reconnectable SSE, terminal event replay, and 2-minute grace period is production-grade. The process group cleanup with SIGTERM-then-SIGKILL is the correct approach for orphaned subprocess management.

3. **Config backup with bounded retention (Section 11.6).** The 5-backup retention limit with best-effort pruning is well-calibrated — enough backups for "oops" recovery without unbounded disk growth. The `tokio::sync::Mutex` protecting the backup+write sequence prevents the subtle interleaving corruption that concurrent saves could cause.

4. **Narrow save-config contract (Section 11.6).** Having the backend own config assembly is the right architectural decision. The frontend sends only what it knows (servers + global overrides), and the backend fills defaults and validates. This cleanly separates concerns and makes the reconfigure flow (merge, not replace) natural.

5. **Validation-launch step (Section 13.5).** The 4-step pipeline (pre-validate, write, spawn-test-server, hot-reload) is rigorous. Returning 200 with `validation_launch.success: false` when the config is written but the test server fails is the correct HTTP semantic — the resource was created, but the caller needs to know it may not work. This avoids the common anti-pattern of overloading HTTP status codes for business-logic outcomes.

6. **Downstream invalidation on re-entry (Section 5.3).** The `invalidatesOnReentry` mechanism that removes TanStack Query data and forces re-fetch when navigating back to a completed step is the correct solution to the stale-derived-state problem from the original finding. It ensures that changing hardware detection results always propagates to config recommendations.

7. **Session correlation without server-side session state (Section 11.7).** Using `X-Setup-Session-Id` as a pure log-correlation token — no server-side session storage, no cookies — is the right trade-off for a localhost-only wizard. It provides the observability benefit (grep by session UUID) without the complexity of session management.
