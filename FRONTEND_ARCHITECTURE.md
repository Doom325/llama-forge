# Front-End Architecture Design: LlamaForge Web UI

**Author:** Sumanth Gottipati  
**Date:** 2026-05-27  
**Status:** Draft  
**Reviewers:** TBD

---

## 1. Overview

### 1.1 Problem Statement

LlamaForge is a production-grade orchestration layer for managing `llama-server` instances. While its gRPC and REST APIs provide full programmatic access, operators and developers need a visual interface to monitor server health, manage configurations, inspect the request queue, explore historical metrics, test models interactively, and (in the future) configure A/B traffic experiments — without dropping to a terminal for every task.

### 1.2 Goals and Non-Goals

**Goals:**

- **G1: Ops Dashboard.** Real-time monitoring of all server instances, queue depth, throughput, latency percentiles, error rates, and VRAM utilization — the single pane of glass for operational health.
- **G2: Server & Process Management.** Start, stop, restart, and configure `llama-server` instances. View slot status, health state, and per-server metrics.
- **G3: Queue Inspector.** View all queued requests with their priorities, positions, and estimated wait times. Cancel or re-prioritize individual requests.
- **G4: Request Explorer.** Query historical requests from SQLite. Filter by model, task type, requestor, time range, and status. Drill into per-request metrics (TTFT, TPS, latency breakdown).
- **G5: Configuration Editor.** View and edit server configurations, priority rules, and global settings through the UI — with crash-safe writeback to TOML config files.
- **G6: Inference Playground.** Interactive chat/completion interface for testing models directly in the browser — prompt construction, parameter tuning, streaming token display, and response comparison.
- **G7: Model Registry.** View all loaded models, their server assignments, quantization details, context sizes, and performance characteristics.
- **G8: Experimentation Platform (v2).** Traffic splitting, A/B model comparison, experiment configuration, and results dashboards. Deferred until backend experimentation infrastructure is built.

**Non-Goals:**

- **NG1: Multi-tenant SaaS features.** No user account management, billing, or tenant isolation in the UI. LlamaForge is an infrastructure tool, not a platform product. Authentication (if needed) is handled at the reverse proxy layer.
- **NG2: Model training or fine-tuning UI.** LlamaForge manages inference, not training pipelines.
- **NG3: Mobile-first design.** The UI is primarily used on desktop workstations. Responsive down to tablet is a courtesy, not a requirement.
- **NG4: Offline operation.** The UI is only useful when the LlamaForge backend is reachable. No service worker or offline caching.
- **NG5: Replacing Grafana.** For teams with existing Prometheus/Grafana stacks, the dashboard supplements — not replaces — their monitoring. LlamaForge provides Prometheus `/metrics` for grafting into existing infrastructure.

### 1.3 Success Criteria

| Metric | Target |
|--------|--------|
| Time to First Contentful Paint (FCP) | < 500ms (served from same host) |
| Time to Interactive (TTI) | < 1s |
| Total JavaScript payload (compressed) | < 250 KB gzipped (excluding lazy-loaded ECharts) |
| Dashboard data freshness | ≤ 2s for critical metrics (server health, queue depth) |
| Accessibility | WCAG 2.1 AA compliance |
| Browser support | Chrome 90+, Firefox 90+, Safari 15+, Edge 90+ |
| Build artifact | Single Rust binary embeds all static assets — no separate frontend deployment |

---

## 2. User Experience

### 2.1 User Flows

**Flow 1 — Operator checks system health (daily driver)**

The operator opens `http://localhost:8000/ui/`. The dashboard loads immediately showing all server instances with color-coded health indicators (green/yellow/red), aggregate queue depth, current throughput (requests/sec, tokens/sec), and latency percentiles. They notice one server is yellow (elevated latency). They click the server card to drill into its detail page — see slot utilization at 95%, KV cache fragmented. They click "Restart" and confirm. The server transitions to "Restarting..." state, the dashboard updates in real-time as the process restarts and health checks pass.

**Unhappy path:** The server restart fails (e.g., VRAM exhaustion). The UI shows the server in "Failed" state with the specific error message from the Server Manager. The operator can view the failure reason, adjust the config (e.g., reduce GPU layers), and retry.

**Flow 2 — Developer tests a new model (playground)**

A developer navigates to the Playground tab. They select a model from the dropdown (populated from the model registry). They compose a system prompt and user message, adjust generation parameters (temperature, top_p, max_tokens), and click "Send." Tokens stream into the response pane in real-time. They see TTFT and TPS metrics alongside the response. They open a second column, select a different model (or different parameters), send the same prompt, and compare outputs side-by-side.

**Unhappy path:** The request is queued behind higher-priority work. The playground shows "Queued — position 3, ETA ~8s" with a live position counter. The developer can choose to wait or cancel.

**Flow 3 — Team lead reviews request patterns (explorer)**

The team lead opens the Request Explorer, sets filters for the last 24 hours, model = "llama-3.1-70b", and task_type = "code_review". A table of matching requests appears, sortable by latency, TTFT, or token count. They click a row to see the full request/response metadata, timing breakdown (queue wait → prefill → decode), and which server/slot handled it. They export the filtered results as CSV.

**Flow 4 — Non-engineer edits priority rules (config)**

A project manager opens Configuration → Priority Rules. They see the current rules rendered as a readable table (not raw TOML). They click "Edit" on a rule, change the priority weight for `task_type = "summarization"` from 500 to 700 using a form field, and click "Save." The UI validates the change, sends `PATCH /api/config`, and shows a confirmation with the diff. The change takes effect immediately via hot reload.

**Unhappy path:** The validation fails (e.g., priority value out of range). The form shows an inline error on the field. No request is sent to the backend.

### 2.2 Key Screens and States

| Screen | Initial Load | Loading | Loaded | Empty | Error |
|--------|-------------|---------|--------|-------|-------|
| **Dashboard** | Skeleton cards with pulsing placeholders for server cards and metric charts | Metrics and server data streaming in, charts animating | Full dashboard with live-updating metrics, server cards, queue depth gauge | "No servers configured. Add a server in Configuration." | "Cannot reach LlamaForge backend. Check if the service is running." with retry button |
| **Server Detail** | Skeleton with server name from URL | Slot grid, metrics charts loading | Full server view: slot grid, health timeline, metrics charts, config summary | N/A (404 if server ID invalid) | Server unreachable or removed — "Server not found" with back link |
| **Queue Inspector** | Skeleton table | Request rows loading | Live-updating table of queued requests with priority, position, ETA | "Queue is empty — all requests are being served immediately." | API error with retry |
| **Request Explorer** | Filter form visible, table area shows "Apply filters to view requests" | Spinner in table area during query | Paginated table with sortable columns | "No requests match your filters." with suggestion to broaden | Query timeout/error with retry |
| **Configuration** | Skeleton config panels | Config loading from backend | Editable forms for server config, priority rules, global settings | "No configuration found. Create `llamaforge.toml`." | Config read error |
| **Playground** | Model selector, empty chat pane, parameter sidebar | "Connecting..." during initial model list fetch | Ready to send prompts, streaming responses | N/A | Model unavailable, server error, queue timeout — each with specific message |
| **Model Registry** | Skeleton cards | Model data loading | Card grid with model details, performance stats, server assignments | "No models loaded." | API error |

---

## 3. Tech Stack Decision

### 3.1 Analysis of Options

The existing backend design document mentions two candidates — `Leptos WASM` and `htmx + Alpine.js` — and chose the latter for the original scope (ops dashboard + config management). With the expanded scope (playground, experimentation platform, side-by-side comparison, broader team usage), we need to re-evaluate.

**Option A: htmx + Alpine.js + minijinja (embedded in Rust binary)**

Advantages: Zero build toolchain, single binary deployment, server-rendered HTML is fast for initial load, aligns with Rust ecosystem, ~5 KB client-side JS (htmx + Alpine), no Node.js dependency.

Disadvantages: Complex interactive patterns (chat streaming, side-by-side comparison, drag-and-drop experiment configuration) become awkward. htmx is designed for document-centric apps, not highly interactive SPAs. Alpine.js is limited for complex state management. The playground and experimentation platform would fight the paradigm — building a real-time token-streaming chat UI with inline metrics in htmx requires excessive server-side fragment orchestration. Security surface area is also higher because htmx executes HTML responses as behavior (the XSS concern already documented in the design review).

**Option B: SvelteKit (compiled to static assets, embedded in binary)**

Advantages: Compiler-first means smaller bundles than React (~30% less JS), reactive model maps naturally to real-time dashboards, less boilerplate than React (40% fewer lines), excellent built-in transitions/animations for dashboard UX. Open WebUI (the most successful local LLM UI) uses SvelteKit. The compiled output is static HTML/JS/CSS that can be embedded in the Rust binary identically to the htmx approach. Svelte's reactivity model handles WebSocket streaming elegantly. Stores are first-class for shared state. Form handling is built-in.

Disadvantages: Smaller ecosystem than React. Fewer off-the-shelf component libraries. Hiring pool is smaller (relevant for broader team contributions). Svelte 5's runes syntax is a paradigm shift from Svelte 4 — the ecosystem is still adapting.

**Option C: React + TypeScript (compiled to static assets, embedded in binary)**

Advantages: Largest ecosystem, most component libraries (Tremor for dashboards, Recharts for charts, shadcn/ui for components). TanStack Query for server state. Massive talent pool. LiteLLM uses React. Concurrent rendering for smooth dashboard updates. Well-understood patterns for every UI need.

Disadvantages: Larger bundle size. More boilerplate. JSX + hooks have a higher complexity floor. Requires a full Node.js build toolchain. React 19 is stable but the ecosystem is still migrating from 18 patterns.

**Option D: Leptos (Rust → WASM)**

Advantages: Full-stack Rust. Type-safe from database to DOM. No JavaScript at all. Fine-grained reactivity (similar to Solid.js). SSR built-in.

Disadvantages: WASM bundles are significantly larger than equivalent JS. The ecosystem is immature for dashboard components — no equivalent of Tremor, Recharts, or ECharts bindings. Compile times are already a concern for the Rust backend; adding a WASM frontend to the compilation doubles build time. Debugging WASM in the browser is painful. The charting story is weak — you'd need to interop with JS charting libraries anyway, negating the "pure Rust" benefit. Production adoption is limited.

### 3.2 Recommendation: SvelteKit 5

**Primary choice: SvelteKit 5** with TypeScript, compiled to static assets and embedded in the Rust binary via `include_dir!` or `rust-embed`.

The reasoning, weighted by LlamaForge's specific needs:

1. **Real-time dashboard is the core use case.** Svelte's reactive stores + fine-grained DOM updates are purpose-built for dashboards where many independent values change frequently. A React virtual DOM diff on every WebSocket message is unnecessary overhead for updating a token counter.

2. **Streaming chat is a first-class pattern.** Svelte's reactive `{#each}` blocks with keyed transitions handle token-by-token streaming naturally. The playground's streaming UX is architecturally simpler in Svelte than in React (no `useEffect` cleanup dance for EventSource connections).

3. **Bundle size discipline.** LlamaForge serves its UI from the same process on localhost. Every KB of JS is embedded in the binary and served over a local connection — but keeping the binary small matters for the "single binary" value proposition. Svelte's compiler-first approach produces significantly smaller bundles than React.

4. **Open WebUI precedent.** The most widely adopted local LLM web interface uses SvelteKit. This validates the framework choice for the exact problem domain (LLM management + chat interfaces).

5. **Deployment simplicity.** `npm run build` produces a `build/` directory of static HTML/JS/CSS. The Rust binary embeds this directory at compile time. No runtime Node.js, no separate web server, no container sidecar. Same deployment model as the htmx approach, but with a richer client-side capability.

6. **Experimentation platform readiness.** The v2 experimentation UI (traffic splitting controls, A/B comparison dashboards, experiment configuration wizards) will require complex interactive patterns that are natural in a component framework but painful in htmx.

**Build integration:** The Rust `build.rs` script will run `npm run build` in the `ui/` directory and embed the output via `rust-embed`. CI builds the frontend as part of `cargo build`. Development uses `vite dev` with a proxy to the running Rust backend.

