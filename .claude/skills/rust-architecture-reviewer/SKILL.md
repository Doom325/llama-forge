---
name: rust-architecture-reviewer
description: >
  Full-spectrum Rust architecture reviewer that orchestrates 6 specialized sub-agent reviews
  covering trait design, architectural boundaries, crate hygiene, runtime safety, observability,
  and CI pipelines — then verifies, deduplicates, and merges their findings into a single
  unified report with inline PR comments. Use this skill whenever the user wants a comprehensive
  Rust code review, architecture audit, or PR review. Trigger on phrases like "review this Rust
  project", "architecture review", "audit this codebase", "review my PR", "check this Rust code",
  or any broad review request that spans more than one concern area. Even if the user only mentions
  one area (e.g., "check my error handling"), consider whether a full review would serve them
  better and offer it. Do NOT use this for quick single-rule checks — use the individual
  sub-skills directly for those.
---

# Rust Architecture Reviewer — Orchestrator

You coordinate a team of 6 specialized review sub-agents, each focused on a distinct architectural domain. Your job is to:

1. **Scope** the review — figure out what files/crates to examine
2. **Dispatch** the sub-agents in parallel
3. **Verify** their outputs for quality and consistency
4. **Resolve** cross-domain conflicts or duplicate findings
5. **Produce** a unified final report and merged inline comments

## Step 1 — Scope the review

Before spawning sub-agents, understand what you're reviewing:

- If given a workspace, identify the member crates and their roles (domain, infrastructure, app, etc.)
- If given specific files or a PR diff, note the file paths — you'll pass these to each sub-agent
- If the codebase is very large, prioritize: changed files first, then their direct dependents

Read the workspace `Cargo.toml` and a few key files to build a mental map. You need this context to write good sub-agent prompts and to verify their output later.

## Step 2 — Dispatch sub-agents in parallel

Spawn all 6 sub-agents **in a single turn** so they run concurrently. Each sub-agent should receive:

- The skill to read (path to its SKILL.md)
- The specific files/crates to review
- Where to write its outputs

Use this template for each sub-agent prompt:

```
You are a specialized Rust code reviewer. Your review skill is at:
  [SKILL_PATH]/SKILL.md

Read that skill file first, then review the following code:
  [LIST OF FILES OR CRATE PATHS]

Write your outputs to:
  [OUTPUT_DIR]/[skill-name]/

Produce both review_report.md and inline_comments.md as described in
the skill's references/output_format.md.
```

### The 6 sub-agents

| Sub-agent | Skill Path | Domain |
|-----------|-----------|--------|
| Traits | `rust-review-traits/SKILL.md` | Composition, newtypes, sealed traits, dispatch, builders, GATs |
| Boundaries | `rust-review-boundaries/SKILL.md` | Architecture layers, dependency rule, ports/adapters, typestate, DI |
| Crates | `rust-review-crates/SKILL.md` | Workspace deps, DAG topology, crate boundaries, visibility |
| Safety | `rust-review-safety/SKILL.md` | unwrap/expect, blocking in async, borrowing vs cloning |
| Observability | `rust-review-observability/SKILL.md` | Structured tracing, subscriber layers, domain decoupling, distributed traces |
| CI | `rust-review-ci/SKILL.md` | Build toolchain, CI verification, supply chain auditing |

**Not every sub-agent is always relevant.** Use your scoping judgment:

- A PR that only touches `Cargo.toml` and CI config? Dispatch Crates + CI only.
- A domain modeling change? Dispatch Traits + Boundaries + Safety.
- A full project audit? Dispatch all 6.

Tell the user which sub-agents you're dispatching and why, so they know what to expect.

## Step 3 — Verify sub-agent outputs

When sub-agents complete, read each `review_report.md` and `inline_comments.md`. Verify:

### Quality checks

- **Evidence-backed findings:** Every violation must cite a specific file and line range. Reject vague claims like "the code generally doesn't follow the builder pattern" — the sub-agent must point to the exact struct and constructor.
- **Correct rule application:** Verify the sub-agent applied the rule correctly. A common error is flagging `Arc::clone()` under Rule 23 (borrowing over cloning) — that's actually fine since it's just a refcount bump. Catch these false positives.
- **Severity calibration:** Check that severity levels match the calibration guide in each skill. A missing `cargo fmt --check` in CI is MEDIUM, not HIGH. An `unwrap()` on startup configuration is LOW, not HIGH.
- **Actionable fixes:** Each violation should include a concrete suggested fix, not just "fix this." If a sub-agent only says "use a newtype wrapper" without showing the wrapper code, that's insufficient.

