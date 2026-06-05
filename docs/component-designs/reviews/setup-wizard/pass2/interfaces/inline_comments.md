<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 680-698 RULE: 15 SEVERITY: LOW -->
**Rule 15 — Be Explicit About Guarantees**
The model scan response defines `truncated` and `truncation_reason` but does not specify the ordering of the `models` array. When `truncated: true` due to `max_results`, the subset returned is undefined — a user re-running the scan could see different models in the truncated set. Add a contract note: "The `models` array order is not guaranteed (filesystem walk order, which varies by platform). The frontend should sort client-side for display. When `truncated` is `true` due to `max_results`, the returned models are those discovered first during the walk."

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 747-757 RULE: 14 SEVERITY: LOW -->
**Rule 14 — Make Interfaces Narrow**
The SSE `complete` event includes `log_path`, `replaced_existing`, and `backup_path` alongside the core `binary_path` and `duration_secs`. These informational fields tie the event shape to the build log persistence and binary backup implementations. Consider documenting which fields are stable contract (`binary_path`, `duration_secs` on `complete`; `phase`, `error`, `last_lines` on `failed`) versus informational/unstable (`log_path`, `replaced_existing`, `backup_path`). This costs zero implementation effort and protects against accidental coupling as the API evolves.

---
