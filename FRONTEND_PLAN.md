# LlamaForge Web UI — Implementation Plan

**Component:** Frontend Web Interface (`FRONTEND_ARCHITECTURE.md`)  
**Estimated LOC:** ~6,500 (~4,800 production + ~1,700 tests)  
**Estimated Duration:** 14–17 working days (single engineer) / 9–11 days (two engineers, PR3/PR4 and PR8/PR9 parallelizable)  
**Design Document:** `FRONTEND_ARCHITECTURE.md` (4th-pass CLEAN PASS — 0 findings across 28 rules)  
**Branch Prefix:** `ui/`  
**Integration Branch:** `frontend/` (long-lived feature branch off `main`)  
**Final Target:** `main` (single merge after all PRs pass end-to-end testing)  
**Upstream Dependency:** Rust backend must serve `/api/*` endpoints and `GET /api/events` (SSE). `build.rs` must support `rust-embed` for static asset embedding.  
**Visual Design Reference:** `design_reference/v1/` — Claude Design prototypes (HTML). Open in any browser to see the visual target for each screen.  
**Agent Model:** Coder agent (Gemini or any LLM agent) implements; Reviewer agent (Claude) reviews PRs against this plan, `FRONTEND_ARCHITECTURE.md`, and the design reference.

> **Branching workflow:** A long-lived `frontend/` branch is created from `main` at the start. Each PR branches from `frontend/` and merges back into `frontend/` via squash-merge. The "Depends on" field on each PR indicates which predecessor(s) must be merged to `frontend/` before that PR's branch is created. Do NOT branch from a predecessor's feature branch — always branch from `frontend/` after the predecessor is merged. Once all 14 PRs are merged and end-to-end testing passes, `frontend/` is merged to `main` as a single integration.
>
> **Setup (run once before PR1):**
> ```bash
> git checkout main && git pull
> git checkout -b frontend/
> git push -u origin frontend/
> ```

---

## Ground Rules for CoderAgents

1. **Read the design doc first.** Every PR description must reference the specific `FRONTEND_ARCHITECTURE.md` sections it implements. Open the corresponding `design_reference/v1/*.html` file(s) in a browser to see the visual target before writing any component.
2. **Svelte 5 runes only.** Use `$state`, `$derived`, `$effect` — never `onMount`, `onDestroy`, `writable`, `readable`, or any Svelte 4 API.
3. **No raw `{@html}` outside `renderMarkdown()`.** The ESLint rule `no-raw-html-without-sanitize` enforces this. The only legal usage is `{@html renderMarkdown(untrustedContent)}`.
4. **Presentational components receive data via props, emit events via callbacks.** No `fetch()` calls, no store subscriptions, no `$effect` side effects in presentational components. Only orchestrator (page-level) components do I/O.
5. **All API calls go through `lib/api/client.ts`.** No raw `fetch()`. The client auto-attaches the CSRF header (`X-LlamaForge-Request: 1`) on non-GET requests and applies timeouts.
6. **TypeScript strict mode.** `"strict": true` in `tsconfig.json`. No `any` types. Prefer discriminated unions over boolean flags.
7. **TanStack Query for all server state.** No Svelte stores for server-originated data. Components consume data via `createQuery()`, never from SSE events directly.
8. **Tailwind CSS 4 only.** No inline `style` attributes except where Svelte scoped styles are required. No external CSS files per component.
9. **Accessibility from day one.** Every interactive element must be keyboard-navigable with visible focus indicator. Every status indicator uses `StatusBadge` (triple-redundancy). axe-core must pass in CI.
10. **Tests run with `npm run test`.** Every PR must pass `npm run check && npm run lint && npm run test:unit` before merge.
11. **Commit messages:** `ui: <imperative summary>` (e.g., `ui: add ServerCard with availableActions prop pattern`).

---

## Dependency Graph (PR merge order)

```
PR1 ─── PR2 ─── PR3 ──┬── PR4 ──┬── PR5 ─── PR6
                       └── PR7 ──┤
                                 ├── PR8
                                 ├── PR9
                                 └── PR10 ─── PR11 ─── PR12 ─── PR13
```

PR4 (Dashboard) and PR7 (Server Detail) can be developed concurrently after PR3 merges. PR5 (ECharts) depends on PR4. PR6 (Queue) depends on PR5. PR8 (Config), PR9 (Explorer), and PR10 (Playground Core) can begin after PR7 merges (they need the full app shell + shared components proven). PR11 (Playground Comparison + Model Registry) depends on PR10. PR12 (Security Hardening + Build Integration) depends on PR11. PR13 (Test Suite + CI) depends on PR12.

---

## PR1: Project Scaffold & Toolchain Setup

**Branch:** `ui/01-scaffold`  
**Merge Target:** `frontend/`  
**Size:** Small (~150 lines of config + stubs)  
**Estimated Time:** 3–4 hours  
**Design Reference:** `design_reference/v1/App Shell.html` — reference for the `app.html` skeleton structure and system font stack

### Purpose

Create the SvelteKit 5 project with all toolchain configuration: TypeScript strict mode, Tailwind CSS 4, ESLint + Prettier, Vitest, Playwright, `adapter-static` for CSR, and the correct `package.json` scripts. No components — just a buildable, lintable, testable empty project.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 1.1 | Initialize SvelteKit project | `npx sv create ui` with TypeScript, ESLint, Prettier, Playwright. Select SvelteKit skeleton project. | `ui/` |
| 1.2 | Configure `adapter-static` | Install `@sveltejs/adapter-static`. Set `fallback: 'index.html'` for CSR SPA routing. Set `paths.base: '/ui'` in `svelte.config.js`. Disable SSR: `export const ssr = false` in root `+layout.ts`. | `ui/svelte.config.js`, `ui/src/routes/+layout.ts` |
| 1.3 | Configure TypeScript strict mode | `"strict": true`, `"noUncheckedIndexedAccess": true`, `"exactOptionalPropertyTypes": true` in `tsconfig.json`. | `ui/tsconfig.json` |
| 1.4 | Install and configure Tailwind CSS 4 | Install `tailwindcss`, `@tailwindcss/vite`. Configure `vite.config.ts` plugin. Add `@import "tailwindcss"` to `app.css`. | `ui/vite.config.ts`, `ui/src/app.css` |
| 1.5 | Install production dependencies | `@tanstack/svelte-query`, `@tanstack/svelte-table`, `bits-ui`, `marked`, `dompurify`, `echarts` (lazy-loaded at runtime but installed at build time), `highlight.js`. | `ui/package.json` |
| 1.6 | Install dev dependencies | `vitest`, `@testing-library/svelte`, `jsdom`, `@playwright/test`, `@axe-core/playwright`, `eslint-plugin-svelte`, `vite-plugin-compression`. | `ui/package.json` |
| 1.7 | Configure Vitest | `vitest.config.ts` with `environment: 'jsdom'`, Svelte transform, coverage thresholds (`statements: 80, branches: 75, functions: 80, lines: 80`). | `ui/vitest.config.ts` |
| 1.8 | Configure ESLint | Enable `eslint-plugin-svelte`. Add custom rule stub for `no-raw-html-without-sanitize` (placeholder — full rule in PR10). | `ui/eslint.config.js` |
| 1.9 | Add package.json scripts | `dev`, `build`, `preview`, `check` (svelte-check), `lint`, `format`, `test:unit` (vitest), `test:e2e` (playwright), `test` (check + lint + test:unit). | `ui/package.json` |
| 1.10 | Create module directory structure | Create all directories from Appendix B: `lib/api/`, `lib/stores/`, `lib/components/{layout,dashboard,server,queue,explorer,config,playground,models,shared}/`, `lib/utils/`, `routes/`. Stub each with empty `index.ts` barrel files where appropriate. | `ui/src/` |
| 1.11 | Create `app.html` shell | Hardcoded HTML with inline critical CSS for skeleton layout (sidebar placeholder, header bar, content area). System font stack. `<noscript>` fallback. `<div id="app">` mount point. `<link rel="modulepreload">` placeholder. Design doc Section 4.2. | `ui/src/app.html` |
| 1.12 | Verify build | `npm run build` produces `ui/build/` with `index.html`. `npm run check` passes. `npm run lint` passes. | — |

### PR1 Acceptance Criteria

- [ ] `npm run build` produces static output in `ui/build/`
- [ ] `npm run check` (svelte-check) passes with zero errors
- [ ] `npm run lint` passes with zero warnings
- [ ] `npm run test:unit` runs (no tests yet, but Vitest initializes)
- [ ] TypeScript strict mode active — `any` types cause compile errors
- [ ] `adapter-static` configured with `fallback: 'index.html'`
- [ ] All directories from Appendix B exist
- [ ] `app.html` contains skeleton CSS, `<noscript>` fallback, and system font stack

### PR1 Creation Steps

```bash
git checkout -b ui/01-scaffold frontend/
# ... implement tasks 1.1–1.12 ...
git add -A && git commit -m "ui: bootstrap SvelteKit 5 project with Tailwind, TypeScript strict, and test toolchain"
git push -u origin ui/01-scaffold
# Open PR: "Web UI PR1: Project Scaffold & Toolchain Setup"
# Description: "Creates the SvelteKit 5 CSR project (FRONTEND_ARCHITECTURE.md Sections 3, 4.1,
# Appendix B, Appendix C). adapter-static for single-binary embedding. TypeScript strict mode.
# Tailwind CSS 4. Vitest + Playwright configured. All directory structure from Appendix B stubbed."
```

---

## PR2: Design Tokens, Theme System & Shared Primitives

**Branch:** `ui/02-design-system`  
**Depends on:** PR1 merged to `frontend/`  
**Size:** Medium (~400 lines)  
**Estimated Time:** 5–7 hours  
**Design Reference:** `design_reference/v1/Component Library.html` — all StatusBadge variants, button states, type scale, spacing. `design_reference/v1/styles.css` — design token values (colors, spacing, typography). `design_reference/v1/All Screens Dark.html` — dark theme targets

### Purpose

