# Front-End Architecture Design: LlamaForge Setup Wizard

**Author:** Sumanth Gottipati  
**Date:** 2026-06-03  
**Status:** Draft  
**Reviewers:** TBD

---

## 1. Overview

### 1.1 Problem Statement

LlamaForge's current `llamaforge init` command writes a static template TOML file and tells the user to fill in `binary_path`, `model_path`, `port`, and `ctx_size` by hand. This requires the operator to already know where their llama-server binary lives, where their GGUF model files are, what GPU they have, how much VRAM is available, and what context size / layer offload is safe for their hardware. For a tool that manages llama.cpp complexity, the setup experience itself shouldn't require llama.cpp expertise.

### 1.2 Goals and Non-Goals

**Goals:**

- **G1: Zero-knowledge onboarding.** A user who has never used llama.cpp should be able to go from `llamaforge init` to a running inference server without reading docs or editing TOML by hand.
- **G2: Hardware-aware recommendations.** Automatically detect GPUs, VRAM, CPU cores, and system RAM. Use detection results to recommend `gpu_layers`, `ctx_size`, `parallel_slots`, `cache_type_k/v`, and `estimated_vram_mb` per model.
- **G3: Model discovery.** Scan the filesystem for `.gguf` files so the user picks from what they have rather than typing paths.
- **G4: Binary acquisition.** If `llama-server` isn't found on the system, guide the user through cloning and building llama.cpp with the correct backend (Metal, CUDA, Vulkan, CPU) for their detected hardware.
- **G5: Dual-mode availability.** Standalone wizard via `llamaforge init --interactive` (no existing config required) AND a first-run setup experience within the main UI when `llamaforge serve` boots with no valid config.
- **G6: Reconfigurability.** The wizard is also accessible from the main UI as a "reconfigure" flow for adding new models or re-detecting hardware after a GPU change.

**Non-Goals:**

- **NG1: Model downloading.** The wizard does not download GGUF models from HuggingFace or other sources. It discovers models already on disk.
- **NG2: Prebuilt binary downloads.** Binary acquisition is clone+build only. Downloading prebuilt releases is a future enhancement.
- **NG3: Remote/cluster setup.** The wizard configures a single local machine. Multi-node fleet setup is out of scope.
- **NG4: Advanced tuning.** The wizard produces a working config with sensible defaults. Advanced features like LoRA adapters, speculative decoding, and priority rules are configured post-setup through the main config UI.
- **NG5: Windows support for build.** The clone+build flow targets macOS and Linux. Windows users must provide a pre-built binary. (Model scanning and hardware detection still work.)

### 1.3 Success Criteria

| Metric | Target |
|--------|--------|
| Time from `llamaforge init --interactive` to saved config (with existing binary + models) | < 2 minutes |
| Time from `llamaforge init --interactive` to saved config (with build from source) | < 10 minutes (build time dependent) |
| Hardware detection accuracy | Correctly identifies all CUDA/Metal/Vulkan GPUs on Linux and macOS |
| Model scan coverage | Finds all `.gguf` files in user-specified directories within 5 seconds for < 10,000 files |
| Config validity | Generated config passes `llamaforge validate` on first try, 100% of the time |
| Accessibility | WCAG 2.1 AA compliance |
| JavaScript payload | < 40 KB gzipped (wizard-specific code, excluding shared chunks) |

---

## 2. User Experience

### 2.1 User Flows

**Flow 1 — Fresh install, happy path (binary + models exist)**

User runs `llamaforge init --interactive`. A lightweight HTTP server starts on an ephemeral port and opens the browser to `http://127.0.0.1:<port>/ui/setup`. The wizard displays a welcome screen explaining what it will do. User clicks "Start Setup."

Step 1 (Hardware): The wizard shows a spinner while scanning hardware. Results appear: "NVIDIA RTX 4090 — 24 GB VRAM, 32 CPU cores, 64 GB RAM." User confirms this looks correct. Step 2 (Binary): The wizard scans `$PATH` and common locations (`/usr/local/bin`, `/opt/homebrew/bin`, `~/.local/bin`). It finds `/usr/local/bin/llama-server` and displays its version. User confirms. Step 3 (Models): User specifies one or more directories to scan (with a sensible default of `~/models`, `./models`, current directory). The wizard lists discovered `.gguf` files with metadata (file size, quantization inferred from filename). User checks the models they want to serve. Step 4 (Configure): For each selected model, the wizard shows recommended config values based on the detected hardware: gpu_layers, ctx_size, parallel_slots, cache quantization. User can adjust any value via form inputs. Step 5 (Review): The wizard shows the complete generated `llamaforge.toml` in a syntax-highlighted preview. User clicks "Save & Finish." Config is written to the XDG config path (or a user-specified path). The wizard shows a "Ready!" screen with the command to start: `llamaforge serve`.

**Unhappy path — hardware detection fails:** The GPU detection command errors (e.g., no NVIDIA drivers). The wizard shows what it could detect (CPU, RAM) and marks GPU as "Not detected — CPU-only mode." It adjusts recommendations to `gpu_layers = 0` and warns about expected performance. The user can override if they know their GPU setup.

**Flow 2 — Fresh install, no binary found**

Steps 1 and 2 are the same, but Step 2 finds no `llama-server`. The wizard shows: "llama-server not found. I can build it for you." User clicks "Build llama.cpp." The wizard shows a build configuration screen with the auto-detected backend (Metal on macOS, CUDA on Linux with NVIDIA, Vulkan otherwise, CPU as fallback). User confirms or overrides. The wizard clones the llama.cpp repo (progress bar for git clone), runs cmake + make (streaming build log output), and on success writes the binary path. On build failure, the wizard shows the last N lines of build output and suggests common fixes (missing cmake, missing CUDA toolkit, etc.).

**Unhappy path — build fails:** The streaming log shows the error in real time. The wizard stops streaming, highlights the error, and offers: "Try again," "Change backend," or "Skip — I'll provide the binary path manually." If the user skips, they get a file path input to enter the binary location by hand.

**Flow 3 — First-run detection in main UI**

User runs `llamaforge serve` with no config file (or an empty/invalid one). Instead of exiting with an error, the server boots in "setup mode" — the full transport layer starts but with no llama-server instances. Navigating to `http://localhost:8000/ui` shows the setup wizard instead of the dashboard. The flow is identical to Flow 1/2, but on completion the config is written and the server hot-reloads it, spawning the configured llama-server instances without requiring a restart.

**Flow 4 — Reconfigure from main UI**

An operator using the running system navigates to Settings → "Setup Wizard" (or clicks "Add Server" → "Guided Setup"). This launches the wizard as a modal/page within the main UI layout, pre-populated with the current config. The operator can re-scan hardware, add models, or re-run the build. On completion the config is updated via the existing hot-reload writeback path.

### 2.2 Key Screens and States

| Screen | Initial Load | Loading | Loaded | Empty | Error |
|--------|-------------|---------|--------|-------|-------|
| Welcome | Immediate (static content) | N/A | Welcome text + "Start" button | N/A | N/A |
| Hardware Detection | "Scanning hardware..." spinner | Animated scan progress | Hardware summary cards (GPU, CPU, RAM) | "No GPU detected — CPU-only mode" | "Detection failed: [reason]. Enter hardware details manually." |
| Binary Locator | "Scanning for llama-server..." spinner | Scan progress with paths being checked | Binary found: path + version | "Not found. Build from source?" + manual path input | "Scan error: [reason]. Enter path manually." |
| Build llama.cpp | Build config form (backend selector, install path) | Streaming build log with cmake target count (`[N/M]`) + elapsed time | "Build complete! Binary at [path]" | N/A | Build failure with log excerpt + retry/skip options |
| Model Scanner | Directory selector + "Scan" button | Scan progress with file count | Table of discovered models with checkboxes | "No .gguf files found in [dirs]. Try a different directory?" | "Scan error: [reason]" |
| Server Config | Form with recommended values pre-filled | N/A (local interaction) | Form with all fields populated | N/A (always has defaults) | Inline validation errors on fields |
| Review | Generating preview spinner | N/A | Syntax-highlighted TOML + "Save" button | N/A | Config generation error (should never happen — validation is continuous) |
| Complete | N/A | N/A | Success message + `llamaforge serve` command | N/A | Write failure: "Permission denied at [path]. Try a different location?" |

---

## 3. Rendering Strategy

### 3.1 Rendering Approach

**Static SPA, same as the main UI.** The wizard is a set of SvelteKit routes under `/ui/setup/*` compiled to static HTML via `adapter-static` and embedded into the Rust binary alongside the rest of the UI. No SSR — LlamaForge serves over plain HTTP on localhost, and the wizard's content is entirely dynamic (hardware detection results, scan results) so server-rendering provides no benefit.

For the standalone wizard mode (`llamaforge init --interactive`), the same embedded assets are served by a minimal axum server that only mounts the UI router — no gRPC, no inference engine, no config handle. This is a new `wizard` transport mode in the binary, not a separate binary.

### 3.2 Progressive Rendering Plan

| Time | What the user sees |
|------|-------------------|
| 0 ms | Pre-hydration skeleton from `app.html` (same pattern as main UI) |
| ~100 ms | SvelteKit hydrates, wizard shell renders (step indicator + content area) |
| ~200 ms | Welcome screen with "Start Setup" button (static content, no API calls) |
| User clicks "Start" | Hardware detection step — immediate spinner, then results stream in as detected |

Each wizard step is a discrete route (`/ui/setup`, `/ui/setup/hardware`, `/ui/setup/binary`, etc.), so SvelteKit's client-side navigation loads only the step-specific component. No waterfall — each step fetches its own data on mount, and the user can't advance until the current step completes.

### 3.3 Document Structure

```
<main role="main" aria-label="Setup Wizard">
  <nav aria-label="Setup progress">
    <!-- Step indicator: ordered list of steps with aria-current -->
    <ol>
      <li aria-current="step">Hardware</li>
      <li>Binary</li>
      <li>Models</li>
      <li>Configure</li>
      <li>Review</li>
    </ol>
  </nav>
  <section aria-live="polite">
    <!-- Current step content swapped by SvelteKit router -->
    <h1>Step title</h1>
    <!-- Step-specific content -->
  </section>
  <footer>
    <!-- Back / Next / Skip navigation -->
  </footer>
</main>
```

Each step has a single `<h1>` (the step title), with `<h2>` for subsections within a step (e.g., "GPU" and "CPU" within the Hardware step). Form fields use `<label>` elements, never `aria-label` on inputs unless there's no visible label.

### 3.4 Layout Stability

Minimal CLS risk because the wizard is a step-by-step flow with fixed-height step content areas. Specific mitigations:

- **Hardware cards:** Reserve a fixed-height container (min 200px) for the detection results. Skeleton cards (reusing the existing `Skeleton.svelte` component) fill the space during scanning. When results arrive, they replace the skeletons without layout shift because the container size is pre-allocated.
- **Model table:** The table container has `min-height: 300px`. During scanning, a skeleton table with placeholder rows is shown. The scan results populate the same structure.
- **Build log:** The log container is a fixed-height scrollable `<pre>` element (400px). New lines scroll into view; the container never grows.

### 3.5 Responsive Strategy

The wizard is primarily a desktop experience (NG3 from the main UI applies here too), but is usable on tablet. The step content is a single centered column (max-width: 720px) so it naturally works at any width above 640px. Below 640px, the step indicator collapses from horizontal labels to numbered circles. Form fields stack vertically at all widths (no multi-column forms). Touch targets are minimum 44×44 px.

