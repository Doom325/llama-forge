# Data, State & Schema Review — LlamaForge Setup Wizard

**Reviewer:** design-review-data
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` (Sections 5, 11, 12, 13), cross-referenced against `crates/llamaforge-core/src/config/types.rs`, `config/error.rs`, `config/handle.rs`, `config/source.rs`, and `config/traits.rs`
**Verdict:** PASS WITH WARNINGS

## Violations Found

### [RULE 11 — Make State Transitions Explicit] — Severity: HIGH

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 13, lines 729–737)

**What's wrong:** The first-run mode transition from `"setup"` to `"running"` is described narratively ("On config save, the server hot-reloads the new config and transitions to `"running"` mode") but the `mode` field has no explicit state machine definition, no enumeration of valid transitions, and no specification of what happens if the transition fails partway through (e.g., config saves successfully to disk but the hot-reload rejects it because a port is already in use by another process on the system).

**Why it matters:** The `"setup" -> "running"` transition is a one-way door in practice — once the server spawns llama-server instances and begins accepting inference requests, reverting to setup mode requires stopping those instances. Without an explicit state machine, the design leaves a gap for a stuck intermediate state where the config is written but the mode never transitions (the UI would still show the wizard while the server has a valid config). Additionally, there is no defined transition for `"running" -> "setup"` to support the reconfigure flow (Flow 4, Section 2.1), even though Section 13 implies the setup endpoints are only mounted during setup mode.

**Suggested fix:** Define the server mode as an explicit state machine with three states and guarded transitions:

```
Mode::Setup
  -> Mode::Transitioning  (on save-config success, before hot-reload)
  -> Mode::Running         (on hot-reload success)
  -> Mode::Setup           (on hot-reload failure; rollback config file)

Mode::Running
  (setup endpoints remain mounted for Flow 4 reconfigure;
   mode stays Running — reconfigure is just a config writeback,
   not a mode transition)
```

Specify what `GET /api/status` returns during the `Transitioning` state, and what the UI should show (a "Starting servers..." interstitial, not the wizard and not the dashboard).

---

### [RULE 11 — Make State Transitions Explicit] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 5.1, lines 255–269)

**What's wrong:** The wizard's step progression is an implicit state machine. Section 5.1 inventories the state pieces but does not define which transitions between steps are valid, what preconditions gate each transition, or what happens when a precondition fails. The prose in Section 2.1 says "the user can't advance until the current step completes" but there is no formal definition of "completes" per step — is it "API returned 200"? "User confirmed results"? "At least one model selected"?

The `completedSteps` field is a `Set<WizardStep>` (Section 5.3), but `WizardStep` is just a union of string literals with no defined ordering or transition rules. The step indicator allows clicking completed steps to go back, but there is no specification of whether going back to a completed step invalidates subsequent steps (e.g., if the user goes back to Hardware and re-scans with different results, are the model recommendations in the Configure step stale?).

**Why it matters:** Without explicit transition guards, the implementation will scatter `if (completedSteps.has('hardware'))` checks across multiple components, creating the exact "implicit state machine with scattered guards" anti-pattern that Rule 11 warns against. Stale derived state after back-navigation is a data integrity concern — the generated TOML could contain recommendations computed against outdated hardware info.

**Suggested fix:** Define a `WizardStateMachine` with explicit transitions and preconditions:

```typescript
// Transition table — each step defines its completion predicate
// and which downstream steps it invalidates on re-entry
const STEP_TRANSITIONS: Record<WizardStep, {
  canAdvance: (store: SetupWizardStore) => boolean;
  invalidatesOnReentry: WizardStep[];
}> = {
  welcome:   { canAdvance: () => true, invalidatesOnReentry: [] },
  hardware:  { canAdvance: (s) => s.hardware !== null, invalidatesOnReentry: ['configure', 'review'] },
  binary:    { canAdvance: (s) => s.binaryPath !== null, invalidatesOnReentry: ['configure', 'review'] },
  models:    { canAdvance: (s) => s.selectedModels.length > 0, invalidatesOnReentry: ['configure', 'review'] },
  configure: { canAdvance: (s) => s.serverConfigs.size > 0 && allValid(s), invalidatesOnReentry: ['review'] },
  review:    { canAdvance: () => true, invalidatesOnReentry: [] },
};
```

When a user re-enters a completed step, invalidate the downstream steps listed and clear their cached TanStack Query data, forcing re-fetch of recommendations.

---

### [RULE 12 — Maximize Reversibility] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.6, lines 679–689)

**What's wrong:** The save-config endpoint has a `force: false` parameter and returns 409 if the file exists, which is good. However, there is no backup-before-overwrite mechanism. When `force: true` is sent, the existing config is silently replaced with no recovery path. The design mentions the `ConfirmDialog` asking before overwriting (Section 5.6), but confirmation is not reversibility — the user gets a "are you sure?" prompt and then the old config is gone.

This is particularly risky in the reconfigure flow (Flow 4, Section 2.1), where a running production system's config is being rewritten by the wizard. If the wizard generates an invalid config that somehow passes client-side validation but fails at runtime (e.g., a model path that exists during scanning but is deleted by the time the server tries to load it), the operator loses their working config.

**Why it matters:** Config files are small but high-value — they encode operator decisions about GPU offload, context sizes, port assignments, and priority rules that may have been tuned over time. Losing them to a wizard overwrite with no undo is a one-way door on data the user spent time creating.

**Suggested fix:** Before overwriting an existing config, the backend should create a timestamped backup:

```
POST /api/setup/save-config (with force=true and file exists)
  1. Copy existing file to <path>.backup.<ISO-timestamp>
  2. Write new config via the existing crash-safe writeback path
  3. Return { "path": "...", "backup_path": "...backup.2026-06-03T10:30:00Z" }