Implement the design token system (CSS custom properties for light/dark themes), the theme store, and the foundational shared components that every page depends on: `StatusBadge`, `DynamicHeading`, `Skeleton`, `Toast`, `ErrorBanner`, `EmptyState`, `ConfirmDialog`. These are the building blocks — no page-specific components yet.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 2.1 | Implement CSS custom properties | Define color pairs for light and dark themes. `--color-text-primary`, `--color-bg-primary`, `--color-status-healthy`, `--color-status-degraded`, `--color-status-failed`, focus ring colors. Both variants must meet WCAG AA (4.5:1 text, 3:1 large text). Design doc Section 9.5. | `ui/src/app.css` |
| 2.2 | Implement theme store | `preferences.ts`: Svelte 5 rune-based store wrapping `localStorage`. Read `prefers-color-scheme` on init. Toggle function. Persist to `localStorage`. | `ui/src/lib/stores/preferences.ts` |
| 2.3 | Implement theme application | `theme.ts`: `$effect` that applies `data-theme="light"` or `data-theme="dark"` to `<html>`. CSS custom properties switch via `[data-theme="dark"]` selector. | `ui/src/lib/stores/theme.ts` |
| 2.4 | Implement focus indicator base styles | Tailwind `@layer base` override: `focus-visible:ring-2 focus-visible:ring-offset-2` with theme-adaptive ring color (blue-600 light, blue-400 dark via CSS custom property). Design doc Section 9.2. | `ui/src/app.css` |
| 2.5 | Implement `prefers-reduced-motion` | `@media (prefers-reduced-motion: reduce)` disabling transitions, animations, `animate:flip`, skeleton pulse. Design doc Section 9.5. | `ui/src/app.css` |
| 2.6 | Implement `StatusBadge.svelte` | Props: `status: 'healthy' \| 'degraded' \| 'failed' \| 'pending' \| ...`, optional `size`. Renders icon + text label + color. Triple-redundancy enforced by component structure. Maps status to icon (✓/⚠/✕/⟳), label, and color class. Design doc Section 9.5. | `ui/src/lib/components/shared/StatusBadge.svelte` |
| 2.7 | Implement `DynamicHeading.svelte` | Props: `level: 1 \| 2 \| 3 \| 4 \| 5 \| 6`. Renders `<h1>` through `<h6>` dynamically. Prevents heading hierarchy violations in composed views. Design doc Section 4.3. | `ui/src/lib/components/shared/DynamicHeading.svelte` |
| 2.8 | Implement `Skeleton.svelte` | Props: `width`, `height`, `variant: 'text' \| 'rect' \| 'circle'`. `aria-busy="true"`, `aria-label="Loading"`. Pulse animation (disabled under `prefers-reduced-motion`). | `ui/src/lib/components/shared/Skeleton.svelte` |
| 2.9 | Implement `Toast.svelte` + toast store | Global toast queue in `$state` array. `role="status"`, `aria-live="polite"`. Auto-dismiss after 5s. Types: `success`, `error`, `warning`, `info`. Max 3 visible. | `ui/src/lib/components/shared/Toast.svelte`, `ui/src/lib/stores/toast.ts` |
| 2.10 | Implement `ErrorBanner.svelte` | Props: `message`, `variant: 'error' \| 'warning' \| 'info'`, `dismissible`, `action?: { label, onClick }`. `role="alert"` for errors, `role="status"` for info. Triple-redundancy (icon + text + color). Design doc Section 7.2. | `ui/src/lib/components/shared/ErrorBanner.svelte` |
| 2.11 | Implement `EmptyState.svelte` | Props: `title`, `description`, `icon?`, `action?: { label, onClick }`. Used when tables/lists have no data. | `ui/src/lib/components/shared/EmptyState.svelte` |
| 2.12 | Implement `ConfirmDialog.svelte` | Props: `open`, `title`, `message`, `confirmLabel`, `cancelLabel`, `variant: 'danger' \| 'default'`. Uses Bits UI dialog primitive. `role="dialog"`, `aria-modal="true"`, focus trap, `Escape` closes. Design doc Section 9.1. | `ui/src/lib/components/shared/ConfirmDialog.svelte` |
| 2.13 | Implement `format.ts` utilities | `formatNumber()`, `formatDuration()`, `formatTimestamp()`, `formatBytes()`, `formatTokensPerSecond()`. All locale-aware. | `ui/src/lib/utils/format.ts` |
| 2.14 | Implement `constants.ts` | API route constants (`API_BASE`, `SSE_ENDPOINT`, route paths), default parameter values (`DEFAULT_TEMPERATURE: 0.7`, `DEFAULT_MAX_TOKENS: 2048`), timeout constants. | `ui/src/lib/utils/constants.ts` |
| 2.15 | Unit tests: StatusBadge | Render each status variant → assert icon, label, and color class present. Assert `aria-label` includes status text. | tests |
| 2.16 | Unit tests: DynamicHeading | Render with `level={1}` through `level={6}` → assert correct HTML element. | tests |
| 2.17 | Unit tests: Toast store | Add 4 toasts → assert only 3 visible (max). Wait 5s → assert auto-dismissed. | tests |
| 2.18 | Unit tests: theme store | Toggle theme → assert `data-theme` attribute changes. Assert localStorage persistence. Assert `prefers-color-scheme` initial read. | tests |
| 2.19 | Unit tests: format utilities | `formatDuration(125)` → `"2m 5s"`. `formatTokensPerSecond(42.7)` → `"42.7 tok/s"`. Edge cases: 0, negative, NaN. | tests |

### PR2 Acceptance Criteria

- [ ] Light and dark themes both meet WCAG AA contrast ratios
- [ ] `StatusBadge` renders icon + text + color for every status variant
- [ ] `DynamicHeading` produces correct heading element for each level
- [ ] Focus indicators visible on all focusable elements via keyboard
- [ ] `prefers-reduced-motion` disables all animations
- [ ] Toast queue caps at 3 visible, auto-dismisses at 5s
- [ ] `ConfirmDialog` traps focus and closes on `Escape`
- [ ] All 5 unit test suites pass
- [ ] `npm run check && npm run lint` clean

### PR2 Creation Steps

```bash
git checkout -b ui/02-design-system frontend/
# ... implement tasks 2.1–2.19 ...
git add -A && git commit -m "ui: add design tokens, theme system, and shared component primitives"
git push -u origin ui/02-design-system
# Open PR: "Web UI PR2: Design Tokens, Theme System & Shared Primitives"
# Description: "Implements the design token system and foundational shared components
# (FRONTEND_ARCHITECTURE.md Sections 4.3, 9.2, 9.5, Appendix B shared/).
# StatusBadge (triple-redundancy), DynamicHeading, Skeleton, Toast, ErrorBanner,
# EmptyState, ConfirmDialog. Light/dark theme with CSS custom properties.
# Focus indicator strategy. prefers-reduced-motion support. 5 test suites."
```

---

## PR3: API Client, SSE LiveDataStore & TanStack Query Setup

**Branch:** `ui/03-data-layer`  
**Depends on:** PR2 merged to `frontend/`  
**Size:** Medium–Large (~500 lines)  
**Estimated Time:** 8–10 hours  
**Design Reference:** `design_reference/v1/Connection States.html` — SSE connected/reconnecting/disconnected visual states

### Purpose

Implement the entire data layer: the centralized API client with CSRF headers and timeouts, the SSE `LiveDataStore` with subscriber-count lifecycle and buffer compaction, TanStack Query configuration with all query definitions, and the error normalization pipeline. This is the nervous system — every page depends on it.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 3.1 | Implement `client.ts` — centralized API client | `apiFetch()` wrapping `fetch` with: base URL resolution, CSRF header (`X-LlamaForge-Request: 1`) on non-GET/HEAD, timeout via `AbortSignal.timeout()` (10s GET, 30s mutations), JSON response parsing, error normalization. Design doc Section 10.2. | `ui/src/lib/api/client.ts` |
| 3.2 | Implement `types.ts` — API type definitions | TypeScript interfaces matching backend API: `ServerState`, `ServerMetricsSummary`, `ServerStatus`, `QueueItem`, `RequestRecord`, `ModelInfo`, `GenerationParams`, `ConfigSection`. Shared action types: `ServerAction = 'restart' \| 'stop' \| 'start'`, `DetailServerAction = ServerAction \| 'force_kill'`. Design doc Section 5.2. | `ui/src/lib/api/types.ts` |
| 3.3 | Implement error normalization | `ApiError` class with `status`, `message`, `retryable` fields. Map HTTP status codes: 4xx → non-retryable, 5xx → retryable, network error → retryable, timeout → retryable. | `ui/src/lib/api/client.ts` |
| 3.4 | Implement `liveData.ts` — SSE transport store | `LiveDataStore` with `connected: boolean` ($state), `subscriberCount` (internal), `subscribe()` / `unsubscribe()` (ref counting), `EventSource` management. Opens SSE on first subscriber, closes on last. Design doc Section 7.1. | `ui/src/lib/stores/liveData.ts` |
| 3.5 | Implement SSE event parsing | Parse SSE event types (`server-update`, `metrics-update`, `queue-update`, `config-changed`, `model-update`) and route to `queryClient.setQueryData()` with correct query keys per the event type → query key mapping table in design doc Section 7.1. | `ui/src/lib/stores/liveData.ts` |
| 3.6 | Implement SSE message batching | Buffer incoming SSE messages. Flush to TanStack Query at most once per 100ms via `requestAnimationFrame`. Design doc Section 7.1. | `ui/src/lib/stores/liveData.ts` |
| 3.7 | Implement SSE background tab compaction | When `document.hidden === true`, pause message processing. Buffer uses last-write-wins per target query key (e.g., `['server', 'gpu0']`). On tab focus, apply bulk update — one `setQueryData()` per query key. Design doc Section 7.1. | `ui/src/lib/stores/liveData.ts` |
| 3.8 | Implement SSE reconnection with jitter | On disconnect: exponential backoff with jitter `delay = min(1000 * 2^attempt + random(0, 1000), 30_000)`. Fall back to 5s polling via TanStack Query `refetchInterval`. Track `Last-Event-ID` for gap-free reconnection. Design doc Section 7.2. | `ui/src/lib/stores/liveData.ts` |
| 3.9 | Implement SSE heartbeat detection | Backend sends heartbeat comments every 15s. If two consecutive heartbeats missed (30s), treat connection as dead → trigger reconnect. Design doc Section 7.2. | `ui/src/lib/stores/liveData.ts` |
| 3.10 | Configure TanStack Query | `QueryClientProvider` in root `+layout.svelte`. Defaults: `staleTime: 5_000`, `gcTime: 300_000`, `refetchOnWindowFocus: true`, `retry: 3`, `retryDelay` with exponential backoff. Mutation defaults: `retry: 0`. Design doc Section 6.2. | `ui/src/lib/api/queries.ts`, `ui/src/routes/+layout.svelte` |
| 3.11 | Implement query definitions | `createServersQuery()`, `createServerDetailQuery(id)`, `createServerMetricsQuery(id)`, `createQueueQuery()`, `createRequestsQuery(filters)`, `createAggregatedMetricsQuery(filters)`, `createConfigQuery(section)`, `createModelsQuery()`. Each returns a `createQuery()` call with correct key, fetch function via `client.ts`, and cache config per the Section 7.3 caching table. | `ui/src/lib/api/queries.ts` |
| 3.12 | Implement mutation definitions | `createRestartServerMutation()`, `createStopServerMutation()`, `createStartServerMutation()`, `createForceKillServerMutation()`, `createCancelRequestMutation()`, `createSaveConfigMutation()`. Each with appropriate `onSuccess` invalidation and toast notifications. | `ui/src/lib/api/queries.ts` |
| 3.13 | Implement prefetching helpers | `prefetchServerDetail(id)`, `prefetchRouteData(routeKey)`, `prefetchExplorerNextPage(filters, currentPage)`. Design doc Section 7.1 prefetching strategy. | `ui/src/lib/api/queries.ts` |
| 3.14 | Implement `getAvailableActions()` | Shared utility function mapping `ServerStatus` → `Array<ServerAction>`. Used by `DashboardPage` and `ServerDetailPage` orchestrators. Design doc Section 5.2. | `ui/src/lib/api/types.ts` |
| 3.15 | Unit tests: API client CSRF header | Non-GET requests include `X-LlamaForge-Request: 1`. GET requests do not. | tests |
| 3.16 | Unit tests: API client timeouts | GET request times out at 10s. Mutation times out at 30s. Verify `AbortSignal.timeout()` usage. | tests |
| 3.17 | Unit tests: error normalization | 404 → non-retryable. 500 → retryable. Network error → retryable. Timeout → retryable with correct message. | tests |
| 3.18 | Unit tests: LiveDataStore lifecycle | Subscribe → connected. Unsubscribe → disconnected. Subscribe twice → one connection. Unsubscribe once → still connected. Unsubscribe twice → disconnected. | tests |
| 3.19 | Unit tests: SSE buffer compaction | Simulate 10 messages for 3 servers while backgrounded → assert only 3 `setQueryData()` calls on focus (one per server query key). | tests |
| 3.20 | Unit tests: SSE reconnection jitter | Assert reconnection delays include random jitter component. Assert max delay capped at 30s. | tests |
| 3.21 | Unit tests: getAvailableActions | `running → ['restart', 'stop']`, `stopped → ['start']`, `failed → ['start']`, `restarting → []`. | tests |
| 3.22 | Unit tests: query key mapping | Verify each SSE event type maps to correct TanStack Query key(s). | tests |

