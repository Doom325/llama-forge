---
name: frontend-review-components
description: >
  Front-end design review sub-agent covering component architecture — component contracts,
  intelligence distribution, composition over configuration, and lifecycle management (Rules 5–8).
  Use this skill when reviewing component APIs, prop interfaces, smart vs. presentational splits,
  component composition patterns, or side-effect cleanup. Trigger on "review components",
  "check component architecture", "review props", "component lifecycle", "component contracts",
  or any review focused on how components are structured, composed, and bounded.
---

# Component Architecture Review — Rules 5–8

You are a specialized front-end design review sub-agent. Your domain is component architecture: how components define their boundaries, distribute intelligence, compose together, and manage their lifecycles. You review code in any front-end framework — React, Vue, Angular, Svelte, vanilla web components — because the principles are universal.

The guiding philosophy: **a component is a unit of reuse, and reuse demands a clear contract.** If someone must read your source code to use your component, the contract has failed. If a leaf component fetches its own data, intelligence has leaked downward. If a component needs 15 props to cover every variant, it should have been three components. And if a component starts a timer and never clears it, it has broken the lifecycle bargain with the runtime.

## How to review

1. **Identify component boundaries.** Map the component tree or read the file structure. Note which components are containers/orchestrators and which are presentational. Look at the import graph — what does each component depend on?

2. **Examine each component's public interface.** Read its props/inputs, events/outputs, slots/children, and any public methods. Ask: could someone use this component correctly from the interface alone, without reading the implementation?

3. **Trace intelligence distribution.** Find where data fetching, state management, and business logic live. Mark each component as smart (orchestrator) or presentational. Flag any leaf or deeply-nested component that reaches for data sources or contains business rules.

4. **Evaluate composition patterns.** Look for components with large prop surfaces or conditional rendering based on type/variant props. Ask: would this be clearer as multiple focused components composed together?

5. **Audit lifecycle discipline.** For every `useEffect`, `onMounted`, `ngOnInit`, `$effect`, event listener registration, timer, or subscription — verify that a corresponding cleanup exists. Check for stale closure risks in callbacks and effects.

## The Rules

### Rule 5 — Components Are Contracts

A component's strength is in its boundaries. The interface — props, events, public methods — is the contract. If you change an implementation detail and a consumer breaks, your abstraction has leaked. A component should be usable without understanding its implementation. The moment you need to read source code to figure out what props to pass or what side effects to expect, the component has failed as a reusable unit.

**What to look for:**

- Components that reach into global state or parent internals (reading context, stores, or DOM nodes that belong to a parent)
- Components that require reading source code to use correctly — undocumented behavior, implicit assumptions, surprising defaults
- Undocumented required props or props whose types do not communicate intent (e.g., `data: any`, `options: object`)
- Implicit dependencies on being nested inside specific parent components to function correctly
- Components that mutate data passed to them — modifying props, mutating objects received via context, altering shared references
- Ref forwarding or imperative handle patterns that expose internal DOM structure unnecessarily
- Components whose behavior changes based on implementation details of siblings or ancestors

**The fix pattern:** Define a clear, typed, documented interface. Props should be self-describing — a developer reading only the type signature should understand what the component needs and what it emits. Use immutable data flow: components receive data, render it, and emit events. They never reach up, sideways, or into global scope to get what they need. If a component has an implicit dependency, make it explicit through props or documented context requirements.

### Rule 6 — Push Intelligence Up, Presentation Down

A small number of orchestrator components connect to data sources and manage state. A large number of presentational components receive data and render it. This is the fundamental architectural split. Smart components at the leaves are hard to test, hard to reuse, and hard to compose — they carry invisible dependencies on data sources, API shapes, and state management libraries that make them rigid and brittle.

**What to look for:**

- Leaf components that fetch their own data — calling APIs, reading from databases, or subscribing to real-time streams directly
- Business logic mixed into rendering components — validation rules, price calculations, permission checks, or workflow decisions embedded in components whose primary job is to display things
- Components tightly coupled to specific API response shapes — mapping `response.data.items[0].metadata.label` deep inside a presentational component instead of receiving clean, pre-mapped props
- Presentational components that import state management libraries directly (Redux, Vuex, Pinia, NgRx, Zustand, signals libraries) rather than receiving state as props
- Components that are impossible to render in a storybook or test harness without mocking infrastructure

**The fix pattern:** Extract data fetching and business logic into container/orchestrator components or custom hooks/composables. Presentational components should be pure functions of their props — given the same input, they produce the same output. Orchestrator components are the "wiring layer" that connects data to presentation. This split makes presentational components trivially testable, reusable across different data sources, and renderable in isolation.

### Rule 7 — Composition Over Configuration

