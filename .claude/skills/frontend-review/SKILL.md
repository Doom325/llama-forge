---
name: frontend-review
description: >
  Full-spectrum front-end design review orchestrating 7 sub-agent reviews covering rendering,
  components, state, network resilience, performance, accessibility, and security — then
  merging findings into a unified report. 28 rules from the "Robust Front-End Design"
  treatise. Trigger on "front-end review", "frontend review", "review this UI", "review
  these components", "review this React/Vue/Angular/Svelte code", "audit the front-end",
  "is this UI production-ready", or any broad front-end review request. Even if the user
  mentions only one area, consider whether a full review would serve them better and offer
  it. Do NOT use for quick single-concern checks — use the individual sub-skills directly.
---

# Front-End Design Review — Orchestrator

You coordinate a team of 7 specialized front-end review sub-agents, each focused on a distinct design domain. Your job is to:

1. **Scope** the review — figure out what code/architecture to examine
2. **Dispatch** the relevant sub-agents in parallel
3. **Verify** their outputs for quality and consistency
4. **Resolve** cross-domain conflicts or duplicate findings
5. **Produce** a unified final report and merged inline comments

The guiding philosophy: **the front-end is where technology meets humanity. Every other system exists to serve it. Build interfaces that work — not just on your machine, in your browser, on your network, but for every person who encounters them.**

This review is framework-agnostic. The principles apply to React, Vue, Angular, Svelte, vanilla JS, server-rendered, client-rendered, or hybrid architectures. The sub-agents look for design patterns and anti-patterns, not framework idioms.

## Step 1 — Scope the review

Before spawning sub-agents, understand what you're reviewing:

- If given a repository, identify the front-end architecture: rendering strategy (SSR, CSR, SSG, hybrid), component library, state management approach, routing, build tooling.
- If given specific files or a PR diff, note the file paths to pass to each sub-agent.
- If the codebase is large, prioritize: changed files first, then direct dependents.
- Understand the context: marketing site, internal tool, consumer product, design system, embedded webview? Performance and accessibility reviews need this.
- Check for existing configuration: a11y linting rules, performance budgets, CSP headers, test coverage targets.

Read a few key files to build a mental map. You need this context to write good sub-agent prompts and verify their output.

## Step 2 — Dispatch sub-agents in parallel

Spawn relevant sub-agents **in a single turn** so they run concurrently. Each receives:

- The skill to read (path to its SKILL.md)
- The files/components to review
- Where to write outputs
- Context information relevant to its domain

Use this template:

```
You are a specialized front-end design reviewer. Your review skill is at:
  [SKILL_PATH]/SKILL.md

Read that skill file first, then review the following code:
  [LIST OF FILES OR COMPONENT PATHS]

Context about this system:
  [FRAMEWORK, RENDERING STRATEGY, TARGET USERS, SCALE]

Write your outputs to:
  [OUTPUT_DIR]/[skill-name]/

Produce both review_report.md and inline_comments.md.
```

### The 7 sub-agents

| Sub-agent | Skill | Domain | Rules |
|-----------|-------|--------|-------|
| Rendering | `frontend-review-rendering` | Progressive rendering, document semantics, layout stability, responsiveness | 1–4 |
| Components | `frontend-review-components` | Component contracts, intelligence distribution, composition, lifecycle | 5–8 |
| State | `frontend-review-state` | State taxonomy, source of truth, optimistic updates, form state | 9–12 |
| Network | `frontend-review-network` | Request resilience, caching, offline support, data fetching | 13–16 |
| Performance | `frontend-review-performance` | Performance budgets, JavaScript cost, image optimization, critical rendering | 17–20 |
| Accessibility | `frontend-review-accessibility` | Semantic HTML, keyboard navigation, screen reader support, ARIA, color & contrast | 21–25 |
| Security | `frontend-review-security` | XSS, CSRF, CSP, third-party scripts, input sanitization | 26–28 |

**Not every sub-agent is always relevant.** Use scoping judgment:

- New component library? → Components + Accessibility + Performance
- Data-heavy dashboard? → State + Network + Performance + Components
- Form-heavy workflow? → State + Accessibility + Network
- Landing page / marketing site? → Rendering + Performance + Accessibility
- Authentication flow? → Security + State + Accessibility
- Full application PR? → All 7
- CSS/styling refactor? → Rendering + Accessibility
- API integration layer? → Network + State
- Design system tokens/theming? → Rendering + Components + Accessibility

Tell the user which sub-agents you're dispatching and why.

## Step 3 — Verify sub-agent outputs

Read each `review_report.md` and `inline_comments.md`. Verify:

### Quality checks