```

The completion screen should show: "Previous config backed up to `<backup_path>`." Optionally, add a "Restore previous config" button on the completion screen that copies the backup back. Keep at most 5 backups (prune oldest) to avoid unbounded disk growth.

---

### [RULE 12 — Maximize Reversibility] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4, lines 599–636)

**What's wrong:** The build endpoint clones the llama.cpp repository and compiles from source. On build failure, the user can retry or skip, which is good. However, on build *success*, the binary is copied to `install_dir` (e.g., `/usr/local/bin/llama-server`). If the user already had a `llama-server` binary at that path (perhaps a different version they compiled with different flags), it is overwritten without backup and without warning.

Open Question 2 (Section 14) acknowledges the clone directory cleanup question but does not address overwriting an existing binary.

**Why it matters:** Overwriting a working binary with a freshly compiled one is a one-way door. If the new binary has a subtle issue (wrong CUDA version, missing backend), the operator loses their known-good binary.

**Suggested fix:** Before copying the built binary to `install_dir`:
1. Check if a binary already exists at the target path.
2. If it does, rename it to `llama-server.backup.<timestamp>` (same pattern as config backup).
3. Include this in the SSE `complete` event: `"replaced_existing": true, "backup_path": "..."`.
4. The wizard completion screen shows: "Previous binary backed up to `<backup_path>`."

---

### [RULE 13 — Version Everything] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11, all subsections)

**What's wrong:** The seven new API endpoints under `/api/setup/*` are unversioned. There is no `/api/v1/setup/*` prefix and no version field in request/response payloads. The design does not discuss backward compatibility strategy for these endpoints.

While the wizard APIs are local-only and the frontend is always co-deployed with the backend (embedded in the binary), versioning matters for: (a) the reconfigure flow where a newer UI version might talk to an older running server during a staged upgrade, (b) future tooling or scripts that call these endpoints directly, and (c) establishing the pattern before it becomes costly to retrofit.

**Why it matters:** The existing main API already lives at `/api/...` without versioning (inherited from the main UI architecture). Adding seven more unversioned endpoints deepens the debt. If the recommendation algorithm changes its output shape (e.g., adding a `memory_bandwidth_mb` field to `ConfigRecommendation`), there is no mechanism for the frontend to know whether the backend supports it.

**Suggested fix:** This is LOW urgency because the wizard is local-only with co-deployed frontend/backend, but the design should at minimum:
1. Add a `"version": 1` field to the top level of every response payload (cheap, zero-cost to add now, expensive to retrofit).
2. Document that the wizard API contract is internal and unstable until v1.0 ships.
3. If the main API eventually moves to `/api/v1/`, the wizard endpoints should move with it.

---

### [RULE 10 — State Is the Root of All Complexity] — Severity: MEDIUM

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 5.3, lines 289–303)

**What's wrong:** The `SetupWizardStore` holds both source-of-truth state and derived/cached state in the same flat structure without distinguishing between them. Specifically:

- `hardware`, `binaryPath`, `binaryVersion` — these are cached copies of server responses (also held in TanStack Query cache).
- `selectedModels` — this is true client-only state.
- `serverConfigs` — this is derived from server recommendations + user overrides, but stored as a flat `Map` with no way to distinguish which values are user-modified vs. server-recommended.
- `savePath` — true client-only state.
- `completedSteps` — this is derivable from the presence of other fields (hardware !== null implies hardware step is complete).

This means two sources of truth exist for server-fetched data: TanStack Query cache and the `SetupWizardStore`. Section 5.2 sets `staleTime: Infinity`, so they should stay in sync, but on page refresh the TanStack Query cache is lost while the store is also lost (Section 9.2 says no persistence) — so they are consistently ephemeral. The redundancy is not dangerous but it is unnecessary complexity.

**Why it matters:** Dual source of truth for the same data creates a surface for consistency bugs. If a refetch happens (e.g., user manually triggers re-scan), the TanStack Query cache updates but the store must be manually synchronized. The `completedSteps` set can drift from the actual data presence — a step could be marked "complete" but its data could be null if there is a bug in the sync logic.

**Suggested fix:** Make `SetupWizardStore` hold only client-owned state that does not exist in any server response:

```typescript
export class SetupWizardStore {
  // Client-only selections
  selectedModelIds = $state<Set<string>>(new Set());
  userOverrides = $state<Map<string, Partial<ServerConfigDraft>>>(new Map());
  savePath = $state<string>('');

  // Derived — no duplication of server data
  get completedSteps(): Set<WizardStep> {
    // Derive from TanStack Query cache state
  }
}
```

Let TanStack Query be the sole source of truth for hardware info, binary scan results, and discovered models. The `ConfigStep` merges server recommendations with `userOverrides` at render time rather than storing merged `serverConfigs`.

---

### [RULE 10 — State Is the Root of All Complexity] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.6, lines 679–689)

**What's wrong:** The save-config endpoint accepts `"config": { /* full AppConfig structure */ }` — the entire config object is assembled on the client and sent as a JSON payload. But `AppConfig` (in `types.rs`) has 9 top-level sections with deeply nested structures including `PriorityRulesConfig`, `RoutingConfig`, `StarvationConfig`, etc. The wizard only configures `[servers.*]` entries and leaves everything else at defaults. Sending the full `AppConfig` from the client means the frontend must construct sections it does not expose to the user (`priority_rules`, `routing`, `starvation`, `preemption`, `cors`, `grpc`, `prometheus`, `gateway`).

**Why it matters:** This creates a tight coupling between the frontend wizard and the full `AppConfig` schema. If a new section is added to `AppConfig` (e.g., a `[cluster]` section for future multi-node support), the wizard's config generation must be updated even though it has nothing to do with clustering. More importantly, in the reconfigure flow (Flow 4), constructing the full `AppConfig` on the client risks overwriting sections the wizard does not display — the operator's hand-tuned `priority_rules` could be reset to defaults.

**Suggested fix:** Change the save-config endpoint to accept only the wizard's output — the server entries — and have the backend merge them into the existing (or default) config:

```
POST /api/setup/save-config
{
  "path": "...",
  "servers": { /* only the [servers.*] entries */ },
  "global_overrides": {  /* optional: only fields the wizard actually sets */
    "bind_address": "127.0.0.1",
    "http_port": 8000
  },
  "force": false
}
```

The backend loads the existing config (or creates a default), merges the wizard's servers and any global overrides, validates the merged result through the existing pipeline, and writes it. This also directly resolves Open Question 4 (merge vs. replace in reconfigure mode) — the endpoint is merge-by-design.

---

### [RULE 11 — Make State Transitions Explicit] — Severity: LOW

**File:** `docs/component-designs/08_SETUP_WIZARD.md` (Section 11.4, lines 599–636)

**What's wrong:** The build process has phases (`cloning`, `configuring`, `building`) that are streamed as SSE events, but the design does not specify what happens if the SSE connection drops mid-build. The build is a server-side long-running process — does it continue? Can the client reconnect and see current progress? Is there a way to cancel a running build?

The `BuildStatus` TypeScript type (Appendix) has `'idle' | 'running' | 'complete' | 'failed'` but no `'cancelled'` state. Section 6.2 mentions SSE reconnect with exponential backoff (3 attempts), but after reconnection, can the client recover the build's current state, or does it see `'idle'`?

**Why it matters:** A build can take several minutes. If the user accidentally closes the tab or the SSE connection drops due to a network hiccup (even on localhost, browser throttling of background tabs can close EventSource connections), they lose visibility into a process that is still running and may have already consumed significant CPU time.

**Suggested fix:** Add a `GET /api/setup/build/status` endpoint that returns the current build state (idle, running with phase/percent, complete, failed). On SSE reconnect, the client first hits this endpoint to recover state, then re-subscribes to the SSE stream for incremental updates. Add a `POST /api/setup/build/cancel` endpoint and a `'cancelled'` state to `BuildStatus`. The server-side build process should check a cancellation token between phases.

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 1     |
| MEDIUM   | 5     |
| LOW      | 2     |

## Commendations

1. **Excellent state inventory (Section 5.1).** The table mapping each piece of state to its source of truth, modifier, and scope is exactly the right level of rigor. Most design docs omit this entirely. This table makes the review possible.

2. **No optimistic updates (Section 5.5).** The explicit decision to avoid optimistic updates for the save-config mutation is the right call. The reasoning ("a failed save is not recoverable without user intervention") demonstrates mature thinking about failure modes.

3. **ConfirmDialog for overwrite (Section 5.6).** Using the existing `ConfirmDialog` component and checking `--force` before overwriting shows awareness of the destructive nature of the save operation. The gap is in backup, not in confirmation.

4. **Double-submit protection (Section 5.6).** Disabling the "Save & Finish" button and showing a spinner during the save request prevents the common bug of duplicate config writes.

5. **TanStack Query with `staleTime: Infinity` (Section 5.2).** This is the correct caching strategy for wizard data that is fetched once and does not change during the session. It avoids unnecessary refetches while still allowing explicit re-fetch on back-navigation.

6. **Client store scoped to wizard lifetime (Section 5.3).** Using Svelte's context API to scope the store to the wizard component tree and garbage-collect it on unmount is clean lifecycle management that avoids leaking state into the global scope.

7. **Discriminated union for `BinaryScanResult` and `BuildStatus` (Appendix).** Using discriminated unions (`found: true | found: false`, status variants) instead of nullable fields is the right pattern for modeling exclusive states in TypeScript.
