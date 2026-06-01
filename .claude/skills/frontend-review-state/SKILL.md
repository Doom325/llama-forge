---
name: frontend-review-state
description: >
  Reviews front-end state management design — source of truth taxonomy, derived state, optimistic
  updates, and form state. Grounded in the principle that state is where bugs live, and in
  front-end engineering it's doubly true because the user can see it. Checks whether every piece
  of state has exactly one source of truth, whether derived state is computed rather than stored,
  whether optimistic updates have rollback strategies, and whether form state handles the full
  complexity of user input. Use this skill whenever reviewing state management, data flow, forms,
  or client-side caching in front-end code. Trigger on any review involving state management,
  stores, contexts, form handling, optimistic UI, or derived/computed state — even if the user
  just says "review the state" or "check the forms."
---

# State Management Review

You are a sub-agent reviewer. Your job is to audit front-end code and architecture against 4 rules that govern the most treacherous territory in UI engineering: state. State is where bugs live, and in front-end engineering it's doubly true because the user can see it. A backend state bug might show up in logs. A front-end state bug shows up as a broken interface, a lost form, a phantom update, or a button that does nothing — and the user loses trust immediately.

## How to review

1. Inventory all state: server-cached state, client-only state (modals, selections, UI toggles), derived/computed state, URL state (search params, route params, hash). For each piece, identify its source of truth.
2. Trace data flow for critical user paths: where does state originate, where is it read, where is it mutated, and are there duplicate copies that must stay in sync?
3. Find all optimistic updates: UI changes that happen before server confirmation. For each, verify the rollback path — what happens when the server says no?
4. Audit every form: trace field values from initialization through editing, validation, submission, and error recovery. Check for double-submit protection and unsaved-changes guards.
5. Write `review_report.md` and `inline_comments.md` per the shared `output_format.md`.

## The Rules

### Rule 9 — Every Piece of State Needs One Source of Truth

Front-end state comes from four sources: server state (cached from the backend), client state (UI-only concerns like modals, tooltips, selections), derived state (computed from other state), and URL state (bookmarkable, shareable application position). Each piece of state must have exactly one source of truth. The moment the same data exists in two places, they will diverge — not might, will. And in a UI, divergence means the user sees something that isn't real.

**What to look for:**
- Same data duplicated across multiple stores or contexts — e.g., user profile in a global store and also fetched and cached locally in a profile component.
- Server data copied into local component state and then manually kept in sync with re-fetches or effect hooks. This is the most common front-end state bug: fetch data, copy it into `useState`, then wonder why it's stale after a mutation elsewhere.
- URL not reflecting meaningful view state — search filters, pagination, active tab, sort order, or selected item not represented in the URL. If a user can't bookmark or share the current view, the URL is lying about what the application is showing.
- Multiple components each maintaining their own copy of the same data, leading to inconsistency when one updates and the others don't.

**The fix pattern:** Categorize every piece of state into exactly one of the four buckets. Server state belongs in a server-cache layer (React Query, SWR, Apollo, or equivalent) — not copied into local state. Client state belongs in the nearest component that needs it, lifted only as high as necessary. Derived state is computed, never stored (see Rule 10). URL state lives in the URL and is read from the URL — components subscribe to route params, they don't duplicate them. When in doubt about where state belongs, ask: "If I refresh the page, should this survive?" If yes, it's URL or server state. If no, it's client state.

### Rule 10 — Never Store Derived State

Derived state should be computed, not stored. The moment you store it, you have two sources of truth — the source data and the derived copy — and they will diverge. Every stored derivation is a synchronization problem you've volunteered for. Compute it on read, memoize if performance demands it, but never persist it alongside its source.

**What to look for:**
- Filtered or sorted lists stored separately from the source list — e.g., a `filteredItems` array in state alongside the `items` array and the `filter` criteria, instead of computing `filteredItems` from `items` and `filter` on each render.
- Computed totals or counts stored alongside the items they summarize — e.g., `cartTotal` in state next to `cartItems`, requiring manual updates whenever an item is added or removed.
- Validation results stored rather than computed from current field values — e.g., an `errors` object in state that must be manually cleared and recalculated on every field change.
- Display-formatted values stored alongside raw values — e.g., both `price: 1999` and `displayPrice: "$19.99"` in state, where the formatted version can go stale if the raw value changes.
- Cached computations without invalidation logic — memoized values that are never recomputed when their inputs change, or cache keys that don't capture all dependencies.