### Cross-domain consistency

Sub-agents review independently, so they may produce contradictory or redundant findings:

- **Duplicate findings:** Rule 26 (observability not in domain) and Rule 12 (dependency rule) can both flag `tracing` in the domain crate's Cargo.toml. Keep the most specific one (Rule 26) and note the cross-reference. Don't report the same violation twice.
- **Contradictory advice:** Rare, but possible. For example, the Traits agent might suggest adding an extension trait while the Boundaries agent flags cross-layer trait leakage. Resolve by understanding the architectural intent and picking the advice that preserves layer boundaries.
- **Severity conflicts:** If two sub-agents flag the same issue at different severities, use the higher severity with a note explaining both perspectives.

## Step 4 — Produce the unified report

Merge all verified findings into a single report. This is the deliverable the user cares about — not the individual sub-agent outputs.

### Final report structure (`architecture_review.md`)

```markdown
# Rust Architecture Review

**Project:** [name]
**Date:** [ISO date]
**Scope:** [what was reviewed — files, crates, PR reference]
**Overall Verdict:** PASS | PASS WITH WARNINGS | FAIL

## Executive Summary

[2-3 sentences: what's the overall health of this code? What's the most
critical issue? What's done well?]

## Critical Findings (HIGH severity)

[Grouped by domain, each with file, line range, rule reference,
explanation, and suggested fix. These block merge.]

## Warnings (MEDIUM severity)

[Same format. Should fix before merge but not blocking.]

## Suggestions (LOW severity)

[Same format. Nice-to-have improvements.]

## Commendations

[Patterns done well, across all domains. Reinforcing good architecture
matters as much as flagging bad.]

## Review Coverage

| Domain | Sub-agent | Violations | Status |
|--------|-----------|------------|--------|
| Trait Design | rust-review-traits | N | Reviewed |
| Boundaries | rust-review-boundaries | N | Reviewed |
| Crate Hygiene | rust-review-crates | N | Reviewed |
| Runtime Safety | rust-review-safety | N | Reviewed |
| Observability | rust-review-observability | N | Reviewed |
| CI Pipeline | rust-review-ci | N | Reviewed / Skipped |

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | N     |
| MEDIUM   | N     |
| LOW      | N     |
| Total    | N     |
```

### Merged inline comments (`inline_comments.md`)

Combine all sub-agent inline comments into a single file, sorted by file path then line number. This gives the user a single document they can walk through file-by-file. Deduplicate any overlapping findings (keep the most specific one).

Each comment retains its `<!-- FILE: ... LINES: ... RULE: ... SEVERITY: ... -->` metadata header so automation can parse it.

### Verdict logic

- **FAIL:** Any HIGH severity finding exists.
- **PASS WITH WARNINGS:** No HIGH findings, but MEDIUM findings exist.
- **PASS:** Only LOW findings or no findings at all.

## Handling partial reviews

If a sub-agent errors out or produces empty/malformed output:

1. Note it in the Review Coverage table as "Error" with a brief explanation.
2. Do NOT silently skip a domain — the user needs to know that area wasn't reviewed.
3. If the failed domain is critical (Boundaries, Safety), flag this prominently in the executive summary.
4. Offer to re-run the failed sub-agent.

## Handling large codebases

For very large projects where reviewing everything at once would overwhelm:

1. Start with the highest-risk crates (domain, application layer).
2. Review infrastructure and CI in a second pass.
3. Clearly state in the report which crates were reviewed and which were deferred.
4. Prioritize changed files if this is a PR review.

## Quick-Rejection Patterns

These patterns warrant immediate HIGH-severity flags. They're common enough that you should scan for them yourself during scoping, even before sub-agents report back:

| Pattern | Rule |
|---------|------|
| Single crate with domain + DB + HTTP + utils | Rule 19 |
| ORM annotations on domain structs | Rule 14 |
| Circular crate dependencies | Rule 18 |
| Mutable `status: String` instead of typestate | Rule 15 |
| 5+ unrelated trait bounds on one parameter | Rule 6 |
| Widespread `.clone()` on heap types | Rule 23 |
| `std::fs` or `thread::sleep` inside `async fn` | Rule 22 |
| `unwrap()` on production code paths | Rule 21 |
| `println!` as operational logging | Rule 24 |
| Dropped trace headers in multi-service setup | Rule 27 |
| Domain crate depending on infrastructure crates | Rule 13 |
| `#[instrument]` inside domain layer | Rule 26 |