### 3.3 Supporting Libraries

| Concern | Library | Rationale |
|---------|---------|-----------|
| Charts & visualization | **Apache ECharts** (via `svelte-echarts`) | Already chosen in the backend design doc. Excellent for real-time dashboards, candlestick-style latency charts, heatmaps. 800+ chart types. |
| Component primitives | **Bits UI** (headless) + custom styling | Headless components for Svelte 5 — accessible by default, unstyled, composable. Avoids the "fighting the component library" problem. |
| CSS | **Tailwind CSS 4** | Utility-first, tree-shaken, pairs well with Svelte's scoped styles. Used by Open WebUI. |
| Server state | **TanStack Query for Svelte** | Caching, background refetching, stale-while-revalidate, request deduplication. Mature, framework-agnostic core. |
| Icons | **Lucide Svelte** | Consistent icon set, tree-shakeable, lightweight. |
| Code display | **highlight.js** (syntax highlighting) | Lighter than Shiki (~15 KB gz vs. ~100+ KB gz). Loaded lazily in the playground only. Sufficient for LLM response code block highlighting. |
| Markdown rendering | **marked** + **DOMPurify** | For rendering LLM responses with markdown formatting. Strict sanitization pipeline: `marked.parse()` → `DOMPurify.sanitize(html, strictConfig)` → `{@html}`. See Section 10.1 for the mandatory DOMPurify allowlist. |
| Table | **TanStack Table for Svelte** | For the Request Explorer — virtual scrolling, sorting, filtering, column resizing. Handles 10K+ rows. |

---

## 4. Rendering Strategy

*Addresses Rules 1–4: progressive rendering, document semantics, layout stability, responsiveness.*

### 4.1 Rendering Approach

**Client-Side Rendering (CSR) with static asset embedding.**

The SvelteKit app is compiled to a static export (`adapter-static`). The Rust binary serves these files at `/ui/*` via axum's `ServeDir` middleware (or embedded via `rust-embed`). All rendering happens in the browser.

This is the correct choice because:

- **No SEO requirement.** This is an internal management tool, not a public website.
- **Same-host serving.** The UI is served from the same process on the same machine — network latency to fetch the initial HTML/JS is sub-millisecond. The SSR performance benefit vanishes when there's no network round-trip.
- **Simplified deployment.** No server-side rendering runtime needed in the Rust binary. The backend serves static files and JSON APIs — clean separation.
- **API-driven data.** Every screen's content comes from REST API calls. There's no "content" to pre-render — just UI chrome and loading states.

### 4.2 Progressive Rendering Plan

| Time | What the user sees |
|------|-------------------|
| 0 ms | Browser receives `index.html` from local server (< 1ms network) |
| 50 ms | Critical CSS renders the **hardcoded HTML shell** in `app.html`: sidebar navigation skeleton (links as `<a>` elements), header bar, and content area placeholder. This is real HTML in the template, not Svelte-rendered — it is visible before any JavaScript executes. |
| 100 ms | Svelte mounts into the shell, router resolves to the current view |
| 200 ms | First API call fires (`GET /api/servers`, `GET /api/metrics/summary`) |
| 300 ms | Dashboard skeleton with pulsing placeholder cards visible |
| 500 ms | First API response arrives, server cards populate, queue depth gauge animates in |
| 800–1200 ms | Charts render with initial data, SSE connection established for live updates (800ms best-case on idle CPU; up to 1200ms on loaded GPU workstations — see Section 8.4 timeline variance note) |
| 1000 ms | Fully interactive — all dashboard data loaded, live updates streaming |

These timings assume same-host serving (localhost). Over a LAN (e.g., headless server accessed from a workstation), add 10–50ms for network latency per API call. If API responses take 1–3 seconds (backend under heavy load), the skeleton state persists with pulsing placeholders — the user sees structure and loading intent, never a blank screen.

**`app.html` shell:** The SvelteKit `app.html` template contains hardcoded HTML for the sidebar skeleton, header bar, and a `<noscript>` fallback:

```html
<noscript>
  <div style="padding: 2rem; font-family: system-ui, sans-serif;">
    LlamaForge UI requires JavaScript. Please enable JavaScript to continue.
  </div>
</noscript>
```

**Important caveat:** Because this is a CSR application, the full semantic document structure (navigation links, headings, ARIA landmarks) is generated by Svelte at runtime. The hardcoded shell provides visual structure before JS loads, but the page is not fully functional or semantically complete without JavaScript. This is an acceptable tradeoff for a localhost/LAN management tool with no SEO requirement.

### 4.3 Document Structure

```html
<body>
  <div id="app">
    <nav aria-label="Main navigation">
      <!-- Sidebar: Dashboard, Servers, Queue, Explorer, Config, Playground, Models -->
      <!-- Each entry is an <a href="/ui/..."> link, not a <div> with onclick -->
      <!-- Note: role="navigation" is omitted — <nav> has implicit "navigation" role -->
    </nav>
    <header>
      <!-- Breadcrumb, connection status indicator, global search -->
      <!-- Note: <header> at top level has implicit "banner" role; no explicit role needed -->
    </header>
    <main>
      <!-- Note: role="main" and aria-label omitted — <main> has implicit "main" role,
           and since there is only one <main>, no label is needed -->
      <article>
        <!-- Page-specific content rendered by router -->
      </article>
    </main>
  </div>
</body>
```

**Semantic intent:** Once Svelte mounts, the DOM contains proper semantic structure — `<nav>` for sidebar, `<main>` for content, `<a>` elements for navigation links, `<button>` elements for actions. Since this is CSR, this structure exists only after JavaScript executes (see Section 4.2 for the pre-JS shell). The semantic HTML is generated by Svelte components, not present in the static `app.html`.

**Heading level management:** To prevent heading hierarchy violations in composed components, heading levels are passed as props. A `ServerCard` used inside a `<section>` with `<h2>` receives `headingLevel={3}` and renders its title as `<h3>`. A shared `<DynamicHeading level={n}>` component enforces this pattern. This prevents heading-level mismatches when components are reused across different page contexts.

**Interactive element types:** Server cards are `<a href="/ui/servers/{id}">` elements (semantic navigation). Action buttons (restart/stop/start) are `<button>` elements nested inside the card. The card's click-to-navigate and button actions are independent — clicking the card navigates, clicking a button triggers the action without navigation (via `event.stopPropagation()`).

### 4.4 Layout Stability

- **Fixed sidebar width with smooth transition.** The navigation sidebar is a fixed 240px (collapsible to 64px icon-only mode) with `transition: width 200ms ease` to prevent jarring content shifts. No layout shift on content load.
- **Skeleton screens match loaded dimensions via shared CSS variables.** Skeleton and loaded components share dimension constants defined as CSS custom properties (`--card-min-height: 180px`, `--chart-aspect-ratio: 16/9`, `--table-row-height: 48px`). This prevents skeleton-to-loaded dimension mismatch. Specific contracts:
  - `ServerCard`: `min-height: var(--card-min-height, 180px)`, `width: 100%` within grid cell
  - Chart containers: `aspect-ratio: var(--chart-aspect-ratio, 16/9)` with skeleton placeholder until ECharts finishes rendering (not just until JS loads)
  - Table rows: fixed `height: var(--table-row-height, 48px)` for both skeleton and data rows
- **No CLS from data loading.** Server cards, metric values, and table rows appear in pre-sized containers. Numbers update in-place without reflowing.
- **Grid item insertion/removal animation.** When servers are added or removed (discovered via SSE), the CSS Grid uses Svelte's `animate:flip` directive with `duration: 200ms` for smooth reflow. New cards fade in (`transition:fade`), removed cards fade out. This makes dynamic grid changes feel intentional.
- **Font loading.** System font stack (`-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif`). No custom font files = no FOIT/FOUT = no layout shift.

### 4.5 Responsive Strategy

Content-driven breakpoints, not device-driven:

| Breakpoint | Layout | Rationale |
|-----------|--------|-----------|
| ≥ 1280px | Full sidebar + content + optional detail panel | Primary desktop experience. Side-by-side playground comparison. |
| 960–1279px | Collapsed sidebar (icons only) + full content | Smaller monitors or split-screen workflows. |
| 640–959px | Hidden sidebar (hamburger toggle) + full content | Tablet or narrow windows. Dashboard cards stack to 2 columns. |
| < 640px | Hidden sidebar + single-column content | Emergency mobile access. Functional but not optimized. |

Dashboard cards use CSS Grid with `auto-fill` and `minmax(320px, 1fr)` — they reflow naturally based on available width. Charts resize via ECharts' built-in responsive API.

**Touch targets:** All interactive elements (buttons, links, toggles) have a minimum touch target size of 44×44 CSS pixels, enforced via `min-height: 44px; min-width: 44px` in the base component styles. This ensures usability on tablets (640–959px range) where touch is the primary input.

**Table responsiveness:** Data tables (Queue Inspector, Request Explorer) use horizontal scrolling with a sticky first column at widths < 960px. TanStack Table's column visibility API hides non-essential columns at narrower breakpoints:
- ≥ 1280px: All columns visible
- 960–1279px: Hide "requestor" and "server_id" columns
- < 960px: Show only essential columns (model, status, latency, timestamp) + horizontal scroll for the rest

**Playground comparison fallback:** Below 1280px, the side-by-side comparison layout stacks vertically with tab switching ("Model A" / "Model B" tabs above a single chat pane). The user can still compare by switching tabs, but doesn't get simultaneous visual comparison.

**Content max-width:** The main content area has `max-width: 1400px` with horizontal centering to prevent unreadable line lengths on ultra-wide monitors (> 1920px).

---

## 5. Component Architecture

*Addresses Rules 5–8: contracts, intelligence distribution, composition, lifecycle.*

### 5.1 Component Hierarchy

```
App
├── Layout (shell)
│   ├── Sidebar (navigation, connection status)
│   ├── Header (breadcrumb, global search, notifications)
│   └── RouterOutlet
│       ├── DashboardPage (orchestrator)
│       │   ├── ServerGrid
│       │   │   └── ServerCard (presentational)
│       │   ├── QueueDepthGauge (presentational)
│       │   ├── ThroughputChart (presentational)
│       │   ├── LatencyPercentileChart (presentational)
│       │   └── SystemHealthBanner (presentational)
│       ├── ServerDetailPage (orchestrator)
│       │   ├── SlotGrid
│       │   │   └── SlotCell (presentational)
│       │   ├── ServerMetricsPanel (presentational)
│       │   ├── ServerConfigSummary (presentational)
│       │   └── ServerActionBar (presentational)
│       ├── QueuePage (orchestrator)
│       │   ├── QueueFilters (presentational)
│       │   └── QueueTable
│       │       └── QueueRow (presentational, inline in QueueTable.svelte)
│       ├── ExplorerPage (orchestrator)
│       │   ├── ExplorerFilters (presentational)
│       │   ├── ExplorerTable (virtual-scrolled)
│       │   │   └── RequestRow (presentational, inline in ExplorerTable.svelte)
│       │   └── RequestDetailDrawer (presentational)
│       ├── ConfigPage (orchestrator)
│       │   ├── ServerConfigEditor
│       │   ├── PriorityRulesEditor
│       │   └── GlobalSettingsEditor
│       ├── PlaygroundPage (orchestrator)
│       │   ├── ModelSelector (presentational)
│       │   ├── ParameterSidebar (presentational)
│       │   ├── ChatPane
│       │   │   ├── MessageBubble (presentational)
│       │   │   └── StreamingTokenDisplay (presentational)
│       │   ├── ComparisonPane (orchestrator, optional second column)
│       │   │   └── ChatPane (reused, independent message state)
│       │   └── ResponseMetrics (presentational, TTFT/TPS/tokens)
│       └── ModelRegistryPage (orchestrator)
│           └── ModelCard (presentational)
```

**Intelligence distribution:** Page-level components (`*Page`) are orchestrators — they own data fetching, SSE subscriptions, and coordinate child components. Everything below them is presentational — receives data via props, emits events via callbacks. This means `ServerCard` knows how to render a server's status but does not know how to fetch server data or restart a server.

