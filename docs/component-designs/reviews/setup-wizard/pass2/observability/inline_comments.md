# Inline Comments — Observability Review (Pass 2)

No violations found. All 6 first-pass findings have been resolved in the updated design document.

---

## Resolution Notes

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 885-945 RULE: 19 SEVERITY: RESOLVED -->
**Finding 1 (was HIGH R19): No backend logging specified**
Section 11.7 "Backend Observability" now provides per-endpoint structured logging specs with explicit field names, log levels, and truncation policies for all seven endpoint categories. R26 compliance (no telemetry in domain logic) is explicitly stated. Fully resolved.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 929-936 RULE: 19 SEVERITY: RESOLVED -->
**Finding 2 (was HIGH R19): Build logs not persisted to disk**
Build Log Persistence subsection specifies tee to `$XDG_DATA_HOME/llamaforge/build-logs/`, `log_path` field in SSE `complete` and `failed` events, UI display of path with bug report guidance, and retention policy (5 builds or 30 days). Fully resolved.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 938-945 RULE: 20 SEVERITY: RESOLVED -->
**Finding 3 (was MEDIUM R20): No session correlation**
Session Correlation subsection specifies `X-Setup-Session-Id` header with frontend UUID generation, backend extraction into top-level `tracing::info_span!`, `"anonymous"` fallback, and no server-side session state. Fully resolved.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 639,893-896 RULE: 19 SEVERITY: RESOLVED -->
**Finding 4 (was MEDIUM R19): Raw hardware detection output not preserved**
`?diagnostics=true` query parameter added to Section 11.1. INFO-level raw stdout logging with 4 KB truncation and ERROR-level raw stderr logging specified in Section 11.7. Fully resolved.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 695,906-909 RULE: 19 SEVERITY: RESOLVED -->
**Finding 5 (was MEDIUM R19): Model scan silently skips errors**
WARN-level logging for permission errors and symlink cycles in Section 11.7. `warnings` field added to model scan response in Section 11.3 with concrete examples. DEBUG-level logging for symlink follow/skip decisions and hidden directory skips. Fully resolved.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 987-991 RULE: 19 SEVERITY: RESOLVED -->
**Finding 6 (was LOW R19): Standalone mode logging not specified**
Section 12.3 "Logging Posture" specifies `tracing_subscriber` with `EnvFilter`, default WARN to stderr, always-emitted INFO startup message with `RUST_LOG=debug` hint, and build log persistence independent of terminal log level. Fully resolved.
