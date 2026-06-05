# Observability & Operational Visibility Review (Pass 2) — LlamaForge Setup Wizard

**Reviewer:** design-review-observability
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` (full design document, Sections 1-15 + appendices)
**Verdict:** PASS

---

## Context Note

This is a second-pass review. The first pass gave a FAIL verdict with 2 HIGH, 3 MEDIUM, and 1 LOW finding. All six findings targeted the absence of backend observability specifications for the wizard's API endpoints. This pass verifies that the fixes landed correctly and checks whether the new material introduced any new concerns.

Rules 18 (Metrics/Percentiles) and 21 (Actionable Alerts) remain not applicable to a setup wizard and are not evaluated. Rule 20 (Distributed Tracing) is evaluated in reduced form (session correlation). The review focuses on Rule 19 (Structured Logging with Context) and Rule 20 (Session Correlation).

---

## First-Pass Finding Resolution

### Finding 1 — HIGH R19: No backend logging specified at all

**Status: RESOLVED**

Section 11.7 "Backend Observability" now provides comprehensive per-endpoint logging specifications. Each of the seven endpoint categories (hardware detection, binary scan, model scan, config recommendation, save config, build process) has explicit log-level assignments (INFO/WARN/ERROR/DEBUG) with structured key-value field names. The cross-cutting span uses `tracing::info_span!` at the adapter boundary with `endpoint` and `setup_session_id` fields, consistent with R26 (no telemetry in domain logic). The opening paragraph of 11.7 explicitly states "No `tracing` calls in domain logic -- instrumentation lives at the adapter boundary (the axum handler), consistent with R26."

All log emissions use structured fields (e.g., `command = %cmd`, `gpus_found = 1`, `path = %p, error = %e`), not string interpolation. This matches the project's `tracing` conventions.

---

### Finding 2 — HIGH R19: Build logs not persisted to disk

**Status: RESOLVED**

Section 11.7 under "Build Log Persistence" specifies:

- Tee to `$XDG_DATA_HOME/llamaforge/build-logs/build-<ISO-timestamp>.log` for all stdout and stderr from git clone, cmake configure, and cmake build.
- Persistence operates independently of terminal log level ("even when `RUST_LOG=warn` suppresses verbose output to stderr, the build log file captures everything").
- The SSE `complete` event includes `log_path` field.
- The SSE `failed` event includes `log_path` field.
- Build failure UI displays the path with "Include this file in bug reports."
- Retention policy: last 5 builds or 30 days, whichever retains more. Cleanup is best-effort at next build start.

This directly addresses the original finding. The `log_path` field appears in the SSE event examples in Section 11.4 (lines 933-934), confirming the contract change is reflected in the API spec, not just the observability section.

---

### Finding 3 — MEDIUM R20: No session correlation

**Status: RESOLVED**

Section 11.7 under "Session Correlation" specifies:

- Frontend generates UUID v4 in `SetupLayout`, stored in `SetupWizardStore`.
- All wizard API calls attach `X-Setup-Session-Id` header.
- Backend extracts header into `setup_session_id` field in the top-level `tracing::info_span!`.
- Missing header falls back to `"anonymous"` (handles direct curl calls).
- No server-side session state -- purely a log correlation token.
- Usage example: `grep setup_session_id=<uuid>` reconstructs the full setup flow.

The design correctly scopes this as a log-only correlation mechanism without introducing server-side session state, which would have been unnecessary complexity for a setup wizard.

---

### Finding 4 — MEDIUM R19: Raw hardware detection output not preserved

**Status: RESOLVED**

Two complementary mechanisms are now specified:

1. **Logging (Section 11.7, Hardware Detection):** INFO-level logging of raw stdout truncated to 4 KB (`stdout = %truncate(raw, 4096)`), exit code, and parsed result summary. ERROR-level logging of raw stderr on command failure.
2. **API response (Section 11.1):** The `?diagnostics=true` query parameter returns diagnostic fields (driver versions, architecture details) for bug reports.

The 4 KB truncation on raw stdout is a reasonable bound -- `nvidia-smi` output is typically well under 1 KB, and `system_profiler` output for GPU info is under 2 KB. The truncation prevents pathological cases from bloating logs while preserving enough for diagnosis.

---

### Finding 5 — MEDIUM R19: Model scan silently skips errors

**Status: RESOLVED**

Two complementary mechanisms are now specified:

1. **Logging (Section 11.7, Model Scan):** WARN-level logging for permission errors (`path = %p, error = %e, kind = "permission_denied"`) and symlink cycles (`path = %p, kind = "symlink_cycle"`). DEBUG-level for symlink follow/skip decisions and hidden directory skips. INFO-level for per-directory file counts.
2. **API response (Section 11.3):** The `warnings` field in the model scan response includes user-visible issues (e.g., `"Permission denied: /data/models/private/"`, `"Symlink cycle detected: /data/models/loop"`).

The design correctly separates user-facing warnings (in the API response, surfaced by the UI) from developer-facing diagnostics (in structured logs at DEBUG level). This means the user sees "Permission denied on /data/models/private/" in the wizard, and the developer can see the full walk decisions by enabling DEBUG logging.

---

### Finding 6 — LOW R19: Standalone mode logging not specified

**Status: RESOLVED**

Section 12.3 "Logging Posture" specifies:

- `tracing_subscriber` with `EnvFilter` for `RUST_LOG` support.
- Default log level: `WARN` to stderr (quiet terminal).
- Single INFO-level startup message always emitted regardless of `RUST_LOG`: the URL and a hint to use `RUST_LOG=debug`.
- Build log file persistence operates independently of terminal log level.

The decision to always emit the startup URL at INFO level (overriding the WARN default) is a good UX choice -- the user needs the URL even in quiet mode. The `RUST_LOG=debug` hint gives a clear path to verbose output for diagnostics.

---

## New Material Review

The new observability material (Section 11.7 and Section 12.3) was reviewed for internal consistency and for any new concerns introduced by the fixes.

### No new violations found.

The new sections are well-structured and internally consistent. Specific observations:

- The per-endpoint logging specifications use consistent patterns (command executed, raw output, parsed result, error chain) across all endpoints.
- Log level assignments follow a sensible hierarchy: INFO for normal operations and results, WARN for recoverable issues surfaced to users, ERROR for failures, DEBUG for internal decisions (symlink resolution, hidden directory skipping).
- The 4 KB truncation on raw command output is applied consistently across hardware detection and build process logging.
- The build log retention policy (5 builds or 30 days) aligns with the config backup retention policy (5 backups) established in Section 11.6, showing consistent policy choices.
- The `setup_session_id` correlation field is placed in the top-level span, meaning it automatically propagates to all child spans without explicit threading through every function call.
- Section 12.3's logging posture is consistent with the main `llamaforge serve` mode's use of `tracing_subscriber` and `EnvFilter`.

---

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | 0     |
| MEDIUM   | 0     |
| LOW      | 0     |

All 6 findings from the first pass are fully resolved. No new violations introduced.

---

## Commendations

1. **Section 11.7 is thorough and well-organized.** The per-endpoint logging specifications are detailed enough to implement directly -- each endpoint has explicit field names, log levels, and truncation policies. This is not a vague "add logging" instruction; it is a concrete specification.

2. **Build log persistence is comprehensive.** The design covers the full lifecycle: file creation, path reporting in SSE events, UI display of the path, retention policy, and best-effort cleanup. The independence from `RUST_LOG` terminal level is a critical detail that ensures build logs are always captured regardless of how the user configured their terminal output.

3. **Session correlation is appropriately scoped.** The `X-Setup-Session-Id` mechanism provides end-to-end correlation without introducing server-side session state. The `"anonymous"` fallback for missing headers is pragmatic -- it handles direct API calls without breaking the logging contract.

4. **Raw output preservation balances completeness with safety.** The 4 KB truncation on raw command output prevents log bloat from pathological cases while preserving enough for diagnosis. The `?diagnostics=true` query parameter provides a user-accessible path to detailed hardware info for bug reports.

5. **Standalone mode logging posture is well-calibrated.** Defaulting to WARN keeps the terminal clean while the user works in the browser, but the always-emitted startup URL and the `RUST_LOG=debug` hint ensure the user is never left without a path to diagnostics. Build log persistence independent of terminal level is the right design choice.