### PR3 Acceptance Criteria

- [ ] API client auto-attaches CSRF header on all non-GET requests
- [ ] API client enforces 10s/30s timeouts via `AbortSignal.timeout()`
- [ ] SSE connection opens on first subscriber, closes on last
- [ ] SSE messages batch-flushed at ≤ 100ms via `requestAnimationFrame`
- [ ] Background tab compaction uses per-query-key last-write-wins
- [ ] Reconnection includes jitter to prevent thundering herd
- [ ] Heartbeat timeout triggers reconnect after 30s silence
- [ ] TanStack Query configured with correct defaults (no mutation retry)
- [ ] All 8 unit test suites pass
- [ ] No raw `fetch()` calls outside `client.ts`

### PR3 Creation Steps

```bash
git checkout -b ui/03-data-layer frontend/
# ... implement tasks 3.1–3.22 ...
git add -A && git commit -m "ui: implement API client, SSE LiveDataStore, and TanStack Query data layer"
git push -u origin ui/03-data-layer
# Open PR: "Web UI PR3: API Client, SSE LiveDataStore & TanStack Query"
# Description: "Implements the complete data layer (FRONTEND_ARCHITECTURE.md Sections 6.2, 7.1–7.3,
# 10.2). Centralized API client with CSRF and timeouts. SSE transport-only store with subscriber-
# count lifecycle, RAF batching, background compaction, jitter reconnection, and heartbeat
# detection. TanStack Query with all query/mutation definitions and prefetching helpers.
# 8 unit test suites."
```

---

## VC-1: Data Layer Verification Checkpoint

**Gate:** PR3 merged. The entire data pipeline works end-to-end.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-1.1 | CSRF header present | Intercept mutation request → inspect headers | `X-LlamaForge-Request: 1` present |
| VC-1.2 | CSRF header absent on GET | Intercept GET request → inspect headers | Header NOT present |
| VC-1.3 | SSE subscriber lifecycle | Subscribe → unsubscribe → subscribe → verify single connection (no leak) | EventSource created once per subscribe cycle |
| VC-1.4 | Background compaction | Background tab, push 100 messages for 5 servers → focus → assert 5 setQueryData calls | Exactly 5 calls |
| VC-1.5 | Reconnection jitter | Trigger 10 reconnections → measure delays → assert non-identical | Delays vary |
| VC-1.6 | Query cache invalidation | Mutation succeeds → related query invalidated → refetch triggered | Fresh data in cache |

---

## PR4: App Shell — Layout, Sidebar, Header & Routing

**Branch:** `ui/04-app-shell`  
**Depends on:** PR3 merged to `frontend/`  
**Size:** Medium (~450 lines)  
**Estimated Time:** 6–8 hours  
**Design Reference:** `design_reference/v1/App Shell.html` — sidebar, header, routing layout, collapse behavior. `design_reference/v1/Connection States.html` — header connection status indicator. `design_reference/v1/Responsive.html` — breakpoint behavior

### Purpose

Build the application shell: root layout with sidebar navigation, header with breadcrumb and connection status, SvelteKit file-based routing for all pages, skip links, keyboard shortcuts overlay, and the `aria-live` status region. After this PR, navigating between page stubs works with proper transitions and focus management.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 4.1 | Implement root `+layout.svelte` | App shell structure: `<nav>` + `<header>` + `<main>`. `QueryClientProvider` wrapper. SSE subscription via `$effect` (subscribe on mount, unsubscribe on cleanup). `aria-live="polite"` region for status announcements. Skip link: "Skip to main content". Design doc Section 4.3. | `ui/src/routes/+layout.svelte` |
| 4.2 | Implement `Sidebar.svelte` | Navigation links for all 7 pages: Dashboard, Servers (placeholder), Queue, Explorer, Config, Playground, Models. `<nav aria-label="Main navigation">`. `aria-current="page"` on active link. Arrow key navigation between links. `Ctrl+B` toggle collapse. Collapse state persisted via preferences store. Design doc Section 9.2. | `ui/src/lib/components/layout/Sidebar.svelte` |
| 4.3 | Implement `Header.svelte` | Breadcrumb trail (reactive to current route). "Dashboard Summary" button (Dashboard only — conditionally rendered). Keyboard shortcuts button (`?`). Connection status badge. Design doc Sections 9.2, 9.4. | `ui/src/lib/components/layout/Header.svelte` |
| 4.4 | Implement `ConnectionStatus.svelte` | Reads `liveDataStore.connected`. Renders `StatusBadge` with connected/reconnecting/disconnected states. `aria-live="polite"` with transition-only announcements ("Live updates connected" / "Live updates paused — reconnecting"). Design doc Section 9.4. | `ui/src/lib/components/layout/ConnectionStatus.svelte` |
| 4.5 | Implement keyboard shortcuts overlay | Triggered by `?` key (when no input focused) or header button. Modal with shortcuts grouped by context: Global, Dashboard, Tables, Playground. Uses `ConfirmDialog` primitive (or Bits UI dialog). Design doc Section 9.2. | `ui/src/lib/components/layout/KeyboardShortcuts.svelte` |
| 4.6 | Implement route-change focus management | `afterNavigate` callback in `+layout.svelte`: focus moves to `<h1>` of new page (`tabindex="-1"` on `<h1>` for programmatic focus). Design doc Section 9.3. | `ui/src/routes/+layout.svelte` |
| 4.7 | Create page stub routes | Stub `+page.svelte` for each route: `/` (Dashboard), `/servers/[id]`, `/queue`, `/explorer`, `/config/servers`, `/config/priority`, `/playground`, `/models`. Each stub renders `<h1>` with page title and `<p>Placeholder</p>`. | `ui/src/routes/*/+page.svelte` |
| 4.8 | Implement sidebar link prefetching | `on:mouseenter` on sidebar links calls `prefetchRouteData(routeKey)`. Design doc Section 7.1 prefetching strategy. | `ui/src/lib/components/layout/Sidebar.svelte` |
| 4.9 | Implement sidebar responsive behavior | Below 768px: sidebar collapses to icon-only. Below 640px: sidebar becomes a slide-out drawer. Toggle button visible on mobile. | `ui/src/lib/components/layout/Sidebar.svelte` |
| 4.10 | Implement skeleton layout dimensions | CSS custom properties: `--sidebar-width: 240px`, `--sidebar-collapsed-width: 64px`, `--header-height: 56px`. Skeleton placeholders match these dimensions to prevent CLS. Design doc Section 4.4. | `ui/src/app.css` |
| 4.11 | Unit tests: Sidebar | Active link has `aria-current="page"`. Arrow keys navigate between links. Collapse toggles width. | tests |
| 4.12 | Unit tests: ConnectionStatus | Render connected → assert green badge. Render disconnected → assert red badge with "reconnecting" text. | tests |
| 4.13 | Unit tests: route-change focus | Navigate to new route → assert `document.activeElement` is `<h1>`. | tests |
| 4.14 | Unit tests: keyboard shortcuts overlay | Press `?` → assert overlay visible. Press `Escape` → assert overlay closed. | tests |

### PR4 Acceptance Criteria

- [ ] All 7 page routes render with correct `<h1>` and breadcrumb
- [ ] Sidebar navigation works with arrow keys
- [ ] Route changes move focus to `<h1>`
- [ ] Skip link visible on focus, jumps to `<main>`
- [ ] Connection status shows live/reconnecting/disconnected with `aria-live`
- [ ] Keyboard shortcuts overlay opens via `?` key and header button
- [ ] Sidebar collapse state persists across sessions
- [ ] SSE subscribes on mount, unsubscribes on cleanup
- [ ] All 4 test suites pass

### PR4 Creation Steps

```bash
git checkout -b ui/04-app-shell frontend/
# ... implement tasks 4.1–4.14 ...
git add -A && git commit -m "ui: implement app shell with sidebar, header, routing, and focus management"
git push -u origin ui/04-app-shell
# Open PR: "Web UI PR4: App Shell — Layout, Sidebar, Header & Routing"
# Description: "Builds the application shell (FRONTEND_ARCHITECTURE.md Sections 4.3, 9.2–9.4).
# Root layout with nav/header/main landmarks. Sidebar with arrow key navigation and
# collapse persistence. Route-change focus management via afterNavigate. Skip links.
# Connection status with aria-live announcements. Keyboard shortcuts overlay.
# Sidebar link prefetching. 4 test suites."
```

---

## PR5: Dashboard Page — ServerGrid, ServerCard, Gauges & Health Banner

**Branch:** `ui/05-dashboard`  
**Depends on:** PR4 merged to `frontend/`  
**Size:** Medium (~450 lines)  
**Estimated Time:** 6–8 hours  
**Design Reference:** `design_reference/v1/Dashboard.html` — server card grid, aggregate metric cards, skeleton/loading/empty/error states. `design_reference/v1/Component Library.html` — ServerCard variants

### Purpose

Implement the complete dashboard page (minus charts — those come in PR6 with ECharts). This includes the server grid with roving tabindex, server cards with the `availableActions` prop pattern, queue depth gauge, system health banner, and the "Dashboard Summary" button for screen readers.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 5.1 | Implement `DashboardPage` (orchestrator) | Route `+page.svelte`. Subscribes to server list, queue, and metrics queries via TanStack Query. Computes `availableActions` per server via `getAvailableActions()`. Passes data to presentational children. Handles server restart/stop/start mutations. "Dashboard Summary" button triggers `aria-live` announcement. Design doc Section 5.1. | `ui/src/routes/+page.svelte` |
| 5.2 | Implement `ServerGrid.svelte` | APG Grid pattern with roving tabindex. Single tab stop for entire grid. Arrow keys navigate between cards. Home/End move to first/last in row. Enter opens server detail. Dynamic removal: focus moves to next card (or previous if last). Design doc Section 9.2. | `ui/src/lib/components/dashboard/ServerGrid.svelte` |
| 5.3 | Implement `ServerCard.svelte` (presentational) | Props: `ServerCardProps` (server, metrics, selected, availableActions). Renders server name, status via `StatusBadge`, key metrics (throughput, latency, slots). Action buttons: renders only the actions in `availableActions`. `onSelect`, `onAction` callbacks. Design doc Section 5.2. | `ui/src/lib/components/dashboard/ServerCard.svelte` |
| 5.4 | Implement `QueueDepthGauge.svelte` (presentational) | Props: `depth`, `capacity`. `role="meter"` with `aria-valuenow/min/max`. Visually hidden `<span>` fallback: "Queue depth: N of M". Design doc Section 9.1. | `ui/src/lib/components/dashboard/QueueDepthGauge.svelte` |
| 5.5 | Implement `SystemHealthBanner.svelte` (presentational) | Props: `healthStatus`, `serverCounts`. Renders banner when any server is degraded/failed. Uses `ErrorBanner` variant. Triple-redundancy via `StatusBadge`. | `ui/src/lib/components/dashboard/SystemHealthBanner.svelte` |
| 5.6 | Implement Dashboard Summary button | Visible button in header (rendered only on dashboard route). `aria-label="Announce dashboard summary"`. On click: compose summary string ("3 servers healthy, 1 degraded. Total throughput 142 tok/s. Queue depth 7.") and inject into `aria-live` region. Design doc Section 9.4. | `ui/src/routes/+page.svelte` |
| 5.7 | Implement server card hover prefetching | `on:mouseenter` on `ServerCard` calls `prefetchServerDetail(server.id)`. Design doc Section 7.1. | `ui/src/lib/components/dashboard/ServerCard.svelte` |
| 5.8 | Implement dashboard skeleton loading state | While server data is loading: render `Skeleton` components matching card dimensions. `--card-min-height: 180px`. CSS Grid with `auto-fill` and `minmax(320px, 1fr)`. Design doc Section 4.4. | `ui/src/routes/+page.svelte` |
| 5.9 | Implement dashboard responsive layout | Grid reflows naturally via CSS Grid `auto-fill`. Below 640px: single column. Cards have `min-width: 320px`. Content max-width: 1400px. Design doc Section 4.5. | `ui/src/routes/+page.svelte` |
| 5.10 | Unit tests: ServerCard renders availableActions | Pass `['restart', 'stop']` → assert 2 buttons. Pass `[]` → assert no buttons. Assert `start` button absent when not in `availableActions`. Design doc test from Section 11.1. | tests |
| 5.11 | Unit tests: getAvailableActions mapping | `running → ['restart', 'stop']`, `stopped → ['start']`, `failed → ['start']`, `restarting → []`. Already tested in PR3 but validate integration here. | tests |
| 5.12 | Unit tests: ServerGrid keyboard | Arrow keys navigate between cards. Enter triggers `onSelect`. Home/End jump to first/last. | tests |
| 5.13 | Unit tests: QueueDepthGauge accessibility | Assert `role="meter"`, `aria-valuenow`, `aria-valuemin`, `aria-valuemax`. Assert hidden fallback span text. | tests |
| 5.14 | Unit tests: Dashboard Summary | Click button → assert `aria-live` region content updated with summary text. | tests |

