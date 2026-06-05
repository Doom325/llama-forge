# Design Review — Pass 2 (Re-Review)

**Project:** LlamaForge Setup Wizard
**Date:** 2026-06-03
**Scope:** `docs/component-designs/08_SETUP_WIZARD.md` — full document after addressing 39 findings from Pass 1
**Overall Verdict:** PASS WITH WARNINGS

---

## Executive Summary

All 39 findings from the first pass (9 HIGH, 18 MEDIUM, 12 LOW) are **resolved**. The observability domain — which previously received a FAIL verdict — now passes cleanly. The design is comprehensive, internally consistent, and ready for implementation. The second pass found only **9 new LOW-severity findings**, all documentation-level nits introduced by the edits — no architectural gaps, no missing failure paths, no new HIGH or MEDIUM issues. The added complexity (Mode state machine, build job pattern, step transition table, session correlation) was evaluated by the simplicity sub-agent and judged to earn its place.

---

## Prior Findings Resolution

**All 39 findings from Pass 1 are RESOLVED:**

| Domain | Pass 1 | Resolved | New |
|--------|--------|----------|-----|
| Resilience | 10 (3H, 4M, 3L) | 10/10 | 2 LOW |
| Simplicity | 6 (0H, 2M, 4L) | 5/6 partial* | 3 LOW |
| Data & State | 8 (1H, 5M, 2L) | 8/8 | 2 LOW |
| Interfaces | 9 (1H, 5M, 3L) | 9/9 | 2 LOW |
| Observability | 6 (2H, 3M, 1L) | 6/6 | 0 |
| Testing | 10 (3H, 5M, 2L) | 10/10 | 2 LOW |
| Scaling | 7 (1H, 4M, 2L) | 7/7 | 0 |

*Simplicity partial: check-path endpoint removed from spec but a stale route reference may remain in `setup_router()`.

---

## New Findings (LOW only)

### Resilience

1. **LOW R2:** Validation-launch server kill has no timeout on the kill itself. Should use SIGTERM → 5s → SIGKILL (same pattern as build cleanup).
2. **LOW R2:** Port availability check in pre-validation is TOCTOU-prone. Add a doc note that the validation-launch is the authoritative check; the pre-check is advisory.

### Simplicity

3. **LOW R6:** Stale `check-path` route may remain in `setup_router()` definition — remove it for consistency with the deleted Section 11.7.
4. **LOW R6:** Dual boot path trade-off (two server assemblies sharing one `setup_router()`) should be explicitly documented as a deliberate choice.
5. **LOW R7:** Section 2.2 screen table still mentions "progress estimation" for the Build screen, contradicting Section 4.4 which rejects time estimation in favor of factual progress.

### Data & State

6. **LOW R13:** TypeScript types appendix omits the `version` field that Section 11 mandates on all responses.
7. **LOW R11:** `STEP_TRANSITIONS` predicates reference store properties (`s.hardware`, `s.binaryPath`) that no longer exist on the redesigned lean store — they should reference TanStack Query cache state instead.

### Interfaces

8. **LOW R15:** Model scan array ordering is unspecified when results are truncated.
9. **LOW R14:** SSE `complete`/`failed` events bundle informational fields (`log_path`, `backup_path`) without marking them as unstable/optional.

### Testing

10. **LOW (bonus):** Build cancel flow (`DELETE /api/setup/build/{id}`) and step invalidation on back-navigation lack dedicated integration tests.

---

## Commendations

1. **Clean resolution of the observability gap.** The new Section 11.7 is thorough — per-endpoint structured logging with specific fields, build log persistence with retention policy, and session correlation. This went from a FAIL to a clean PASS.

2. **Mode state machine is well-designed.** `Setup → Transitioning → Running` with `ArcSwap`, bounded Transitioning duration, clean rollback, and 503 guards on inference endpoints. The simplicity sub-agent confirmed this earns its complexity.

3. **Build job pattern is textbook.** POST (create) + GET (status) + GET/stream (reconnectable SSE) + DELETE (cancel) with idempotent POST and terminal event replay. This is exactly how long-running operations should be modeled.

4. **Validation-launch step prevents the "Ready! ...but broken" scenario.** Spawning one server and waiting for a health check before showing success is the right design for a zero-knowledge onboarding tool.

5. **Backup-before-overwrite on both config and binary.** Timestamped backups with retention limits and UI display of backup paths. Simple, effective reversibility.

6. **Testing strategy is now comprehensive.** Sections 10.6 (Backend API), 10.7 (Contract Tests), 10.8 (Standalone Mode) fill the gaps. The ts-rs codegen recommendation for frontend-backend type safety is practical.

7. **New complexity justified.** Every addition (state machine, job pattern, step transitions, session correlation, backups) was evaluated by the simplicity sub-agent and found to earn its place relative to the problems it solves.

---

## Review Coverage

| Domain | Sub-agent | Prior Findings | Resolved | New | Verdict |
|--------|-----------|---------------|----------|-----|---------|
| Resilience | design-review-resilience | 10 | 10 | 2 LOW | PASS |
| Simplicity | design-review-simplicity | 6 | 5+1 partial | 3 LOW | PASS WITH WARNINGS |
| Data & State | design-review-data | 8 | 8 | 2 LOW | PASS |
| Interfaces | design-review-interfaces | 9 | 9 | 2 LOW | PASS |
| Observability | design-review-observability | 6 | 6 | 0 | PASS |
| Testing | design-review-testing | 10 | 10 | 2 LOW | PASS |
| Scaling & Evolution | design-review-scaling | 7 | 7 | 0 | PASS |

## Summary

| Severity | Pass 1 | Pass 2 |
|----------|--------|--------|
| HIGH     | 9      | **0**  |
| MEDIUM   | 18     | **0**  |
| LOW      | 12     | **9**  |
| **Total**| **39** | **9**  |

**Verdict: PASS WITH WARNINGS** (0 HIGH, 0 MEDIUM, 9 LOW — all documentation-level nits)
