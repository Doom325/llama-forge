# Inline Comments — Scaling Pass 2

No findings. All 7 first-pass findings are resolved. No new violations discovered.

## Resolution Notes

Below is the per-finding resolution trace for the record. No action required.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 1040-1125 RULE: 26 SEVERITY: n/a STATUS: RESOLVED -->
**Finding 1 (was HIGH R26): Setup mode blast radius leaks into inference API**

Resolved by Section 13.1 (Config Load Triage — explicit enumeration of `ConfigLoadError` variants with `ParseError` hard-exiting), Section 13.3 (503 on gRPC with status 14 UNAVAILABLE + setup-url metadata; 503 on HTTP with `Retry-After: 60` + JSON body), and the `SetupModeGuard` tower middleware applied to inference router only.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 776-797 RULE: 26 SEVERITY: n/a STATUS: RESOLVED -->
**Finding 2 (was MEDIUM R26): Build resource consumption unbounded**

Resolved by Section 11.4 "Resource controls": jobs cap at `min(requested, num_cpus / 2)`, `nice 10` on subprocess, 5 GB disk check before clone (returns 409), first-run confirmation dialog. Tests in Section 10.6 cover boundary cases.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 668-701 RULE: 28 SEVERITY: n/a STATUS: RESOLVED -->
**Finding 3 (was MEDIUM R28): No evolution seams for NG1/NG2**

Resolved by `"source": "local"` discriminator on scan-models (Section 11.3), future `POST /api/setup/acquire-binary` noted (Section 11.4), and TypeScript types including the discriminator from day one (Appendix).

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 668-701 RULE: 27 SEVERITY: n/a STATUS: RESOLVED -->
**Finding 4 (was MEDIUM R27): Model scanner no bounds**

Resolved by `max_depth` (default 5), `max_results` (default 100), 30-second backend timeout, `truncated` boolean, and `truncation_reason` enum in the response (Section 11.3). Backend-enforced timeout replaces the suggested client-tunable `timeout_ms` — acceptable trade-off.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 1133-1232 RULE: 28 SEVERITY: n/a STATUS: RESOLVED -->
**Finding 5 (was MEDIUM R28): No rollback path for setup-to-running**

Resolved by the four-step save-config flow (Section 13.5): pre-validation before write, validation-launch after write (spawn first server, 30s health check, kill on success/failure), hot-reload with `Transitioning` state and `Setup` fallback on failure. CompletionScreen verification (Section 13.7) gates dashboard navigation on multiple checks.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 949-972 RULE: 27 SEVERITY: n/a STATUS: RESOLVED -->
**Finding 6 (was LOW R27): Dual-boot mode complexity**

Resolved by shared `setup_router()` function (Section 12.1) used identically in both standalone and first-run mode. Setup endpoints in `Running` mode require `X-Setup-Reconfigure: true` header (Section 13.8), returning 403 without it.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 789-797 RULE: 26 SEVERITY: n/a STATUS: RESOLVED -->
**Finding 7 (was LOW R26): Orphaned build subprocesses**

Resolved by Section 11.4 "Orphaned subprocess cleanup": process group via `setsid`/`setpgid`, `SIGTERM` + 5s wait + `SIGKILL` on shutdown or SSE disconnect grace period expiry. Section 10.8 includes a test verifying no orphaned processes after SIGINT.

---