---

## 4. Component Architecture

### 4.1 Component Hierarchy

```
SetupLayout (orchestrator)
├── StepIndicator (presentational)
├── StepNavigation (presentational — Back/Next/Skip buttons)
└── [Step content — one per route]
    ├── WelcomeStep (presentational)
    ├── HardwareStep (orchestrator)
    │   ├── HardwareCard (presentational — GPU/CPU/RAM)
    │   └── HardwareManualEntry (presentational — fallback form)
    ├── BinaryStep (orchestrator — uses inline {#if} blocks for found/not-found/manual states)
    │   ├── BuildConfigurator (presentational — backend selector)
    │   └── BuildProgress (presentational — streaming log)
    ├── ModelStep (orchestrator)
    │   ├── DirectorySelector (presentational — path inputs)
    │   ├── ScanProgress (presentational — spinner + file count)
    │   └── ModelTable (presentational — checkbox list of discovered models)
    ├── ConfigStep (orchestrator)
    │   └── ServerConfigForm (presentational — per-model config form; handles GPU layers, context size, parallel slots, and recommendation display internally)
    └── ReviewStep (orchestrator)
        ├── TomlPreview (presentational — syntax-highlighted TOML)
        ├── SavePathSelector (presentational — config file destination)
        └── CompletionScreen (presentational — success + next steps)
```

**Intelligence distribution:** Each step route (e.g., `HardwareStep`) is an orchestrator that owns the API calls and step-completion logic. It passes data down to pure presentational children. The `SetupLayout` is the top-level orchestrator that owns the wizard state (current step, collected data from all steps) and controls navigation.

### 4.2 Component Contracts

**StepIndicator**
- Props: `steps: { label: string; status: 'upcoming' | 'current' | 'complete' | 'skipped' }[]`
- Events: none (display-only)
- Does NOT know: what each step does, current wizard state

**StepNavigation**
- Props: `canGoBack: boolean`, `canAdvance: boolean`, `canSkip: boolean`, `nextLabel: string` (default "Next", changes to "Save & Finish" on last step), `isLoading: boolean`
- Events: `onback`, `onnext`, `onskip`
- Does NOT know: which step is active, what navigation means

**HardwareCard**
- Props: `type: 'gpu' | 'cpu' | 'ram'`, `label: string`, `details: string`, `status: 'detected' | 'not_detected' | 'error'`, `icon: Snippet`
- Events: none
- Uses: `StatusBadge` for the status indicator (triple redundancy: icon + text + color)

**ModelTable**
- Props: `models: DiscoveredModel[]`, `selectedIds: Set<string>`
- Events: `onselectionchange: (ids: Set<string>) => void`
- Does NOT know: where models came from, what happens with the selection

**ServerConfigForm**
- Props: `model: DiscoveredModel`, `hardware: HardwareInfo`, `recommendation: ConfigRecommendation`, `values: ServerConfigDraft`
- Events: `onchange: (values: ServerConfigDraft) => void`
- Does NOT know: how recommendations were computed, whether the config is final
- Handles GPU layers, context size, parallel slots, and recommendation badge display internally using standard `<input>` and `<label>` elements. No sub-components — each field is a labeled input with an inline "Recommended: X" hint derived from the `recommendation` prop.

**BuildProgress**
- Props: `status: 'idle' | 'cloning' | 'building' | 'complete' | 'failed' | 'cancelled'`, `logLines: string[]`, `progress: number | null` (0-100 for clone, null for build), `errorSummary: string | null`
- Events: `onretry`, `onskip`, `onchangebackend`, `oncancel`

**TomlPreview**
- Props: `toml: string`
- Events: none
- Renders syntax-highlighted TOML. No `{@html}` — uses `renderMarkdown()` with a TOML code block, consistent with the existing security convention.

### 4.3 Composition Strategy

The wizard reuses existing shared components wherever possible:

- `StatusBadge` — for hardware detection status, binary found/not-found, build status
- `Skeleton` — for loading placeholders in hardware cards and model table
- `ErrorBanner` — for step-level errors (scan failure, build failure)
- `EmptyState` — for "no models found" and "no GPU detected"
- `Toast` — for transient notifications (config saved successfully)
- `ConfirmDialog` — for destructive actions (overwrite existing config)

New shared components introduced by the wizard (reusable in the main UI):

- `ProgressBar` — determinate/indeterminate progress indicator (used for git clone progress, build progress)
- `LogViewer` — scrollable, auto-tailing log display for build output (useful for future server log viewing)
- `StepIndicator` — wizard progress tracker (reusable for any multi-step flow)

### 4.4 Side Effects and Lifecycle

**SSE subscriptions:** The build step opens an `EventSource` connection to `/api/setup/build/{id}/stream` to receive streaming build log output. This is the only long-lived connection in the wizard. Cleanup: the `EventSource` is closed when the user navigates away from the build step (in the component's `$effect` cleanup) or when the build completes/fails.

**Polling:** Hardware detection and model scanning are one-shot requests, not subscriptions. No polling.

**Timers:** The build step displays factual progress only: cmake's `[N/M]` target count (parsed from build log output) and wall-clock elapsed time. No estimated time remaining — build time estimation from log heuristics is unreliable and misleading. The elapsed time timer updates every second. Cleanup: `clearInterval` in `$effect` return.

**No `onMount` / `onDestroy`:** Per project convention, all lifecycle management uses Svelte 5 `$effect` with cleanup returns.

---

## 5. State Management

### 5.1 State Inventory

| State | Source of Truth | Who Modifies | Scope |
|-------|----------------|-------------|-------|
| Current wizard step | URL (`/ui/setup/hardware`, etc.) | SvelteKit router (user navigation + programmatic) | Global |
| Hardware detection results | TanStack Query cache (`['setup', 'hardware']`) | `HardwareStep` fetches on mount | Wizard session |
| Binary scan results | TanStack Query cache (`['setup', 'scan-binary']`) | `BinaryStep` fetches on mount | Wizard session |
| Build status + log | Server (SSE stream) | `BinaryStep` subscribes | Wizard session |
| Discovered models | TanStack Query cache (`['setup', 'models', directories]`) | `ModelStep` fetches on scan | Wizard session |
| Selected model IDs | Client (`SetupWizardStore.selectedModelIds`) | `ModelTable` checkbox interactions | Wizard session |
| Per-model user overrides | Client (`SetupWizardStore.userOverrides`) | `ConfigStep` form interactions | Wizard session |
| Config recommendations | TanStack Query cache (`['setup', 'recommend', ...]`) | `ConfigStep` fetches when entered | Wizard session |
| Generated TOML | Server (via `POST /api/setup/preview-config`) | `ReviewStep` fetches on mount | Derived |
| Save path | Client (`SetupWizardStore.savePath`) | `SavePathSelector` form input | Wizard session |
| Completed steps | Derived (computed from store + query cache state) | N/A (read-only computation) | Derived |

### 5.2 Server State Strategy

**TanStack Query for all API calls**, consistent with the main UI convention. Each wizard step defines its own query:

```typescript
// Hardware detection — no refetch, result is stable for the session
createQuery({ queryKey: ['setup', 'hardware'], queryFn: () => apiFetch('/api/setup/hardware'), staleTime: Infinity })

// Model scan — refetches when directories change
createQuery({ queryKey: ['setup', 'models', directories], queryFn: () => apiFetch('/api/setup/scan-models', { method: 'POST', body: JSON.stringify({ directories }) }), enabled: directories.length > 0 })

// Config recommendations — refetches when selected models change
createQuery({ queryKey: ['setup', 'recommend', selectedModelIds], queryFn: () => apiFetch('/api/setup/recommend', { method: 'POST', body: JSON.stringify({ models: selectedModelIds }) }) })
```

No polling. No background refetch. The wizard is a linear flow — data is fetched once per step entry and doesn't go stale within a session.

### 5.3 Client State Strategy

The wizard's client-owned state lives in a single `SetupWizardStore` — a Svelte 5 rune-based class (`.svelte.ts`) in `$lib/stores/setup.svelte.ts`. TanStack Query is the sole source of truth for server-fetched data (hardware, binary scan, models, recommendations). The store holds only client-owned state that does not exist in any API response. The `ConfigStep` merges server recommendations with `userOverrides` at render time rather than storing merged `serverConfigs`.

```typescript
export class SetupWizardStore {
  // Client-only selections — not duplicated from any server response
  selectedModelIds = $state<Set<string>>(new Set());
  userOverrides = $state<Map<string, Partial<ServerConfigDraft>>>(new Map());
  savePath = $state<string>('');

  // Derived from TanStack Query cache — no duplication
  // completedSteps is computed, not stored
}
```

Each orchestrator step reads from the TanStack Query cache for server data and from this store for client selections. The store is instantiated in `SetupLayout` and provided via Svelte's context API (`setContext` / `getContext`) so it is scoped to the wizard and garbage-collected when the wizard is unmounted.

#### Step State Machine

Navigation between wizard steps is governed by a state machine that enforces advancement guards and downstream invalidation on re-entry:

```typescript
const STEP_TRANSITIONS: Record<WizardStep, {
  canAdvance: (qc: QueryClient, store: SetupWizardStore) => boolean;
  invalidatesOnReentry: WizardStep[];
}> = {
  welcome:   { canAdvance: () => true, invalidatesOnReentry: [] },
  hardware:  { canAdvance: (qc) => qc.getQueryData(['setup', 'hardware']) != null, invalidatesOnReentry: ['configure', 'review'] },
  binary:    { canAdvance: (qc) => qc.getQueryData<BinaryScanResult>(['setup', 'scan-binary'])?.found === true, invalidatesOnReentry: ['configure', 'review'] },
  models:    { canAdvance: (_qc, store) => store.selectedModelIds.size > 0, invalidatesOnReentry: ['configure', 'review'] },
  // Completion = a VALID MERGED config for every selected model (recommendation
  // + override), NOT a non-empty override set — accepting the recommendation as-is
  // must advance (goal G1: get running without hand-editing config).
  configure: { canAdvance: (qc, store) => store.selectedModelIds.size > 0 && allSelectedConfigsValid(qc, store), invalidatesOnReentry: ['review'] },
  review:    { canAdvance: () => true, invalidatesOnReentry: [] },
};
```

The function signature accepts both `QueryClient` and `SetupWizardStore` since some checks need server data from the TanStack Query cache (hardware, binary) and some need client state (selectedModelIds, userOverrides).

**Advancement guard:** The `StepNavigation` component's "Next" button is disabled unless `STEP_TRANSITIONS[currentStep].canAdvance(queryClient, store)` returns `true`. This is the single source of truth for whether a step is complete — the `completedSteps` set is derived by iterating the transition table, not stored independently.

**Downstream invalidation on re-entry:** When a user navigates back to a completed step (e.g., clicking "Hardware" in the step indicator after reaching "Review"), the `SetupLayout` checks `STEP_TRANSITIONS[targetStep].invalidatesOnReentry` and for each listed step:

1. Removes the step's TanStack Query data via `queryClient.removeQueries({ queryKey: ['setup', stepQueryKey] })`, forcing a re-fetch when the step is next entered.
2. Clears any client-owned state associated with the invalidated steps (e.g., re-entering "Models" clears `userOverrides` for models that may no longer be selected).

This ensures that downstream steps never display stale data derived from an earlier step's now-changed results. For example, if the user re-runs hardware detection and gets different GPU results, the "Configure" and "Review" steps are invalidated because their recommendations were based on the old hardware profile.

### 5.4 URL State

Each wizard step is a distinct URL:

- `/ui/setup` — Welcome
- `/ui/setup/hardware` — Hardware detection
- `/ui/setup/binary` — Binary locator / builder
- `/ui/setup/models` — Model scanner
- `/ui/setup/configure` — Server configuration
- `/ui/setup/review` — Review and save

This means: browser back/forward works, refreshing stays on the current step (data is re-fetched but not lost if the store is re-populated from API), and the URL is shareable (though sharing a wizard URL is uncommon).

No query parameters. The wizard steps are sequential; filter/sort state within a step (e.g., model table sort order) is ephemeral client state.

### 5.5 Optimistic Update Plan

No optimistic updates. The wizard is a sequential configuration flow with no concurrent writers and no data that can go stale. "Save config" is the only mutation, and it shows a loading state while the backend writes the file — no optimistic behavior because a failed save is not recoverable without user intervention (e.g., permission error).

### 5.6 Form Design

**Server configuration form (Step 4)** is the only significant form.

- **Validation strategy:** Inline validation on blur + on form submission. Validation rules are lightweight (number ranges, non-empty paths) because the backend is the authoritative validator — the wizard validates just enough to prevent obviously wrong configs.
- **Specific validations with concrete bounds:**
  - `gpu_layers`: integer, -1 (auto) or 0–512. Tests: -2 → rejected, 513 → rejected, -1 → accepted, 0 → accepted.
  - `ctx_size`: integer, 0 (auto) or 128–1048576. Tests: 64 → rejected, 2000000 → rejected.
  - `parallel_slots`: integer, 1–64. Tests: 0 → rejected, 65 → rejected.
  - `port`: integer, 1024–65535, unique across servers. Tests: 1023 → rejected, 65536 → rejected.
  - `binary_path`: non-empty string.
  - `model_path`: non-empty string.
- **Unsaved changes protection:** The `SetupLayout` listens to `beforeunload` and warns if the user has reached Step 4+ with unsaved changes.
- **Double-submit protection:** The "Save & Finish" button is disabled and shows a spinner while the save request is in flight.
- **Error mapping:** If the backend rejects the generated config (e.g., port conflict), the error is mapped back to the specific form field(s) using the backend's error path. Fallback: show the error as a banner above the form.

**Save path input (Step 5):**

- Text input pre-filled with the default XDG path.
- If the path exists and `--force` was not passed, a `ConfirmDialog` asks before overwriting. Writability is checked server-side during the save attempt — the save-config endpoint returns `403 Forbidden` if the path is not writable, and the error is surfaced inline.

---

## 6. Network Resilience

### 6.1 Data Fetching Architecture

All API calls go through the existing `$lib/api/client.ts` (`apiFetch`). The wizard adds no new fetching primitives. TanStack Query handles caching and deduplication. Each step fetches its data on mount — no prefetching of subsequent steps. The wizard is a linear flow where each step depends on user confirmation of the previous step's results, so prefetching would either be wasted (user goes back) or premature (user hasn't confirmed yet).