### PR5 Acceptance Criteria

- [ ] Dashboard renders server cards from TanStack Query data
- [ ] ServerCard renders only `availableActions` from orchestrator
- [ ] ServerGrid uses roving tabindex (single tab stop, arrow key navigation)
- [ ] QueueDepthGauge has `role="meter"` with hidden fallback
- [ ] Dashboard Summary button announces via `aria-live`
- [ ] Server card hover prefetches detail data
- [ ] Skeleton loading state matches card dimensions (no CLS)
- [ ] All 5 test suites pass

### PR5 Creation Steps

```bash
git checkout -b ui/05-dashboard frontend/
# ... implement tasks 5.1–5.14 ...
git add -A && git commit -m "ui: implement dashboard with ServerGrid, ServerCard, and availableActions pattern"
git push -u origin ui/05-dashboard
# Open PR: "Web UI PR5: Dashboard Page"
# Description: "Implements the complete dashboard (FRONTEND_ARCHITECTURE.md Sections 5.1–5.2,
# 9.1–9.4). ServerGrid with APG Grid roving tabindex. ServerCard with availableActions prop
# pattern (no business logic in presentational component). QueueDepthGauge with role=meter
# fallback. Dashboard Summary button for screen readers. Skeleton loading states.
# Server card hover prefetching. 5 test suites."
```

---

## VC-2: App Shell & Dashboard Verification Checkpoint

**Gate:** PR5 merged. The app shell and first real page are working.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-2.1 | Navigation works | Click each sidebar link → correct page renders with `<h1>` | All 7 routes render |
| VC-2.2 | Focus management | Navigate Dashboard → Queue → Dashboard → verify `<h1>` focused each time | `document.activeElement` is `<h1>` |
| VC-2.3 | SSE → Dashboard update | Mock backend pushes `server-update` SSE → card updates without refresh | Card reflects new data |
| VC-2.4 | SSE disconnect → polling fallback | Kill SSE → banner shows "reconnecting" → data still refreshes via polling | Data stays current |
| VC-2.5 | ServerCard actions | Card for running server shows Restart/Stop. Card for stopped server shows Start. | Actions match status |
| VC-2.6 | Keyboard navigation | Tab into dashboard → arrow keys between cards → Enter opens detail → Escape returns | Full keyboard flow works |
| VC-2.7 | axe-core clean | Run `axe-core` on dashboard page in both themes | Zero violations |

---

## PR6: EChartsWrapper & Dashboard Charts

**Branch:** `ui/06-charts`  
**Depends on:** PR5 merged to `frontend/`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 5–7 hours  
**Design Reference:** `design_reference/v1/Dashboard.html` — chart placement, 16:9 aspect ratio, throughput and latency chart layout

### Purpose

Implement the lazy-loaded ECharts integration via the `EChartsWrapper` shared component, then the dashboard-specific chart components: `ThroughputChart` and `LatencyPercentileChart`. This includes RAF-batched updates, 500-point sliding window, `chart.dispose()` + `ResizeObserver.disconnect()` on unmount, and the visually hidden accessible table alternative.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 6.1 | Implement `EChartsWrapper.svelte` | Lazy-loads ECharts via dynamic `import('echarts')`. Shows `Skeleton` placeholder until loaded. Creates chart instance on container. `ResizeObserver` for responsive resize (disconnected on unmount). `chart.dispose()` on unmount. `$effect` cleanup returns both `observer.disconnect()` and `chart.dispose()`. Design doc Section 5.3. | `ui/src/lib/components/shared/EChartsWrapper.svelte` |
| 6.2 | Implement `requestAnimationFrame` batched updates | `setOption()` calls throttled to at most 1 per animation frame per chart. Multiple SSE-driven data updates within one frame coalesced. Design doc Section 8.2. | `ui/src/lib/components/shared/EChartsWrapper.svelte` |
| 6.3 | Implement 500-point sliding window | Each chart data series capped at 500 points. Older points evicted on push. Design doc Section 8.2. | `ui/src/lib/components/shared/EChartsWrapper.svelte` |
| 6.4 | Implement background tab pausing | When `document.hidden === true`, skip chart `setOption()` calls. On tab focus, single bulk update. Design doc Section 8.2. | `ui/src/lib/components/shared/EChartsWrapper.svelte` |
| 6.5 | Implement accessible chart table | Visually hidden `<table>` inside chart container with most recent 20 data points. "Showing most recent 20 of N data points. Full data available via CSV export." Chart `<canvas>` has `aria-hidden="true"`, container has `role="img"` with `aria-label` summary. Design doc Section 9.4. | `ui/src/lib/components/shared/EChartsWrapper.svelte` |
| 6.6 | Implement `ThroughputChart.svelte` (presentational) | Props: `data: TimeSeriesPoint[]`. Line chart showing aggregate throughput over time. ECharts colorblind-safe palette. Pattern textures for line differentiation. | `ui/src/lib/components/dashboard/ThroughputChart.svelte` |
| 6.7 | Implement `LatencyChart.svelte` (presentational) | Props: `data: LatencyPercentilePoint[]`, `percentiles: number[]`. Multi-line chart for p50/p90/p99 latency. | `ui/src/lib/components/dashboard/LatencyChart.svelte` |
| 6.8 | Wire charts into DashboardPage | Add `ThroughputChart` and `LatencyChart` below `ServerGrid`. Feed from metrics TanStack Query. Chart containers use `aspect-ratio` with skeleton placeholders. Design doc Section 4.4. | `ui/src/routes/+page.svelte` |
| 6.9 | Unit tests: EChartsWrapper lifecycle | Mount → assert `import('echarts')` called lazily. Unmount → assert `chart.dispose()` and `observer.disconnect()` called. | tests |
| 6.10 | Unit tests: sliding window | Push 600 points → assert array length === 500, oldest 100 evicted. | tests |
| 6.11 | Unit tests: accessible table | Assert hidden table has ≤ 20 rows. Assert "Showing most recent 20 of N" text. Assert `aria-hidden="true"` on canvas. | tests |

### PR6 Acceptance Criteria

- [ ] ECharts loads lazily — not in initial bundle (verify via `npm run build` output)
- [ ] Skeleton visible until ECharts initializes
- [ ] `chart.dispose()` and `observer.disconnect()` called on unmount (no leaks)
- [ ] Sliding window caps at 500 points
- [ ] Chart updates throttled to 1 per animation frame
- [ ] Background tab pauses chart updates
- [ ] Hidden accessible table limited to 20 data points with "N of M" note
- [ ] Colorblind-safe palette used
- [ ] All 3 test suites pass

### PR6 Creation Steps

```bash
git checkout -b ui/06-charts frontend/
# ... implement tasks 6.1–6.11 ...
git add -A && git commit -m "ui: implement EChartsWrapper with lazy loading, RAF batching, and accessible table"
git push -u origin ui/06-charts
# Open PR: "Web UI PR6: EChartsWrapper & Dashboard Charts"
# Description: "Implements ECharts integration (FRONTEND_ARCHITECTURE.md Sections 5.3, 8.2, 9.4).
# Lazy-loaded via dynamic import. RAF-batched updates. 500-point sliding window. Background tab
# pausing. ResizeObserver + chart.dispose() cleanup. Visually hidden accessible table (20 points).
# ThroughputChart and LatencyChart for dashboard. 3 test suites."
```

---

## PR7: Server Detail Page

**Branch:** `ui/07-server-detail`  
**Depends on:** PR4 merged to `frontend/` (can be developed concurrently with PR5/PR6)  
**Size:** Medium (~350 lines)  
**Estimated Time:** 5–7 hours  
**Design Reference:** `design_reference/v1/Server Detail.html` — slot grid, metrics panel, action bar with pending/failed/confirm states

### Purpose

Implement the server detail page with slot grid, server metrics panel, config summary, and the `ServerActionBar` with `DetailServerAction` type. Reuses `EChartsWrapper` from PR6 for per-server metrics charts.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 7.1 | Implement `ServerDetailPage` (orchestrator) | Route `+page.svelte` at `/servers/[id]`. Fetches server detail, metrics, and slot data via TanStack Query. Computes `availableActions` (extended with `force_kill`). Handles all server actions. `$effect` subscribes to SSE. | `ui/src/routes/servers/[id]/+page.svelte` |
| 7.2 | Implement `SlotGrid.svelte` (presentational) | Props: `slots: SlotState[]`, `serverName: string`. Grid of slot cells showing slot status (idle/processing/loading). Color + icon per status via `StatusBadge`. | `ui/src/lib/components/server/SlotGrid.svelte` |
| 7.3 | Implement `ServerMetrics.svelte` (presentational) | Props: server-level metrics. Renders throughput, latency percentiles, error rate, uptime. Uses `EChartsWrapper` for per-server time series charts. | `ui/src/lib/components/server/ServerMetrics.svelte` |
| 7.4 | Implement `ServerActionBar.svelte` (presentational) | Props: `ServerActionBarProps` (server, availableActions: `Array<DetailServerAction>`, pendingAction: `DetailServerAction \| null`). `onAction: (action: DetailServerAction) => void`. `role="menu"` with `role="menuitem"` children (4 possible actions justifies menu pattern). Disables buttons during pending state. Design doc Section 5.2, 9.1. | `ui/src/lib/components/server/ServerActionBar.svelte` |
| 7.5 | Implement server action focus management | Action button retains focus on state transition — DOM element not replaced, only label/handler updated. Result announced via `aria-live`. Design doc Section 9.3. | `ui/src/lib/components/server/ServerActionBar.svelte` |
| 7.6 | Implement `ConfirmDialog` integration | Destructive actions (stop, force_kill) show confirmation dialog before executing. Non-destructive (restart, start) execute immediately with status feedback. | `ui/src/routes/servers/[id]/+page.svelte` |
| 7.7 | Unit tests: ServerActionBar | Pass `['restart', 'stop', 'force_kill']` → assert 3 menu items. Set `pendingAction: 'restart'` → assert restart button disabled. Verify `role="menu"` and `role="menuitem"`. | tests |
| 7.8 | Unit tests: SlotGrid | Render 8 slots (5 idle, 2 processing, 1 loading) → assert correct StatusBadge for each. | tests |
| 7.9 | Unit tests: action focus retention | Trigger stop action → action completes → verify same button element focused with new label. | tests |

