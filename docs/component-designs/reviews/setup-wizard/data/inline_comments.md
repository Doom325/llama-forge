<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 729-737 RULE: 11 SEVERITY: HIGH -->
**Rule 11 — Make State Transitions Explicit**
The `"setup" -> "running"` mode transition is described narratively with no explicit state machine. Missing: (1) what state does `GET /api/status` return while hot-reload is in progress? (2) what happens if the hot-reload fails after the config is already written to disk? (3) is there a `"running" -> "setup"` transition for the reconfigure flow, or do the setup endpoints stay mounted in running mode?

Define a three-state machine (`Setup -> Transitioning -> Running`) with explicit transition guards. Specify rollback behavior if hot-reload fails during the `Transitioning` state (restore the previous config? leave the new config on disk but stay in setup mode?). Clarify whether setup endpoints are available in `Running` mode for Flow 4.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 255-269 RULE: 11 SEVERITY: MEDIUM -->
**Rule 11 — Make State Transitions Explicit**
The wizard step progression is an implicit state machine. `completedSteps: Set<WizardStep>` tracks which steps are done, but there is no formal definition of what "done" means per step, what preconditions gate each transition, or what happens to downstream steps when a completed step is re-entered.

Define a transition table with per-step completion predicates and invalidation rules. Example: re-scanning hardware should invalidate the configure and review steps because recommendations depend on hardware info. Without this, stale derived state can leak into the generated TOML.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 679-689 RULE: 12 SEVERITY: MEDIUM -->
**Rule 12 — Maximize Reversibility**
`POST /api/setup/save-config` with `force: true` overwrites the existing config with no backup. The `ConfirmDialog` confirms intent but does not provide reversibility. If the wizard generates a config that causes runtime issues, the operator loses their previous working config.

Add a backup-before-overwrite step: copy the existing file to `<path>.backup.<ISO-timestamp>` before writing the new config. Return the backup path in the response. Show the backup path on the completion screen. Prune backups older than 5 to bound disk usage.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 599-636 RULE: 12 SEVERITY: MEDIUM -->
**Rule 12 — Maximize Reversibility**
The build endpoint copies the compiled binary to `install_dir` on success. If a binary already exists at that path (e.g., a manually compiled version with specific flags), it is overwritten without backup or warning.

Before copying, check if a binary exists at the target path. If so, rename it to `llama-server.backup.<timestamp>`. Include `"replaced_existing": true, "backup_path": "..."` in the SSE `complete` event.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 519-695 RULE: 13 SEVERITY: MEDIUM -->
**Rule 13 — Version Everything**
The seven `/api/setup/*` endpoints have no version field in their request or response payloads, and no `/api/v1/` prefix. While the wizard frontend is co-deployed with the backend (embedded in the binary), adding a `"version": 1` field to responses now is zero-cost and avoids expensive retrofitting later.

At minimum, add `"version": 1` to every response payload. Document that the wizard API contract is internal and unstable.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 289-303 RULE: 10 SEVERITY: MEDIUM -->
**Rule 10 — State Is the Root of All Complexity**
`SetupWizardStore` duplicates server-fetched data that already lives in TanStack Query cache (`hardware`, `binaryPath`, `binaryVersion`). It also stores `completedSteps` which is derivable from the presence of other fields. This creates two sources of truth for the same data.

Narrow the store to client-only state: `selectedModelIds`, `userOverrides` (per-model config changes from defaults), and `savePath`. Derive `completedSteps` from TanStack Query cache status. Let server data live exclusively in TanStack Query.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 679-689 RULE: 10 SEVERITY: LOW -->
**Rule 10 — State Is the Root of All Complexity**
The save-config endpoint accepts the full `AppConfig` structure from the client, but the wizard only populates `[servers.*]` entries. This forces the frontend to construct config sections it does not expose to the user (`priority_rules`, `routing`, `starvation`, etc.), creating tight coupling to the full schema.

Change the endpoint to accept only the wizard's output (server entries + optional global overrides). Have the backend merge them into the existing or default config. This also resolves Open Question 4 (merge vs. replace in reconfigure mode) — the endpoint becomes merge-by-design.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 599-636 RULE: 11 SEVERITY: LOW -->
**Rule 11 — Make State Transitions Explicit**
The build process has server-side phases streamed over SSE, but no specification for: (1) what happens if the SSE connection drops mid-build (does the build continue? can the client reconnect?), (2) how to cancel a running build, (3) recovering build state after reconnection.

Add `GET /api/setup/build/status` for state recovery on reconnect. Add `POST /api/setup/build/cancel` with a `'cancelled'` state in `BuildStatus`. The server-side process should check a cancellation token between phases.

---