### 6.2 Error Handling Strategy

| API call | On timeout | On error | On network failure |
|----------|-----------|----------|-------------------|
| Hardware detection | **30-second overall timeout** on the backend endpoint, with per-command sub-timeouts: `nvidia-smi` 10 s, `rocm-smi` 10 s, `vulkaninfo` 10 s, `system_profiler` 15 s, `/proc` reads 5 s, `sysctl` 5 s. If a probe times out it is reported as **"detection timed out"** (not "not detected") in the response. On overall timeout, the backend returns **partial results** with a `warnings` field. Frontend shows "Detection is taking longer than expected" after **10 s** AND a **"Cancel and enter manually"** button. If the overall timeout fires, partial results are displayed with a banner: "Some hardware probes timed out — verify the results or enter details manually." | Show `ErrorBanner` with error details + "Retry" button + link to manual entry form | Show `ErrorBanner`: "Cannot reach LlamaForge server. Is it running?" (only possible in main-UI mode) |
| Binary scan | Same as hardware | Same + "Enter path manually" fallback | Same as hardware |
| Build (SSE) | If no SSE events for 60 seconds, show "Build appears stalled" warning (not an error — builds can be slow) | Build failure shown inline in log viewer with error highlighting | SSE reconnect with exponential backoff and jitter (base 1s, max 8s, 3 attempts), then show "Connection lost" with manual retry button |
| Model scan | 30-second timeout, then show partial results + "Scan timed out. Showing what was found so far." | Show `ErrorBanner` + retry | Same as hardware |
| Save config | 10-second timeout → "Save is taking longer than expected" | Show `ErrorBanner` with specific error (permission denied, disk full, etc.) | Same as hardware |

**No blank screens. No stuck spinners.** Every loading state has a timeout that transitions to an actionable error state.

### 6.3 Caching Strategy

TanStack Query caches all wizard API responses for the duration of the wizard session (`staleTime: Infinity`). Navigating back to a previous step shows cached results instantly. The cache is scoped to the wizard — when the wizard is unmounted, the query client removes the `['setup', ...]` queries.

No HTTP caching headers. The wizard APIs return real-time scan results that shouldn't be cached at the HTTP layer.

### 6.4 Offline Strategy

Not applicable. The wizard requires a running LlamaForge server process (which hosts the API endpoints). If the server isn't running, the wizard isn't reachable. No service worker, no offline caching. Stated explicitly per NG4 from the main UI architecture.

---

## 7. Performance Budget

### 7.1 Performance Targets

| Metric | Target |
|--------|--------|
| Wizard-specific JS (compressed) | < 40 KB gzipped |
| Time to interactive (welcome screen) | < 500 ms |
| Step transition latency (client-side navigation) | < 100 ms |
| Hardware detection API response | < 10 seconds (system dependent, not a front-end target) |
| Model scan API response (< 10,000 files) | < 5 seconds |
| Cumulative Layout Shift | < 0.05 |

### 7.2 JavaScript Strategy

The wizard routes are **code-split** from the main UI. SvelteKit's dynamic imports ensure that navigating to `/ui/setup/*` loads only the wizard chunk, and navigating to `/ui/` (dashboard) doesn't load wizard code. The split boundary is the `routes/setup/+layout.svelte` file.

No additional libraries beyond what the main UI already bundles (TanStack Query, ECharts is NOT loaded in the wizard). The TOML syntax highlighting in `ReviewStep` uses a lightweight regex-based highlighter (< 2 KB) rather than a full CodeMirror/Monaco instance.

### 7.3 Image Strategy

The wizard contains no user-content images. Hardware type icons (GPU, CPU, RAM) use inline SVG or icon components, not raster images. No `<img>` tags in the wizard.

### 7.4 Critical Rendering Path

Same as the main UI: SvelteKit's hashed inline bootstrap script hydrates the app shell. The wizard-specific chunk is loaded on first navigation to `/ui/setup`. No additional CSS files — Tailwind utilities are tree-shaken at build time and shared with the main UI's stylesheet.

No font loading concerns — the wizard uses the same font stack as the main UI, which is already loaded by the time the user navigates to the wizard (in main-UI mode) or loaded in the initial `app.html` (in standalone mode).

---

## 8. Accessibility Plan

### 8.1 Semantic Structure

The wizard uses a single `<h1>` per step (the step title), with `<h2>` for subsections. The step indicator is an `<ol>` (ordered list) with `aria-current="step"` on the active step. Form fields use visible `<label>` elements. The hardware cards use `<article>` with accessible names. The build log uses `<pre><code>` with `role="log"` and `aria-live="polite"` for new lines.

Native elements used everywhere possible: `<button>` for actions, `<input>` for text fields, `<select>` for the backend picker, `<table>` for the model list (with `<thead>`, `<th scope="col">`). Checkboxes in the model table use native `<input type="checkbox">`, not custom toggle components.

### 8.2 Keyboard Navigation Plan

- **Tab order:** Step indicator → step content (form fields / buttons in document order) → Back / Skip / Next buttons. Logical top-to-bottom, left-to-right order.
- **Step indicator:** Arrow keys move focus between steps (same pattern as the sidebar nav). Enter/Space activates a completed step to navigate back.
- **Model table:** Standard table keyboard navigation. Space toggles the focused row's checkbox. Shift+Space selects a range.
- **Build log:** The log container is focusable (`tabindex="0"`) and scrollable via arrow keys. Not in the default tab order — the user tabs past it unless they explicitly focus it.
- **Back / Next buttons:** Standard button keyboard behavior. Enter or Space activates.

### 8.3 Focus Management Plan

- **Step transition:** When the user advances to the next step (programmatic navigation after API success), focus moves to the new step's `<h1>`. This is the same pattern as the main UI's `afterNavigate` hook that focuses `main h1`.
- **Error appearance:** When an `ErrorBanner` appears (e.g., scan failure), focus moves to the banner so screen readers announce it immediately. The banner has `role="alert"`.
- **Build completion:** When the build completes (success or failure), focus moves to the result message.
- **Dialog (confirm overwrite):** Focus is trapped inside the `ConfirmDialog` (existing component already handles this). On dismiss, focus returns to the "Save" button.

### 8.4 Screen Reader Strategy

- **Step indicator:** `aria-current="step"` announces "Step 2 of 5: Binary, current step." Completed steps are announced as "Step 1: Hardware, complete."
- **Hardware detection:** During scanning, an `aria-live="polite"` region announces "Scanning hardware..." then "Hardware scan complete. Found: NVIDIA RTX 4090, 24 GB VRAM."
- **Build progress:** The build log container has `aria-live="polite"` but throttled — it announces summary milestones ("Cloning repository...", "Configuring build...", "Compiling: 50%...", "Build complete.") rather than every log line, to avoid overwhelming screen reader users.
- **Model scan results:** Announced as "Found 3 GGUF model files" when the scan completes.
- **Validation errors:** Inline errors use `aria-describedby` linked to the input, so the error is announced when the field is focused.

### 8.5 Color and Contrast