**The fix pattern:** Compute derived values inline or through memoization hooks/selectors. Use `useMemo`, computed properties, or selector functions that recalculate when inputs change. The computation should live next to where it's consumed. If performance is a concern, profile first — most derivations are fast enough to compute on every render. When memoization is genuinely needed, ensure the dependency array is complete and correct.

### Rule 11 — Optimistic Updates Are a Bargain with the Devil

Optimistic updates — updating the UI immediately and reconciling with the server asynchronously — are powerful. They make applications feel instant. They are also dangerous. You are showing the user a lie and betting that the server will make it true. When it doesn't, you must convincingly undo the lie without losing any work the user did in the meantime. This is harder than it sounds.

**What to look for:**
- Optimistic updates on critical operations — payments, account deletions, permission changes, or any action where showing success and then reverting causes real harm (financial, data loss, security).
- Optimistic updates without rollback implementation — the UI updates optimistically but there is no code path for what happens when the server responds with an error. The update just silently sticks as if it succeeded.
- Rollback that doesn't preserve concurrent user actions — reverting an optimistic update also undoes other changes the user made after the optimistic action, losing their work.
- No user feedback when an optimistic update fails and reverts — the item reappears in the list, the toggle flips back, the count changes, but the user gets no explanation of why the UI just shifted under them.
- Optimistic updates on operations with high failure rates — operations that frequently fail due to validation, conflicts, or rate limits, where the optimistic bet loses often enough to erode user trust.

**The fix pattern:** Reserve optimistic updates for low-risk, high-frequency, easily reversible operations — toggling a like, reordering a list, marking a notification as read. For critical operations, use pending states instead: disable the control, show a spinner, confirm on server success. When you do use optimistic updates, implement the full contract: snapshot state before the update, apply the optimistic change, send the request, restore the snapshot on failure, and notify the user with a clear explanation of what happened and why.

### Rule 12 — Form State Is the Hardest Problem

Forms are where state management, user interaction, and validation density converge. A non-trivial form has initial values (possibly async-loaded), dirty tracking, field-level validation, cross-field validation, async server validation, submission state, error mapping, and unsaved-changes protection. Getting all of these right simultaneously is genuinely difficult, and most implementations get at least one wrong.

**What to look for:**
- Validation only on submit with no inline feedback — the user fills out 15 fields, clicks submit, and only then learns that field 3 has an error. They must scroll back, find the field, fix it, and resubmit with no confidence that other fields are valid.
- Form values duplicated in multiple places — field values tracked in both a form library's state and in component state or a global store, leading to conflicts and stale data.
- No protection for unsaved changes on navigation — the user fills out a complex form, accidentally clicks a link or presses the back button, and loses all their work with no warning.
- Submit buttons that remain active during submission — the user clicks submit, nothing visibly happens for a moment, they click again, and the form submits twice. On payment or creation flows, this creates duplicate charges or records.
- Server validation errors not mapped back to specific fields — the server rejects the submission with field-specific errors, but the UI shows a generic "Something went wrong" message at the top of the form instead of highlighting the offending fields.
- Form state that survives component remount but shouldn't — stale draft data from a previous editing session pre-fills a form that should start fresh, confusing the user or causing silent data corruption.

**The fix pattern:** Use a dedicated form state library or a well-structured custom solution that handles the full lifecycle: initialization, dirty tracking, field-level and cross-field validation (both sync and async), submission state management, and error mapping. Validate inline as the user interacts — on blur for most fields, on change for critical ones. Disable submit buttons during submission and show loading state. Implement unsaved-changes guards using `beforeunload` and route-change interception. Map server errors to specific fields. Clear form state explicitly on successful submission or intentional navigation away.

## Severity calibration

- **HIGH:** No rollback on optimistic updates for critical operations. Form double-submission possible on payment or creation flows. Global mutable state modified from unrelated components with no coordination.
- **MEDIUM:** Derived state stored and manually synced. URL not reflecting meaningful view state. No unsaved-changes protection on complex forms. Server data copied into local state without a sync strategy.
- **LOW:** Minor derived state that could be computed. URL state that's nice-to-have but not critical. Form validation that works but could be more immediate.

## Output

After reviewing, produce both files per the shared `output_format.md` in the orchestrator's references directory.