### PR7 Acceptance Criteria

- [ ] Server detail page loads data via TanStack Query with correct `[id]` param
- [ ] `ServerActionBar` uses `DetailServerAction` type (not `string`)
- [ ] `role="menu"` pattern with keyboard navigation
- [ ] Destructive actions require confirmation dialog
- [ ] Action button retains focus through state transitions
- [ ] `EChartsWrapper` reused for per-server charts
- [ ] All 3 test suites pass

### PR7 Creation Steps

```bash
git checkout -b ui/07-server-detail frontend/
# ... implement tasks 7.1–7.9 ...
git add -A && git commit -m "ui: implement server detail page with SlotGrid and ServerActionBar"
git push -u origin ui/07-server-detail
# Open PR: "Web UI PR7: Server Detail Page"
# Description: "Implements server detail (FRONTEND_ARCHITECTURE.md Sections 5.1–5.2, 9.1, 9.3).
# SlotGrid, ServerMetrics with EChartsWrapper reuse, ServerActionBar with DetailServerAction
# type safety and role=menu pattern. Destructive action confirmation. Focus retention through
# state transitions. 3 test suites."
```

---

## PR8: Queue Inspector Page

**Branch:** `ui/08-queue`  
**Depends on:** PR7 merged to `frontend/`  
**Size:** Small–Medium (~250 lines)  
**Estimated Time:** 4–5 hours  
**Design Reference:** `design_reference/v1/Queue Inspector.html` — filter bar, table layout, cancel confirmation, empty state

### Purpose

Implement the queue inspector with filters and the simple interactive table (Tab/Shift-Tab keyboard model — not the APG Grid pattern, which is for the Explorer). Includes cancel request mutation with optimistic UI.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 8.1 | Implement `QueuePage` (orchestrator) | Route `+page.svelte`. Fetches queue via TanStack Query + SSE. Handles cancel mutation with optimistic "Cancelling..." state. | `ui/src/routes/queue/+page.svelte` |
| 8.2 | Implement `QueueFilters.svelte` (presentational) | Props: `filters`, `onChange`. Filter by model, requestor, status. All filters as `<select>` or text input. | `ui/src/lib/components/queue/QueueFilters.svelte` |
| 8.3 | Implement `QueueTable.svelte` | Props: `items: QueueItem[]`, `onCancel`. Table with `<thead>`, `<th scope="col">`. Columns: position, request ID, model, requestor, status, wait time, actions. `QueueRow` inline (not separate file). Simple keyboard model: Tab/Shift-Tab between interactive elements. Design doc Section 9.2. | `ui/src/lib/components/queue/QueueTable.svelte` |
| 8.4 | Implement cancel optimistic UI | Click cancel → row shows "Cancelling..." (grayed, spinner) immediately. On SSE update removing request → row disappears. On API failure → restore original state + error toast. Race condition: if naturally completed, SSE removes row and cancel response absorbed silently. Design doc Section 6.5. | `ui/src/routes/queue/+page.svelte` |
| 8.5 | Implement queue empty state | No items → render `EmptyState` with "Queue is empty" message. | `ui/src/routes/queue/+page.svelte` |
| 8.6 | Unit tests: QueueTable | Render 5 items → assert 5 rows. Assert sort headers have `aria-sort`. Assert cancel button present on each row. | tests |
| 8.7 | Unit tests: cancel optimistic UI | Click cancel → assert row shows "Cancelling..." state. Mock API failure → assert original state restored. | tests |

### PR8 Acceptance Criteria

- [ ] Queue table renders from SSE-fed TanStack Query data
- [ ] Cancel shows optimistic "Cancelling..." state
- [ ] Race condition handled: natural completion + cancel doesn't error
- [ ] Simple keyboard model (Tab/Shift-Tab, not arrow keys)
- [ ] Empty state shown when queue is empty
- [ ] All 2 test suites pass

### PR8 Creation Steps

```bash
git checkout -b ui/08-queue frontend/
# ... implement tasks 8.1–8.7 ...
git add -A && git commit -m "ui: implement queue inspector with cancel optimistic UI"
git push -u origin ui/08-queue
# Open PR: "Web UI PR8: Queue Inspector Page"
# Description: "Implements queue inspector (FRONTEND_ARCHITECTURE.md Sections 6.5, 9.2).
# Simple interactive table with Tab/Shift-Tab keyboard model. Cancel with optimistic
# Cancelling state and race condition handling. 2 test suites."
```

---

## PR9: Request Explorer — TanStack Table, Virtual Scroll & Detail Drawer

**Branch:** `ui/09-explorer`  
**Depends on:** PR7 merged to `frontend/`  
**Size:** Large (~550 lines)  
**Estimated Time:** 8–10 hours  
**Design Reference:** `design_reference/v1/Request Explorer.html` — filter bar, TanStack Table layout, detail drawer, export buttons, virtual scroll

### Purpose

Implement the request explorer with TanStack Table, TanStack Virtual for virtual scrolling over 10K+ rows, the APG Grid keyboard pattern, filter state synced to URL, and the `RequestDetailDrawer` slide-out panel.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 9.1 | Implement `ExplorerPage` (orchestrator) | Route `+page.svelte`. URL-synced filters (model, task type, time range, sort, page). Fetches via `createRequestsQuery(filters)`. Handles export (serializes data, triggers download). Manages drawer open/close state. | `ui/src/routes/explorer/+page.svelte` |
| 9.2 | Implement `ExplorerFilters.svelte` (presentational) | Props: `filters`, `onChange`. Model dropdown, task type dropdown, date range pickers, search input. All filter changes update URL params bidirectionally. | `ui/src/lib/components/explorer/ExplorerFilters.svelte` |
| 9.3 | Implement `ExplorerTable.svelte` | Props: `ExplorerTableProps`. Uses TanStack Table for column definitions, sorting, and row model. TanStack Virtual for virtualized scrolling. APG Grid keyboard pattern: arrow keys navigate cells, Enter enters cell interaction, Escape returns to cell navigation. `RequestRow` inline. `aria-sort` on sortable headers. Design doc Section 9.2. | `ui/src/lib/components/explorer/ExplorerTable.svelte` |
| 9.4 | Implement virtual scroll focus preservation | Save logical index on focus. On DOM recycle: restore focus to same logical position. If row left virtual window: focus moves to table container in grid-navigation mode, announce "Row N is no longer visible". Design doc Section 9.2. | `ui/src/lib/components/explorer/ExplorerTable.svelte` |
| 9.5 | Implement `ExplorerTable` export | `onExport: (format) => void` — signals intent only. `ExplorerPage` handles serialization (CSV via manual string building, JSON via `JSON.stringify`). Triggers browser download via `Blob` + `URL.createObjectURL`. | `ui/src/routes/explorer/+page.svelte` |
| 9.6 | Implement next-page prefetching | When at page N, prefetch page N+1: `prefetchExplorerNextPage(filters, page)`. Design doc Section 7.1. | `ui/src/routes/explorer/+page.svelte` |
| 9.7 | Implement `RequestDetailDrawer.svelte` (presentational) | Props: `request: RequestRecord \| null`, `open: boolean`. Slide-out panel showing full request/response data, timing breakdown, server/slot assignment, prompt/completion text. Focus moves to close button on open, returns to selected row on close. Design doc Section 5.2, 9.3. | `ui/src/lib/components/explorer/RequestDetailDrawer.svelte` |
| 9.8 | Implement responsive table | Three-tier column visibility: all columns > 1024px, hide low-priority columns 768–1024px, minimum columns < 768px. Sticky first column. Design doc Section 4.5. | `ui/src/lib/components/explorer/ExplorerTable.svelte` |
| 9.9 | Unit tests: ExplorerTable sorting | Click column header → assert `onSort` called with correct column. Assert `aria-sort` attribute updates. | tests |
| 9.10 | Unit tests: virtual scroll focus | Scroll focused row out of view → assert focus moves to container. Assert "no longer visible" announcement. | tests |
| 9.11 | Unit tests: RequestDetailDrawer focus | Open drawer → assert close button focused. Close drawer → assert triggering row focused. | tests |
| 9.12 | Unit tests: URL filter sync | Set filters → assert URL params updated. Load URL with params → assert filters populated. | tests |
| 9.13 | Unit tests: export | Click export CSV → assert download triggered with correct content-type. | tests |

### PR9 Acceptance Criteria

- [ ] TanStack Table with sortable columns and `aria-sort`
- [ ] Virtual scroll handles 10K+ rows without performance degradation
- [ ] APG Grid keyboard pattern (arrow key cell navigation)
- [ ] Focus preservation on virtual scroll recycle
- [ ] URL filter sync is bidirectional
- [ ] Next-page prefetching active
- [ ] Detail drawer manages focus correctly (open → close button, close → row)
- [ ] Export signals intent only — orchestrator handles execution
- [ ] Responsive column visibility
- [ ] All 5 test suites pass

### PR9 Creation Steps

```bash
git checkout -b ui/09-explorer frontend/
# ... implement tasks 9.1–9.13 ...
git add -A && git commit -m "ui: implement request explorer with TanStack Table, virtual scroll, and APG Grid keyboard"
git push -u origin ui/09-explorer
# Open PR: "Web UI PR9: Request Explorer"
# Description: "Implements request explorer (FRONTEND_ARCHITECTURE.md Sections 5.2, 6.4, 9.2–9.3).
# TanStack Table + TanStack Virtual for 10K+ row virtual scroll. APG Grid keyboard pattern with
# focus preservation on DOM recycle. URL-synced filters. RequestDetailDrawer with focus management.
# Export as signal-only pattern. Next-page prefetching. Responsive column visibility. 5 test suites."
```

---

## VC-3: Feature Pages Verification Checkpoint

**Gate:** PR9 merged. Dashboard, Server Detail, Queue, and Explorer all working.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-3.1 | Dashboard → Server Detail navigation | Click server card → detail page loads with correct server data | Data matches clicked card |
| VC-3.2 | SSE updates all pages | Push SSE events → verify dashboard cards, queue table, and server detail all update | Live updates work across pages |
| VC-3.3 | Explorer virtual scroll | Load 5000 rows → scroll to bottom → verify smooth scroll, no jank | FPS > 30 during scroll |
| VC-3.4 | Explorer keyboard | APG Grid: arrow keys between cells, Enter to interact, Escape to return | Full keyboard flow works |
| VC-3.5 | Queue cancel lifecycle | Cancel request → "Cancelling..." → SSE removes → row gone | Full lifecycle completes |
| VC-3.6 | axe-core all pages | Run axe-core on dashboard, server detail, queue, explorer | Zero violations on all |

---

## PR10: Configuration Pages — Forms, Diff Preview & Concurrent Editing

**Branch:** `ui/10-config`  
**Depends on:** PR7 merged to `frontend/`  
**Size:** Medium (~450 lines)  
**Estimated Time:** 6–8 hours  
**Design Reference:** `design_reference/v1/Configuration.html` — form layout, diff preview modal, validation errors, concurrent edit banner, priority rules editor

### Purpose