All text meets WCAG AA contrast ratios (4.5:1 for normal text, 3:1 for large text) against both light and dark theme backgrounds (the wizard inherits the main UI's theme system).

Status information is never conveyed by color alone — the `StatusBadge` component provides triple redundancy (icon + text label + color). Hardware detection status: green checkmark + "Detected" + green badge; red X + "Not detected" + red badge; yellow warning + "Partial" + yellow badge.

The build progress bar uses both color (fill) and a numeric percentage label. The step indicator uses both color and icons (checkmark for complete, circle for current, dash for upcoming).

`prefers-reduced-motion`: The scanning spinner, progress bar animation, and step transition animations respect the media query. When reduced motion is preferred, spinners are replaced with static "Loading..." text, progress bars update without animation, and step transitions are instant (no slide).

---

## 9. Security Posture

### 9.1 Input Handling

User input in the wizard consists of: directory paths for model scanning, a file path for binary location, a file path for config save location, and numeric/string values for server config fields.

**Path inputs** are sent to the backend as plain strings — the backend validates and resolves them. The frontend does NOT perform path traversal checks because the backend is authoritative (it uses `std::fs::canonicalize` and validates the resolved path is within allowed directories). The frontend only validates that path inputs are non-empty.

**Numeric inputs** are validated client-side (integer parsing, range checks) before being sent to the backend. The backend re-validates.

**No `{@html}` anywhere in the wizard.** The TOML preview uses `renderMarkdown()` (the existing sanitized rendering function) to display a code block. Build log output is rendered as text content in a `<pre>` element — never injected as HTML.

### 9.2 Authentication and Data Storage

The wizard runs on localhost. No authentication tokens, no cookies, no sensitive data in the browser. The wizard APIs are localhost-only and protected by the same CSRF header (`X-LlamaForge-Request`) as the main UI's mutation endpoints.

No data is stored in `localStorage` or `sessionStorage`. Wizard state lives in Svelte component state and is lost on page refresh (the user re-scans, which is the correct behavior — hardware and files may have changed).

### 9.3 Content Security Policy

The wizard uses the same CSP as the main UI — SvelteKit's hash-mode `<meta>` CSP for scripts/styles, plus the server's header-only directives for `frame-ancestors`, `object-src`, `base-uri`, and `form-action`. No changes needed.

The build log SSE endpoint is on the same origin (`connect-src: 'self'` covers it). No external resources are loaded.

### 9.4 Third-Party Risk

No third-party scripts. No CDN resources. No analytics. The wizard is entirely self-contained within the embedded static assets.

---

## 10. Testing Strategy

### 10.1 Component Tests

Each presentational component is tested with `@testing-library/svelte`:

- `StepIndicator` — renders correct number of steps, marks current step, marks completed steps
- `HardwareCard` — renders detection results, shows correct status badge for each state
- `ModelTable` — renders model list, checkbox selection, empty state
- `ServerConfigForm` — renders recommended values, validates input ranges, emits change events
- `BuildProgress` — renders log lines, auto-scrolls, shows error summary on failure
- `TomlPreview` — renders syntax-highlighted TOML

Tests verify rendering for all five states (initial, loading, loaded, empty, error) per component where applicable.

### 10.2 Integration Tests

- **Full wizard flow (happy path):** Mock all API responses. Walk through all five steps, verify state accumulates correctly, verify the save request contains the expected TOML.
- **Build failure flow:** Mock build SSE to emit lines then an error event. Verify error UI appears, retry button works, skip navigates to manual entry.
- **Hardware detection failure:** Mock 500 response. Verify error banner with manual entry fallback.
- **Navigation:** Verify back/forward between steps preserves entered data. Verify the step indicator updates correctly. Verify that refreshing re-fetches data.
- **Keyboard-only flow:** Full wizard completion using only keyboard (Tab, Enter, Space, Arrow keys).

**Recovery path tests:**

- **Build failure → retry → success:** Mock SSE stream that emits log lines then a `failed` event. Verify error UI appears with the error summary. Simulate clicking "Retry." Mock a successful SSE stream on the second attempt. Verify transition to the binary-found state with the correct path.
- **Build failure → skip → manual entry → next step:** Mock SSE `failed` event. Click "Skip." Verify manual path input appears. Enter a binary path manually. Advance to the Models step. Verify the manually-entered binary path is carried through to the final config in the Review step.
- **Hardware detection failure → manual entry → config:** Mock a 500 response from `/api/setup/hardware`. Verify error banner with manual entry fallback form. Enter hardware details manually (GPU name, VRAM, CPU cores, RAM). Advance to the Config step. Verify that recommendations use the manually-entered hardware values (not defaults or empty).
- **SSE reconnect during build:** Mock SSE stream that delivers N log lines, then disconnects without a `complete` or `failed` event. Verify the client attempts reconnection. Mock a successful reconnect that resumes from the current position. Verify the log continues without duplicate lines.
- **SSE reconnect exhaustion:** Mock 3 consecutive failed reconnect attempts (each disconnect immediately after opening). Verify "Connection lost" UI appears with a manual "Retry" button. Verify clicking "Retry" initiates a fresh connection attempt.

**SSE streaming tests:**

- **SSE event parsing:** Mock each SSE event type (`phase`, `progress`, `log`, `complete`, `failed`) individually. Verify the correct UI update for each: `phase` updates the phase label, `progress` updates the progress bar, `log` appends to the log viewer, `complete` shows the success state, `failed` shows the error state.
- **SSE unexpected disconnect:** Mock SSE stream that closes without emitting a `complete` or `failed` event. Verify the client triggers reconnect behavior (not a success or error state).
- **SSE malformed event:** Mock an SSE event with invalid JSON in the `data` field. Verify graceful handling: the error is logged to the console, the stream continues processing subsequent events, and the UI does not crash or show a blank state.
- **Long build log:** Mock 5000+ log lines emitted rapidly. Verify the `LogViewer` auto-scrolls to the bottom. Verify no excessive memory growth (log buffer is bounded or windowed). Verify the UI remains responsive during rapid updates.
- **SSE navigation-away cleanup:** Start a build (open SSE connection). Navigate away from the build step (e.g., click "Back"). Verify the `EventSource` is closed (the `$effect` cleanup fires). Verify no further SSE events are processed after navigation.

### 10.3 Visual Regression

Playwright screenshot tests for each wizard step in both light and dark themes, at 1280×800 (desktop) and 768×1024 (tablet). Baseline captures include: loaded state, error state, empty state. Comparison threshold: 0.1% pixel difference.

### 10.4 Accessibility Testing

- **Automated:** axe-core runs on every wizard step in CI. Zero violations at WCAG AA level.
- **Manual:** Screen reader testing (VoiceOver on macOS) for the full wizard flow — verify step announcements, error announcements, build progress announcements, and focus management.
- **Keyboard:** Playwright test that completes the full wizard using only keyboard events.

### 10.5 Performance Testing

- **Bundle size:** CI check verifies the wizard chunk stays under 40 KB gzipped. The check uses the existing `bundle-baseline.json` mechanism.
- **Lighthouse CI:** Runs on the wizard welcome page. Targets: Performance > 95, Accessibility 100, Best Practices > 95.

### 10.6 Backend API Tests

**Input validation boundary tests** for every endpoint:

- `scan-models` with empty `directories` array → 400.
- `scan-models` with non-existent directory → partial success (skip invalid, return valid models from other directories).
- `scan-models` with path traversal (`../../etc`) → 400 or sanitized (resolved path must be within allowed directories).
- `scan-models` with > 10 directories → 400.
- `scan-binary` with `extra_paths` containing null bytes → 400.
- `save-config` with path to read-only location → 403.
- `save-config` with path outside allowed directories → 403.
- `build` with `backend` not in (`cuda` | `metal` | `vulkan` | `cpu`) → 400.
- `build` with `install_dir` outside home directory → 400.
- `build` with `jobs` = 0 or negative → 400.
- `build` with `jobs` > `num_cpus` → clamped with warning (not rejected; the response includes the effective job count).
- `recommend` with empty `models` array → 400.
- Hardware detection with mocked `nvidia-smi` / `system_profiler` output → correct parsing of GPU name, VRAM, and driver version.
- Hardware detection with gibberish command output → error (not 200 with `vram_mb: 0`).
- Hardware detection with empty output (no GPU) → 200 with empty `gpus` array + `recommended_backend: "cpu"`.
- Multi-GPU `nvidia-smi` output → all GPUs in the `gpus` array with correct per-GPU VRAM.
- `system_profiler` with Apple Silicon unified memory → correct `vram_mb` reflecting shared GPU memory.

**Happy-path response contract tests:** Each endpoint returns the documented JSON shape. Verify all required fields are present, types are correct, and no undocumented fields leak through. These tests pin the contract so accidental shape changes are caught.

**Error response contract tests:** Each error returns structured JSON with `error` and `message` fields (not a 500 with a stack trace or raw text). Verify every 4xx/5xx response has `Content-Type: application/json` and follows the project's error envelope format.

**Backend integration tests** with mocked shell commands and filesystem fixtures:

- Use `tempdir` with `.gguf` files (varying sizes and filenames with different quantization hints) for model scan tests.
- Use a mock `llama-server` binary (shell script that responds to `--version`) for binary scan tests.
- Use mock `git` and `cmake` scripts that emit expected stdout/stderr patterns for build tests.
- Verify that the hardware detection handler correctly parses output from fixture files containing real `nvidia-smi`, `rocm-smi`, `system_profiler`, and `/proc/cpuinfo` outputs captured from actual systems.

### 10.7 Contract Tests

**JSON schema snapshot tests** bridge the gap between backend API responses and frontend TypeScript types, ensuring they stay in sync as the codebase evolves.

- Generate JSON schema snapshots from backend test output (the happy-path response contract tests in Section 10.6 produce canonical response shapes). Store these snapshots in version control.
- Validate frontend TypeScript types against the stored snapshots. If the backend changes a response shape, the snapshot test fails, forcing the corresponding TypeScript type update.
- **Alternative (preferred if feasible):** Use the `ts-rs` crate to generate TypeScript type definitions directly from the Rust response structs at build time. This eliminates manual synchronization entirely — the Rust struct IS the source of truth, and the TypeScript types are derived artifacts. If `ts-rs` is adopted, the snapshot tests become a CI verification step that confirms the generated types match what is checked in (preventing stale generated files).
- **Coverage:** Every response type in Section 11 (Hardware Detection, Binary Scanner, Model Scanner, Build SSE events, Config Recommendation, Save Config) must have either a snapshot or a `ts-rs` derivation. The CI check fails if a new response struct is added without a corresponding contract test.

### 10.8 Standalone Wizard Mode Tests

Tests for the standalone wizard server launched by `llamaforge init --interactive`:

- **Save failure → server stays alive → retry → success → clean shutdown:** Mock a filesystem error on the first save attempt (e.g., permission denied). Verify the server remains running and the wizard shows the error. Retry with a valid path. Verify the save succeeds and the server exits cleanly with exit code 0.
- **Ctrl+C → SIGINT → exit 130, no zombie process:** Send SIGINT to the wizard server process. Verify it exits with code 130. Verify no child processes (especially build subprocesses if a build was in progress) are left running. Check the process group is fully terminated.
- **Idle timeout → clean shutdown after configured duration:** Start the wizard with `--timeout=5s` (short timeout for testing). Make no API calls. Verify the server prints "No activity detected" and exits with code 0 after approximately 5 seconds. Verify the bound port is released.
- **Concurrent tabs → first save wins, second gets 409 or waits:** Open two browser tabs to the wizard. Complete the wizard in both tabs. Submit save-config from both tabs concurrently. Verify the first save succeeds. Verify the second save either receives a 409 Conflict (if the file now exists and `force` is false) or waits behind a server-side mutex and succeeds if `force` is true. Verify no config file corruption from concurrent writes.

---

## 11. Backend API Contract

All `/api/setup/*` responses include a top-level `"version": 1` field. The wizard API contract is internal and unstable until v1.0. When the main API moves to `/api/v1/`, wizard endpoints move with it.

The wizard requires new endpoints. All are under `/api/setup/*` and require the CSRF header on mutations.

### 11.1 Hardware Detection

```
GET /api/setup/hardware
→ 200 OK
{
  "version": 1,
  "gpus": [
    {
      "index": 0,
      "name": "NVIDIA RTX 4090",
      "vram_mb": 24564
    }
  ],
  "cpu": {
    "name": "AMD Ryzen 9 7950X",
    "cores_physical": 16,
    "cores_logical": 32
  },
  "ram_mb": 65536,
  "recommended_backend": "cuda"
}
```

Diagnostic fields (driver versions, architecture details) are available via the optional `?diagnostics=true` query parameter for bug reports.

Each subprocess probe has a per-command timeout enforced via `tokio::time::timeout`. If a probe times out it is killed (`SIGKILL`) and its field is reported as `"detection timed out"` rather than `null` or absent, so the frontend can distinguish "not present" from "present but unresponsive." The overall endpoint has a **30-second hard timeout**; if it fires, the handler returns whatever results have been collected so far plus a `warnings` entry per timed-out probe.

### 11.2 Binary Scanner

```
POST /api/setup/scan-binary
{ "extra_paths": ["/opt/custom/bin"] }   // optional additional search paths
→ 200 OK
{
  "version": 1,
  "found": true,
  "path": "/usr/local/bin/llama-server",
  "binary_version": "b4567 (2026-05-01)",
  "backends": ["cuda", "cpu"]
}
// or
{
  "version": 1,
  "found": false,
  "searched_paths": ["/usr/local/bin", "/opt/homebrew/bin", "~/.local/bin", "/usr/bin"]
}
```

Searches `$PATH` plus common install locations. If found, runs `llama-server --version` (5-second timeout) to extract version and supported backends. A timed-out or hung binary is reported as `"found": true` with `"binary_version": "found but unresponsive to --version"` so the user can confirm or reject it in the UI.

### 11.3 Model Scanner

```
POST /api/setup/scan-models
{
  "source": "local",
  "directories": ["/home/user/models", "./models"],
  "max_depth": 5,        // optional, default 5
  "max_results": 100     // optional, default 100
}
→ 200 OK
{
  "version": 1,
  "models": [
    {
      "id": "sha256-prefix-of-path",
      "path": "/home/user/models/llama-3-8b-Q4_K_M.gguf",
      "filename": "llama-3-8b-Q4_K_M.gguf",
      "size_bytes": 4370000000,
      "quantization_hint": "Q4_K_M",
      "architecture_hint": "llama",
      "modified": "2026-05-15T10:30:00Z"
    }
  ],
  "scan_duration_ms": 1234,
  "files_scanned": 5678,
  "truncated": false,
  "truncation_reason": null,   // "timeout" | "max_results" | "max_depth" — only present when truncated is true
  "warnings": ["Permission denied: /data/models/private/", "Symlink cycle detected: /data/models/loop"]
}
```

Recursive directory walk with `.gguf` extension filter, bounded by `max_depth` (default 5) and `max_results` (default 100). The backend enforces a **30-second scan timeout**. If any limit is hit, the response returns the models found so far with `"truncated": true` and `truncation_reason` indicating which limit fired first.

The `models` array is ordered by discovery time (filesystem walk order). When `truncated` is true, the array contains the first `max_results` models found — not a representative sample. If the UI needs sorted display, sort client-side by filename, size, or modification date.

Future: `{ "source": "huggingface", "query": "...", "quantization": "..." }` for model downloading (currently NG1).

### 11.4 Build llama.cpp

The build endpoint uses a job pattern: POST creates the build, GET polls status, GET /stream reconnects to the SSE stream, DELETE cancels.

```
POST /api/setup/build
{ "backend": "cuda", "install_dir": "/usr/local/bin", "jobs": 16 }
→ 202 Accepted { "version": 1, "build_id": "uuid-here" }
```

POST is idempotent: if a build is already running for the same backend, return the existing `build_id`.

```
GET /api/setup/build/{id}
→ 200 { "version": 1, "status": "running", "phase": "building", "percent": 50, "last_lines": [...] }
```

```
GET /api/setup/build/{id}/stream
→ 200 (SSE stream — reconnectable, replays terminal event if build finished)

event: phase
data: {"phase": "cloning", "message": "Cloning llama.cpp repository..."}

event: progress
data: {"phase": "cloning", "percent": 45}

event: log
data: {"line": "Receiving objects:  45% (1234/2742)"}

event: phase
data: {"phase": "configuring", "message": "Running cmake..."}

event: log
data: {"line": "-- Found CUDA: /usr/local/cuda (found version 12.4)"}

event: phase
data: {"phase": "building", "message": "Compiling (target 1/42)..."}

event: log
data: {"line": "[  2%] Building CXX object ..."}

event: progress
data: {"phase": "building", "percent": 50}

event: complete
data: {
  "build_id": "uuid-here",
  "binary_path": "/usr/local/bin/llama-server",
  "duration_secs": 342,
  "log_path": "/home/user/.local/share/llamaforge/build-logs/build-2026-06-03T10-30-00.log",  // optional, present when build log persistence is enabled
  "replaced_existing": false,  // optional, present when install_dir had an existing binary
  "backup_path": null           // optional, present when replaced_existing is true
}

// or on failure:
event: failed
data: {
  "phase": "building",
  "error": "cmake failed with exit code 1",
  "last_lines": ["..."],
  "log_path": "/home/user/.local/share/llamaforge/build-logs/build-2026-06-03T10-30-00.log"  // optional, present when build log persistence is enabled
}
```

```
DELETE /api/setup/build/{id}
→ 200 { "version": 1, "cancelled": true }
```

#### Stream Contract

- **Event ordering:** `phase` always precedes `progress`/`log` for that phase. First event is always `phase`. Last is always `complete` or `failed`.
- **`progress.percent`** is monotonically non-decreasing within a phase.
- **`log` events** contain complete lines (backend buffers partial lines).
- **Log throttling:** Backend throttles `log` events to max 10/s by batching: `log.lines: string[]` when batched.
- **Terminal event replay:** Terminal event (`complete` or `failed`) is replayed on reconnection if build finished while disconnected.

Future: a separate `POST /api/setup/acquire-binary` with `{ "method": "download", "release": "..." }` for prebuilt binary downloads (currently NG2).

**Binary backup before overwrite:** Before copying the built binary to `install_dir`, the handler checks if a file already exists at the target path (e.g., `/usr/local/bin/llama-server`). If it does, the existing binary is renamed to `llama-server.backup.<ISO-timestamp>` (e.g., `llama-server.backup.2026-06-03T14-30-00`) in the same directory. The SSE `complete` event includes `replaced_existing: true` and `backup_path` pointing to the renamed file. If no prior binary existed, `replaced_existing` is `false` and `backup_path` is `null`. The wizard `CompletionScreen` displays "Previous binary backed up to `<backup_path>`" when `replaced_existing` is `true`.

**Resource controls:**

- **Jobs cap:** The server caps parallelism at `jobs = min(requested_jobs, num_cpus / 2)` to avoid saturating all cores during a build. The effective job count is reported in the first `phase` SSE event.
- **Process priority:** The build subprocess tree is spawned with a lowered nice value (`nice 10` on Linux/macOS) so interactive workloads are not starved.
- **Disk space check:** Before starting the clone, the backend checks for a minimum of **5 GB free disk** at the temp directory mount point. If insufficient, the endpoint returns `409 Conflict` with `{"error": "insufficient_disk", "available_gb": N, "required_gb": 5}` and no SSE stream is opened.
- **First-run warning:** In first-run mode, the frontend shows a confirmation dialog before starting the build: "Building llama.cpp from source is resource-intensive and may take 5–30 minutes depending on your hardware. Your system may be less responsive during the build."

**Timeouts:**

- **Overall build timeout:** 30 minutes (configurable via `--build-timeout` CLI flag, minimum 5 minutes).
- **Per-phase timeouts:** clone 5 min, configure 5 min, build 20 min. If any phase exceeds its timeout, the build is aborted.
- **On timeout:** The backend kills the entire child process tree (`SIGKILL` on the process group via `kill(-pgid, SIGKILL)`), sends an SSE `failed` event with `{"phase": "<phase>", "error": "Build timed out during <phase> (exceeded <N>m limit)", "last_lines": ["..."]}`, and cleans up the temp directory.

**Cleanup:**

- **Build temp directory location:** The build UI displays the temp directory path to the user: "Building in /tmp/llamaforge-build-xxxx" (where xxxx is a random suffix from `mkdtemp`).
- **On build failure:** Clean up the temp directory UNLESS the user clicks "Retry." On retry, the existing clone is reused to skip the re-clone step.
- **On "Skip":** Clean up the temp directory immediately.
- **On SSE disconnect:** If the SSE client disconnects mid-build, the build continues server-side for a **2-minute grace period**. If the client reconnects within that window (via `GET /api/setup/build/{id}/stream`), it resumes from the current position in the log stream. If no reconnect occurs within the grace period, the build is killed and the temp directory is cleaned up.
- **On build success:** The temp directory is cleaned up after the binary is copied to `install_dir`. The cloned source is NOT preserved (keeping it for future rebuilds is a future enhancement per Open Question #2).
- **Orphaned subprocess cleanup:** The build process (git clone, cmake, make) is spawned in its own **process group** (`setsid` on Linux, `setpgid` on macOS). On wizard shutdown or SSE disconnect grace period expiry, send `SIGTERM` to the process group, wait 5 seconds, then `SIGKILL` if the process group is still alive. This ensures child processes spawned by cmake/make are also terminated.

### 11.5 Config Recommendation

```
POST /api/setup/recommend
{
  "models": [
    { "path": "/home/user/models/llama-3-8b-Q4_K_M.gguf", "size_bytes": 4370000000 }
  ]
}
→ 200 OK
{
  "version": 1,
  "servers": [
    {
      "model_path": "/home/user/models/llama-3-8b-Q4_K_M.gguf",
      "recommended": {
        "gpu_layers": -1,
        "ctx_size": 8192,
        "parallel_slots": 2,
        "cache_type_k": "q8_0",
        "cache_type_v": "q8_0",
        "flash_attn": "auto",
        "estimated_vram_mb": 6200,
        "port": 8080
      },
      "rationale": {
        "gpu_layers": "Full offload — model (4.1 GB) + KV cache (1.2 GB) fits in 24 GB VRAM",
        "ctx_size": "8192 tokens — balanced for 24 GB VRAM with room for a second model",
        "parallel_slots": "2 slots — enough VRAM headroom for concurrent requests"
      }
    }
  ],
  "warnings": [
    "Total estimated VRAM for all models: 12.4 GB / 24 GB available. You have headroom for additional models."
  ]
}
```

The backend caches hardware detection results in memory. The recommendation endpoint reads cached hardware internally. If hardware needs refreshing, the client calls `GET /api/setup/hardware` first.

### 11.6 Save Config

```
POST /api/setup/save-config
{
  "path": "/home/user/.config/llamaforge/llamaforge.toml",
  "force": false,
  "servers": [
    {
      "model_path": "/home/user/models/llama-3-8b-Q4_K_M.gguf",
      "alias": "llama-3-8b",
      "binary_path": "/usr/local/bin/llama-server",
      "port": 8080,
      "gpu_layers": -1,
      "ctx_size": 8192,
      "parallel_slots": 2,
      "cache_type_k": "q8_0",
      "cache_type_v": "q8_0",
      "flash_attn": "auto"
    }
  ],
  "global_overrides": {
    "bind_address": "127.0.0.1",
    "http_port": 8000
  }
}
→ 200 OK { "version": 1, "path": "/home/user/.config/llamaforge/llamaforge.toml", "backup_path": null }
→ 200 OK { "version": 1, "path": "/home/user/.config/llamaforge/llamaforge.toml", "backup_path": "/home/user/.config/llamaforge/llamaforge.toml.backup.2026-06-03T14-30-00" }
→ 409 Conflict { "message": "File exists. Set force=true to overwrite." }
→ 403 Forbidden { "message": "Permission denied: /etc/llamaforge/llamaforge.toml" }
```

The frontend never sees or constructs `AppConfig`. The backend is the sole owner of config assembly -- it takes the wizard's server list and global overrides, fills defaults for priority_rules, routing, and other advanced settings, validates through the existing pipeline, and writes.

```
POST /api/setup/preview-config
(same request body as save-config)
→ 200 OK { "version": 1, "toml": "# Generated by LlamaForge Setup Wizard\n..." }
```

The preview endpoint assembles and validates the config identically to save-config, but returns the rendered TOML string without writing to disk. Used by the ReviewStep to display the syntax-highlighted TOML preview.

**Backup before overwrite:** When `force=true` and the target file already exists, the handler copies the existing file to `<path>.backup.<ISO-timestamp>` (e.g., `llamaforge.toml.backup.2026-06-03T14-30-00`) before writing the new config. The response includes `backup_path` when a backup was created (`null` otherwise). The `CompletionScreen` displays "Previous config backed up to `<backup_path>`" when the field is non-null. The handler retains a maximum of **5 backup files** per config path, pruning the oldest on each save. Pruning is best-effort — failure to delete old backups is logged at WARN and does not block the save.

**Write mutex:** The save-config handler acquires a `tokio::sync::Mutex` before performing the backup + write sequence. If a second save-config request arrives while a write is already in progress, it waits for the in-progress write to complete and then returns the result of its own write (not the in-progress one's result, since the configs may differ). This prevents concurrent writes from interleaving backup and write operations, which could corrupt the backup or the config file.

### 11.7 Backend Observability

All `/api/setup/*` handlers use the project's existing `tracing` infrastructure with structured key-value fields. No `tracing` calls in domain logic — instrumentation lives at the adapter boundary (the axum handler), consistent with R26.

**Cross-cutting span:** Every `/api/setup/*` handler wraps its body in a `tracing::info_span!("setup_request", endpoint = %endpoint_name, setup_session_id = %session_id)` where `setup_session_id` is extracted from the `X-Setup-Session-Id` request header (a UUID generated by the frontend — see Session Correlation below). This span is the parent for all child spans within the request, enabling grep-by-session to reconstruct a full setup flow.

**Per-endpoint logging:**

**Hardware detection (`GET /api/setup/hardware`):**
- INFO: each detection command executed (e.g., `command = "nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader"`)
- INFO on success: raw stdout (truncated to 4 KB via `stdout = %truncate(raw, 4096)`), exit code, parsed result summary (e.g., `gpus_found = 1, ram_mb = 65536`)
- WARN on partial detection: GPU not found but CPU/RAM detected (`gpu_detection = "not_found"`, `fallback = "cpu_only"`)
- ERROR on command failure: raw stderr (truncated to 4 KB), exit code, command path

**Binary scan (`POST /api/setup/scan-binary`):**
- INFO: paths searched (`searched_paths = ?paths`)
- INFO per path: existence check result (`path = %p, exists = %exists`)
- INFO on found: `llama-server --version` raw output (`raw_version_output = %out`) and parsed version (`parsed_version = %ver`)
- ERROR on version parse failure or execution error: raw stderr, exit code

**Model scan (`POST /api/setup/scan-models`):**
- INFO: directories entered for scanning (`scan_dir = %dir`)
- INFO at completion per directory: file count (`dir = %dir, gguf_files_found = %count`)
- WARN: permission errors with path and error detail (`path = %p, error = %e, kind = "permission_denied"`) — these are also surfaced in the response `warnings` field (see Section 11.3)
- WARN: symlink cycle detection (`path = %p, kind = "symlink_cycle"`) — also surfaced in response `warnings`
- DEBUG: symlink follow/skip decisions (`path = %p, target = %target, decision = "follow"` or `"skip_cycle"`)
- DEBUG: hidden directories skipped (`path = %p, reason = "hidden_directory"`)

**Config recommendation (`POST /api/setup/recommend`):**
- INFO: input hardware summary (`gpus = ?gpu_names, total_vram_mb = %vram, ram_mb = %ram, cpu_cores = %cores`)
- INFO per model: VRAM calculation steps (`model = %path, model_size_mb = %size, kv_cache_mb = %kv, overhead_mb = %overhead, total_estimated_mb = %total`)
- INFO: final recommended values with rationale (`model = %path, gpu_layers = %gl, ctx_size = %ctx, parallel_slots = %ps, rationale = %rationale`)

**Save config (`POST /api/setup/save-config`):**
- INFO: resolved path (`resolved_path = %path`), overwrite status (`overwrite = %exists, force = %force`)
- INFO: backup creation if overwriting (`backup_path = %backup`)
- ERROR on I/O failure: full error chain (`path = %path, error = %e`) using `Display` chain (not just the root cause)

**Build process (`POST /api/setup/build`):**
- INFO: build initiation (`backend = %backend, install_dir = %dir, jobs = %jobs`)
- INFO per phase transition: phase name and command (`phase = %phase, command = %cmd`)
- INFO on command stdout/stderr (truncated to 4 KB per log emission): `phase = %phase, stream = "stdout"/"stderr"`, raw output
- INFO on success: binary path, build duration (`binary_path = %path, duration_secs = %dur`)
- ERROR on failure: phase, exit code, last N lines of output, full error chain

#### Build Log Persistence

All stdout and stderr from git clone, cmake configure, and cmake build are tee'd to a persistent log file at `$XDG_DATA_HOME/llamaforge/build-logs/build-<ISO-timestamp>.log` (e.g., `build-2026-06-03T14-30-00.log`). This happens regardless of terminal log level — even when `RUST_LOG=warn` suppresses verbose output to stderr, the build log file captures everything.

- The SSE `complete` event includes a `log_path` field: `{"binary_path": "...", "duration_secs": 342, "log_path": "/home/user/.local/share/llamaforge/build-logs/build-2026-06-03T14-30-00.log"}`
- The SSE `failed` event also includes `log_path`: `{"phase": "building", "error": "...", "last_lines": [...], "log_path": "/home/user/.local/share/llamaforge/build-logs/build-2026-06-03T14-30-00.log"}`
- The build failure UI displays the log path with the message: "Build log saved to [path]. Include this file in bug reports."
- **Retention policy:** keep the logs from the last 5 builds or 30 days, whichever retains more. Older logs are auto-cleaned at the start of the next build (before the new log file is created). Cleanup is best-effort — failures to delete old logs are logged at WARN and do not block the build.

#### Session Correlation

The `X-Setup-Session-Id` header provides end-to-end correlation across all wizard API calls:

- **Frontend:** `SetupLayout` generates a UUID v4 on initialization and stores it in the `SetupWizardStore`. All API calls from the wizard attach it as the `X-Setup-Session-Id` header.
- **Backend:** Every `/api/setup/*` handler extracts this header and includes it as a `setup_session_id` field in the top-level `tracing::info_span!`. If the header is missing (e.g., direct curl calls), the field is set to `"anonymous"`.
- **Usage:** `grep setup_session_id=<uuid>` across the log output reconstructs the full setup flow for a single wizard session — hardware detection, binary scan, model scan, config recommendation, and save — in chronological order with full structured context.
- **No server-side session state.** The ID is purely a correlation token for logs. The backend stores nothing per-session; all state lives in the frontend store (Section 5.3).

---

## 12. Standalone Wizard Mode

### 12.1 Shared Setup Router

Both standalone mode (Section 12) and first-run mode (Section 13) mount the same set of setup endpoints. A single `setup_router()` function defines them:

```rust
fn setup_router() -> Router {
    Router::new()
        .route("/api/setup/hardware", get(hardware_handler))
        .route("/api/setup/scan-models", post(scan_models_handler))
        .route("/api/setup/build", post(build_handler))
        .route("/api/setup/build/{id}", get(build_status_handler).delete(build_cancel_handler))
        .route("/api/setup/build/{id}/stream", get(build_stream_handler))
        .route("/api/setup/recommend", post(recommend_handler))
        .route("/api/setup/save-config", post(save_config_handler))
        .route("/api/setup/preview-config", post(preview_config_handler))
        .route("/api/setup/validate-launch", post(validate_launch_handler))
}
```

This eliminates divergence between the two modes. Each mode composes its final router differently (see below), but the setup endpoint handlers are identical in both.

The standalone and first-run modes use two distinct server assembly paths (minimal axum vs. full transport layer) that share a single `setup_router()`. This is a deliberate trade-off: the standalone mode avoids booting inference infrastructure that doesn't exist yet, while the first-run mode needs the full stack for the setup-to-running transition. The shared `setup_router()` ensures identical endpoint behavior in both modes.

### 12.2 Lifecycle

When invoked as `llamaforge init --interactive`, the binary boots a minimal axum server:

1. Bind to `127.0.0.1:0` (ephemeral port — avoids conflicts).
2. Mount only: `ui_router().merge(setup_router())`. No gRPC, no inference engine, no config handle, no SSE events, no management API.
3. Print the URL to stdout: `Setup wizard running at http://127.0.0.1:54321/ui/setup`.
4. Attempt to open the browser (`open` on macOS, `xdg-open` on Linux).
5. Start the idle timeout tracker (Section 12.5).
6. Wait for the save-config and validate-launch sequence to complete successfully. On success, shut down the server and exit 0.
7. On Ctrl+C, shut down and exit 130.

### 12.3 Logging Posture

The standalone wizard server configures `tracing_subscriber` with `RUST_LOG` support via `EnvFilter`:

- **Default log level:** `WARN` to stderr. The terminal stays quiet unless something goes wrong -- the wizard is a UI-driven experience, not a log-watching one.
- **Startup message:** A single INFO-level line is always emitted regardless of `RUST_LOG`: `"Setup wizard running at http://127.0.0.1:<port>/ui/setup -- for verbose output, run with RUST_LOG=debug"`. This ensures the user always sees the URL and knows how to get more detail if needed.
- **Build logs always persisted:** Build log file persistence (Section 11.7, Build Log Persistence) operates independently of the terminal log level. Even at `RUST_LOG=warn`, all build stdout/stderr is written to the log file under `$XDG_DATA_HOME/llamaforge/build-logs/`.

### 12.4 CLI Interface

The `Commands` enum in `main.rs` gains an `--interactive` flag and a `--timeout` option on the `Init` subcommand:

```rust
Init {
    #[arg(long)]
    interactive: bool,

    /// Idle timeout for the interactive wizard. Exits if no API activity
    /// for this duration. Format: "30m", "1h", "45m". Default: 30 minutes.
    #[arg(long, default_value = "30m")]
    timeout: humantime::Duration,

    // ... existing fields
}
```

When `interactive` is false (default), the current template-writing behavior is preserved. When true, the wizard server starts.

### 12.5 Idle Timeout

The standalone wizard runs unattended on a bound port. Without a timeout, a forgotten wizard process stays open indefinitely. The idle timeout prevents this:

- **Default timeout:** 30 minutes. Overridable with `--timeout=<duration>` (e.g., `--timeout=1h`, `--timeout=45m`).
- **Idle timer resets** on any API call to any `/api/setup/*` endpoint. The reset is implemented as middleware on the setup router that touches a shared `Arc<AtomicU64>` holding the last-activity timestamp (monotonic `Instant` serialized as epoch millis).
- **Periodic stdout reminder:** Every 5 minutes, a Tokio interval task prints `Wizard still running at http://127.0.0.1:<port>/ui/setup` to stdout. This serves as a breadcrumb if the user's terminal scrolled past the initial URL.
- **On idle timeout expiry:** Print `No activity detected for 30 minutes. Shutting down.` to stdout and exit 0. The exit code is 0 (not an error -- the user simply didn't finish).
- **Timer check frequency:** The timeout task wakes every 60 seconds and compares `now - last_activity` against the configured timeout. This avoids per-second wake-ups while keeping the timeout granularity within one minute of the configured value.

```
# Example stdout timeline for a forgotten wizard:
Setup wizard running at http://127.0.0.1:54321/ui/setup
[5 min]  Wizard still running at http://127.0.0.1:54321/ui/setup
[10 min] Wizard still running at http://127.0.0.1:54321/ui/setup
...
[30 min] No activity detected for 30 minutes. Shutting down.
```

---

## 13. First-Run Detection and Mode State Machine

### 13.1 Config Load Triage

When `llamaforge serve` is invoked, the binary attempts to load the config file. The outcome determines the boot path:

| `ConfigLoadError` variant | Behavior |
|---------------------------|----------|
| `NotFound` | Enter setup mode (Section 13.2). The user has no config -- this is a fresh install. |
| `EmptyFile` | Enter setup mode (Section 13.2). The user has a placeholder file with no content. |
| `ParseError(err)` | **Hard exit** with `process::exit(78)` (EX_CONFIG). Print `Error: failed to parse config: {err}` to stderr. A parse error means the user has a config that is syntactically broken -- the wizard cannot help with that; the user must fix the TOML. |
| (no error) | Normal boot into running mode. |

Only `NotFound` and `EmptyFile` trigger the wizard. This is a deliberate constraint: the wizard generates configs from scratch; it does not repair malformed ones.

### 13.2 Mode State Machine

The server maintains an explicit mode as an `Arc<ArcSwap<Mode>>` shared across all handlers:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// Wizard is active. Inference endpoints return 503.
    Setup,

    /// Config was saved and validated. Hot-reload is in progress.
    /// Inference endpoints still return 503.
    Transitioning,

    /// Normal operation. Inference endpoints are live.
    /// Setup endpoints remain mounted but guarded.
    Running,
}
```

**State transitions:**

```
                  +-----------------------------------+
                  |                                   |
                  v                                   |
  boot ----> [Setup] --save+validate--> [Transitioning]
               ^                              |
               |                              +-- hot-reload succeeds --> [Running]
               |                              |
               +-- hot-reload fails ----------+
```

- **Setup -> Transitioning:** Triggered when `/api/setup/save-config` successfully writes the config file AND the validation-launch step passes (Section 13.5).
- **Transitioning -> Running:** The server attempts `config_handle.reload()`. If the hot-reload succeeds and the new config is active, the mode advances to `Running`.
- **Transitioning -> Setup:** If the hot-reload fails (e.g., the config references a path that became invalid between write and reload), the mode reverts to `Setup`. The config file remains on disk (it was already validated once). The save-config response includes the `reload_error` so the UI can display it.

There is no `Running -> Setup` transition. Once in `Running`, the setup endpoints remain mounted for the reconfigure flow (Flow 4), but the mode stays `Running`. See Section 13.8 for the reconfigure guard.

### 13.3 Inference Endpoint Behavior During Setup

While the mode is `Setup` or `Transitioning`, all inference endpoints return **503 Service Unavailable** with a JSON body directing clients to the wizard:

**gRPC RPCs** (all services in `llamaforge.proto`):

```
grpc-status: 14 (UNAVAILABLE)
grpc-message: "llamaforge is in setup mode"
grpc-metadata: setup-url=http://127.0.0.1:8000/ui/setup
```

**HTTP shim endpoints** (`/v1/chat/completions`, `/v1/completions`, `/v1/embeddings`, `/v1/models`):

```json
HTTP/1.1 503 Service Unavailable
Retry-After: 60
Content-Type: application/json

{
  "error": {
    "message": "llamaforge is in setup mode",
    "type": "service_unavailable",
    "setup_url": "http://127.0.0.1:8000/ui/setup"
  }
}
```

This is implemented as a tower middleware layer (`SetupModeGuard`) that checks the current mode before forwarding to the handler. The middleware is applied to the inference router, not the setup or management routers:

```rust
let inference_router = inference_router()
    .layer(SetupModeGuard::new(mode.clone()));
```

The `Retry-After: 60` header gives well-behaved HTTP clients a backoff hint. The `setup_url` in the body lets programmatic clients surface the wizard URL to the operator.

### 13.4 Boot Sequence in Setup Mode

When setup mode is triggered:

1. Create a default empty `AppConfig` (all defaults, no servers).
2. Boot the full transport layer (HTTP + gRPC) on the default ports (`:8000` for HTTP, `:50051` for gRPC).
3. Compose the router: `full_app_router().merge(setup_router().layer(setup_mode_guard))`. The setup router is merged into the full app router -- not replacing it -- so the management API, status endpoint, and UI routes are all available. Inference routes are gated by the `SetupModeGuard` middleware (Section 13.3).
4. Set mode to `Mode::Setup`.
5. The UI detects setup mode via `GET /api/status` (Section 13.6) and redirects accordingly.

### 13.5 Save-Config Flow with Pre-Validation and Validation-Launch

The save-config endpoint in first-run mode performs a multi-step validation pipeline before committing:

**Step 1 -- Pre-validation (before writing to disk):**

```
POST /api/setup/save-config
{
  "path": "/home/user/.config/llamaforge/llamaforge.toml",
  "force": false,
  "servers": [ ... ],
  "global_overrides": { ... }
}
```

(See Section 11.6 for the full request schema.)

Before writing the file, the handler assembles the full `AppConfig` from the wizard inputs (filling defaults for priority_rules, routing, etc.), then:

1. **Round-trip validation:** Serialize the assembled `AppConfig` to TOML, then re-parse it to confirm round-trip fidelity. Catches structural issues the assembly might miss.
2. **Path resolution:** Resolve all paths in the config (`binary_path`, `model_path` for each server) to absolute paths. Verify each exists and is readable.
3. **Port availability:** For each server's configured port, attempt a non-blocking `TcpListener::bind`. Release immediately. If any port is in use, return an error naming the conflicting port and the process holding it (via `lsof` / `ss` where available). The port availability check is advisory — another process may bind the port between the check and the actual server spawn. The validation-launch step (which attempts to start the server) is the authoritative check.

If any pre-validation step fails:

```json
HTTP/1.1 422 Unprocessable Entity
{
  "error": "pre_validation_failed",
  "details": [
    {"field": "servers.llama-8b.model_path", "error": "File not found: /home/user/models/missing.gguf"},
    {"field": "servers.llama-8b.port", "error": "Port 8080 is in use by process 'node' (PID 12345)"}
  ]
}
```

The mode stays `Setup`. Nothing is written to disk.

**Step 2 -- Write to disk:**

If pre-validation passes, write the config file. Same conflict/permission semantics as Section 11.6 (409 for existing file without `force`, 403 for permission denied).

**Step 3 -- Validation-launch (spawn a test server):**

After writing the config, the handler spawns the first configured server as a validation step:

1. Pick the first server entry from the config.
2. Spawn the `llama-server` process with the configured flags.
3. Wait for the health check to pass (HTTP `GET /health` returning 200). Timeout: **30 seconds**.
4. On health check success: kill the validation server, proceed to Step 4.
5. On health check failure (timeout or error): kill the validation server, return the failure to the client. The config remains on disk. The mode stays `Setup`.

If the validation server must be stopped (health check fails or times out), use the same shutdown sequence as build cleanup: SIGTERM → 5s grace → SIGKILL.

```json
// Validation-launch failure:
HTTP/1.1 200 OK
{
  "path": "/home/user/.config/llamaforge/llamaforge.toml",
  "validation_launch": {
    "success": false,
    "server": "llama-8b",
    "error": "Health check timed out after 30s. llama-server stderr: 'CUDA error: out of memory'",
    "hint": "Try reducing gpu_layers or ctx_size for this model."
  }
}
```

Note: this returns 200 because the config *was* written successfully. The `validation_launch.success: false` flag tells the UI the config is saved but may not work. The wizard presents the failure in the CompletionScreen and allows the user to go back and adjust settings.

**Step 4 -- Hot-reload and mode transition:**

If the validation-launch succeeds:

1. Set mode to `Mode::Transitioning`.
2. Call `config_handle.reload()` to hot-reload the new config into the running server.
3. If hot-reload succeeds: set mode to `Mode::Running`. Return success:

```json
HTTP/1.1 200 OK
{
  "path": "/home/user/.config/llamaforge/llamaforge.toml",
  "validation_launch": { "success": true, "server": "llama-8b" },
  "reload": { "success": true }
}
```

4. If hot-reload fails: revert mode to `Mode::Setup`. Return the reload error:

```json
HTTP/1.1 200 OK
{
  "path": "/home/user/.config/llamaforge/llamaforge.toml",
  "validation_launch": { "success": true, "server": "llama-8b" },
  "reload": { "success": false, "error": "Failed to bind gRPC port 50051: address already in use" }
}
```

The config file stays on disk in both cases -- it was already validated. The user can retry (which re-attempts hot-reload) or adjust settings.

### 13.6 Status Endpoint

The existing `GET /api/status` endpoint gains a `setup_required` boolean field:

```json
GET /api/status
{
  "setup_required": true,
  "version": "0.1.0",
  "uptime_secs": 12,
  "servers": []
}
```

| `setup_required` | Meaning |
|-------------------|---------|
| `true` | Mode is `Setup` or `Transitioning`. The wizard should be shown. Inference endpoints return 503. |
| `false` | Mode is `Running`. Normal operation. |

The field is a simple boolean rather than an enum to keep the API surface minimal. Clients do not need to distinguish `Setup` from `Transitioning` -- the internal mode state machine is a server-side implementation detail.

The SvelteKit root layout checks `setup_required` on mount. If `true`, it redirects to `/ui/setup`. The sidebar hides all nav links except "Setup" and "Settings."

### 13.7 CompletionScreen Verification

The CompletionScreen (the final wizard step) does not navigate to the dashboard on save-config response alone. It follows this verification sequence:

1. **Call save-config** and wait for the response.
2. **Check `validation_launch.success`** in the response. If `false`, display the error with the server name, stderr output, and hint. Show a "Go Back" button to return to the ConfigStep and a "Retry" button to re-attempt save with the same config.
3. **Check `reload.success`** in the response. If `false`, display the reload error. Show a "Retry" button that re-calls save-config (which re-attempts hot-reload since the file already exists -- use `force: true`).
4. **Poll `GET /api/status`** every 2 seconds, up to 5 attempts. Verify `setup_required == false`.
5. **If `setup_required` becomes `false`:** Show the success state (green checkmark, "LlamaForge is running" message, server summary). Enable the "Go to Dashboard" button.
6. **If `setup_required` is still `true` after 5 polls (10 seconds):** Show an error state: "Setup completed but the server hasn't fully transitioned yet." Show a "Retry" button that restarts the polling sequence and a "Go to Dashboard Anyway" link (in case the user wants to inspect the main UI).

```typescript
// CompletionScreen polling logic (simplified)
async function verifySetupComplete(): Promise<boolean> {
  for (let i = 0; i < 5; i++) {
    await sleep(2000);
    const status = await api.getStatus();
    if (!status.setup_required) return true;
  }
  return false;
}
```

### 13.8 Setup Endpoints in Running Mode (Reconfigure Flow)

The setup endpoints remain mounted in `Running` mode to support the reconfigure flow (Flow 4 -- adding models after a GPU upgrade, etc.). However, they are guarded by a middleware that checks:

1. **Mode is `Running`:** The request must include a `X-Setup-Reconfigure: true` header. Without it, setup endpoints return 403 with `{"error": "Setup endpoints require X-Setup-Reconfigure header in running mode"}`. This prevents accidental calls from stale wizard tabs or confused clients.
2. **Mode is `Setup` or `Transitioning`:** No header required. Setup endpoints work normally.

The reconfigure flow does not change the mode to `Setup`. It operates entirely within `Running` mode -- the existing config remains active while the user configures new servers. The save-config endpoint in reconfigure mode merges new server entries into the existing config rather than replacing it (see Open Question #4 in Section 14).

---

## 14. Open Questions

1. **GGUF header parser timeline.** The `sync_parse` function in `vram_estimate.rs` is stubbed. Model scanning currently infers metadata from filenames. Should we implement the real GGUF KV header parser as part of this feature, or ship with filename heuristics first? (Recommendation: filename heuristics for v1, real parser as fast-follow.)

2. **Build directory cleanup.** After a successful build, should the cloned llama.cpp source be deleted, or kept for future rebuilds? Keeping it allows `git pull && rebuild` for updates; deleting it saves disk space. (Recommendation: keep it in `$XDG_DATA_HOME/llamaforge/llama.cpp/`, with a "Clean up build files" option in the wizard completion screen.)

3. **Multi-GPU support.** The recommendation engine needs to handle multi-GPU systems (e.g., 2× RTX 3090). Should it recommend splitting models across GPUs, or assign one model per GPU? (Recommendation: one model per GPU for v1 — multi-GPU tensor parallelism is a llama.cpp-level feature that's complex to configure.)

4. **Existing config preservation.** In "reconfigure" mode (Flow 4), should the wizard merge new servers into the existing config, or replace the entire config? (Recommendation: merge — add new `[servers.*]` entries, preserve existing global settings and priority rules.)

5. **Build prerequisites check.** Before attempting the build, should the wizard verify that `git`, `cmake`, and a C++ compiler are installed? (Recommendation: yes — check prerequisites first and show a clear "Install these first" message with platform-specific instructions rather than letting cmake fail with cryptic errors.)

---

## 15. Alternatives Considered

### Rendering: Standalone React app vs. SvelteKit routes

**Rejected.** A separate React/Vite app for the wizard would mean a second frontend build pipeline, a second set of shared components, and a second chunk embedded in the binary. Using SvelteKit routes under `/ui/setup/*` reuses the entire existing build infrastructure, shared component library, theme system, and CSP configuration. The only cost is that the wizard code ships inside the same binary even when the user never uses it — but the entire wizard is < 40 KB gzipped, which is negligible.

### Binary acquisition: Download prebuilt releases vs. Build from source

**Build from source chosen per user requirement.** Prebuilt releases from GitHub would be faster but have downsides: they may not match the user's exact CUDA/driver version, they may not include all backends, and they go stale quickly. Building from source guarantees the binary matches the user's environment. A "Download prebuilt" option is a natural future enhancement but is explicitly out of scope for v1.

### Hardware detection: Rust-native vs. Shell commands

**Shell commands chosen.** Rust-native GPU detection would require crate dependencies for CUDA (`nvml-wrapper`), Metal (Objective-C FFI), and Vulkan (`ash`). These are heavy dependencies that would slow the build and increase binary size for a feature used once during setup. Shell-commanding `nvidia-smi`, `system_profiler`, etc. is reliable, well-documented, and zero-dependency. The parsing layer is straightforward and testable in isolation.

### Wizard state: URL params vs. Client store vs. Backend session

**Client store (Svelte 5 runes) chosen.** URL params would make the wizard bookmarkable but would require serializing complex nested state (hardware info, model lists) into query strings — ugly and fragile. Backend session state would survive page refreshes but adds server complexity for minimal benefit (how often does someone refresh mid-wizard?). A client-side store is simple, type-safe, and sufficient.

### Step navigation: Linear-only vs. Free navigation

**Linear with back-navigation chosen.** Free navigation (jump to any step) would require every step to handle the case where prerequisite data doesn't exist yet (e.g., Configure step without hardware detection). Linear navigation with "Back" is simpler and matches the wizard mental model — but completed steps are clickable in the step indicator for review purposes, and re-entering a completed step re-uses cached data.

---

## Appendix: Decision Log

| Decision | Choice | Rationale | Date |
|----------|--------|-----------|------|
| Rendering | SvelteKit SPA routes under `/ui/setup/*` | Reuses existing build pipeline, components, theme, CSP | 2026-06-03 |
| State management | Svelte 5 rune class + TanStack Query | Consistent with main UI; wizard state is session-scoped | 2026-06-03 |
| Hardware detection | Shell commands (`nvidia-smi`, `system_profiler`, etc.) | Zero Rust dependencies; reliable; testable in isolation | 2026-06-03 |
| Binary acquisition | Clone + build from source | Matches user's exact environment; no stale binaries | 2026-06-03 |
| Config recommendations | Backend computes, frontend displays with override | Leverages existing `compute_vram_estimate`; keeps complex logic server-side | 2026-06-03 |
| Standalone mode | Minimal axum server on ephemeral port | No config dependency; avoids port conflicts; auto-opens browser | 2026-06-03 |
| First-run mode | Server boots in "setup mode" with empty config | No restart needed after setup; smooth transition to running mode | 2026-06-03 |
| Step navigation | Linear with back-navigation + completed-step click | Simple mental model; prerequisite data always available | 2026-06-03 |
| TOML preview rendering | `renderMarkdown()` with code block | No `{@html}`; consistent with existing security convention | 2026-06-03 |
| Build log streaming | SSE via `EventSource` | Same pattern as main UI's `LiveDataStore`; native browser API | 2026-06-03 |

---

## Appendix: New TypeScript Types

```typescript
// $lib/api/types.ts additions

/** All /api/setup/* responses include this envelope. */
export interface SetupApiResponse<T> {
  version: 1;
  data: T;
}

// --- Hardware Detection ---
export interface GpuInfo {
  index: number;
  name: string;
  vram_mb: number;
}

export interface CpuInfo {
  name: string;
  cores_physical: number;
  cores_logical: number;
}

export interface HardwareInfo {
  gpus: GpuInfo[];
  cpu: CpuInfo;
  ram_mb: number;
  recommended_backend: 'cuda' | 'metal' | 'vulkan' | 'cpu';
}

// --- Binary Scanner ---
export type BinaryScanResult =
  | { found: true; path: string; binary_version: string; backends: string[] }
  | { found: false; searched_paths: string[] };

// --- Model Scanner ---
export interface ModelScanRequest {
  source: 'local';
  directories: string[];
  max_depth?: number;    // default 5
  max_results?: number;  // default 100
}

export interface DiscoveredModel {
  id: string;
  path: string;
  filename: string;
  size_bytes: number;
  quantization_hint: string;
  architecture_hint: string;
  modified: string;
}

export interface ModelScanResult {
  models: DiscoveredModel[];
  scan_duration_ms: number;
  files_scanned: number;
  truncated: boolean;
  truncation_reason: 'timeout' | 'max_results' | 'max_depth' | null;
  warnings: string[];
}

// --- Build ---
export type BuildPhase = 'cloning' | 'configuring' | 'building';
export type BuildBackend = 'cuda' | 'metal' | 'vulkan' | 'cpu';

export type BuildStatus =
  | { status: 'idle' }
  | { status: 'running'; build_id: string; phase: BuildPhase; percent: number | null; logLines: string[] }
  | { status: 'complete'; build_id: string; binary_path: string; duration_secs: number; log_path?: string; replaced_existing?: boolean; backup_path?: string | null }
  | { status: 'failed'; build_id: string; phase: BuildPhase; error: string; lastLines: string[]; log_path?: string }
  | { status: 'cancelled'; build_id: string };

// --- Config Recommendation ---
export interface RecommendationRequest {
  models: Array<{ path: string; size_bytes: number }>;
}

export interface ConfigRecommendation {
  gpu_layers: number;
  ctx_size: number;
  parallel_slots: number;
  cache_type_k: string;
  cache_type_v: string;
  flash_attn: string;
  estimated_vram_mb: number;
  port: number;
}

export interface ServerRecommendation {
  model_path: string;
  recommended: ConfigRecommendation;
  rationale: Record<string, string>;
}

export interface RecommendationResult {
  servers: ServerRecommendation[];
  warnings: string[];
}

// --- Save Config ---
export interface SaveConfigRequest {
  path: string;
  force: boolean;
  servers: ServerConfigDraft[];
  global_overrides?: {
    bind_address?: string;
    http_port?: number;
  };
}

// --- Wizard State ---
export type WizardStep = 'welcome' | 'hardware' | 'binary' | 'models' | 'configure' | 'review';

export interface ServerConfigDraft {
  model_path: string;
  alias: string;
  binary_path: string;
  port: number;
  gpu_layers: number;
  parallel_slots: number;
  ctx_size: number;
  cache_type_k: string;
  cache_type_v: string;
  flash_attn: string;
  estimated_vram_mb: number;
}
```

---

## Appendix: New Route Structure

```
ui/src/routes/setup/
├── +layout.svelte          ← SetupLayout (step indicator + nav + store provider)
├── +layout.ts              ← Disables SSR (adapter-static), guards setup mode
├── +page.svelte            ← WelcomeStep (redirect target for /ui/setup)
├── hardware/
│   └── +page.svelte        ← HardwareStep
├── binary/
│   └── +page.svelte        ← BinaryStep
├── models/
│   └── +page.svelte        ← ModelStep
├── configure/
│   └── +page.svelte        ← ConfigStep
└── review/
    └── +page.svelte        ← ReviewStep
```

---

## Appendix: Implementation Notes

These are backend implementation details extracted from the API contract section. They guide implementation but are not part of the API surface.

### Hardware Detection

On Linux, run `nvidia-smi --query-gpu=...` for NVIDIA, `rocm-smi` for AMD, `vulkaninfo` for Vulkan. On macOS, use `system_profiler SPDisplaysDataType` and the Metal API to detect GPU and unified memory. CPU info from `/proc/cpuinfo` (Linux) or `sysctl` (macOS). RAM from `/proc/meminfo` or `sysctl hw.memsize`. Per-command timeouts: `nvidia-smi` 10 s, `rocm-smi` 10 s, `vulkaninfo` 10 s, `system_profiler` 15 s, `/proc` reads 5 s, `sysctl` 5 s.

### Binary Scanner

Search `$PATH` plus common install locations (`/usr/local/bin`, `/opt/homebrew/bin`, `~/.local/bin`, `/usr/bin`). If found, run `llama-server --version` to extract version and supported backends.

### Model Scanner

Recursive directory walk (respecting symlinks, skipping hidden directories) with `.gguf` extension filter. `quantization_hint` and `architecture_hint` are parsed from the filename using common naming conventions (not GGUF header parsing -- that is too slow for a scan). If the GGUF header parser is implemented by then (currently stubbed), use it for selected files.

### Build Process

`git clone --depth=1 https://github.com/ggerganov/llama.cpp.git` into a temp directory. `cmake -B build -DGGML_CUDA=ON` (or `-DGGML_METAL=ON`, `-DGGML_VULKAN=ON`, `-DGGML_CPU_ONLY=ON` depending on backend). `cmake --build build -j <jobs> --target llama-server`. Stream stdout/stderr to the SSE connection. On success, copy the binary to `install_dir`.

### Config Recommendation

Use the existing `compute_vram_estimate` function from `llamaforge-server/src/vram_estimate.rs`, enhanced with actual GGUF metadata parsing. Port assignment starts at 8080 and auto-increments to avoid conflicts.