- **Evidence-backed:** Every violation cites specific file and line range. Reject vague claims.
- **Correct rule application.** Common false positives to catch:
  - Rule 2: Flagging server-rendered content as "not progressively rendered" (it already is)
  - Rule 5: Flagging a component with legitimately complex props as "too configurable" when the use cases are real
  - Rule 9: Flagging framework-managed state (e.g., React Context for theming) as "wrong source of truth"
  - Rule 17: Flagging a necessary large dependency as "JavaScript cost" when there's no lighter alternative
  - Rule 21: Flagging custom components that correctly implement ARIA as "not semantic HTML"
  - Rule 26: Flagging framework-provided sanitization as "missing XSS protection" when the framework handles it
- **Severity calibration:** Match each skill's calibration guide. Missing alt text on decorative images = LOW. Missing alt text on informational images = MEDIUM. No keyboard access to primary actions = HIGH.
- **Actionable fixes:** Concrete suggested fix with code/architecture, not just "fix this."

### Cross-domain consistency

Check for known overlap between front-end review domains:

- **Rendering ↔ Performance:** Layout shift findings may appear in both. Keep the most specific.
- **Components ↔ State:** State management violations often surface in component reviews. Deduplicate.
- **Accessibility ↔ Rendering:** Semantic HTML findings overlap. Keep the accessibility agent's version (more detailed).
- **Network ↔ State:** Caching and data fetching touch both. Keep the more specific finding.
- **Performance ↔ Rendering:** Image optimization and lazy loading touch both. Keep performance's version.
- **Security ↔ Network:** Input sanitization and request handling overlap. Keep security's version (stricter).
- **Contradictory advice:** Resolve by understanding design intent. Accessibility always wins over aesthetics. Security always wins over convenience.
- **Severity conflicts:** Use the higher severity with a note.

## Step 4 — Produce the unified report

### Final report structure (`frontend_review.md`)

```markdown
# Front-End Design Review

**Project:** [name]
**Date:** [ISO date]
**Framework:** [React/Vue/Angular/Svelte/vanilla/etc.]
**Rendering Strategy:** [SSR/CSR/SSG/hybrid]
**Scope:** [what was reviewed]
**Overall Verdict:** PASS | PASS WITH WARNINGS | FAIL

## Executive Summary

[2-3 sentences: overall front-end health, most critical issue, what's done well]

## Critical Findings (HIGH severity)

[Grouped by domain, each with file, line range, rule, explanation, fix]

## Warnings (MEDIUM severity)

[Same format]

## Suggestions (LOW severity)

[Same format]

## Commendations

[Patterns done well — reinforcing good design matters as much as flagging bad]

## Review Coverage

| Domain | Sub-agent | Violations | Status |
|--------|-----------|------------|--------|
| Rendering & Document | frontend-review-rendering | N | Reviewed |
| Component Architecture | frontend-review-components | N | Reviewed |
| State Management | frontend-review-state | N | Reviewed |
| Network Resilience | frontend-review-network | N | Reviewed |
| Performance | frontend-review-performance | N | Reviewed |
| Accessibility | frontend-review-accessibility | N | Reviewed |
| Browser Security | frontend-review-security | N | Reviewed / Skipped |

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | N     |
| MEDIUM   | N     |
| LOW      | N     |
| Total    | N     |
```

### Merged inline comments (`inline_comments.md`)

Combine all sub-agent inline comments into one file, sorted by file path then line number. Deduplicate overlapping findings. Each retains its `<!-- FILE: ... LINES: ... RULE: ... SEVERITY: ... -->` metadata header.

### Verdict logic

- **FAIL:** Any HIGH severity finding.
- **PASS WITH WARNINGS:** No HIGH, but MEDIUM findings exist.
- **PASS:** Only LOW or none.

## Handling partial reviews

If a sub-agent errors or produces malformed output:
1. Note in Review Coverage as "Error."
2. Do NOT silently skip — the user must know.
3. If the failed domain is critical (Accessibility, Security, Rendering), flag in executive summary.
4. Offer to re-run the failed sub-agent.

## Quick-Rejection Patterns

Scan for these during scoping — they warrant immediate HIGH flags:

| Pattern | Rule |
|---------|------|
| No semantic HTML — all `<div>` and `<span>` with click handlers | 3 |
| Interactive elements not keyboard-accessible | 21 |
| Missing alt text on informational images | 23 |
| No loading/error states for async data | 13 |
| `dangerouslySetInnerHTML` or equivalent with unsanitized input | 26 |
| No CSP headers configured | 28 |
| JavaScript bundle > 500 KB uncompressed on critical path | 18 |
| Layout shifts from images without dimensions | 4 |
| Form with no validation feedback until submission | 12 |
| Derived state stored and manually synced instead of computed | 10 |
| No focus management on dynamic content changes (modals, etc.) | 22 |
| Global mutable state modified from multiple unrelated components | 9 |
| User data in localStorage without encryption consideration | 27 |
| No offline/error fallback — blank screen on network failure | 15 |
| Component that only works with JavaScript — no progressive enhancement | 1 |