Implement all three config editors (server config, priority rules, global settings) using the shared `createConfigForm()` factory. Includes `DiffPreview`, `beforeunload` + `beforeNavigate` dirty form protection, and lock-on-load + ETag concurrent editing protection with 409 Conflict handling.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 10.1 | Implement `createConfigForm()` factory | Returns reactive form state: `saved` ($state with `structuredClone`), `current` ($state), `isDirty` ($derived via `deepEqual`), `errors` ($derived from validation), `isSubmitting` ($state), `submit()`, `reset()`. Design doc Section 6.6. | `ui/src/lib/utils/configForm.ts` |
| 10.2 | Implement unsaved changes guards | `beforeunload` listener for browser navigation. `beforeNavigate` from `$app/navigation` for SvelteKit client-side routing. Both gated on `isDirty`. Design doc Section 6.6. | `ui/src/lib/utils/configForm.ts` |
| 10.3 | Implement concurrent editing protection | Submit uses `If-Match` with ETag. On 409 Conflict: show banner "Configuration was updated by another source" with Reload button. Focus moves to Reload button. Reload refetches and reinitializes form. Design doc Section 6.6. | `ui/src/lib/utils/configForm.ts` |
| 10.4 | Implement `ServerConfigEditor.svelte` | Uses `createConfigForm()`. Fields per server config schema. Inline validation on blur. Backend-mapped validation errors. | `ui/src/lib/components/config/ServerConfigEditor.svelte` |
| 10.5 | Implement `PriorityRulesEditor.svelte` | Uses `createConfigForm()`. Priority rules table with add/remove/edit rows. | `ui/src/lib/components/config/PriorityRulesEditor.svelte` |
| 10.6 | Implement `GlobalSettingsEditor.svelte` | Uses `createConfigForm()`. Global settings form fields. | `ui/src/lib/components/config/GlobalSettingsEditor.svelte` (or inline in route) |
| 10.7 | Implement `DiffPreview.svelte` (presentational) | Props: `original`, `modified`, `format: 'toml' \| 'json'`. Side-by-side or unified diff view. Mounted in modal via "Review Changes" button (visible when `isDirty`). Design doc Section 6.6. | `ui/src/lib/components/config/DiffPreview.svelte` |
| 10.8 | Implement config route pages | `/config/servers/+page.svelte` and `/config/priority/+page.svelte`. Each is an orchestrator fetching config via TanStack Query and passing to editor components. Tab navigation between config sections using `role="tablist"`. | `ui/src/routes/config/*/+page.svelte` |
| 10.9 | Unit tests: createConfigForm | Change field → assert `isDirty === true`. Reset → assert `isDirty === false`. Submit with validation error → assert button not disabled, errors shown. | tests |
| 10.10 | Unit tests: concurrent editing | Simulate 409 response → assert banner shown, Reload button focused. Click Reload → assert form reinitialized with fresh data. | tests |
| 10.11 | Unit tests: DiffPreview | Pass original/modified objects → assert diff output shows changed fields. | tests |
| 10.12 | Unit tests: beforeNavigate guard | Set `isDirty = true` → trigger client navigation → assert `beforeNavigate` callback fires. | tests |

### PR10 Acceptance Criteria

- [ ] `createConfigForm()` provides reactive `isDirty`, `errors`, `isSubmitting`
- [ ] `isDirty` is `$derived` from deep comparison (never stored manually)
- [ ] `beforeunload` fires on browser navigation when dirty
- [ ] `beforeNavigate` fires on SvelteKit client navigation when dirty
- [ ] 409 Conflict shows banner with Reload button, focus moves to button
- [ ] DiffPreview renders changes in modal
- [ ] Submit disables button (no double-submit)
- [ ] All 4 test suites pass

### PR10 Creation Steps

```bash
git checkout -b ui/10-config frontend/
# ... implement tasks 10.1–10.12 ...
git add -A && git commit -m "ui: implement config editors with createConfigForm, diff preview, and concurrent editing"
git push -u origin ui/10-config
# Open PR: "Web UI PR10: Configuration Pages"
# Description: "Implements all config editors (FRONTEND_ARCHITECTURE.md Section 6.6).
# Shared createConfigForm() factory with $derived isDirty/errors. DiffPreview modal.
# beforeunload + beforeNavigate dirty form guards. Lock-on-load + ETag concurrent editing
# with 409 Conflict handling and Reload button focus. 4 test suites."
```

---

## PR11: Playground Core — Chat, Streaming, Markdown & DOMPurify

**Branch:** `ui/11-playground`  
**Depends on:** PR10 merged to `frontend/`  
**Size:** Large (~550 lines)  
**Estimated Time:** 8–10 hours  
**Design Reference:** `design_reference/v1/Playground.html` — chat layout, parameter sidebar, streaming states, message bubbles, error states, queued state

### Purpose

Implement the playground page with single-model chat: `ChatPane`, `StreamingTokenDisplay`, `MessageBubble`, `ParameterSidebar`, `ModelSelector`, the markdown sanitization pipeline (marked → DOMPurify → `{@html}`), and the ESLint enforcement rule. This PR covers the core streaming experience — comparison mode is PR12.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 11.1 | Implement `PlaygroundPage` (orchestrator) | Route `+page.svelte`. URL-synced model + parameters. Default resolution: model-specific defaults from registry, hardcoded fallbacks (`temp=0.7`, `max_tokens=2048`). URL updated on first render. Manages `StreamingState` ($state). Owns `AbortController` in $state. $effect subscribes to SSE for streaming. Design doc Sections 5.4, 6.4. | `ui/src/routes/playground/+page.svelte` |
| 11.2 | Implement `ChatPane.svelte` (presentational) | Props: `ChatPaneProps` (messages, streamingState). `onSend`, `onCancel`, `onClear` callbacks. Renders message list + input. Streaming state drives UI: idle (input), queued (position indicator), streaming (tokens + cancel button), complete (full response), error (retry button). Design doc Section 5.2. | `ui/src/lib/components/playground/ChatPane.svelte` |
| 11.3 | Implement `MessageBubble.svelte` (presentational) | Props: `message: Message`. Renders user messages as plain text, assistant messages via `renderMarkdown()`. Displays timing metrics on complete messages. | `ui/src/lib/components/playground/MessageBubble.svelte` |
| 11.4 | Implement `StreamingDisplay.svelte` (presentational) | Props: `tokens: string`, `isStreaming: boolean`. Typewriter cursor when streaming, removed when done. `aria-live="off"` during streaming. "Generation complete" announcement on finish. Design doc Section 9.4. | `ui/src/lib/components/playground/StreamingDisplay.svelte` |
| 11.5 | Implement markdown sanitization pipeline | `marked.parse()` → `DOMPurify.sanitize(html, DOMPURIFY_CONFIG)` → `{@html}`. Strict config: `ALLOWED_TAGS`, `ALLOWED_ATTR: ['href', 'class']`, `FORBID_TAGS: ['style', 'script', 'iframe', 'form', 'input', 'img', 'svg', 'math', 'object', 'embed']`, `FORBID_ATTR: ['style', 'onerror', 'onload', 'onclick']`, `ALLOWED_URI_REGEXP: /^(?:https?\|mailto):/i`. Design doc Section 10.1. | `ui/src/lib/utils/markdown.ts` |
| 11.6 | Implement ESLint `no-raw-html-without-sanitize` | Custom ESLint rule: `{@html}` must only appear with `renderMarkdown()` as its argument. Any other `{@html}` usage fails CI. Design doc Section 10.1. | `ui/eslint.config.js` (or `eslint-rules/`) |
| 11.7 | Implement lazy highlight.js integration | Dynamic `import('highlight.js')` in `renderMarkdown()` for code blocks. Only loaded when playground page mounts. Selected grammars: JavaScript, Python, TypeScript, JSON, YAML, Bash (~15 KB). | `ui/src/lib/utils/markdown.ts` |
| 11.8 | Implement `ParameterSidebar.svelte` (presentational) | Props: `ParameterSidebarProps` (params, defaults). Sliders for temperature, top_p. Number inputs for max_tokens, top_k. Reset button. `onChange`, `onReset` callbacks. | `ui/src/lib/components/playground/ParameterSidebar.svelte` |
| 11.9 | Implement `ModelSelector.svelte` (presentational) | Props: `models: ModelInfo[]`, `selected: ModelInfo \| null`. Dropdown with model names. Model list prefetched on mount. | `ui/src/lib/components/playground/ModelSelector.svelte` |
| 11.10 | Implement `ResponseMetrics.svelte` (presentational) | Props: TTFT, tokens per second, total tokens. Shown below completed responses. | `ui/src/lib/components/playground/ResponseMetrics.svelte` |
| 11.11 | Implement `AbortController` pattern | Controller in `$state<AbortController \| null>`. `$effect` creates controller, cleanup calls `controller.abort()`. `handleCancel()` accesses same controller. `AbortSignal.any([controller.signal, AbortSignal.timeout(120_000)])` for dual abort. Design doc Section 5.4. | `ui/src/routes/playground/+page.svelte` |
| 11.12 | Implement queue position announcements | On queue entry: `aria-live="polite"` announcement "Request queued — position N, estimated wait Xs." Silent position updates. On processing start: "Request processing — generating response." Design doc Section 9.4. | `ui/src/lib/components/playground/ChatPane.svelte` |
| 11.13 | Unit tests: renderMarkdown | Input with `<script>` → assert stripped. Input with `<img onerror=...>` → assert stripped. Valid markdown → assert correct HTML. Code block → assert syntax highlighted. | tests |
| 11.14 | Unit tests: ChatPane streaming states | Render with `idle` → assert input visible. Render with `streaming` → assert cancel button. Render with `error` → assert retry button, partial response preserved. | tests |
| 11.15 | Unit tests: AbortController lifecycle | Mount → assert controller created. Cancel → assert aborted. Unmount → assert aborted. | tests |
| 11.16 | Unit tests: ESLint rule | `{@html someVar}` → assert lint error. `{@html renderMarkdown(x)}` → assert passes. | tests |
| 11.17 | Unit tests: URL parameter sync | Navigate to `/playground?temp=0.5` → assert temperature slider at 0.5. Navigate to `/playground` (no params) → assert defaults applied and URL updated. | tests |

### PR11 Acceptance Criteria

- [ ] Streaming lifecycle works: idle → queued → streaming → complete (or error)
- [ ] `StreamingState` is discriminated union — impossible states unrepresentable
- [ ] Markdown sanitization strips all XSS vectors (script, img, onerror, etc.)
- [ ] ESLint rule enforces `{@html}` only with `renderMarkdown()`
- [ ] highlight.js lazy-loaded only on playground page
- [ ] `AbortController` in `$state` accessible to both `$effect` cleanup and cancel handler
- [ ] `AbortSignal.any()` combines user cancel + 120s timeout
- [ ] Queue position announced once, position updates silent
- [ ] URL params sync bidirectionally with defaults on absence
- [ ] All 5 test suites pass

### PR11 Creation Steps

```bash
git checkout -b ui/11-playground frontend/
# ... implement tasks 11.1–11.17 ...
git add -A && git commit -m "ui: implement playground with streaming chat, markdown pipeline, and DOMPurify"
git push -u origin ui/11-playground
# Open PR: "Web UI PR11: Playground Core"
# Description: "Implements playground single-model chat (FRONTEND_ARCHITECTURE.md Sections 5.2,
# 5.4, 6.4, 9.4, 10.1). StreamingState discriminated union. marked → DOMPurify → {@html}
# sanitization pipeline with ESLint CI enforcement. Lazy highlight.js. AbortController in $state
# with AbortSignal.any() dual abort. Queue position announcements. URL parameter sync with
# default resolution. 5 test suites."
```

