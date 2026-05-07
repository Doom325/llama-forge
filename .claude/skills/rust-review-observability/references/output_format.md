# Review Output Format

Every review produces two deliverables: a **summary report** and **inline code comments**. Both go into the output directory.

## 1. Summary Report (`review_report.md`)

```markdown
# [Skill Domain] Review — [Project/Crate Name]

**Reviewer:** [skill name]
**Date:** [ISO date]
**Scope:** [files/crates examined]
**Verdict:** PASS | PASS WITH WARNINGS | FAIL

## Violations Found

### [RULE N — Rule Name] — Severity: HIGH | MEDIUM | LOW

**File:** `path/to/file.rs` (lines X–Y)

**What's wrong:** One-sentence description of the violation.

**Why it matters:** Brief explanation connecting to the architectural principle.

**Suggested fix:**
```rust
// concrete code showing the corrected pattern
```

_(repeat for each violation)_

## Summary

| Severity | Count |
|----------|-------|
| HIGH     | N     |
| MEDIUM   | N     |
| LOW      | N     |

## Commendations

Note any patterns done well — reviewers should reinforce good architecture, not just flag bad.
```

### Severity guide

- **HIGH** — Architectural boundary violation, unsound design, production safety hazard. Blocks merge.
- **MEDIUM** — Deviation from best practice that degrades maintainability or performance. Should fix before merge.
- **LOW** — Style or idiom preference that would improve the code but isn't blocking.

## 2. Inline Comments (`inline_comments.md`)

Each comment is a self-contained block ready to paste into a PR review tool:

```markdown
<!-- FILE: path/to/file.rs LINES: 42-48 RULE: 7 SEVERITY: MEDIUM -->
**Rule 7 — Static Dispatch by Default**
This `Vec<Box<dyn Handler>>` could be an enum since the handler set is known at compile time. Enums preserve cache locality and enable exhaustive matching.

Suggested:
```rust
enum Handler {
    Auth(AuthHandler),
    Logging(LoggingHandler),
}
```
---
```

The `<!-- ... -->` header lets automation parse file, line range, rule number, and severity programmatically.

## Producing both outputs

When you finish reviewing, write:
1. `review_report.md` — the full structured report
2. `inline_comments.md` — the PR-ready comments

If no violations are found, still produce the report with verdict PASS and an empty violations section. Include commendations for good patterns observed.