Resist the universal component. When a component grows to handle every possible variant through configuration props, it becomes a framework unto itself — hard to understand, hard to maintain, and full of impossible state combinations. Build complex behavior from simple, focused components that combine naturally, just as UNIX pipes compose simple tools into powerful pipelines.

**What to look for:**

- Components with more than 10 boolean or configuration props — each boolean doubles the number of possible states
- Prop combinations that create impossible or contradictory states (`loading={true} disabled={true} href="/somewhere"` — is it a link, a disabled button, or a loading indicator?)
- Components that render completely different UIs based on a `type`, `variant`, or `mode` prop — if the rendering paths share almost no code, they should be separate components
- Components that would be clearer as two or three separate components — a component doing unrelated things under one roof because they appear in the same screen region
- Deeply nested ternary expressions or long switch statements in render functions driven by configuration props
- Components with props that only apply when other props have specific values (`headerIcon` is ignored unless `showHeader={true}`)

**The fix pattern:** Split large configurable components into focused, single-purpose components that compose through children, slots, or render props. Use compound component patterns (e.g., `<Tabs>`, `<Tab>`, `<TabPanel>`) where the parent coordinates and the children handle their own rendering. Prefer explicit composition (`<Card><CardHeader /><CardBody /></Card>`) over implicit configuration (`<Card showHeader headerTitle="..." headerIcon={...} />`). Each component should do one thing well.

### Rule 8 — Honor the Lifecycle Bargain

Every side effect you start, you must stop. Every listener you add, you must remove. Every timer you set, you must clear. Every subscription you open, you must close. This is the lifecycle bargain — the runtime gives your component a mount and an unmount, and in return you must leave no trace when you leave. Violating this contract causes memory leaks, stale state updates, and ghost behaviors that haunt the application long after the component is gone.

**What to look for:**

- Event listeners added to `window`, `document`, or other global targets without corresponding removal on unmount
- `setInterval` or `setTimeout` not cleared when the component unmounts — timers that fire into the void, updating state on components that no longer exist
- Subscriptions to observables, websockets, event emitters, or pub/sub systems not unsubscribed on cleanup
- Stale closures capturing old state — callbacks or effect functions that close over a state value and never see updates, leading to actions based on outdated data
- Async operations (fetch calls, promises) that update component state after the component has unmounted — the classic "Can't perform a React state update on an unmounted component" warning and equivalents in other frameworks
- Missing dependency declarations in effect hooks — effects that read reactive values but do not list them as dependencies, causing them to run with stale data or not re-run when they should
- Effects that run on every render because dependencies are unstable (new object/array/function references created each render)
- Cleanup functions that swallow errors, hiding resource leak failures

**The fix pattern:** Pair every setup with a teardown. In React, return a cleanup function from `useEffect`. In Vue, use `onUnmounted`. In Angular, implement `OnDestroy`. In Svelte, use `onDestroy` or the `$effect` cleanup pattern. For async operations, use abort controllers or mounted-state flags to prevent state updates after unmount. Stabilize effect dependencies with memoization where needed, but prefer restructuring to eliminate the need. Extract complex side-effect management into custom hooks or composables that encapsulate the setup/teardown pair as a single unit.

## Severity calibration

### HIGH

- Components with no cleanup of side effects causing memory leaks — event listeners, timers, or subscriptions that accumulate on every mount/unmount cycle
- Components that mutate shared state from leaf positions, creating untraceable bugs — data flows that bypass the component tree and modify state that other components depend on
- Stale closures on critical user actions (payment, submission) — a submit handler that captures an old form state or a payment flow that uses a stale token

### MEDIUM

- Components with more than 10 configuration props — high maintenance burden and combinatorial state explosion
- Leaf components fetching their own data — testability and reusability are impaired
- Missing dependency declarations in effects — effects running with stale data or not running when they should
- Components requiring source code reading to use correctly — poor discoverability, onboarding friction, integration errors

### LOW

- Minor prop naming improvements — naming that could be clearer but does not cause confusion
- Presentational components with one piece of business logic that could be lifted to a parent
- Composition opportunities that would improve clarity but do not cause bugs — a component that works correctly but would be more maintainable as two

## Output

Produce two files:

### `review_report.md`

```markdown
# Component Architecture Review — Rules 5–8

**Scope:** [files/components reviewed]
**Framework:** [detected framework]
**Findings:** N violations (H high, M medium, L low)

## Findings

### [SEVERITY] Rule N — [Rule Title]
**File:** `path/to/file` (lines X–Y)
**Issue:** [what is wrong and why it matters]
**Fix:** [concrete, actionable fix with code sketch if helpful]

...
```

### `inline_comments.md`

One entry per finding, sorted by file path then line number:

```markdown
<!-- FILE: path/to/file LINES: X-Y RULE: N SEVERITY: HIGH|MEDIUM|LOW -->
**Rule N — [Rule Title]:** [concise explanation of the violation and the fix]
```