---

## PR12: Playground Comparison, Model Registry & Remaining Pages

**Branch:** `ui/12-comparison`  
**Depends on:** PR11 merged to `frontend/`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 5–7 hours  
**Design Reference:** `design_reference/v1/Playground.html` — side-by-side comparison, tab fallback at <1280px. `design_reference/v1/Model Registry.html` — model card grid

### Purpose

Add comparison mode to the playground (side-by-side model output), implement the model registry page, and close out any remaining page stubs. ComparisonPane is an orchestrator — it manages its own SSE stream and AbortController.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 12.1 | Implement `ComparisonPane.svelte` (orchestrator) | Props: `ComparisonPaneProps`. Owns its own `ChatPane` instance, `AbortController`, and SSE stream. Closing aborts only this stream. `<section aria-label="Response from {model} (comparison)">` with `<h3>`. `aria-live` announcement on toggle: "Comparison pane opened for {model}." Design doc Sections 5.2, 9.4. | `ui/src/lib/components/playground/ComparisonPane.svelte` |
| 12.2 | Wire comparison into PlaygroundPage | Toggle button adds/removes `ComparisonPane`. Below 1280px: tab switching instead of side-by-side. Shared prompt sync via `sharedPrompt` prop. Design doc Section 4.5. | `ui/src/routes/playground/+page.svelte` |
| 12.3 | Implement comparison responsive fallback | Below 1280px: primary and comparison panes become tabs. `role="tablist"` / `role="tab"` / `role="tabpanel"`. Design doc Section 4.5. | `ui/src/routes/playground/+page.svelte` |
| 12.4 | Implement `ModelRegistryPage` | Route `+page.svelte` at `/models`. Fetches model list via TanStack Query. Renders `ModelCard` grid. | `ui/src/routes/models/+page.svelte` |
| 12.5 | Implement `ModelCard.svelte` (presentational) | Props: `model: ModelInfo`. Displays model name, loaded status, assigned server(s), capabilities. | `ui/src/lib/components/models/ModelCard.svelte` |
| 12.6 | Unit tests: ComparisonPane independence | Open comparison → send message in primary → assert comparison pane unaffected. Cancel comparison → assert only comparison stream aborted. | tests |
| 12.7 | Unit tests: comparison screen reader | Assert `<section aria-label>` with model name. Assert `aria-live` announcement on toggle. | tests |
| 12.8 | Unit tests: responsive fallback | Viewport < 1280px → assert tab interface rendered instead of side-by-side. | tests |

### PR12 Acceptance Criteria

- [ ] ComparisonPane manages its own stream independently
- [ ] Closing comparison aborts only its stream (primary unaffected)
- [ ] Screen reader sections with `aria-label` for each pane
- [ ] `aria-live` announcement on comparison toggle
- [ ] Below 1280px: tab switching instead of side-by-side
- [ ] Model registry page renders model list
- [ ] All 3 test suites pass

### PR12 Creation Steps

```bash
git checkout -b ui/12-comparison frontend/
# ... implement tasks 12.1–12.8 ...
git add -A && git commit -m "ui: implement playground comparison mode and model registry page"
git push -u origin ui/12-comparison
# Open PR: "Web UI PR12: Playground Comparison & Model Registry"
# Description: "Adds comparison mode (FRONTEND_ARCHITECTURE.md Sections 5.2, 9.4).
# ComparisonPane as orchestrator with independent SSE stream and AbortController.
# Screen reader section landmarks. Responsive tab fallback below 1280px.
# Model registry page with ModelCard grid. 3 test suites."
```

---

## VC-4: Full Feature Verification Checkpoint

**Gate:** PR12 merged. All pages implemented.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-4.1 | All routes render | Navigate to each of the 8 route patterns | No blank screens, correct data |
| VC-4.2 | Streaming chat | Send message → tokens stream in → complete with metrics | Full lifecycle works |
| VC-4.3 | Comparison mode | Open comparison, different model → both stream independently | Independent streams |
| VC-4.4 | Config save lifecycle | Edit config → diff preview → save → toast → form reset | Full lifecycle works |
| VC-4.5 | Config concurrent edit | Save from another tab → 409 on submit → banner → reload | Conflict handled correctly |
| VC-4.6 | Explorer 10K rows | Load 10K requests → virtual scroll → no jank | FPS > 30 |
| VC-4.7 | Dark mode all pages | Toggle dark mode → verify every page renders correctly | No contrast violations |
| VC-4.8 | Keyboard-only navigation | Complete all user flows without mouse | All flows possible |
| VC-4.9 | axe-core all pages both themes | Run axe-core on every page in light and dark modes | Zero violations |

---

## PR13: Security Hardening & Rust Build Integration

**Branch:** `ui/13-security-build`  
**Depends on:** PR12 merged to `frontend/`  
**Size:** Medium (~300 lines)  
**Estimated Time:** 4–6 hours  
**Design Reference:** None — this PR is infrastructure (CSP headers, rust-embed, build pipeline). No visual changes.

### Purpose