**Classification of unlabeled components:** `QueueFilters`, `ExplorerFilters`, `ModelSelector` are **presentational** — they receive current filter/selection state as props and emit change events. `ServerMetricsPanel` and `ServerConfigSummary` are **presentational** — they receive data props from `ServerDetailPage` and render read-only views. `StreamingTokenDisplay` is **presentational** — it receives a `tokens: string` prop and renders it; the parent `PlaygroundPage` owns the SSE connection and appends tokens to the prop. `ComparisonPane` is an **orchestrator** (it manages its own ChatPane instance and SSE stream via its own `AbortController`). `ResponseMetrics` is **presentational** (receives TTFT/TPS numbers as props).

### 5.2 Component Contracts

**ServerCard**
```typescript
// Props (inputs)
interface ServerCardProps {
  server: ServerState;            // id, name, status, model, health, slotCount, slotsFree
  metrics: ServerMetricsSummary;  // throughput, latency_p50, latency_p99, error_rate
  selected: boolean;
  availableActions: Array<ServerAction>;  // pre-filtered by orchestrator (see shared types above)
}

// Events (outputs)
onSelect: (serverId: string) => void;
onAction: (serverId: string, action: ServerAction) => void;

// Does NOT know about: API calls, WebSocket connections, other servers, navigation,
// or which actions are valid for a given server state (that logic lives in the orchestrator)
```

**Shared action types** ensure type safety across all action-rendering components:

```typescript
// lib/api/types.ts — shared across ServerCard, ServerActionBar, and orchestrators:
type ServerAction = 'restart' | 'stop' | 'start';
type DetailServerAction = ServerAction | 'force_kill';
```

**Action-availability mapping** lives in the orchestrator, not in `ServerCard`:

```typescript
// In DashboardPage or ServerDetailPage:
function getAvailableActions(status: ServerStatus): Array<ServerAction> {
  switch (status) {
    case 'running': return ['restart', 'stop'];
    case 'stopped': return ['start'];
    case 'failed': return ['start'];
    case 'restarting': return [];
  }
}
```

This separation means `ServerCard` is a pure presentational component — it renders whatever actions it receives without embedding business rules about state-to-action mapping. The mapping function is tested independently.

**ChatPane**
```typescript
// Streaming lifecycle as a discriminated union — impossible states are unrepresentable
type StreamingState =
  | { status: 'idle' }
  | { status: 'queued'; position: number; eta: number | null }
  | { status: 'streaming'; tokens: string; ttft: number }
  | { status: 'complete'; response: string; metrics: ResponseMetrics }
  | { status: 'error'; error: ChatError; partialResponse: string | null };

interface ChatPaneProps {
  messages: Message[];            // role, content, timestamp, metrics
  streamingState: StreamingState; // discriminated union for current streaming lifecycle
}

onSend: (content: string) => void;
onCancel: () => void;
onClear: () => void;

// Does NOT know about: which model is selected, parameter values, API endpoints, SSE connections
```

**ComparisonPane**
```typescript
interface ComparisonPaneProps {
  model: ModelInfo;               // which model this pane is configured for
  parameters: GenerationParams;   // temperature, top_p, max_tokens, etc.
  sharedPrompt: string | null;    // prompt synced from the primary pane (null = independent)
}

onClose: () => void;
onModelChange: (model: ModelInfo) => void;

// ComparisonPane is an orchestrator — it owns its own SSE stream via a dedicated AbortController.
// Closing the pane calls controller.abort() for this stream only; full page navigation aborts all.
```

**StreamingTokenDisplay**
```typescript
interface StreamingTokenDisplayProps {
  tokens: string;                 // accumulated tokens so far (append-only string)
  isStreaming: boolean;           // whether more tokens are expected
}
// Pure presentational — renders tokens with typewriter cursor when streaming, removes cursor when done.
// Does NOT know about: SSE connections, abort controllers, message lifecycle
```

**DiffPreview**
```typescript
interface DiffPreviewProps {
  original: Record<string, unknown>;  // current config values
  modified: Record<string, unknown>;  // proposed config values
  format: 'toml' | 'json';           // display format
}
// Pure presentational — renders a side-by-side or unified diff view. Does NOT know about config API.
```

**ParameterSidebar**
```typescript
interface ParameterSidebarProps {
  params: GenerationParams;       // current parameter values
  defaults: GenerationParams;     // default values (for reset button)
}

onChange: (params: GenerationParams) => void;
onReset: () => void;

// Presentational — renders sliders/inputs for temperature, top_p, max_tokens, etc.
// Does NOT know about: URL sync, model capabilities, API
```

**RequestDetailDrawer**
```typescript
interface RequestDetailDrawerProps {
  request: RequestRecord | null;  // full request data from ExplorerPage — null = closed
  open: boolean;
}

onClose: () => void;

// Presentational — receives full request data from ExplorerPage orchestrator.
// Does NOT fetch its own data. Renders request/response metadata, timing breakdown,
// server/slot assignment, and full prompt/completion text.
```

**ServerActionBar**
```typescript
interface ServerActionBarProps {
  server: ServerState;
  availableActions: Array<DetailServerAction>;  // pre-filtered by orchestrator (uses shared DetailServerAction type)
  pendingAction: DetailServerAction | null;  // which action is currently in-flight (null = none)
}

onAction: (action: DetailServerAction) => void;

// Presentational — renders action buttons, disables during pending state.
// Uses DetailServerAction (extends ServerAction with 'force_kill') because force_kill
// is only available on the server detail page, not the dashboard card.
```

**ExplorerTable**
```typescript
interface ExplorerTableProps {
  requests: RequestRecord[];
  totalCount: number;
  page: number;
  pageSize: number;
  sortColumn: string;
  sortDirection: 'asc' | 'desc';
  loading: boolean;
}

onSort: (column: string) => void;
onPageChange: (page: number) => void;
onRowSelect: (requestId: string) => void;
onExport: (format: 'csv' | 'json') => void;  // signals intent only — ExplorerPage orchestrator handles data serialization and download

// Does NOT know about: filter state, API queries, SQLite, export execution
```

### 5.3 Composition Strategy

**Composition over configuration.** Complex UI patterns are built from simple components, not from mega-components with many boolean flags.

Example — the Dashboard doesn't have a `<Dashboard mode="compact" showQueue={true} showCharts={true} />`. Instead:

```svelte
<DashboardPage>
  <ServerGrid servers={$servers} on:select={handleSelect} />
  <div class="metrics-row">
    <QueueDepthGauge depth={$queueDepth} capacity={$maxQueueDepth} />
    <ThroughputChart data={$throughputHistory} />
  </div>
  <LatencyPercentileChart data={$latencyHistory} percentiles={[50, 90, 99]} />
</DashboardPage>
```

**Slot pattern for extensibility.** Generic wrapper components (not domain-specific ones like `ServerCard`) use Svelte's slot system for customizable sections. For example, a generic `<Card>` layout component uses `<slot name="header" />` and `<slot name="content" />`. Domain-specific components like `ServerCard` use typed props (see Section 5.2) — not slots — because their structure is well-known and type safety matters more than flexibility.

**Shared composables for cross-cutting patterns:**
- **`createConfigForm()`** — Factory that returns reactive form state, validation, `isDirty` computation (via `$derived`), and submit handler. Used by `ServerConfigEditor`, `PriorityRulesEditor`, and `GlobalSettingsEditor` to avoid triplicating form lifecycle logic.
- **`EChartsWrapper`** — Shared chart container component that handles lazy-loading ECharts, responsive resize via `ResizeObserver` (disconnected on unmount), `chart.dispose()` on unmount, and skeleton placeholder during initialization.

### 5.4 Side Effects and Lifecycle

All side effects are confined to orchestrator components (page-level) and Svelte stores. Side effects use **Svelte 5 runes** (`$effect` with cleanup returns), not the Svelte 4 `onMount`/`onDestroy` API:

- **SSE connection** for live dashboard data uses **subscriber-count-based lifecycle** in `liveDataStore`. Each page that needs live data calls `liveDataStore.subscribe()` (increments counter, opens SSE on first subscriber) and returns a cleanup that calls `liveDataStore.unsubscribe()` (decrements counter, closes SSE on last subscriber). This prevents the connection from being dropped when navigating between Dashboard → Queue (both subscribe), and prevents leaks when all subscribing pages unmount.

  ```typescript
  // In any page that needs live data:
  $effect(() => {
    liveDataStore.subscribe();
    return () => liveDataStore.unsubscribe();
  });
  ```

- **Polling intervals** use `$effect` with cleanup:
  ```typescript
  $effect(() => {
    const id = setInterval(pollMetrics, 5000);
    return () => clearInterval(id);
  });
  ```

- **API subscriptions** via TanStack Query handle their own lifecycle — automatic garbage collection when the last subscriber unmounts.

- **Playground SSE streams** use **per-pane `AbortController` instances**. Each `ChatPane` / `ComparisonPane` owns its own `AbortController`. Closing a comparison column calls `controller.abort()` for that stream only. Full page navigation aborts all controllers. This prevents partial teardown (closing one comparison pane) from cancelling both streams.

  The `AbortController` is stored in `$state` so that both the `$effect` cleanup and user-triggered cancel handlers can access it:

  ```typescript
  // In PlaygroundPage / ComparisonPane:
  let controller = $state<AbortController | null>(null);

  $effect(() => {
    controller = new AbortController();
    // ... set up SSE with controller.signal ...
    return () => {
      controller?.abort();
      controller = null;
    };
  });

  function handleCancel() {
    controller?.abort();  // user clicks "Cancel" mid-stream
  }
  ```

  For requests that need both user cancellation and timeout protection, the signals are combined via `AbortSignal.any([controller.signal, AbortSignal.timeout(120_000)])`. This ensures playground streams are aborted on either user cancel or a 2-minute generation timeout — whichever comes first.

**Cleanup guarantee:** Every `$effect` that creates a subscription returns a cleanup function. No orphaned listeners, no leaked connections.

---

## 6. State Management

*Addresses Rules 9–12: source of truth, derived state, optimistic updates, forms.*

### 6.1 State Inventory

| State | Source of Truth | Who Modifies | Scope |
|-------|----------------|-------------|-------|
| Server list & health | TanStack Query cache (fed by SSE) | Backend via SSE → query cache | Global, refreshed via SSE |
| Server metrics (current) | TanStack Query cache (fed by SSE) | Backend via SSE → query cache | Per-server, refreshed via SSE |
| Queue contents | TanStack Query cache (fed by SSE) | Backend via SSE; user can cancel | Global, refreshed via SSE |
| Historical request data | TanStack Query cache (GET /api/metrics/requests) | Backend only | Per-query, cached |
| Aggregated metrics | TanStack Query cache (GET /api/metrics/aggregated) | Backend only | Per-query, cached |
| Dashboard aggregate metrics | `$derived` from per-server metrics in TanStack Query cache | Computed, never stored | Derived |
| Configuration (servers) | TanStack Query cache (GET /api/config/servers) | User via PATCH /api/config | Global, refetched on mutation |
| Configuration (priority rules) | TanStack Query cache (GET /api/config/priority-rules) | User via PATCH /api/config | Global, refetched on mutation |
| Playground messages | Client (in-memory `$state`) | User input + LLM responses | Per-pane (primary ChatPane and each ComparisonPane maintain independent message arrays), lost on navigation |
| Playground streaming state | Client (in-memory `$state`, discriminated union) | SSE events from inference API | Per-pane, see `StreamingState` type in Section 5.2 |
| Playground parameters | URL query params | User input | URL-synced (shareable) |
| Comparison pane layout | Client (`$state`) | User toggle (open/close second pane) | Component-local, not persisted |
| Active filters (explorer) | URL query params | User input | URL-synced |
| SSE connection status | `liveDataStore.connected` (`$state`) | SSE open/close/error events | Global store |
| Server action pending states | Client (`$state` per server ID) | User triggers action, backend confirms | Per-server, transient |
| Toast notification queue | Client (`$state` array) | Any action result (success/error) | Global store, auto-dismissed |
| Sidebar collapsed state | Client (localStorage) | User toggle | Persisted across sessions |
| Selected theme (light/dark) | Client (localStorage + prefers-color-scheme) | User toggle | Persisted |
| Active page / route | URL path | Router | URL |
| Modal/drawer open state | Client (`$state`) | User interaction | Component-local, not persisted |