Implement all security response headers, verify CSP correctness, set up `build.rs` integration with `rust-embed` for single-binary distribution, and configure the Vite build for optimal output.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 13.1 | Implement Rust UI serving handler | axum handler serving embedded UI assets at `/ui/*`. Sets all security response headers. | `src/transport/ui.rs` |
| 13.2 | Implement CSP header generation | Dynamic `connect-src 'self'` (SSE over same origin). Full CSP: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'; upgrade-insecure-requests`. Design doc Section 10.3. | `src/transport/ui.rs` |
| 13.3 | Implement response headers | `X-Frame-Options: DENY`, `Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=(), usb=()`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`. Design doc Section 10.3. | `src/transport/ui.rs` |
| 13.4 | Implement CSRF header enforcement | Backend middleware rejects POST/PUT/PATCH/DELETE requests to `/api/*` without `X-LlamaForge-Request: 1` header. Returns `403 Forbidden`. Design doc Section 10.2. | `src/transport/` (middleware) |
| 13.5 | Implement CORS policy | Do NOT set `Access-Control-Allow-Origin` on any `/api/*` endpoint. Browser default blocks cross-origin reads. Design doc Section 10.2. | `src/transport/` (verify no CORS headers) |
| 13.6 | Configure `build.rs` | Detect `ui/` directory. Run `npm ci && npm run build`. Use `rust-embed` to include `ui/build/` in binary. `debug-embed = false` for development (serves from filesystem). Design doc Appendix C. | `build.rs` |
| 13.7 | Configure Vite production build | `vite.config.ts`: enable `vite-plugin-compression` for gzip sizes. Verify code splitting produces per-route chunks matching Section 8.2 budget. Generate build manifest (`.vite/manifest.json`). | `ui/vite.config.ts` |
| 13.8 | Implement `<link rel="modulepreload">` | In `app.html`, add modulepreload hint for dashboard chunk. Design doc Section 8.4. | `ui/src/app.html` |
| 13.9 | Verify no eval() or new Function() | Grep codebase for `eval(`, `new Function(`, `setTimeout(string)`, `setInterval(string)`. Assert zero matches. Design doc Section 10.1. | — |
| 13.10 | Integration test: CSP blocks inline script | Serve UI with CSP → inject `<script>alert(1)</script>` → verify browser blocks execution. | manual test |
| 13.11 | Integration test: CSRF blocks cross-origin | Send POST to `/api/servers/1/restart` without custom header → assert 403 Forbidden. | tests |
| 13.12 | Integration test: single binary serves UI | `cargo build --release` → run binary → navigate to `/ui/` → assert UI renders. | manual test |

### PR13 Acceptance Criteria

- [ ] CSP header correct on all `/ui/*` responses
- [ ] All 4 response headers present
- [ ] CSRF middleware rejects mutating requests without custom header
- [ ] No `Access-Control-Allow-Origin` header on `/api/*`
- [ ] `cargo build --release` embeds UI assets in binary
- [ ] `debug-embed = false` enables filesystem serving in dev builds
- [ ] No `eval()` or `new Function()` in codebase
- [ ] Per-route chunk sizes within Section 8.2 budgets

### PR13 Creation Steps

```bash
git checkout -b ui/13-security-build frontend/
# ... implement tasks 13.1–13.12 ...
git add -A && git commit -m "ui: implement security headers, CSRF enforcement, and rust-embed build integration"
git push -u origin ui/13-security-build
# Open PR: "Web UI PR13: Security Hardening & Build Integration"
# Description: "Implements security posture and build integration (FRONTEND_ARCHITECTURE.md
# Sections 10.2–10.4, Appendix C). CSP, X-Frame-Options, Permissions-Policy, CSRF middleware.
# rust-embed for single-binary distribution. Vite production build optimization.
# No eval() audit. 2 integration tests."
```

---

## PR14: Test Suite, Bundle Size CI & Final Polish

**Branch:** `ui/14-verification`  
**Depends on:** PR13 merged to `frontend/`  
**Size:** Medium (~400 lines of tests + CI config)  
**Estimated Time:** 6–8 hours  
**Design Reference:** `design_reference/v1/All Screens Dark.html` — dark theme visual regression targets. `design_reference/v1/Responsive.html` — responsive breakpoint regression targets

### Purpose

The final PR implements the Playwright E2E test suite, Playwright visual regression tests, the `check-bundle-size.js` CI script, axe-core accessibility CI, and Lighthouse CI configuration. This is primarily test and CI code — no production logic changes.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 14.1 | Implement E2E tests: Dashboard | Playwright: load dashboard → verify server cards render → click card → verify navigation to detail. | `ui/tests/e2e/dashboard.spec.ts` |
| 14.2 | Implement E2E tests: Explorer | Playwright: load explorer → apply filter → verify table updates → open drawer → verify data. | `ui/tests/e2e/explorer.spec.ts` |
| 14.3 | Implement E2E tests: Config | Playwright: load config → change value → diff preview → save → verify toast. | `ui/tests/e2e/config.spec.ts` |
| 14.4 | Implement E2E tests: Playground | Playwright: load playground → send message → verify streaming response renders. | `ui/tests/e2e/playground.spec.ts` |
| 14.5 | Implement MSW mock backend | Mock Service Worker intercepting all API endpoints and SSE for E2E tests. Realistic data volumes: 8 server cards, 1000 table rows, 3 chat messages with code blocks. Design doc Section 11.2. | `ui/tests/mocks/handlers.ts` |
| 14.6 | Implement visual regression tests | Playwright screenshot comparison for: dashboard (3 servers), explorer with data, config with validation errors, playground with streaming — each in light and dark themes. Threshold: 0.1% pixel diff. Design doc Section 11.3. | `ui/tests/visual/` |
| 14.7 | Implement `check-bundle-size.js` | Reads chunks from `.vite/manifest.json` (auto-discovery). Enforces per-route limits from Section 8.2 and 250 KB aggregate. Compares against `bundle-baseline.json` with 5 KB drift threshold. Design doc Section 11.5. | `ui/scripts/check-bundle-size.js` |
| 14.8 | Create `bundle-baseline.json` | Initial baseline from current build. Committed to version control. | `ui/bundle-baseline.json` |
| 14.9 | Implement axe-core CI | `@axe-core/playwright` runs on every page in both themes. Zero violations = pass. Design doc Section 11.4. | `ui/tests/a11y/axe.spec.ts` |
| 14.10 | Configure Lighthouse CI | Run on dashboard, explorer, playground. Fail thresholds: Performance < 90, Accessibility < 95, Best Practices < 90. Design doc Section 11.5. | `ui/.lighthouserc.js` |
| 14.11 | Implement CI pipeline config | GitHub Actions (or equivalent): `npm ci`, `npm run check`, `npm run lint`, `npm run test:unit`, `npm run build`, `node scripts/check-bundle-size.js`, E2E tests, visual regression, axe-core, Lighthouse. Design doc Appendix C CI Pipeline. | `.github/workflows/ui.yml` |
| 14.12 | Final audit: no raw fetch | `grep -rn "fetch(" ui/src/ --include="*.ts" --include="*.svelte"` — only in `client.ts`. | — |
| 14.13 | Final audit: no bare {@html} | `grep -rn "{@html" ui/src/ --include="*.svelte"` — only with `renderMarkdown()`. ESLint rule also enforces. | — |
| 14.14 | Final audit: no unwanted dependencies | `npm ls --production` — verify no unexpected packages. All runtime deps match Section 8.1 budget methodology. | — |

### PR14 Acceptance Criteria

- [ ] All 4 E2E test suites pass against MSW mock backend
- [ ] Visual regression screenshots captured for 8 screens (4 pages × 2 themes)
- [ ] `check-bundle-size.js` enforces per-route + aggregate limits
- [ ] axe-core reports zero violations on all pages in both themes
- [ ] Lighthouse scores: Performance ≥ 90, Accessibility ≥ 95, Best Practices ≥ 90
- [ ] CI pipeline passes end-to-end
- [ ] No raw `fetch()` outside `client.ts`
- [ ] No bare `{@html}` outside `renderMarkdown()`
- [ ] Bundle sizes within Section 8.2 budgets

### PR14 Creation Steps

```bash
git checkout -b ui/14-verification frontend/
# ... implement tasks 14.1–14.14 ...
git add -A && git commit -m "ui: add E2E tests, visual regression, bundle size CI, axe-core, and Lighthouse"
git push -u origin ui/14-verification
# Open PR: "Web UI PR14: Test Suite, Bundle Size CI & Final Polish"
# Description: "Pure test and CI PR. Playwright E2E tests (4 suites), visual regression (8
# screenshots), check-bundle-size.js with Vite manifest auto-discovery and baseline drift
# tracking, axe-core accessibility CI in both themes, Lighthouse CI.
# MSW mock backend with realistic data volumes. Final codebase audits.
# No production code changes."
```

---

## VC-5: Final Sign-Off Checkpoint

**Gate:** All 14 PRs merged. LlamaForge Web UI is complete.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-5.1 | All tests pass | `npm run test && npm run test:e2e` | Zero failures |
| VC-5.2 | Zero lint warnings | `npm run check && npm run lint` | Exit code 0 |
| VC-5.3 | Bundle size within budget | `node scripts/check-bundle-size.js` | All routes pass, aggregate < 250 KB |
| VC-5.4 | axe-core clean | All pages, both themes | Zero violations |
| VC-5.5 | Lighthouse green | Dashboard, Explorer, Playground | Perf ≥ 90, A11y ≥ 95, BP ≥ 90 |
| VC-5.6 | Single binary serves UI | `cargo build --release && ./target/release/llamaforge` → navigate `/ui/` | UI renders correctly |
| VC-5.7 | CSP active | Check response headers in browser DevTools | All headers present |
| VC-5.8 | CSRF enforced | `curl -X POST http://localhost:8000/api/servers/1/restart` → 403 | Rejected without header |
| VC-5.9 | No XSS vectors | Inject `<script>alert(1)</script>` in playground → verify stripped | Alert never fires |
| VC-5.10 | No raw fetch audit | `grep -rn "fetch(" ui/src/ --include="*.ts" --include="*.svelte"` | Only in `client.ts` |
| VC-5.11 | No bare {@html} audit | ESLint rule + manual grep | Only with `renderMarkdown()` |
| VC-5.12 | Keyboard-only audit | Complete every user flow (dashboard, server action, queue cancel, explorer filter/export, config save, playground chat, comparison) without a mouse | All flows possible |
| VC-5.13 | Screen reader audit | VoiceOver walkthrough of dashboard, playground, and explorer | Announcements are correct, no flooding |
| VC-5.14 | Dark mode audit | Visual inspection of every page in dark mode | No contrast issues, no invisible elements |
| VC-5.15 | Design doc compliance | Cross-check every Section in `FRONTEND_ARCHITECTURE.md` against implementation | Checklist complete |

---

## Summary: PR → File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `ui/package.json`, `svelte.config.js`, `vite.config.ts`, `tsconfig.json`, `app.html`, `app.css` | ~150 |
| PR2 | `lib/stores/preferences.ts`, `lib/stores/theme.ts`, `lib/components/shared/*.svelte`, `lib/utils/format.ts` | ~400 |
| PR3 | `lib/api/client.ts`, `lib/api/types.ts`, `lib/api/queries.ts`, `lib/stores/liveData.ts` | ~500 |
| PR4 | `routes/+layout.svelte`, `lib/components/layout/*.svelte`, `routes/*/+page.svelte` (stubs) | ~450 |
| PR5 | `routes/+page.svelte`, `lib/components/dashboard/*.svelte` | ~450 |
| PR6 | `lib/components/shared/EChartsWrapper.svelte`, `lib/components/dashboard/*Chart.svelte` | ~350 |
| PR7 | `routes/servers/[id]/+page.svelte`, `lib/components/server/*.svelte` | ~350 |
| PR8 | `routes/queue/+page.svelte`, `lib/components/queue/*.svelte` | ~250 |
| PR9 | `routes/explorer/+page.svelte`, `lib/components/explorer/*.svelte` | ~550 |
| PR10 | `routes/config/*/+page.svelte`, `lib/components/config/*.svelte`, `lib/utils/configForm.ts` | ~450 |
| PR11 | `routes/playground/+page.svelte`, `lib/components/playground/*.svelte`, `lib/utils/markdown.ts` | ~550 |
| PR12 | `lib/components/playground/ComparisonPane.svelte`, `routes/models/+page.svelte`, `lib/components/models/ModelCard.svelte` | ~350 |
| PR13 | `src/transport/ui.rs`, `build.rs`, `ui/vite.config.ts` additions | ~300 |
| PR14 | `ui/tests/**`, `ui/scripts/check-bundle-size.js`, `.github/workflows/ui.yml` | ~400 |
| **Total** | | **~5,500** |

> Note: LOC estimate includes ~1,700 lines of tests. Production code is ~3,800 lines. Total may reach ~6,500 with CSS, config files, and test fixtures.

---

## Timeline (Single Engineer)

| Day | Work | PRs |
|-----|------|-----|
| Day 1 | Project scaffold + design system + shared primitives | PR1, PR2 |
| Day 2 | API client, SSE LiveDataStore, TanStack Query | PR3 |
| Day 3 | App shell — layout, sidebar, header, routing | PR4 |
| Day 4 | Dashboard page — ServerGrid, ServerCard, gauges | PR5 |
| Day 5 | EChartsWrapper + dashboard charts | PR6 |
| Day 6 | Server detail page | PR7 |
| Day 7 | Queue inspector | PR8 |
| Day 8–9 | Request explorer (TanStack Table, virtual scroll, drawer) | PR9 |
| Day 10 | Configuration pages (forms, diff preview, concurrent editing) | PR10 |
| Day 11–12 | Playground core (streaming, markdown, DOMPurify) | PR11 |
| Day 13 | Playground comparison + model registry | PR12 |
| Day 14 | Security hardening + Rust build integration | PR13 |
| Day 15–16 | Test suite, bundle CI, Lighthouse, final audits | PR14 |
| Day 17 | Final verification (VC-5), code review fixes | — |

## Timeline (Two Engineers)

| Day | Engineer A | Engineer B |
|-----|-----------|-----------|
| Day 1 | PR1 (scaffold) + PR2 (design system) | Reviews PR1/PR2, writes shared test fixtures, MSW mock setup |
| Day 2 | PR3 (data layer) | Reviews PR3, starts mock backend handlers |
| Day 3 | PR4 (app shell) | PR7 (server detail — can start with app shell stub) |
| Day 4 | PR5 (dashboard) | PR8 (queue) |
| Day 5 | PR6 (charts) | PR9 part 1 (explorer table + virtual scroll) |
| Day 6 | PR10 (config pages) | PR9 part 2 (explorer drawer, export, tests) |
| Day 7 | PR11 part 1 (playground streaming + markdown) | Reviews PR10, starts PR12 (comparison + model registry) |
| Day 8 | PR11 part 2 (ESLint rule, parameter sync, tests) | PR12 complete |
| Day 9 | PR13 (security + build) | PR14 part 1 (E2E tests, visual regression) |
| Day 10 | PR14 part 2 (bundle CI, axe-core, Lighthouse) | VC-5 verification, docs |
| Day 11 | Code review fixes | Code review fixes |

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| ECharts bundle exceeds 200 KB budget | Medium | Medium | Custom build with tree-shaking (`echarts/core` + only used chart types). Verify with `vite-plugin-compression` output. Fallback: replace bar charts with CSS-based gauges. |
| TanStack Virtual performance on 10K+ rows | Low | High | Virtual scroll tested in PR9 with 10K rows. If jank occurs, reduce overscan, simplify row rendering, or paginate server-side. |
| SSE EventSource reconnection inconsistency across browsers | Medium | Low | Firefox and Chrome handle `Last-Event-ID` differently. Integration test with each target browser. Fallback: polling catches up within 5s. |
| Bits UI component accessibility gaps | Low | Medium | axe-core CI catches regressions. Manual VoiceOver testing in VC-5. Replace individual primitives with custom ARIA implementations if needed. |
| Tailwind CSS 4 breaking changes (new major version) | Low | Low | Pin version in `package.json`. Tailwind 4 stable since late 2025. |
| DOMPurify bypass via new HTML spec elements | Low | High | FORBID_TAGS uses an explicit allowlist pattern — new elements are forbidden by default. Keep DOMPurify updated. |
| `adapter-static` limitations for dynamic routes | Low | Low | All routes use `[id]` params resolved client-side. `fallback: 'index.html'` handles all paths. No SSR needed. |
| Vite build manifest format changes | Low | Low | `check-bundle-size.js` reads `.vite/manifest.json` — format stable since Vite 5. Pin Vite version. |
| Svelte 5 runes API changes before stable | Low | Medium | Svelte 5 stable since late 2024. Pin SvelteKit version. |
| `prefers-reduced-motion` not fully tested | Medium | Low | CI runs with motion enabled. Manual testing with `prefers-reduced-motion: reduce` in VC-5. Document which animations are affected. |

---

## Integration Points (for other components)

After the Web UI is complete, the following integration seams are ready:

1. **Rust Backend API** must serve all `/api/*` endpoints matching the TypeScript types in `lib/api/types.ts`. The UI's TanStack Query definitions (`queries.ts`) document every endpoint, expected response shape, and caching behavior.
2. **SSE Endpoint** (`GET /api/events`) must emit the 5 event types documented in the Section 7.1 event type table. Sequential event IDs for `Last-Event-ID` resumption are recommended.
3. **CSRF Middleware** must reject mutating requests without `X-LlamaForge-Request: 1`. The UI's `client.ts` auto-attaches this header.
4. **`build.rs`** runs `npm ci && npm run build` in the `ui/` directory and `rust-embed` includes `ui/build/` in the binary. `debug-embed = false` serves from filesystem during development.
5. **CSP and response headers** are set by the Rust UI handler (`transport/ui.rs`), not by the frontend build process.
6. **Config Manager** provides the data for all three config editors. The UI uses `If-Match` with ETags for optimistic concurrency — the backend must support this.
7. **Model Registry** API provides the model list for playground model selection and the model registry page.