### 6.2 Server State Strategy

**TanStack Query** manages all server-originated data with the following defaults:

```typescript
// Query defaults (GET requests)
const queryDefaults = {
  staleTime: 5_000,       // data considered fresh for 5s
  gcTime: 300_000,        // cached for 5 min after last subscriber
  refetchOnWindowFocus: true,
  retry: 3,
  retryDelay: (attempt: number) => Math.min(1000 * 2 ** attempt, 10_000),
};

// Mutation defaults (POST/PATCH/DELETE) — NO auto-retry to prevent duplicate side effects
const mutationDefaults = {
  retry: 0,               // never auto-retry mutations (server restarts, config saves)
                           // retry is explicit and user-initiated via retry button
};
```

**Client-enforced timeouts:** The centralized API client applies `AbortSignal.timeout()` to all requests:
- GET requests: 10s timeout (matches the UX table in Section 7.2)
- Mutations (POST/PATCH/DELETE): 30s timeout (config saves and server restarts may be slow)

```typescript
// In api.ts:
function get<T>(url: string): Promise<T> {
  return fetch(url, { signal: AbortSignal.timeout(10_000) }).then(handleResponse);
}
function mutate<T>(url: string, options: RequestInit): Promise<T> {
  return fetch(url, { ...options, signal: AbortSignal.timeout(30_000) }).then(handleResponse);
}
```

For **real-time data** (server health, queue depth), TanStack Query caches are updated directly from SSE messages using `queryClient.setQueryData()` — no additional polling needed while the SSE connection is active. Polling at 5s intervals is the automatic fallback when SSE disconnects.

For **historical data** (request explorer, aggregated metrics), standard fetch-on-demand with the above caching policy.

### 6.3 Client State Strategy

Minimal client-only state:

- **UI preferences** (sidebar collapsed, theme, table column widths): `localStorage` via a thin Svelte store wrapper.
- **Playground conversation**: In-memory Svelte store scoped to the playground page. Not persisted — conversations are ephemeral test sessions, not production data. If users need conversation history, it's in the SQLite audit log.
- **Transient UI state** (modal open, tooltip visible, dropdown expanded): Component-local `$state()` runes. Never lifted to global stores.

### 6.4 URL State

The URL is the source of truth for any view state that a user might want to bookmark or share:

| URL Pattern | State Encoded |
|------------|---------------|
| `/ui/` | Dashboard (default view) |
| `/ui/servers/:id` | Server detail for specific server |
| `/ui/queue` | Queue inspector |
| `/ui/explorer?model=llama-3.1-70b&task=code_review&from=2026-05-26&to=2026-05-27&sort=latency&dir=desc&page=2` | Explorer with full filter state |
| `/ui/config/servers` | Server configuration editor |
| `/ui/config/priority` | Priority rules editor |
| `/ui/playground?model=llama-3.1-70b&temp=0.7&max_tokens=2048` | Playground with model + parameters pre-selected |
| `/ui/models` | Model registry |

Filter state in the explorer and playground parameters are synced to URL query params bidirectionally — changing a filter updates the URL, loading a URL sets the filters. **Default resolution for absent URL params:** When URL params are absent (e.g., navigating to `/ui/playground` without query params), the `PlaygroundPage` orchestrator applies model-specific defaults from the model registry (or hardcoded fallbacks if no model is selected: `temperature=0.7`, `max_tokens=2048`, first available model). The URL is updated to include these resolved defaults on first render, ensuring the URL always reflects the active parameters and is shareable.

### 6.5 Optimistic Update Plan

| Operation | Optimistic? | Rollback Strategy |
|-----------|------------|-------------------|
| Server restart/stop/start | **No** | These are critical infrastructure operations. Show "Restarting..." state and wait for backend confirmation. Failure shows error toast. |
| Cancel queued request | **Partial** | Show "Cancelling..." state (grayed-out row with spinner) immediately. If the cancel API succeeds, the next SSE update removes the row. If it fails, restore the row to its original state and show error toast. **Race condition note:** If the request is naturally completed by the backend while showing "Cancelling...", the next SSE update removes the row normally (it's no longer in the queue). The cancel API response (success or "not found") is silently absorbed — no error toast, since the user's intent (remove the request) was achieved. |
| Config save | **No** | Config changes affect the entire system. Show saving spinner, wait for backend confirmation, then update the UI with the confirmed config. |
| Playground send message | **Yes** (append user message immediately) | User message appears instantly. If the API rejects, mark the message with an error indicator and offer retry. |
| Theme/sidebar toggle | **Yes** (instant) | Client-only state — no rollback needed. |

### 6.6 Form Design

**Configuration Editor forms:**

All three config editors (`ServerConfigEditor`, `PriorityRulesEditor`, `GlobalSettingsEditor`) share a common form lifecycle via the `createConfigForm()` factory (see Section 5.3):

```typescript
// Shared form factory — prevents triplicating form logic across editors
function createConfigForm<T>(initialData: T, validate: (data: T) => ValidationErrors) {
  let saved = $state(structuredClone(initialData));
  let current = $state(structuredClone(initialData));
  let isDirty = $derived(!deepEqual(current, saved));  // computed, never stored manually
  let errors = $derived(validate(current));             // revalidates on every change
  let isSubmitting = $state(false);
  // ... submit(), reset(), field-level error mapping ...
}
```

- **Validation:** Inline validation on blur + full validation on submit. Frontend validation is intentionally loose (range checks, type checks, required fields) — the backend is the authoritative validator. Frontend and backend validation rules will diverge because they are written in different languages (TypeScript vs. Rust). To minimize drift, validation boundaries are documented in a shared `VALIDATION_RULES.md` file, but no code generation mechanism is used in v1. This is an accepted tradeoff — the backend rejects invalid config with specific error messages that are mapped back to form fields.
- **Unsaved changes protection:** Two complementary guards prevent accidental data loss. A `beforeunload` listener warns when navigating away via the browser (closing tab, typing a new URL, browser back button). For in-app navigation, SvelteKit's `beforeNavigate` callback (from `$app/navigation`) intercepts client-side route changes and shows a confirmation dialog when `isDirty` is true. Together, these provide complete unsaved-changes coverage for both browser-level and SPA-internal navigation. The `isDirty` flag is `$derived` (see above) — computed from deep comparison, never stored as a separate boolean.
- **Submission handling:** Submit button disables on click to prevent double-submit. Shows "Saving..." state. On success: toast notification + form resets `saved` to match `current`. On error: re-enables button, shows validation errors mapped to specific fields.
- **Concurrent editing protection (lock-on-load):** The form captures `saved` at initialization and does not live-rebase from TanStack Query cache updates. If another operator (or another browser tab) saves config changes while the form is open, the submit endpoint uses optimistic concurrency control (`If-Match` with ETag or config version field) to reject stale writes with `409 Conflict`. The form shows a banner: "Configuration was updated by another source. Reload to see the latest changes." with a Reload button that refetches and reinitializes the form. Focus moves to the Reload button when this banner appears, since the user's form edits are stale and reloading is the most likely next action. This is simpler and safer than live-rebasing edited fields.
- **Diff preview:** Before submitting config changes, the `DiffPreview` component renders a side-by-side view of `saved` vs. `current` so the user can review what will change. The diff is mounted in a modal triggered by the "Review Changes" button, which appears when `isDirty` is true.

**Playground prompt form:**

- **Submit on Enter** (configurable — Shift+Enter for newline). Submit button also available.
- **No validation** beyond non-empty input.
- **No unsaved changes warning** — playground conversations are ephemeral.

---

## 7. Network Resilience

*Addresses Rules 13–16: request resilience, caching, offline, data fetching.*

### 7.1 Data Fetching Architecture

**Centralized API client.** A single `api.ts` module wraps `fetch` with:

- Base URL resolution (relative to the serving origin — no hardcoded host)
- Response type parsing (JSON for `/api/*`, SSE for streaming endpoints)
- Error normalization (HTTP status codes → typed error objects)
- Request/response logging in development mode

**TanStack Query** sits above the API client for all GET requests:

- Automatic deduplication (two components requesting `/api/servers` share one network request)
- Background refetching (stale data is shown immediately, fresh data replaces it)
- Prefetching on hover (server detail data prefetched when hovering a server card)

**SSE transport layer** (custom Svelte store):

```typescript
// LiveDataStore is transport-only — it does NOT expose data directly.
// Components always read from TanStack Query, never from this store.
interface LiveDataStore {
  connected: boolean;              // SSE connection status (readable by ConnectionStatus component)
  subscriberCount: number;         // number of active page subscribers (internal)
  subscribe(): void;               // increment subscriber count, open SSE on first
  unsubscribe(): void;             // decrement subscriber count, close SSE on last
}
```

The SSE connection hits `GET /api/events` (server-sent events endpoint) and receives a stream of JSON-encoded state updates. **Gap-free reconnection:** The backend assigns sequential event IDs to SSE messages. The `LiveDataStore` relies on `EventSource`'s built-in `Last-Event-ID` header on reconnection to replay missed events, ensuring no state updates are lost during brief disconnections. If the backend does not support event ID replay in v1, the polling fallback (5s) catches up within seconds — this is an acceptable gap for metric and status data that self-corrects on the next update. The store parses each SSE message and calls `queryClient.setQueryData()` to update TanStack Query caches — dashboard components re-render via TanStack Query subscriptions, never via direct store access. This means the same components work identically whether data arrives via SSE (live) or polling (fallback when SSE disconnects).

**Expected SSE event types:** The `LiveDataStore` handles the following event types from `GET /api/events`:

| SSE Event Type | Query Key Updated | Description |
|----------------|-------------------|-------------|
| `server-update` | `['servers']`, `['server', id]` | Server status, health, slot changes |
| `metrics-update` | `['metrics', serverId]` | Per-server throughput, latency, error rate |
| `queue-update` | `['queue']` | Queue contents, positions, ETAs |
| `config-changed` | `['config', section]` | Config modified (from another tab/operator or API) |
| `model-update` | `['models']` | Model loaded/unloaded/swapped |

The `config-changed` event is critical for cross-tab consistency — it ensures that if another tab or API client saves config, all open tabs update their TanStack Query caches.

**SSE message batching:** To prevent main thread saturation from high-frequency updates, incoming SSE messages are buffered and flushed to TanStack Query caches at most once per 100ms via `requestAnimationFrame`. When `document.hidden === true` (tab backgrounded), message processing is paused entirely. **Buffer compaction:** While backgrounded, the buffer uses last-write-wins compaction per target query key (e.g., `['server', 'gpu0']`, `['metrics', 'gpu1']`) — not per SSE event type. This ensures per-server updates are retained independently: if the server pushes updates for 10 different servers, all 10 are preserved with their latest values. On tab focus, the bulk update applies at most one `setQueryData()` per query key, not one per buffered message. The buffer size is bounded by the number of distinct query keys (number of servers + queue + models — typically < 30 entries), keeping both memory usage and processing time constant regardless of how long the tab was backgrounded.

**Prefetching strategy:** Prefetching is applied systematically at navigation boundaries:

- **Server card hover:** `queryClient.prefetchQuery(['server', id])` — server detail data prefetched when hovering a dashboard card, so the detail page loads instantly.
- **Sidebar link hover:** `queryClient.prefetchQuery([routeQueryKey])` — hovering a sidebar navigation link prefetches that route's primary data (e.g., hovering "Queue" prefetches queue contents).
- **Explorer next-page:** When the explorer table is at page N, prefetch page N+1 (`queryClient.prefetchQuery(['requests', { ...filters, page: page + 1 }])`).
- **Playground model list:** The model list is prefetched on PlaygroundPage mount (not on dropdown open) so the dropdown opens instantly.

### 7.2 Error Handling Strategy

| Scenario | User Sees | Recovery |
|----------|-----------|----------|
| API request fails (500) | Red toast: "Failed to load server data. Retrying..." | Auto-retry 3x with exponential backoff. After 3 failures: "Unable to reach LlamaForge. Check if the service is running." with manual retry button. |
| API request fails (4xx) | Contextual inline error (e.g., "Invalid filter value" on the explorer) | User corrects input. No auto-retry for client errors. |
| SSE disconnects | Yellow banner with ⚠ icon at top: "Live updates paused — reconnecting..." + "Last updated: Xs ago" timestamp in dashboard header during polling fallback | Auto-reconnect with exponential backoff plus jitter: `delay = min(1000 * 2^attempt + random(0, 1000), 30_000)`. The jitter prevents thundering-herd reconnection when multiple tabs/operators lose SSE simultaneously (e.g., backend restart). Falls back to 5s polling while disconnected. Banner turns green with ✓ icon on reconnect. All banners use triple-redundancy: icon + text + color. |
| SSE stream error (playground) | Error indicator on the message: "Generation failed: [error message]" | Retry button on the failed message. |
| Playground generation stall | After 30s with no tokens received, transition from `streaming` to `error` state: "Generation appears stalled — no tokens received for 30s." | Cancel and Retry buttons. The backend sends SSE heartbeat comments (`: heartbeat\n\n`) every 15s; if two consecutive heartbeats are missed, the client treats the connection as dead. |
| Network offline | Red banner with ✕ icon: "No network connection" | Monitors `navigator.onLine` as supplementary signal (primary detection is failed API requests and SSE disconnection). Reconnects automatically when online. |
| Request timeout (10s) | "Request timed out. The server may be under heavy load." | Retry button. Timeout is configurable. |

**No blank screens.** Every error state has a specific UI treatment. Every loading state has a timeout that transitions to an error state.

### 7.3 Caching Strategy

| Data | Cache Duration | Invalidation |
|------|---------------|-------------|
| Server list & health | 5s stale, live via SSE | SSE message, manual refresh |
| Server detail metrics | 5s stale, live via SSE | SSE message, navigation to page |
| Queue contents | 2s stale, live via SSE | SSE message, cancel/boost action |
| Historical requests | 30s stale, 5min GC | Manual refresh, new query |
| Aggregated metrics | 30s stale, 5min GC | Manual refresh, time range change |
| Configuration | 60s stale, 10min GC | Config save action (immediate invalidation) |
| Model registry | 60s stale, 10min GC | Server start/stop action |

### 7.4 Offline Strategy

**Explicitly out of scope.** The LlamaForge UI is only useful when the backend is running on the same machine or LAN. There is no meaningful offline experience — you can't manage inference servers that aren't reachable.

The UI gracefully degrades when the backend is unreachable (see error handling above) but does not cache data for offline viewing or queue actions for later sync.

---

## 8. Performance Budget

*Addresses Rules 17–20: budgets, JS cost, images, critical path.*

### 8.1 Performance Targets

| Metric | Budget | Notes |
|--------|--------|-------|
| JavaScript (compressed, all routes combined) | < 250 KB gzipped | Excludes lazy-loaded ECharts. Measured via `vite-plugin-compression` in CI. |
| JavaScript (initial route — dashboard) | < 60 KB gzipped | Shell + dashboard route chunk only. ECharts lazy-loaded after chrome renders. |
| CSS (compressed) | < 25 KB gzipped | Tailwind tree-shaken + Svelte scoped styles. |
| HTML shell | < 5 KB | Hardcoded skeleton in `app.html`. |
| Total initial load (dashboard) | < 100 KB transferred | Before ECharts loads — shell + route chunk + CSS. |
| Total dashboard (charts loaded) | < 300 KB transferred | Fully-functional dashboard including lazy-loaded ECharts. This is the real cost to reach a dashboard with visible charts. |
| First Contentful Paint | < 300ms (localhost), < 500ms (LAN) | |
| Time to Interactive | < 800ms (localhost), < 1.2s (LAN) | |
| Largest Contentful Paint | < 1s (localhost) | |
| Cumulative Layout Shift | < 0.05 | |
| ECharts library (lazy-loaded) | ~200 KB gzipped | Loaded only on pages with charts. |

**Budget methodology:** Estimates are based on measured dependency sizes via bundlephobia.dev, not guesses. The 250 KB total accounts for: Svelte runtime (~10 KB), TanStack Query (~12 KB), TanStack Table (~18 KB), highlight.js with 6 grammars (~15 KB), marked + DOMPurify (~12 KB), Bits UI primitives (~8 KB), application code (~80 KB), and router/utilities (~20 KB). A 50 KB buffer covers underestimation. ECharts (~200 KB) is excluded from the budget because it is lazy-loaded and never on the critical path. **Growth risk dependencies:** The markdown pipeline (marked + DOMPurify, currently ~12 KB) is in the growth-risk category — feature additions like math rendering (KaTeX ~28 KB) or diagram support (mermaid ~200 KB) pull in large sub-dependencies. Any addition to the markdown pipeline should be evaluated against the playground route budget (~40 KB) before merging.

### 8.2 JavaScript Strategy

**Route-based code splitting.** SvelteKit's file-based routing produces per-route chunks automatically:

| Route | Includes | Estimated Size |
|-------|----------|---------------|
| Shell (always loaded) | Svelte runtime, router, sidebar, API client, SSE store, TanStack Query core | ~45 KB gz |
| Dashboard | ServerCard, gauge components, EChartsWrapper (lazy-loads ECharts) | ~15 KB gz + ECharts on demand |
| Server Detail | SlotGrid, metrics panels, EChartsWrapper | ~12 KB gz |
| Queue Inspector | Queue table, filters | ~8 KB gz |
| Request Explorer | TanStack Table, virtual scroll, filters, detail drawer | ~35 KB gz |
| Configuration | Form components (createConfigForm), diff viewer, validation | ~18 KB gz |
| Playground | Chat components, marked + DOMPurify, highlight.js (lazy), parameter controls | ~40 KB gz |
| Model Registry | Card grid | ~5 KB gz |

**Lazy loading:** ECharts (~200 KB gz) is loaded via dynamic `import()` only when an `EChartsWrapper` component mounts. Pages without charts (queue, config, playground) never pay this cost. highlight.js (~15 KB gz with selected grammars) is lazy-loaded only in the playground for code block highlighting.

**ECharts runtime cost management:** The dashboard may have 3–4 chart instances updating from SSE data. To prevent main thread saturation:

1. **`requestAnimationFrame`-batched updates:** Chart `setOption()` calls are throttled to at most 1 per animation frame per chart instance. Multiple SSE updates within one frame are coalesced.
2. **Sliding data window:** Each chart retains at most 500 data points in memory. Older points are evicted as new ones arrive. This caps memory usage and `setOption()` serialization cost.
3. **`chart.dispose()` on route navigation:** The `EChartsWrapper` component calls `chart.dispose()` in its cleanup function to release memory when leaving a page with charts.
4. **Background tab pausing:** When `document.hidden === true`, chart updates are skipped entirely. On tab focus, a single bulk update brings charts current.

### 8.3 Image Strategy

The UI is primarily text, numbers, and charts — minimal images. What images exist:

- **LlamaForge logo:** SVG, inline in the HTML shell. ~2 KB.
- **Status icons:** SVG sprites or Lucide icons. Tree-shaken to only include used icons.
- **No photographs or raster images** in the management UI.
- **Chart exports:** When users export charts as PNG (ECharts feature), the image is generated client-side and downloaded directly.

### 8.4 Critical Rendering Path

1. **`index.html`** (~3 KB) contains: inline critical CSS for the app shell (sidebar, header, skeleton placeholders), `<script type="module">` for the entry chunk, system font stack declaration.
2. **Entry JS chunk** (~40 KB gz) loads, Svelte mounts the shell, router resolves.
3. **Route chunk** (~15 KB gz for dashboard) loads in parallel via `<link rel="modulepreload">`.
4. **ECharts** loads via dynamic import after the dashboard chrome is visible — charts appear after the skeleton, not blocking it.

**No render-blocking resources.** CSS is either inline (critical) or loaded async. JS is `type="module"` (deferred by default). No external fonts.

**Resource hints:**
```html
<link rel="modulepreload" href="/ui/assets/dashboard-[hash].js">
```

No `preconnect` hint is needed — the SSE connection runs over the same HTTP origin that serves the page, so the TCP connection is already established. `preconnect` does not apply to SSE or WebSocket connections initiated from JavaScript.

**Timeline variance note:** The 800ms chart target is best-case (idle CPU). On GPU inference workstations where the CPU may be partially loaded, ECharts parse/compile (~600 KB uncompressed) could push chart first-paint to 1000–1200ms. The skeleton remains visible during this window — the page is usable before charts appear.

---

## 9. Accessibility Plan

*Addresses Rules 21–25: semantics, keyboard, focus, screen readers, color.*

### 9.1 Semantic Structure

```
<nav>           Sidebar navigation (landmark)
<main>          Content area (landmark)
  <header>      Page header with breadcrumb
  <h1>          Page title (e.g., "Dashboard", "Server: llama-70b-gpu0")
  <section>     Logical content sections within a page
    <h2>        Section headings (e.g., "Server Health", "Queue Depth")
    <table>     Data tables (explorer, queue) with <thead>, <th scope="col">
    <form>      Config editors with <label>, <fieldset>, <legend>
```

Native elements are used wherever possible. Custom widgets use ARIA patterns from the [APG (ARIA Authoring Practices Guide)](https://www.w3.org/WAI/ARIA/apg/):

| Widget | ARIA Pattern |
|--------|-------------|
| Sidebar navigation | `<nav aria-label="Main navigation">` (implicit navigation role), `aria-current="page"` on active link |
| Dashboard gauge | `role="meter"`, `aria-valuenow`, `aria-valuemin`, `aria-valuemax`, `aria-label`. **Fallback:** Because `role="meter"` has inconsistent screen reader support, a visually hidden `<span>` inside the gauge provides equivalent text (e.g., "Queue depth: 7 of 100") that is always readable regardless of ARIA support. |
| Server action menu | `role="menu"` with `role="menuitem"` children. **Note:** The APG Menu pattern is used because the actions (restart/stop/start) are commands that don't navigate — they trigger operations. If the action list is short (≤ 3 items), a simple group of `<button>` elements is preferred over a `role="menu"` to reduce keyboard complexity. For the dashboard `ServerCard` (1–2 actions), use plain buttons. For the server detail page `ServerActionBar` (up to 4 actions including force_kill), the `role="menu"` pattern is justified. |
| Modal dialogs (confirm restart) | `role="dialog"`, `aria-modal="true"`, `aria-labelledby` |
| Toast notifications | `role="status"`, `aria-live="polite"` |
| Error banners | `role="alert"`, `aria-live="assertive"` |
| Tabs (config sections) | `role="tablist"` / `role="tab"` / `role="tabpanel"` |
| Explorer table sort | `aria-sort="ascending"` / `"descending"` on `<th>` |

### 9.2 Keyboard Navigation Plan

- **Tab order** follows visual layout: sidebar links → header controls → page content (top to bottom, left to right).
- **Sidebar:** Arrow keys navigate between links. Enter activates. `Ctrl+B` toggles sidebar collapse.
- **Dashboard cards:** The `ServerGrid` uses the **APG Grid pattern with roving tabindex** — the entire grid is a single tab stop. Arrow keys navigate between cards within the grid. Home/End move to the first/last card in the row. Enter on a card opens server detail. This prevents 8+ tab stops for a grid of server cards. **Dynamic removal:** If the focused card is removed (server deleted via SSE), focus moves to the next card, or to the previous card if the removed card was last.
- **Tables — two keyboard models based on complexity:**
  - **Queue Inspector (simple interactive table):** Tab/Shift-Tab moves between interactive elements only (sort headers, cancel buttons, clickable rows). Simpler to implement and navigate for a table with few rows and few actions per row.
  - **Request Explorer (APG Grid pattern):** Arrow keys navigate between cells. Enter enters a cell to interact with its contents (sort toggle, row detail link). Escape returns to cell navigation. This provides faster navigation for the large, data-dense explorer table.
  - **Virtual scroll focus preservation (Explorer):** When TanStack Virtual recycles DOM nodes during scroll, the focused row's logical index is saved. If the focused element is recycled, focus is restored to the newly rendered element at the same logical position. If the row is no longer in the virtual window, focus moves to the table container in **grid-navigation mode** (not cell-interaction mode) and a visually hidden "Row N is no longer visible" announcement fires. This ensures that regardless of whether the user was in cell-interaction mode when the recycle occurred, the post-recycle state is always the broader grid-navigation mode — preventing the user from being stranded in cell-interaction mode with no cell to interact with.
- **Forms:** Standard tab/shift-tab between fields. Enter submits (except in textareas).
- **Modals:** Focus trapped inside. Tab cycles through modal controls. `Escape` closes.
- **Playground:** `Enter` sends message (configurable). `Ctrl+Enter` always sends. `Escape` cancels streaming.

**Skip links:** "Skip to main content" link at the top of the page, visible on focus.

**Keyboard shortcut discoverability:** A "Keyboard shortcuts" help overlay is accessible via `?` key (when no input is focused) or via a button in the header. The overlay lists all keyboard shortcuts grouped by context (global, dashboard, tables, playground). New shortcuts (like the dashboard summary) are announced in the overlay so users can discover them without reading documentation.

**Drag-and-drop keyboard alternative (v2 principle):** When the v2 experimentation platform introduces drag-and-drop experiment configuration, every drag-and-drop interaction must have a keyboard equivalent. The principle is established now: use "pick up" (Space), "move" (Arrow keys), "drop" (Space), and "cancel" (Escape) — following the APG drag-and-drop pattern. This will be fully specified when the experimentation UI is designed.

**Focus indicator strategy (design system requirement):** All focusable elements display a visible focus ring when focused via keyboard (`focus-visible`). Specification: 2px solid ring with ≥ 3:1 contrast against the surrounding background, applied via Tailwind's `focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:ring-blue-500` utilities in the base component styles. This overrides Tailwind's default `outline: none` reset. The focus ring color adapts to the active theme (light: blue-600, dark: blue-400) via CSS custom properties.

### 9.3 Focus Management Plan

| Interaction | Focus Behavior |
|------------|---------------|
| Navigate to new page | Focus moves to `<h1>` of the new page (`tabindex="-1"` on `<h1>` enables programmatic focus without adding it to tab order; announced by screen reader) |
| Open modal dialog | Focus moves to the first focusable element inside the modal |
| Close modal dialog | Focus returns to the element that triggered the modal |
| Open detail drawer (explorer) | Focus moves to the drawer's close button |
| Close detail drawer | Focus returns to the selected table row |
| Toast notification | No focus change — announced via `aria-live` region |
| Server action completes | Focus stays on the action button. If the button's label/availability changes due to the state transition (e.g., "Stop" becomes "Start" after stopping), the same button retains focus with its new label — the DOM element is not replaced, only its text and handler change. Result is announced via the `aria-live` status region. |
| Banner dismissed | Focus moves to the element that was focused before the banner appeared. If no prior focus is recorded (banner appeared on page load), focus moves to the page `<h1>`. |
| Delete/cancel operation | Focus moves to the next logical element (next row, or the empty state) |
| Virtual scroll (Explorer) | When DOM recycling removes the focused row, focus is managed by the virtual scroll focus preservation mechanism (see Section 9.2). |

### 9.4 Screen Reader Strategy

- **Live regions:** A single persistent `aria-live="polite"` region in the app shell for discrete status updates ("Server restarted successfully", "Config saved", "3 requests in queue"). This region is used only for actionable events — not for continuous metric changes.
- **Dashboard metrics — silent updates with on-demand summary:** Individual metric values (throughput, latency, queue depth) update silently — they do NOT use `aria-live`. Announcing every SSE-driven metric change (potentially multiple per second across multiple cards) would flood the screen reader and make the dashboard unusable. Instead, each metric card has a descriptive `aria-label` with full context ("Server throughput: 42 tokens per second") that is read when the user navigates to it. A visible **"Dashboard Summary" button** (with `aria-label="Announce dashboard summary"`) in the dashboard header triggers a single summary announcement into the `aria-live` region: "Dashboard summary: 3 servers healthy, 1 degraded. Total throughput 142 tok/s. Queue depth 7." This is a button rather than a hidden keyboard shortcut because: (a) buttons are discoverable without documentation, (b) `Ctrl+Shift+S` conflicts with browser "Save As" in Chrome/Firefox, and (c) screen reader users rely heavily on browser shortcuts. The button is also listed in the keyboard shortcuts overlay (accessible via `?`).
- **Streaming tokens (playground):** The response area uses `aria-live="off"` during streaming (token-by-token updates would flood the screen reader). A "Generation complete" announcement fires when streaming ends, and the user can then read the full response.
- **Charts:** Every chart has a text alternative — a visually hidden `<table>` within the chart container that presents the same data in tabular form, accessible to screen readers. The `EChartsWrapper` component generates this table from the chart's data series, limited to the most recent 20 data points (not all 500 from the sliding window) to keep screen reader navigation manageable. A visually hidden note precedes the table: "Showing most recent 20 of {N} data points. Full data available via CSV export." The chart `<canvas>` element carries `aria-hidden="true"` and the container has `role="img"` with an `aria-label` summarizing the trend ("Throughput over last hour: trending upward from 30 to 45 tok/s").
- **Comparison pane screen reader navigation:** When the playground is in comparison mode, each pane is wrapped in `<section aria-label="Response from {model name}">` with an `<h3>` identifying the model. Screen reader users navigate between panes via landmark navigation (sections are landmarks). The primary pane is `<section aria-label="Response from llama-3.1-70b">` and the comparison pane is `<section aria-label="Response from llama-3.1-8b (comparison)">`. When comparison mode is toggled on, an `aria-live` announcement fires: "Comparison pane opened for {model name}."
- **Connection status announcements:** The `ConnectionStatus` component in the header uses `aria-live="polite"` for state changes. Announcements fire only on transitions: "Live updates connected" (on reconnect), "Live updates paused — reconnecting" (on disconnect). The steady-state "Connected" label is read on navigation but not announced repeatedly.
- **Playground queue position:** When a playground request is queued, the queue position is announced once via `aria-live="polite"`: "Request queued — position 3, estimated wait 8 seconds." Position updates are silent (they change every second as the queue moves). The user can navigate to the queue indicator to read the current position on demand. When the request moves to processing, a single announcement fires: "Request processing — generating response."
- **Loading states:** Skeleton containers carry `aria-busy="true"` and `aria-label="Loading server data"`. When data arrives, `aria-busy` is removed. Screen readers announce the transition. This is preferred over injecting "Loading..." text that may briefly flash for sighted users.

### 9.5 Color and Contrast

- **Contrast ratios:** All text meets WCAG AA (4.5:1 for normal text, 3:1 for large text). Verified with automated tooling in CI.
- **Status indicator triple-redundancy (design system rule):** All status indicators across the entire application use icon + text label + color — never color alone. This applies to:
  - Server health: ✓ Healthy (green) / ⚠ Degraded (yellow) / ✕ Failed (red)
  - Queue request status in table cells: same pattern
  - Explorer request status in table cells: same pattern
  - Connection status badge: ● Connected (green) / ◌ Reconnecting (yellow) / ✕ Disconnected (red)
  - Server action pending states: ⟳ Restarting... / ⟳ Stopping... / ⟳ Starting...
  This rule is enforced as a design system component (`<StatusBadge status={status} />`) so individual implementations cannot accidentally use color-only indicators.
- **Charts:** ECharts configured with patterns/textures in addition to colors for line/bar differentiation. Colorblind-safe palette (Category10 or similar).
- **Dark mode (design constraint, not post-hoc verification):** Dark mode contrast ratios are a design-time constraint, not a test-time check. The design token system defines color pairs where both light and dark variants meet WCAG AA. The CSS custom property definitions in `app.css` include both `--color-text-primary: #1a1a1a` (light) and `--color-text-primary: #e5e5e5` (dark), and each pair is verified against its background at definition time. The axe-core CI check runs against both themes to catch regressions, but the intent is that contrast is correct by construction.
- **Reduced motion:** `@media (prefers-reduced-motion: reduce)` disables chart animations, skeleton pulse, sidebar transitions, grid `animate:flip`, and `transition:fade` effects. Functional behavior unchanged.

---

## 10. Security Posture

*Addresses Rules 26–28: XSS, data safety, CSP.*

### 10.1 Input Handling

**User input enters the system in three places:**

1. **Configuration editor forms.** Input is validated client-side (type checks, range checks) and sent as JSON to `PATCH /api/config`. The backend is the authoritative validator. The UI never renders user-supplied config values as raw HTML — Svelte's default behavior escapes all interpolated values.

2. **Playground prompt input.** Free-text sent to `POST /v1/chat/completions`. Responses are rendered as markdown via the strict sanitization pipeline described below (no raw `{@html}` on untrusted content). Code blocks are syntax-highlighted via highlight.js (text-only, no HTML execution).

3. **Explorer filter inputs.** Query parameters sent to `GET /api/metrics/requests`. Values are URL-encoded. The UI renders filter values as text content, never as HTML.

**Svelte's default escaping** handles the primary XSS vector — `{variable}` in Svelte templates is always text-escaped. The `{@html}` directive (raw HTML rendering) is used only for the markdown renderer's output, which passes through the strict sanitization pipeline below.

**Server-reflected data** (model names, requestor strings, error messages from the backend) is always rendered as text content via Svelte's default escaping. No `{@html}` on any server-originated data.

**Markdown sanitization pipeline (strict chain — do not shortcircuit):**

LLM outputs are untrusted — they can be influenced by prompt injection from upstream data. The rendering pipeline must be an explicit, ordered chain:

```typescript
// lib/utils/markdown.ts — the ONLY place {@html} content is produced
import { marked } from 'marked';
import DOMPurify from 'dompurify';

const DOMPURIFY_CONFIG = {
  ALLOWED_TAGS: [
    'p', 'br', 'strong', 'em', 'code', 'pre', 'ul', 'ol', 'li',
    'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'a',
    'table', 'thead', 'tbody', 'tr', 'th', 'td', 'hr', 'del', 'sup', 'sub',
    'span', // for syntax highlighting classes only
  ],
  ALLOWED_ATTR: [
    'href', // on <a> only — protocol-restricted below
    'class', // for syntax highlighting CSS classes
  ],
  FORBID_TAGS: ['style', 'script', 'iframe', 'form', 'input', 'img', 'svg', 'math', 'object', 'embed'],
  FORBID_ATTR: ['style', 'onerror', 'onload', 'onclick'], // DOMPurify strips all on* event handlers by default; these are listed explicitly for documentation clarity
  ALLOW_DATA_ATTR: false,
  // Restrict <a> href to safe protocols only
  ALLOWED_URI_REGEXP: /^(?:https?|mailto):/i,
};

// Step 1: marked parses markdown to HTML (sanitize: false is marked's default — fine because DOMPurify runs next)
// Step 2: DOMPurify strips anything not in the allowlist
// Step 3: Only the return value of this function may be used with {@html}
export function renderMarkdown(untrustedMarkdown: string): string {
  const rawHtml = marked.parse(untrustedMarkdown);
  return DOMPurify.sanitize(rawHtml, DOMPURIFY_CONFIG);
}
```

In Svelte components, the pipeline is invoked as:

```svelte
<div class="prose">{@html renderMarkdown(message.content)}</div>
```

**Enforcement rule:** `{@html}` must never appear with any argument other than the output of `renderMarkdown()`. This is verified by a custom ESLint rule (`no-raw-html-without-sanitize`) that fails CI if `{@html}` is used with any other expression.

### 10.2 Authentication and Data Storage

**v1: No authentication.** LlamaForge is a local/LAN tool. The threat model assumes the network is trusted. If auth is needed, it's handled at the reverse proxy layer (nginx, Caddy, or similar) — the UI does not manage sessions or tokens.

**CSRF protection (v1 — required even without authentication):**

Even without authentication, CSRF is a real threat for localhost tools. An attacker can host a page that sends `POST http://localhost:8000/api/servers/gpu0/restart` via a hidden form — the browser happily sends the request to localhost with no auth needed. This is the exact attack model that historically affected Redis, Elasticsearch, and Docker daemon.

Mitigation: All state-changing requests (POST, PUT, PATCH, DELETE) must include a custom header:

```
X-LlamaForge-Request: 1
```

The Rust backend rejects any mutating request missing this header with `403 Forbidden`. Browsers do not attach custom headers to cross-origin form submissions or `no-cors` fetch requests, so this blocks CSRF from external origins.

**CORS policy (protects SSE and all API endpoints from cross-origin reads):** The Rust backend does NOT set `Access-Control-Allow-Origin` on any `/api/*` endpoint. This is the default behavior — without an explicit CORS header, browsers block cross-origin reads. This protects the SSE endpoint (`GET /api/events`) from cross-origin data exfiltration: a malicious page at `evil.example.com` cannot open an `EventSource` to `http://localhost:8000/api/events` and read server health, queue contents, or metrics. The CSRF custom header protects writes; the CORS policy protects reads. Together, they provide complete cross-origin defense.

The centralized API client (`lib/api/client.ts`) adds the CSRF header automatically to every non-GET request:

```typescript
// lib/api/client.ts
async function apiFetch(url: string, options: RequestInit = {}) {
  const method = (options.method || 'GET').toUpperCase();
  const headers = new Headers(options.headers);
  if (method !== 'GET' && method !== 'HEAD') {
    headers.set('X-LlamaForge-Request', '1');
  }
  return fetch(url, { ...options, headers });
}
```

**Future (v2): Token-based auth.** If multi-user authentication is added:

- JWT stored in `HttpOnly`, `Secure`, `SameSite=Strict` cookie — never in `localStorage`.
- `SameSite=Strict` provides a second layer of CSRF defense (browser won't attach the cookie on cross-origin requests), combined with the custom header for defense-in-depth.
- No sensitive data in `localStorage` — only UI preferences (theme, sidebar state).
- Token refresh: the API client detects `401` responses and triggers a transparent refresh via a dedicated `/api/auth/refresh` endpoint. Open design questions for v2: token expiry duration, refresh token rotation strategy, server-side revocation mechanism, and handling of in-flight SSE connections when a token expires.

**Currently in the browser:** Only UI preferences in `localStorage`. No sensitive data.

### 10.3 Content Security Policy

The Rust backend sets the following CSP header on all `/ui/*` responses. The `connect-src` port is dynamically generated from the configured listen port (not hardcoded):

```
Content-Security-Policy:
  default-src 'self';
  script-src 'self';
  style-src 'self' 'unsafe-inline';
  img-src 'self' data: blob:;
  connect-src 'self';
  font-src 'self';
  object-src 'none';
  base-uri 'self';
  form-action 'self';
  frame-ancestors 'none';
  upgrade-insecure-requests;
```

Additionally, the following response headers are set on all `/ui/*` and `/api/*` responses:

```
X-Frame-Options: DENY
Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=(), usb=()
X-Content-Type-Options: nosniff
Referrer-Policy: strict-origin-when-cross-origin
```

- **No `unsafe-eval`** — Svelte's compiled output doesn't use `eval()`. ECharts works without it.
- **`unsafe-inline` for styles** — Required for Svelte's scoped styles and ECharts' inline style mutations. **Accepted risk:** nonce-based `style-src` is impractical because ECharts programmatically generates inline styles on `<canvas>` wrapper elements that cannot be nonce-tagged. The risk is mitigated by: (a) the DOMPurify config in Section 10.1 strips `style` attributes from all user/LLM content, and (b) `script-src 'self'` (no `unsafe-inline`) prevents style injection from escalating to script execution. CSS-based data exfiltration via `background: url(...)` remains theoretically possible only if an attacker first achieves HTML injection past DOMPurify — a chained attack with low probability given the strict allowlist.
- **`data:` and `blob:` for images** — Required for ECharts chart-to-image export and SVG icon data URIs. **Defense-in-depth note:** This directive is broader than ideal, but the DOMPurify config (Section 10.1) includes `img` in `FORBID_TAGS`, preventing any user/LLM-controlled `<img>` injection. The `img-src` allowance applies only to images created by application code. If `img` is ever added to the DOMPurify allowlist, `img-src` should be tightened simultaneously.
- **`connect-src 'self'`** — SSE connections run over the same HTTP origin that serves the page, so `'self'` is sufficient. No WebSocket (`ws://`/`wss://`) entries are needed in v1's SSE-only architecture. If WebSocket transport is added in v2, the appropriate `ws://` directive should be added at that time with the dynamically configured port.
- **`upgrade-insecure-requests`** — Ensures subresources use HTTPS when LlamaForge is deployed behind a TLS-terminating reverse proxy.
- **`Permissions-Policy`** — Explicitly disables browser features not used by LlamaForge. Defense-in-depth against injected code attempting to access camera, microphone, geolocation, or payment APIs.
- **`frame-ancestors 'none'`** — Prevents clickjacking. Combined with `X-Frame-Options: DENY` (see response headers above) for defense-in-depth against older browsers.

### 10.4 Third-Party Risk

| Resource | Trust Level | Mitigation |
|----------|------------|-----------|
| SvelteKit | Build-time only | Not shipped to the browser. Build output is vanilla JS. Supply chain risk managed via `npm audit` and lockfile pinning. |
| ECharts | Runtime (loaded in browser) | Loaded from embedded static assets (not CDN). Version pinned. SRI not needed since served from self. |
| DOMPurify | Runtime (markdown sanitization) | Well-audited, single-purpose library. Version pinned. |
| TanStack Query | Runtime | Widely adopted, well-maintained. Version pinned. |

**No external CDN resources.** All JavaScript, CSS, and assets are bundled and embedded in the Rust binary. The browser makes zero requests to third-party domains. This eliminates CDN compromise, availability dependency, and privacy leakage risks.

---

## 11. Testing Strategy

### 11.1 Component Tests

**Svelte Testing Library + Vitest.** Every presentational component gets tests for:

- Rendering in all states (loaded, loading, empty, error)
- User interactions (click, keyboard, form input)
- Accessibility assertions (role, aria attributes, focus management)

Example:
```typescript
test('ServerCard renders only the actions provided via availableActions prop', async () => {
  const { getByRole, queryByRole } = render(ServerCard, {
    server: { ...mockServer, status: 'running' },
    availableActions: ['restart', 'stop'],
  });
  expect(getByRole('button', { name: /restart/i })).toBeDefined();
  expect(getByRole('button', { name: /stop/i })).toBeDefined();
  expect(queryByRole('button', { name: /start/i })).toBeNull();
});

test('getAvailableActions returns correct actions for each server status', () => {
  expect(getAvailableActions('running')).toEqual(['restart', 'stop']);
  expect(getAvailableActions('stopped')).toEqual(['start']);
  expect(getAvailableActions('failed')).toEqual(['start']);
  expect(getAvailableActions('restarting')).toEqual([]);
});
```

### 11.2 Integration Tests

**Playwright** for end-to-end flows against a mock backend:

- Dashboard loads and displays server data
- Navigate to server detail, trigger restart, verify state transitions
- Explorer: apply filters, verify table updates, open detail drawer
- Config editor: change a value, see diff preview, save, verify confirmation
- Playground: send a message, verify streaming response renders

Mock backend uses MSW (Mock Service Worker) to intercept fetch/SSE at the network level — tests run against the real UI code, not a mocked DOM.

### 11.3 Visual Regression

**Playwright screenshot comparison** for key screens in both light and dark themes:

- Dashboard with 3 servers (healthy, degraded, failed)
- Explorer table with data
- Config editor with validation errors
- Playground with streaming response

Threshold: 0.1% pixel difference. Stored in version control.

### 11.4 Accessibility Testing

- **Automated (CI):** axe-core via `@axe-core/playwright` on every page in both themes. Zero violations = pass.
- **Manual (pre-release):** VoiceOver walkthrough of each user flow. Keyboard-only navigation of all interactive elements.

### 11.5 Performance Testing

- **Bundle size check (CI):** A `check-bundle-size.js` script reads route chunks from Vite's build manifest (`.vite/manifest.json`) — auto-discovering chunks rather than relying on a hardcoded list. It enforces both per-route limits (from Section 8.2) and the 250 KB aggregate limit. A baseline file (`bundle-baseline.json`) is committed to version control; CI fails if any chunk grows by more than 5 KB from its baseline, catching gradual drift before it accumulates into a budget violation. `vite-plugin-compression` provides the compressed sizes.
- **Lighthouse CI:** Run on every PR against all heavy routes — dashboard, request explorer, and playground. These three routes cover the heaviest dependency combinations (ECharts, TanStack Table + virtual scroll, and marked + highlight.js respectively). Fail thresholds: Performance < 90, Accessibility < 95, Best Practices < 90. Lighthouse runs against a mock backend with realistic data volumes (8 server cards, 1000 table rows, 3 chat messages with code blocks).

---

## 12. Open Questions

1. ~~**WebSocket protocol design.**~~ **Resolved: SSE for v1.** The real-time transport uses Server-Sent Events (SSE) for all server→client push (dashboard metrics, server status, queue depth). SSE is simpler, works through HTTP proxies without upgrade negotiation, and handles automatic reconnection natively. The `liveData.ts` store is a transport-only layer that feeds SSE events into TanStack Query caches. If bidirectional needs emerge (e.g., live queue manipulation), WebSocket can be added as a second transport alongside SSE without architectural changes — the store abstraction isolates the transport from consumers.

2. **Playground conversation persistence.** Should playground conversations be saved to SQLite (queryable, shareable) or remain ephemeral (simpler, no storage growth)? All requests already hit the audit log, but the conversational context (system prompt, multi-turn history) isn't captured there. **Leaning toward:** Ephemeral for v1, with an "Export conversation" button (JSON/Markdown download).

3. **Config editor scope.** The existing design doc says config changes require TOML file editing + SIGHUP. The UI currently only triggers reload. Should the config editor support full inline editing (with writeback to TOML), or remain read-only with a reload button? The `ConfigOverlayStore` (v1.1) changes this equation. **Leaning toward:** Read-only display + reload button for v1.0, full editing via `ConfigOverlayStore` API in v1.1.

4. **Embedding strategy.** `rust-embed` (compile-time embedding) vs. `tower-serve-dir` (runtime file serving). Compile-time embedding is simpler for distribution (true single binary) but slower to iterate during development. **Leaning toward:** `rust-embed` with `debug-embed = false` (serves from filesystem in debug builds, embeds in release builds).

5. **Multi-instance dashboard.** If LlamaForge evolves to multi-node (Phase 2), the dashboard needs to aggregate across instances. Should the UI architecture account for this now (e.g., instance selector in the header, cross-instance aggregation in charts), or design for single-instance and refactor later? **Leaning toward:** Design the data model to include an `instance_id` field from the start, but render single-instance views in v1. Adding an instance selector later is additive, not a refactor.

6. **Playground model comparison.** Side-by-side comparison of two model outputs is a key feature. Should both requests be sent simultaneously (parallel) or sequentially? Parallel is faster but doubles the load. **Leaning toward:** Parallel by default with a "sequential" toggle for resource-constrained setups.

---

## 13. Alternatives Considered

### 13.1 htmx + Alpine.js (Original Design Choice)

**Why considered:** Already specified in the backend design doc. Zero build toolchain, embedded in binary, server-rendered HTML is fast. Fits the "management dashboard" scope perfectly.

**Why rejected for the expanded scope:** The requirements now include a streaming chat playground, side-by-side model comparison, future experimentation platform with drag-and-drop experiment configuration, and a request explorer with virtual scrolling over 10K+ rows. These are SPA patterns that fight the htmx paradigm. Building a token-streaming chat UI with htmx requires server-side fragment orchestration that's more complex than just using a client-side framework. The XSS surface area documented in the security review (htmx executes injected `hx-*` attributes) is eliminated entirely by moving to a framework where HTML responses are data, not behavior.

**Would still be appropriate if:** The scope remained limited to ops dashboard + config management (no playground, no experimentation).

### 13.2 React + TypeScript

**Why considered:** Largest ecosystem, most component libraries (Tremor, shadcn/ui), biggest hiring pool. LiteLLM uses it.

**Why rejected:** Larger bundle sizes conflict with the single-binary embedding goal. The virtual DOM diff overhead is unnecessary for a dashboard that updates individual values — Svelte's compile-time reactivity is more efficient for this pattern. React's ecosystem advantages (massive component library selection) are less relevant here — LlamaForge needs ~20 custom components, not a component marketplace. The "hiring pool" argument is weaker for an infrastructure tool that will be maintained by the same team that builds the Rust backend.

**Would be preferred if:** This were a large-team product with many frontend engineers, or if extensive third-party UI component reuse were required.

### 13.3 Leptos (Rust → WASM)

**Why considered:** Full-stack Rust. Type safety from database to DOM. No JavaScript toolchain.

**Why rejected:** WASM bundles are 2–3x larger than equivalent JS for the same functionality. The charting ecosystem is non-existent — you'd need JS interop for ECharts anyway, negating the "pure Rust" benefit. Compile times would double (adding WASM target to an already large Rust workspace). Debugging WASM in the browser is significantly harder than debugging JS. The framework is production-ready but the ecosystem (components, libraries, tooling) is still immature compared to JS frameworks. The team would need to learn a new reactive model (Leptos signals) on top of maintaining the Rust backend.

**Would be preferred if:** The team had strong Rust frontend experience and no JavaScript expertise, or if the UI had hard real-time requirements (sub-frame latency) that WASM could uniquely serve.

### 13.4 Vue 3 + TypeScript

**Why considered:** Good middle ground — smaller than React, larger ecosystem than Svelte, excellent composition API.

**Why rejected:** No compelling advantage over Svelte for this specific use case. Larger bundles than Svelte, smaller ecosystem than React — it falls in the middle without winning on any axis. The composition API is excellent but Svelte's runes are more concise for the same patterns.

---

## Appendix A: Decision Log

| Decision | Choice | Rationale | Date |
|----------|--------|-----------|------|
| Frontend framework | SvelteKit 5 | Best fit for real-time dashboards + streaming chat. Smallest bundles. Open WebUI precedent. | 2026-05-27 |
| Rendering strategy | CSR with static export | No SEO need. Same-host serving eliminates SSR benefit. Simpler deployment. | 2026-05-27 |
| Charting library | Apache ECharts | Already chosen in backend design doc. Best real-time dashboard support. | 2026-05-27 |
| Component primitives | Bits UI (headless) | Accessible by default, unstyled, composable. No "fighting the library" tax. | 2026-05-27 |
| CSS framework | Tailwind CSS 4 | Utility-first, tree-shaken, consistent with Open WebUI. | 2026-05-27 |
| Server state management | TanStack Query | Caching, deduplication, background refetch. Framework-agnostic. | 2026-05-27 |
| Real-time updates | SSE + polling fallback | SSE is simpler than WebSocket for server→client push. Works through HTTP proxies. | 2026-05-27 |
| Asset embedding | rust-embed | True single binary in release. Filesystem serving in debug. | 2026-05-27 |
| Font strategy | System font stack | Zero font files = zero FOIT/FOUT = zero layout shift. | 2026-05-27 |
| Authentication | None (v1) | Local/LAN tool. Auth at reverse proxy if needed. | 2026-05-27 |
| Markdown rendering | marked + DOMPurify | For playground LLM responses. Sanitized before `{@html}`. | 2026-05-27 |
| Code highlighting | highlight.js (lazy) | 15 KB gz with 6 grammars vs Shiki's ~100 KB. Loaded only in playground. Good-enough quality for LLM response code blocks. | 2026-05-27 |
| Table component | TanStack Table | Virtual scrolling for 10K+ rows in explorer. | 2026-05-27 |
| Real-time transport | SSE (not WebSocket) | Server→client push only. Simpler, auto-reconnect, works through HTTP proxies. Transport-only store feeds TanStack Query. | 2026-05-27 |
| CSRF protection | Custom `X-LlamaForge-Request` header | Blocks cross-origin form/fetch attacks even without auth. Backend rejects mutating requests without this header. | 2026-05-27 |
| Markdown sanitization | marked → DOMPurify (strict allowlist) → `{@html}` | Explicit ordered pipeline. DOMPurify config restricts to safe tags/attrs only. `img`, `style`, `script` forbidden. CI lint rule enforces pipeline usage. | 2026-05-27 |
| Dashboard screen reader strategy | Silent metrics + on-demand summary button | Prevents screen reader flooding from continuous SSE metric updates. Visible button (not hidden shortcut) for discoverability. | 2026-05-27 |
| CSP `style-src` | `'unsafe-inline'` (accepted risk) | Required for Svelte scoped styles + ECharts inline mutations. Nonce-based impractical for ECharts. Mitigated by DOMPurify stripping `style` attrs from user content. | 2026-05-27 |
| Cross-origin protection | CORS (no `Access-Control-Allow-Origin`) + CSRF header | CORS blocks cross-origin reads (SSE data exfiltration). CSRF header blocks cross-origin writes. Complete defense. | 2026-05-27 |
| ServerCard action logic | `availableActions` prop from orchestrator | Business rules (state→action mapping) live in orchestrator, not presentational component. Enables independent testing. | 2026-05-27 |
| Table keyboard model | Simple table (Queue) + APG Grid (Explorer) | Two patterns based on table complexity. Simple for few-row tables, Grid for data-dense Explorer. | 2026-05-27 |
| Status indicators | Triple-redundancy via `StatusBadge` component | Icon + text + color on all status indicators. Design system component enforces consistency. | 2026-05-27 |
| Config concurrent editing | Lock-on-load + ETag concurrency control | Form captures snapshot at init, backend rejects stale writes with 409. Simpler than live-rebase. | 2026-05-27 |
| Bundle size CI | Per-route + aggregate enforcement via Vite manifest | Auto-discovers chunks, enforces both limits, baseline tracking catches drift. | 2026-05-27 |

---

## Appendix B: Project Structure

```
llama-forge/
├── ui/                              # Frontend source (SvelteKit)
│   ├── package.json
│   ├── svelte.config.js
│   ├── vite.config.ts
│   ├── tailwind.config.ts
│   ├── tsconfig.json
│   ├── src/
│   │   ├── app.html                 # HTML shell with critical CSS
│   │   ├── app.css                  # Tailwind base + custom properties
│   │   ├── lib/
│   │   │   ├── api/
│   │   │   │   ├── client.ts        # Centralized fetch wrapper
│   │   │   │   ├── types.ts         # TypeScript types matching backend API
│   │   │   │   └── queries.ts       # TanStack Query definitions
│   │   │   ├── stores/
│   │   │   │   ├── liveData.ts      # SSE transport-only store (subscriber lifecycle)
│   │   │   │   ├── preferences.ts   # localStorage-backed UI preferences
│   │   │   │   └── theme.ts         # Light/dark theme store
│   │   │   ├── components/
│   │   │   │   ├── layout/
│   │   │   │   │   ├── Sidebar.svelte
│   │   │   │   │   ├── Header.svelte
│   │   │   │   │   └── ConnectionStatus.svelte
│   │   │   │   ├── dashboard/
│   │   │   │   │   ├── ServerGrid.svelte
│   │   │   │   │   ├── ServerCard.svelte
│   │   │   │   │   ├── SystemHealthBanner.svelte
│   │   │   │   │   ├── QueueDepthGauge.svelte
│   │   │   │   │   ├── ThroughputChart.svelte
│   │   │   │   │   └── LatencyChart.svelte
│   │   │   │   ├── server/
│   │   │   │   │   ├── SlotGrid.svelte
│   │   │   │   │   ├── ServerMetrics.svelte
│   │   │   │   │   └── ServerActionBar.svelte
│   │   │   │   ├── queue/
│   │   │   │   │   ├── QueueTable.svelte
│   │   │   │   │   └── QueueFilters.svelte
│   │   │   │   ├── explorer/
│   │   │   │   │   ├── ExplorerTable.svelte   # includes RequestRow inline
│   │   │   │   │   ├── ExplorerFilters.svelte
│   │   │   │   │   └── RequestDetailDrawer.svelte
│   │   │   │   ├── config/
│   │   │   │   │   ├── ServerConfigEditor.svelte
│   │   │   │   │   ├── PriorityRulesEditor.svelte
│   │   │   │   │   └── DiffPreview.svelte
│   │   │   │   ├── playground/
│   │   │   │   │   ├── ChatPane.svelte
│   │   │   │   │   ├── ComparisonPane.svelte
│   │   │   │   │   ├── MessageBubble.svelte
│   │   │   │   │   ├── StreamingDisplay.svelte
│   │   │   │   │   ├── ResponseMetrics.svelte
│   │   │   │   │   ├── ModelSelector.svelte
│   │   │   │   │   └── ParameterSidebar.svelte
│   │   │   │   ├── models/
│   │   │   │   │   └── ModelCard.svelte
│   │   │   │   └── shared/
│   │   │   │       ├── Skeleton.svelte
│   │   │   │       ├── Toast.svelte
│   │   │   │       ├── ErrorBanner.svelte
│   │   │   │       ├── EmptyState.svelte
│   │   │   │       ├── ConfirmDialog.svelte
│   │   │   │       ├── StatusBadge.svelte   # triple-redundancy status (icon+text+color)
│   │   │   │       ├── DynamicHeading.svelte # heading level passed as prop
│   │   │   │       └── EChartsWrapper.svelte # lazy-load, resize, dispose, skeleton
│   │   │   └── utils/
│   │   │       ├── format.ts         # Number/date/duration formatting
│   │   │       └── constants.ts      # API routes, defaults
│   │   └── routes/
│   │       ├── +layout.svelte        # App shell (sidebar + header + slot)
│   │       ├── +page.svelte          # Dashboard (default route)
│   │       ├── servers/
│   │       │   └── [id]/+page.svelte # Server detail
│   │       ├── queue/+page.svelte
│   │       ├── explorer/+page.svelte
│   │       ├── config/
│   │       │   ├── servers/+page.svelte
│   │       │   └── priority/+page.svelte
│   │       ├── playground/+page.svelte
│   │       └── models/+page.svelte
│   └── static/
│       └── favicon.svg
├── src/                              # Rust backend
│   └── transport/
│       └── ui.rs                     # axum handler: serve embedded UI assets
└── build.rs                          # Triggers `npm run build` + rust-embed
```

---

## Appendix C: Build Integration

### Development Workflow

```bash
# Terminal 1: Run Rust backend
cargo run

# Terminal 2: Run SvelteKit dev server with proxy
cd ui/
npm run dev -- --proxy http://localhost:8000
```

The Vite dev server proxies `/api/*` and `/metrics` to the running Rust backend. The SSE endpoint (`GET /api/events`) is included in the `/api/*` glob. Hot module replacement provides instant feedback on UI changes.

### Production Build

```bash
# Full build (frontend + backend)
cargo build --release
```

`build.rs` detects the `ui/` directory and runs:
1. `npm ci` (install dependencies from lockfile)
2. `npm run build` (SvelteKit compiles to `ui/build/`)
3. `rust-embed` includes `ui/build/` contents in the binary

The resulting binary serves the UI at `/ui/*` with no external dependencies.

### CI Pipeline

```yaml
# Simplified — adapt to actual CI system
steps:
  - name: Install Node.js
    uses: actions/setup-node@v4
    with: { node-version: '22' }

  - name: Frontend checks
    working-directory: ui/
    run: |
      npm ci
      npm run check          # svelte-check (type checking)
      npm run lint           # eslint + prettier
      npm run test:unit      # vitest
      npm run build          # production build
      # Bundle size check
      node scripts/check-bundle-size.js

  - name: E2E tests
    run: npx playwright test

  - name: Lighthouse CI
    run: npx lhci autorun

  - name: Rust build (embeds UI)
    run: cargo build --release
```
