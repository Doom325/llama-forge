# LlamaForge Setup Wizard — Implementation Plan

**Component:** Setup Wizard (`docs/component-designs/08_SETUP_WIZARD.md`)  
**Estimated LOC:** ~8,200 (~5,600 production + ~2,600 tests)  
**Estimated Duration:** 20–25 working days (single engineer) / 13–16 days (two engineers, backend/frontend parallelizable after Phase 2)  
**Design Document:** `docs/component-designs/08_SETUP_WIZARD.md` (2nd-pass CLEAN PASS — 0 findings)  
**Backend Branch Prefix:** `setup/`  
**Frontend Branch Prefix:** `ui/setup-`  
**Backend Merge Target:** `main`  
**Frontend Merge Target:** `frontend`  
**Final Target:** `main` (frontend branch merged after all frontend PRs pass end-to-end testing)  
**Upstream Dependencies:**  
- Backend PRs (Phase 1–2) have no frontend dependency and merge directly to `main`.  
- Frontend PRs (Phase 3) depend on Phase 1 and Phase 2 backend APIs being merged to `main`.  
- The existing frontend `frontend` branch must be up-to-date with `main` before Phase 3 begins.  
**Visual Design Reference:** None — the wizard has no static design reference files. The design doc (Section 2.2, Section 4) defines all screen states and component hierarchy.  
**Agent Model:** Coder agent (Gemini or any LLM agent) implements; Reviewer agent (Claude) reviews PRs against this plan, `docs/component-designs/08_SETUP_WIZARD.md`, and `docs/design.md`.

> **Branching workflow:**
>
> **Backend PRs (Phase 1–2):** Each PR branches from `main` and merges back into `main` via squash-merge. The "Depends on" field indicates which predecessor(s) must be merged before the branch is created. Always branch from `main` after predecessors merge.
>
> **Frontend PRs (Phase 3):** Each PR branches from `frontend` and merges into `frontend` via squash-merge, identical to the main UI workflow. The `frontend` branch must be rebased onto `main` after Phase 2 completes (to pick up the backend setup endpoints) before any Phase 3 PR begins.
>
> **Setup (run once before PR1):**
> ```bash
> git checkout main && git pull
> ```

---

## Ground Rules for CoderAgents

### Rust (Backend — Phase 1, 2, 4)

1. **Read the design doc first.** Every PR description must reference the specific `08_SETUP_WIZARD.md` sections it implements.
2. **Workspace deps only.** New dependencies belong in `[workspace.dependencies]`; member crates use `workspace = true`. A versioned dep added directly to a member `Cargo.toml` is a finding.
3. **No unwrap/expect on production paths.** `unwrap()` and `expect()` are permitted only in tests and `build.rs`. All production error paths must use `?` or explicit matching.
4. **No blocking in async.** Filesystem I/O, subprocess spawning, and GGUF parsing must use `tokio::task::spawn_blocking` or `tokio::process::Command`. Direct `std::fs` or `std::process::Command` in async functions is a finding.
5. **Telemetry stays at adapter boundaries.** `#[instrument]` and `tracing::info!` belong on axum handlers, not in domain logic (hardware detection, model scanning, recommendation engine). Rule R26.
6. **Tests use `mock-llama` or fixture data.** No tests against real `llama-server` binaries in default CI. Subprocess tests use shell script mocks or fixture stdout/stderr files.
7. **Commit messages:** `setup: <imperative summary>` for backend PRs (e.g., `setup: add hardware detection module with nvidia-smi parsing`).
8. **SQLite is behind traits.** No direct `rusqlite` use outside trait implementations.
9. **gRPC is canonical.** New types that cross crate boundaries should originate in `.proto` where applicable. The setup wizard's HTTP-only API contract (Section 11) is an exception — setup endpoints are axum-only, not gRPC.
10. **Hot-reload contract.** Config readers `.load()` from `ArcSwap` once per request and treat the snapshot as immutable for the request's lifetime.

### Svelte (Frontend — Phase 3)

1. **Read the design doc first.** Every PR description must reference the specific `08_SETUP_WIZARD.md` sections it implements. Open the design doc's Section 2.2 and Section 4 to understand all screen states before writing any component.
2. **Svelte 5 runes only.** Use `$state`, `$derived`, `$effect` — never `onMount`, `onDestroy`, `writable`, `readable`, or any Svelte 4 API.
3. **No raw `{@html}` outside `renderMarkdown()`.** The ESLint rule `no-raw-html-without-sanitize` enforces this. The only legal usage is `{@html renderMarkdown(untrustedContent)}`.
4. **Presentational components receive data via props, emit events via callbacks.** No `fetch()` calls, no store subscriptions, no `$effect` side effects in presentational components. Only orchestrator (page-level) components do I/O.
5. **All API calls go through `lib/api/client.ts`.** No raw `fetch()`. The client auto-attaches the CSRF header (`X-LlamaForge-Request: 1`) on non-GET requests and applies timeouts.
6. **TypeScript strict mode.** `"strict": true` in `tsconfig.json`. No `any` types. Prefer discriminated unions over boolean flags.
7. **TanStack Query for all server state.** No Svelte stores for server-originated data. Components consume data via `createQuery()`, never from SSE events directly.
8. **Tailwind CSS 4 only.** No inline `style` attributes except where Svelte scoped styles are required. No external CSS files per component.
9. **Accessibility from day one.** Every interactive element must be keyboard-navigable with visible focus indicator. Every status indicator uses `StatusBadge` (triple-redundancy). axe-core must pass in CI.
10. **Tests run with `npm run test`.** Every PR must pass `npm run check && npm run lint && npm run test:unit` before merge.
11. **Commit messages:** `ui: <imperative summary>` for frontend PRs (e.g., `ui: add wizard scaffold with StepIndicator and step state machine`).

---

## Pre-Implementation Decisions

These Open Questions from the design doc (Section 14) must be resolved before Phase 1 begins:

| # | Question | Decision | Rationale |
|---|----------|----------|-----------|
| OQ#1 | GGUF header parser timeline | **Filename heuristics for v1.** Real GGUF KV parser is a fast-follow. | Scanning speed > metadata accuracy for initial setup. Users can verify model details post-setup. |
| OQ#2 | Build directory cleanup | **Keep source in `$XDG_DATA_HOME/llamaforge/llama.cpp/`.** "Clean up build files" option on completion screen. | Enables `git pull && rebuild` for updates without re-cloning. |
| OQ#3 | Multi-GPU support | **One model per GPU for v1.** | Multi-GPU tensor parallelism is a llama.cpp-level feature that adds significant configuration complexity. |
| OQ#4 | Existing config preservation | **Merge by design.** The save-config endpoint accepts `servers[]` + `global_overrides` and merges into existing config. | Already resolved by the API contract redesign (design review finding #6). |
| OQ#5 | Build prerequisites check | **Yes — check before build.** Verify `git`, `cmake`, and a C++ compiler are installed. Show platform-specific install instructions if missing. | Better UX than letting cmake fail with cryptic errors. |

---

## Dependency Graph (PR merge order)

```
                     Phase 1: Backend Foundation
    PR1 ──┐
    PR2 ──┼── PR4 ──── PR5
    PR3 ──┘

                     Phase 2: Build System + CLI
                           PR5 ──── PR6 ──── PR7

                     Phase 3: Frontend Wizard (depends on Phase 1 + Phase 2)
                           PR8 ──── PR9 ──── PR10 ──── PR11

                     Phase 4: Integration & Testing (depends on Phase 3)
                           PR12 ──── PR13 ──── PR14
```

**Parallelism opportunities:**
- PR1 (hardware), PR2 (models), and PR3 (binary) are fully independent — all three can be developed concurrently after branching from `main`.
- PR4 (recommendation) depends on PR1 and PR2 (needs hardware + model types).
- PR3 (binary) is independent of PR4 — it is only needed by PR5.
- PR5 (setup API) depends on PR1–PR4 (mounts all endpoints).
- PR6 (build orchestrator) depends on PR5 (needs the setup router to mount build endpoints).
- PR7 (CLI changes) depends on PR6 (needs build + setup endpoints for the interactive wizard server).
- Phase 3 (frontend) depends on Phase 1 + Phase 2 being merged — all backend APIs must be available.
- PR9 depends on PR8 (wizard scaffold must exist).
- PR10 depends on PR9 (needs hardware/binary steps wired).
- PR11 depends on PR10 (needs model/config steps wired).
- Phase 4 PRs are sequential and depend on all of Phase 3.

---

## Phase 1: Backend Foundation

---

## PR1: Hardware Detection Module

**Branch:** `setup/01-hardware-detection`  
**Merge Target:** `main`  
**Depends on:** None  
**Size:** Medium (~400 lines)  
**Estimated Time:** 6–8 hours  

### Purpose

Implement the hardware detection module in `llamaforge-server` that executes shell commands (`nvidia-smi`, `system_profiler`, `rocm-smi`, `vulkaninfo`, `/proc/cpuinfo`, `sysctl`) to discover GPUs, CPU, and RAM. Each command has an independent timeout. The module returns partial results on failure — a timed-out probe reports `"detection timed out"`, not `null`. The overall detection has a 30-second hard timeout. This module is pure domain logic with no HTTP/axum dependency — it will be called by the setup API handler in PR5.

Design doc references: Section 11.1 (Hardware Detection API contract), Section 11.7 (Backend Observability — hardware detection logging), Appendix: Implementation Notes (Hardware Detection).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 1.1 | Define hardware detection types | `GpuInfo`, `CpuInfo`, `HardwareInfo`, `HardwareProbeResult`, `HardwareWarning`. Use `serde::Serialize` for JSON responses. | `crates/llamaforge-server/src/hardware.rs` |
| 1.2 | Implement shell command executor | Generic `run_command(cmd, args, timeout) -> Result<String, ProbeError>` using `tokio::process::Command` with `tokio::time::timeout`. On timeout, kill the child process (`SIGKILL`) and return `ProbeError::Timeout`. | `crates/llamaforge-server/src/hardware.rs` |
| 1.3 | Implement NVIDIA GPU detection | Parse `nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader`. Handle multi-GPU output (multiple CSV lines). 10-second timeout. | `crates/llamaforge-server/src/hardware.rs` |
| 1.4 | Implement AMD GPU detection | Parse `rocm-smi --showproductname --showmeminfo vram --csv`. 10-second timeout. | `crates/llamaforge-server/src/hardware.rs` |
| 1.5 | Implement Vulkan detection | Parse `vulkaninfo --summary`. Extract device name and memory. 10-second timeout. | `crates/llamaforge-server/src/hardware.rs` |
| 1.6 | Implement macOS GPU detection | Parse `system_profiler SPDisplaysDataType -json`. Extract GPU name and VRAM. Handle Apple Silicon unified memory. 15-second timeout. | `crates/llamaforge-server/src/hardware.rs` |
| 1.7 | Implement CPU detection (Linux) | Parse `/proc/cpuinfo` for model name, physical cores, logical cores. 5-second timeout. | `crates/llamaforge-server/src/hardware.rs` |
| 1.8 | Implement CPU detection (macOS) | Parse `sysctl -n machdep.cpu.brand_string`, `hw.physicalcpu`, `hw.logicalcpu`. 5-second timeout. | `crates/llamaforge-server/src/hardware.rs` |
| 1.9 | Implement RAM detection | Linux: `/proc/meminfo` (MemTotal). macOS: `sysctl hw.memsize`. 5-second timeout. | `crates/llamaforge-server/src/hardware.rs` |
| 1.10 | Implement `detect_hardware()` orchestrator | Run all probes concurrently via `tokio::join!`. Collect results into `HardwareInfo`. Compute `recommended_backend` (cuda > metal > vulkan > cpu). Overall 30-second timeout — return partial results if it fires. Populate `warnings` for timed-out probes. | `crates/llamaforge-server/src/hardware.rs` |
| 1.11 | Export module from lib.rs | Add `pub mod hardware;` to `llamaforge-server/src/lib.rs`. | `crates/llamaforge-server/src/lib.rs` |
| 1.12 | Unit tests: nvidia-smi parsing | Fixture: real `nvidia-smi` CSV output (single GPU, multi-GPU). Assert correct `GpuInfo` structs. | `crates/llamaforge-server/tests/hardware_tests.rs` |
| 1.13 | Unit tests: system_profiler parsing | Fixture: real `system_profiler -json` output (Apple Silicon, Intel discrete GPU). Assert correct parsing. | `crates/llamaforge-server/tests/hardware_tests.rs` |
| 1.14 | Unit tests: rocm-smi parsing | Fixture: real `rocm-smi` CSV output. Assert correct parsing. | `crates/llamaforge-server/tests/hardware_tests.rs` |
| 1.15 | Unit tests: partial failure | Mock one probe timing out. Assert `HardwareInfo` contains partial results + warning. | `crates/llamaforge-server/tests/hardware_tests.rs` |
| 1.16 | Unit tests: all probes fail | Mock all probes failing. Assert CPU-only fallback with `recommended_backend: "cpu"`. | `crates/llamaforge-server/tests/hardware_tests.rs` |
| 1.17 | Unit tests: gibberish output | Mock `nvidia-smi` returning garbage. Assert error, not a parsed `GpuInfo` with `vram_mb: 0`. | `crates/llamaforge-server/tests/hardware_tests.rs` |

### PR1 Acceptance Criteria

- [ ] `detect_hardware()` returns `HardwareInfo` with correct GPU/CPU/RAM on the test machine
- [ ] Each probe has an independent timeout (nvidia-smi 10s, system_profiler 15s, /proc 5s, sysctl 5s, rocm-smi 10s, vulkaninfo 10s)
- [ ] Overall 30-second hard timeout returns partial results
- [ ] Timed-out probes report `"detection timed out"` in warnings, not null fields
- [ ] Multi-GPU nvidia-smi output parses all GPUs
- [ ] Apple Silicon unified memory parses correctly
- [ ] `recommended_backend` follows priority: cuda > metal > vulkan > cpu
- [ ] All 6 test suites pass
- [ ] `cargo clippy` clean, `cargo test` passes
- [ ] No `tracing` calls inside domain logic — only in the types/orchestrator boundary

### PR1 Creation Steps

```bash
git checkout -b setup/01-hardware-detection main
# ... implement tasks 1.1–1.17 ...
git add -A && git commit -m "setup: add hardware detection module with per-command timeouts and partial failure handling"
git push -u origin setup/01-hardware-detection
# Open PR: "Setup Wizard PR1: Hardware Detection Module"
# Description: "Implements hardware detection (08_SETUP_WIZARD.md Section 11.1, Appendix: Hardware Detection).
# Shell-command based GPU/CPU/RAM detection with per-command timeouts (nvidia-smi 10s, system_profiler 15s,
# /proc 5s, sysctl 5s). Concurrent probe execution. Partial results on timeout. recommended_backend
# priority: cuda > metal > vulkan > cpu. 6 test suites with fixture data."
```

---

## PR2: Model Scanner Module

**Branch:** `setup/02-model-scanner`  
**Merge Target:** `main`  
**Depends on:** None (can be developed concurrently with PR1)  
**Size:** Medium (~350 lines)  
**Estimated Time:** 5–7 hours  

### Purpose

Implement the model scanner that recursively walks directories to discover `.gguf` files. The scanner respects `max_depth`, `max_results`, and a 30-second timeout, returning partial results when any limit fires. Metadata (quantization, architecture) is parsed from filenames using heuristics — not GGUF header parsing (per Open Question #1: filename heuristics for v1). The module reports permission errors and symlink cycles as warnings in the response, not as fatal errors.

Design doc references: Section 11.3 (Model Scanner API contract), Section 11.7 (Backend Observability — model scan logging), Appendix: Implementation Notes (Model Scanner).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 2.1 | Define model scanner types | `DiscoveredModel`, `ModelScanRequest`, `ModelScanResult`, `ScanWarning`. Use `serde::Serialize`/`Deserialize`. `id` is SHA-256 prefix of the canonical path. `ModelScanRequest` includes `source: "local"` discriminator field (only `"local"` implemented; future values documented in design doc Section 11.3). | `crates/llamaforge-server/src/model_scanner.rs` |
| 2.2 | Implement filename heuristic parser | Extract `quantization_hint` and `architecture_hint` from GGUF filenames. Common patterns: `llama-3-8b-Q4_K_M.gguf`, `mistral-7b-instruct-v0.3-Q5_K_S.gguf`. Handle uppercase/lowercase variants. | `crates/llamaforge-server/src/model_scanner.rs` |
| 2.3 | Implement recursive directory walker | `tokio::task::spawn_blocking` wrapping a synchronous `walkdir::WalkDir` with `max_depth`. Filter `.gguf` extension. Skip hidden directories (names starting with `.`). Follow symlinks but detect cycles (via canonical path set). | `crates/llamaforge-server/src/model_scanner.rs` |
| 2.4 | Implement bounded scanning | Enforce `max_results` (default 100). Enforce 30-second timeout via `tokio::time::timeout`. When any limit fires, return models found so far with `truncated: true` and `truncation_reason`. | `crates/llamaforge-server/src/model_scanner.rs` |
| 2.5 | Implement warning collection | Collect permission errors (`std::io::ErrorKind::PermissionDenied`) and symlink cycle detections as `warnings` strings. Continue scanning after non-fatal errors. | `crates/llamaforge-server/src/model_scanner.rs` |
| 2.6 | Implement `scan_models()` public API | Takes `ModelScanRequest`, returns `ModelScanResult`. Validates inputs: directories non-empty, max 10 directories, paths resolved via `canonicalize`. | `crates/llamaforge-server/src/model_scanner.rs` |
| 2.7 | Export module from lib.rs | Add `pub mod model_scanner;` to `llamaforge-server/src/lib.rs`. | `crates/llamaforge-server/src/lib.rs` |
| 2.8 | Add `walkdir` to workspace dependencies | Add `walkdir = "2"` to `[workspace.dependencies]` in root `Cargo.toml`. Add `walkdir = { workspace = true }` to `llamaforge-server/Cargo.toml`. | `Cargo.toml`, `crates/llamaforge-server/Cargo.toml` |
| 2.9 | Unit tests: filename heuristic parsing | `"llama-3-8b-Q4_K_M.gguf"` -> quantization `Q4_K_M`, architecture `llama`. `"mistral-7b-instruct-v0.3-Q5_K_S.gguf"` -> quantization `Q5_K_S`, architecture `mistral`. Unknown patterns -> `"unknown"`. | `crates/llamaforge-server/tests/model_scanner_tests.rs` |
| 2.10 | Unit tests: tempdir scanning | Create tempdir with 5 `.gguf` files at varying depths. Assert all 5 found with correct metadata. | `crates/llamaforge-server/tests/model_scanner_tests.rs` |
| 2.11 | Unit tests: max_depth enforcement | Create tempdir with files at depth 3 and 7. Set `max_depth: 5`. Assert depth-3 found, depth-7 not found. | `crates/llamaforge-server/tests/model_scanner_tests.rs` |
| 2.12 | Unit tests: max_results enforcement | Create tempdir with 20 `.gguf` files. Set `max_results: 10`. Assert `truncated: true`, `truncation_reason: "max_results"`, exactly 10 models returned. | `crates/llamaforge-server/tests/model_scanner_tests.rs` |
| 2.13 | Unit tests: permission errors | Create a directory with restricted permissions (if running as non-root). Assert warning in response, scan continues for other directories. | `crates/llamaforge-server/tests/model_scanner_tests.rs` |
| 2.14 | Unit tests: empty directories | Scan a tempdir with no `.gguf` files. Assert `models: []`, `truncated: false`. | `crates/llamaforge-server/tests/model_scanner_tests.rs` |
| 2.15 | Unit tests: hidden directory skipping | Create `.hidden/model.gguf`. Assert not found. | `crates/llamaforge-server/tests/model_scanner_tests.rs` |

### PR2 Acceptance Criteria

- [ ] `scan_models()` discovers all `.gguf` files in specified directories recursively
- [ ] Filename heuristic parser extracts quantization and architecture hints
- [ ] `max_depth` (default 5) limits recursion
- [ ] `max_results` (default 100) limits result count with `truncated: true`
- [ ] 30-second timeout returns partial results with `truncation_reason: "timeout"`
- [ ] Permission errors collected as warnings, not fatal
- [ ] Symlink cycles detected and reported as warnings
- [ ] Hidden directories (`.foo`) skipped
- [ ] Empty directories return empty model list (not an error)
- [ ] All 7 test suites pass
- [ ] `cargo clippy` clean, `cargo test` passes

### PR2 Creation Steps

```bash
git checkout -b setup/02-model-scanner main
# ... implement tasks 2.1–2.15 ...
git add -A && git commit -m "setup: add model scanner with recursive walk, filename heuristics, and bounded results"
git push -u origin setup/02-model-scanner
# Open PR: "Setup Wizard PR2: Model Scanner Module"
# Description: "Implements model scanning (08_SETUP_WIZARD.md Section 11.3, Appendix: Model Scanner).
# Recursive directory walk with .gguf filter. Filename heuristic parsing for quantization/architecture.
# Bounded by max_depth (5), max_results (100), and 30s timeout. Partial results on any limit.
# Permission errors and symlink cycles as warnings. 7 test suites with tempdir fixtures."
```

---

## PR3: Binary Scanner Module

**Branch:** `setup/03-binary-scanner`  
**Merge Target:** `main`  
**Depends on:** None (can be developed concurrently with PR1 and PR2)  
**Size:** Small (~200 lines)  
**Estimated Time:** 3–4 hours  

### Purpose

Implement the binary scanner that searches `$PATH` plus common install locations for `llama-server`, then runs `llama-server --version` (5-second timeout) to extract version and supported backends. A found-but-unresponsive binary is reported as `"found but unresponsive to --version"`, not as "not found."

Design doc references: Section 11.2 (Binary Scanner API contract), Section 11.7 (Backend Observability — binary scan logging), Appendix: Implementation Notes (Binary Scanner).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 3.1 | Define binary scanner types | `BinaryScanResult` (discriminated: found vs not-found), `BinaryScanRequest`. `serde::Serialize`/`Deserialize`. | `crates/llamaforge-server/src/binary_scanner.rs` |
| 3.2 | Implement PATH + common location search | Search `$PATH` entries plus `/usr/local/bin`, `/opt/homebrew/bin`, `~/.local/bin`, `/usr/bin`. Check file existence and executability. Return first match. | `crates/llamaforge-server/src/binary_scanner.rs` |
| 3.3 | Implement extra_paths support | Accept optional `extra_paths` from the request. Prepend to search list. Validate: no null bytes, paths are absolute or resolvable. | `crates/llamaforge-server/src/binary_scanner.rs` |
| 3.4 | Implement version extraction | Run `llama-server --version` with 5-second `tokio::time::timeout`. Parse stdout for version string and backend list. | `crates/llamaforge-server/src/binary_scanner.rs` |
| 3.5 | Implement unresponsive fallback | If `--version` times out or returns non-zero, report `found: true` with `version: "found but unresponsive to --version"`. | `crates/llamaforge-server/src/binary_scanner.rs` |
| 3.6 | Implement `scan_binary()` public API | Takes `BinaryScanRequest`, returns `BinaryScanResult`. | `crates/llamaforge-server/src/binary_scanner.rs` |
| 3.7 | Export module from lib.rs | Add `pub mod binary_scanner;` to `llamaforge-server/src/lib.rs`. | `crates/llamaforge-server/src/lib.rs` |
| 3.8 | Unit tests: binary found | Create a mock shell script at a temp path that responds to `--version`. Assert `found: true` with parsed version. | `crates/llamaforge-server/tests/binary_scanner_tests.rs` |
| 3.9 | Unit tests: binary not found | Search in empty temp directories. Assert `found: false` with `searched_paths`. | `crates/llamaforge-server/tests/binary_scanner_tests.rs` |
| 3.10 | Unit tests: version timeout | Create a mock script that sleeps 10 seconds on `--version`. Assert `found: true` with `"found but unresponsive"`. | `crates/llamaforge-server/tests/binary_scanner_tests.rs` |
| 3.11 | Unit tests: extra_paths searched first | Pass `extra_paths: ["/custom/path"]`. Assert it appears first in searched_paths. | `crates/llamaforge-server/tests/binary_scanner_tests.rs` |

### PR3 Acceptance Criteria

- [ ] `scan_binary()` searches `$PATH` plus 4 common locations
- [ ] `extra_paths` are prepended to the search list
- [ ] `llama-server --version` has a 5-second timeout
- [ ] Timed-out binary reported as `found: true` with `"found but unresponsive"` (not `found: false`)
- [ ] `searched_paths` list returned when binary not found
- [ ] All 4 test suites pass
- [ ] `cargo clippy` clean, `cargo test` passes

### PR3 Creation Steps

```bash
git checkout -b setup/03-binary-scanner main
# ... implement tasks 3.1–3.11 ...
git add -A && git commit -m "setup: add binary scanner with PATH search, version extraction, and timeout fallback"
git push -u origin setup/03-binary-scanner
# Open PR: "Setup Wizard PR3: Binary Scanner Module"
# Description: "Implements binary scanning (08_SETUP_WIZARD.md Section 11.2, Appendix: Binary Scanner).
# PATH + common location search. llama-server --version with 5s timeout. Found-but-unresponsive
# fallback. extra_paths support. 4 test suites with mock shell scripts."
```

---

## PR4: Config Recommendation Engine

**Branch:** `setup/04-recommendation`  
**Merge Target:** `main`  
**Depends on:** PR1 (hardware types) and PR2 (model types) merged to `main`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 5–7 hours  

### Purpose

Implement the recommendation engine that takes hardware detection results and selected models, then computes `gpu_layers`, `ctx_size`, `parallel_slots`, `cache_type_k/v`, `flash_attn`, and `estimated_vram_mb` per model. Leverages the existing `compute_vram_estimate` function from `llamaforge-server/src/vram_estimate.rs`. Includes rationale strings explaining each recommendation. Port assignment starts at 8080 and auto-increments.

Design doc references: Section 11.5 (Config Recommendation API contract), Section 11.7 (Backend Observability — recommendation logging), Appendix: Implementation Notes (Config Recommendation).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 4.1 | Define recommendation types | `RecommendationRequest`, `ConfigRecommendation`, `ServerRecommendation`, `RecommendationResult`. `serde::Serialize`/`Deserialize`. | `crates/llamaforge-server/src/recommendation.rs` |
| 4.2 | Implement VRAM budget calculation | For each model: estimate model size from file_size_bytes, estimate KV cache from ctx_size, add overhead. Per Open Question #1 (filename heuristics in v1; GGUF header parsing deferred, `parse_gguf` still stubbed), use a transparent file-size-based heuristic — **not** `compute_vram_estimate`, which requires full GGUF metadata. | `crates/llamaforge-server/src/recommendation.rs` |
| 4.3 | Implement gpu_layers recommendation | If total estimated VRAM <= available GPU VRAM: `gpu_layers: -1` (full offload). Otherwise: compute partial offload. If no GPU: `gpu_layers: 0`. | `crates/llamaforge-server/src/recommendation.rs` |
| 4.4 | Implement ctx_size recommendation | Default 8192. Scale down if VRAM is tight (4096 minimum). Do **not** scale above 8192 — design doc Section 11.5's canonical example keeps ctx 8192 for an 8B-on-24GB box ("room for a second model"), and design > PLAN. | `crates/llamaforge-server/src/recommendation.rs` |
| 4.5 | Implement parallel_slots recommendation | Default 2. Decrease to 1 if VRAM is tight. Do **not** auto-increase above 2 — design doc Section 11.5's canonical example uses 2 slots for an 8B-on-24GB box. | `crates/llamaforge-server/src/recommendation.rs` |
| 4.6 | Implement cache type recommendation | Default `q8_0` for both k and v; `flash_attn: "auto"`. Do **not** switch to `f16` when VRAM is ample — design doc Section 11.5's canonical example uses `q8_0`, matching the merged engine. | `crates/llamaforge-server/src/recommendation.rs` |
| 4.7 | Implement port auto-assignment | Start at 8080. Increment for each additional server. Avoid conflicts with known ports. | `crates/llamaforge-server/src/recommendation.rs` |
| 4.8 | Implement rationale generation | For each recommended field, generate a human-readable rationale string explaining the reasoning. | `crates/llamaforge-server/src/recommendation.rs` |
| 4.9 | Implement multi-model VRAM warnings | If total estimated VRAM across all models exceeds available VRAM, emit a warning. | `crates/llamaforge-server/src/recommendation.rs` |
| 4.10 | Implement `recommend()` public API | Takes `HardwareInfo` + `Vec<ModelInput>`, returns `RecommendationResult`. | `crates/llamaforge-server/src/recommendation.rs` |
| 4.11 | Export module from lib.rs | Add `pub mod recommendation;` to `llamaforge-server/src/lib.rs`. | `crates/llamaforge-server/src/lib.rs` |
| 4.12 | Unit tests: single model, ample VRAM | 8B Q4_K_M model + 24GB GPU. Assert `gpu_layers: -1`, `ctx_size: 8192`, `parallel_slots: 2`. | `crates/llamaforge-server/tests/recommendation_tests.rs` |
| 4.13 | Unit tests: single model, tight VRAM | 70B Q4_K_M model + 24GB GPU. Assert partial offload, reduced ctx_size. | `crates/llamaforge-server/tests/recommendation_tests.rs` |
| 4.14 | Unit tests: no GPU (CPU-only) | Assert `gpu_layers: 0`, rationale mentions CPU-only mode. | `crates/llamaforge-server/tests/recommendation_tests.rs` |
| 4.15 | Unit tests: multi-model port assignment | 3 models. Assert ports 8080, 8081, 8082. | `crates/llamaforge-server/tests/recommendation_tests.rs` |
| 4.16 | Unit tests: multi-model VRAM warning | 2 models exceeding total VRAM. Assert warning in result. | `crates/llamaforge-server/tests/recommendation_tests.rs` |

### PR4 Acceptance Criteria

- [ ] `recommend()` produces valid `ConfigRecommendation` for each model
- [ ] `gpu_layers` correctly reflects VRAM availability (full offload, partial, or CPU-only)
- [ ] `ctx_size` scales with available VRAM
- [ ] Port auto-assignment starts at 8080 and increments
- [ ] Rationale strings are non-empty and explain the reasoning
- [ ] Multi-model VRAM overflow produces a warning
- [ ] All 5 test suites pass
- [ ] `cargo clippy` clean, `cargo test` passes

### PR4 Creation Steps

```bash
git checkout -b setup/04-recommendation main
# ... implement tasks 4.1–4.16 ...
git add -A && git commit -m "setup: add config recommendation engine with VRAM-aware gpu_layers, ctx_size, and rationale"
git push -u origin setup/04-recommendation
# Open PR: "Setup Wizard PR4: Config Recommendation Engine"
# Description: "Implements config recommendations (08_SETUP_WIZARD.md Section 11.5, Appendix: Config Recommendation).
# Takes hardware + models, computes gpu_layers/ctx_size/parallel_slots/cache types using existing
# vram_estimate.rs. Rationale strings for each recommendation. Port auto-assignment from 8080.
# Multi-model VRAM overflow warnings. 5 test suites with deterministic inputs."
```

---

## PR5: Setup API Endpoints

**Branch:** `setup/05-api-endpoints`  
**Merge Target:** `main`  
**Depends on:** PR1, PR2, PR3, PR4 all merged to `main`  
**Size:** Large (~600 lines)  
**Estimated Time:** 8–10 hours  

### Purpose

Mount all `/api/setup/*` endpoints on an axum router. This is the adapter layer that wires the domain modules (hardware, model_scanner, binary_scanner, recommendation) to HTTP. Includes the `setup_router()` shared function (Section 12.1), the `save-config` and `preview-config` endpoints (Section 11.6), session correlation via `X-Setup-Session-Id` (Section 11.7), and all backend observability (structured tracing spans per Section 11.7). Also includes the `validate-launch` endpoint for first-run mode validation.

Design doc references: Section 11 (all API contracts), Section 11.7 (Backend Observability), Section 12.1 (Shared Setup Router), Section 13.5 (Save-Config Flow with Pre-Validation).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 5.1 | Create setup module in llamaforge-api | Create `crates/llamaforge-api/src/setup/mod.rs` with handler module stubs. If `llamaforge-api` crate does not yet exist, create it with appropriate Cargo.toml using workspace dependencies. | `crates/llamaforge-api/src/setup/mod.rs` |
| 5.2 | Implement `hardware_handler` | `GET /api/setup/hardware`. Calls `detect_hardware()`. Optional `?diagnostics=true` query param. Tracing span: `setup_request`, `endpoint = "hardware"`, `setup_session_id`. | `crates/llamaforge-api/src/setup/hardware.rs` |
| 5.3 | Implement `scan_binary_handler` | `POST /api/setup/scan-binary`. Calls `scan_binary()`. CSRF header required. Tracing span with session ID. | `crates/llamaforge-api/src/setup/binary.rs` |
| 5.4 | Implement `scan_models_handler` | `POST /api/setup/scan-models`. Validates input (non-empty directories, max 10, no path traversal). Calls `scan_models()`. CSRF header required. Tracing span. | `crates/llamaforge-api/src/setup/models.rs` |
| 5.5 | Implement `recommend_handler` | `POST /api/setup/recommend`. Reads cached hardware from in-memory state. Calls `recommend()`. CSRF header required. Tracing span. | `crates/llamaforge-api/src/setup/recommend.rs` |
| 5.6 | Implement `save_config_handler` | `POST /api/setup/save-config`. Assembles `AppConfig` from wizard inputs. Pre-validation: round-trip TOML fidelity, path resolution, port availability. Write mutex (`tokio::sync::Mutex`). Backup before overwrite (max 5 backups). CSRF required. Tracing span. | `crates/llamaforge-api/src/setup/save_config.rs` |
| 5.7 | Implement `preview_config_handler` | `POST /api/setup/preview-config`. Same assembly as save-config but returns rendered TOML without writing. CSRF required. | `crates/llamaforge-api/src/setup/preview_config.rs` |
| 5.8 | Implement `validate_launch_handler` | `POST /api/setup/validate-launch`. Spawns the first configured server, waits for health check (30s timeout), kills validation server. Returns success/failure with stderr. | `crates/llamaforge-api/src/setup/validate_launch.rs` |
| 5.9 | Implement hardware cache | In-memory `Arc<RwLock<Option<HardwareInfo>>>`. Populated by hardware_handler. Read by recommend_handler. | `crates/llamaforge-api/src/setup/state.rs` |
| 5.10 | Implement session correlation middleware | Extract `X-Setup-Session-Id` header. Set `setup_session_id` field on the tracing span. Default to `"anonymous"` if missing. | `crates/llamaforge-api/src/setup/mod.rs` |
| 5.11 | Implement `setup_router()` | Compose all routes into a single `Router`. This is the shared function used by both standalone and first-run modes. | `crates/llamaforge-api/src/setup/mod.rs` |
| 5.12 | Implement input validation | `scan-models` with empty directories -> 400. `scan-binary` with null bytes -> 400. `save-config` with unwritable path -> 403. `recommend` with empty models -> 400. `build` with invalid backend -> 400. | `crates/llamaforge-api/src/setup/` (various handlers) |
| 5.13 | Implement version field | All responses include `"version": 1` at the top level. | `crates/llamaforge-api/src/setup/` (response types) |
| 5.14 | Integration tests: hardware endpoint | Mock shell commands (set PATH to fixture directory). Hit `GET /api/setup/hardware`. Assert response shape. | `crates/llamaforge-api/tests/setup_api_tests.rs` |
| 5.15 | Integration tests: scan-models endpoint | Create tempdir with .gguf files. Hit `POST /api/setup/scan-models`. Assert response contains discovered models. | `crates/llamaforge-api/tests/setup_api_tests.rs` |
| 5.16 | Integration tests: scan-binary endpoint | Create mock binary script. Hit `POST /api/setup/scan-binary` with extra_paths. Assert found. | `crates/llamaforge-api/tests/setup_api_tests.rs` |
| 5.17 | Integration tests: recommend endpoint | Pre-populate hardware cache. Hit `POST /api/setup/recommend` with model list. Assert recommendations. | `crates/llamaforge-api/tests/setup_api_tests.rs` |
| 5.18 | Integration tests: input validation | Hit each endpoint with invalid input. Assert 400/403 with structured error. | `crates/llamaforge-api/tests/setup_api_tests.rs` |
| 5.19 | Integration tests: save-config + preview-config | Hit preview-config, verify TOML returned. Hit save-config, verify file written. Hit save-config again without force, verify 409. | `crates/llamaforge-api/tests/setup_api_tests.rs` |
| 5.20 | Integration tests: session correlation | Send requests with `X-Setup-Session-Id` header. Verify session ID appears in tracing output. | `crates/llamaforge-api/tests/setup_api_tests.rs` |

### PR5 Acceptance Criteria

- [ ] All 8 `/api/setup/*` endpoints mounted and responding
- [ ] `setup_router()` function is reusable for standalone and first-run modes
- [ ] CSRF header required on all POST endpoints
- [ ] `X-Setup-Session-Id` extracted and attached to tracing spans
- [ ] All responses include `"version": 1`
- [ ] Save-config: pre-validation (round-trip, path resolution, port check), write mutex, backup (max 5)
- [ ] Preview-config: same assembly, no write
- [ ] Input validation: empty directories 400, null bytes 400, unwritable path 403, empty models 400
- [ ] Hardware cache shared between hardware and recommend endpoints
- [ ] All 7 integration test suites pass
- [ ] `cargo clippy` clean, `cargo test` passes

### PR5 Creation Steps

```bash
git checkout -b setup/05-api-endpoints main
# ... implement tasks 5.1–5.20 ...
git add -A && git commit -m "setup: mount all /api/setup/* endpoints with session correlation and observability"
git push -u origin setup/05-api-endpoints
# Open PR: "Setup Wizard PR5: Setup API Endpoints"
# Description: "Implements all setup API endpoints (08_SETUP_WIZARD.md Section 11, 12.1).
# setup_router() shared between standalone and first-run modes. hardware, scan-binary, scan-models,
# recommend, save-config (with pre-validation, write mutex, backup), preview-config, validate-launch.
# Session correlation via X-Setup-Session-Id. Structured tracing spans per Section 11.7.
# Input validation with 400/403 error responses. 7 integration test suites."
```

---

## VC-1: Backend Foundation Verification Checkpoint

**Gate:** PR1–PR5 merged to `main`. All scan/detect/recommend endpoints return correct data.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-1.1 | Hardware detection | `curl http://127.0.0.1:8000/api/setup/hardware` | Returns GPU/CPU/RAM for the test machine |
| VC-1.2 | Binary scan | `curl -X POST -H "X-LlamaForge-Request: 1" http://127.0.0.1:8000/api/setup/scan-binary` | Finds `llama-server` if installed, returns searched_paths if not |
| VC-1.3 | Model scan | `curl -X POST -H "X-LlamaForge-Request: 1" -d '{"source":"local","directories":["./models"]}' http://127.0.0.1:8000/api/setup/scan-models` | Returns discovered .gguf files |
| VC-1.4 | Recommendation | `curl -X POST -H "X-LlamaForge-Request: 1" -d '{"models":[...]}' http://127.0.0.1:8000/api/setup/recommend` | Returns gpu_layers, ctx_size, rationale per model |
| VC-1.5 | Preview config | `curl -X POST -H "X-LlamaForge-Request: 1" -d '...' http://127.0.0.1:8000/api/setup/preview-config` | Returns valid TOML string |
| VC-1.6 | Save config | Write config to temp path, verify file contents match preview | File written, valid TOML |
| VC-1.7 | Input validation | Send invalid inputs to each endpoint | All return structured 400/403 errors |
| VC-1.8 | Session correlation | `grep setup_session_id` in log output | Session ID present on all requests |

---

## Phase 2: Build System + CLI

---

## PR6: Build Orchestrator

**Branch:** `setup/06-build-orchestrator`  
**Merge Target:** `main`  
**Depends on:** PR5 merged to `main`  
**Size:** Large (~700 lines)  
**Estimated Time:** 10–12 hours  

### Purpose

Implement the build orchestrator that clones llama.cpp, runs cmake + make as child processes in a process group, and streams build output via SSE. Uses the job pattern: `POST /api/setup/build` creates a build, `GET /api/setup/build/{id}` polls status, `GET /api/setup/build/{id}/stream` connects to the SSE stream, `DELETE /api/setup/build/{id}` cancels. Includes per-phase timeouts (clone 5m, configure 5m, build 20m), process group management (`setsid`/`setpgid`), temp directory cleanup, binary backup-before-overwrite, disk space check, build log persistence, and SSE reconnection with terminal event replay.

Design doc references: Section 11.4 (Build API contract), Section 11.7 (Backend Observability — build process logging, Build Log Persistence), Appendix: Implementation Notes (Build Process).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 6.1 | Define build types | `BuildRequest`, `BuildStatus` (enum: idle/running/complete/failed/cancelled), `BuildPhase` (cloning/configuring/building), SSE event types (`phase`, `progress`, `log`, `complete`, `failed`). | `crates/llamaforge-server/src/build.rs` |
| 6.2 | Implement process group management | Spawn child processes in their own process group via `setsid` (Linux) or `setpgid` (macOS). Implement `kill_process_group()`: SIGTERM -> 5s grace -> SIGKILL. | `crates/llamaforge-server/src/build.rs` |
| 6.3 | Implement git clone phase | `git clone --depth=1 https://github.com/ggerganov/llama.cpp.git` into temp directory. Parse progress from stdout. 5-minute timeout. | `crates/llamaforge-server/src/build.rs` |
| 6.4 | Implement cmake configure phase | `cmake -B build -DGGML_CUDA=ON` (or Metal/Vulkan/CPU). Parse output. 5-minute timeout. | `crates/llamaforge-server/src/build.rs` |
| 6.5 | Implement cmake build phase | `cmake --build build -j <jobs> --target llama-server`. Parse `[N/M]` progress from stdout. 20-minute timeout. Jobs capped at `min(requested, num_cpus / 2)`. Nice value 10. | `crates/llamaforge-server/src/build.rs` |
| 6.6 | Implement build log persistence | Tee all stdout/stderr to `$XDG_DATA_HOME/llamaforge/build-logs/build-<ISO-timestamp>.log`. Retention: last 5 builds or 30 days. Cleanup on build start. | `crates/llamaforge-server/src/build.rs` |
| 6.7 | Implement SSE stream | `GET /api/setup/build/{id}/stream` returns `text/event-stream`. Events: `phase`, `progress`, `log`, `complete`, `failed`. Log throttled to max 10 events/s via batching. | `crates/llamaforge-api/src/setup/build_stream.rs` |
| 6.8 | Implement terminal event replay | If build finished while client was disconnected, replay `complete` or `failed` event on reconnect. | `crates/llamaforge-api/src/setup/build_stream.rs` |
| 6.9 | Implement build job state | In-memory `HashMap<Uuid, BuildJob>` behind `Arc<RwLock<>>`. Tracks status, log lines, SSE sender. | `crates/llamaforge-api/src/setup/build_state.rs` |
| 6.10 | Implement `POST /api/setup/build` handler | Creates build job. Idempotent: if a build is already running for the same backend, return existing build_id. Disk space check (5 GB minimum). CSRF required. | `crates/llamaforge-api/src/setup/build_handler.rs` |
| 6.11 | Implement `GET /api/setup/build/{id}` handler | Returns current status, phase, percent, last_lines. | `crates/llamaforge-api/src/setup/build_handler.rs` |
| 6.12 | Implement `DELETE /api/setup/build/{id}` handler | Cancels the build. Kills process group. Cleans up temp directory. CSRF required. | `crates/llamaforge-api/src/setup/build_handler.rs` |
| 6.13 | Implement binary backup-before-overwrite | Before copying built binary to `install_dir`, rename existing binary to `llama-server.backup.<ISO-timestamp>`. Include `replaced_existing` and `backup_path` in `complete` SSE event. | `crates/llamaforge-server/src/build.rs` |
| 6.14 | Implement temp directory cleanup | On failure: clean up unless retry pending (2-minute grace). On skip: clean up immediately. On success: clean up after binary copy. On SSE disconnect: 2-minute grace, then kill + cleanup. | `crates/llamaforge-server/src/build.rs` |
| 6.15 | Mount build endpoints on setup_router | Add build routes to the `setup_router()` function from PR5. | `crates/llamaforge-api/src/setup/mod.rs` |
| 6.16 | Add SSE dependencies to workspace | Add `tokio-stream` and `axum-extra` (with `sse` feature) to `[workspace.dependencies]` if not already present. | `Cargo.toml`, `crates/llamaforge-api/Cargo.toml` |
| 6.17 | Integration tests: build with mock git/cmake | Create shell scripts that mimic git clone + cmake progress output. Assert SSE events arrive in correct order. | `crates/llamaforge-api/tests/build_tests.rs` |
| 6.18 | Integration tests: build cancellation | Start build, DELETE it mid-clone. Assert process group killed, temp dir cleaned. | `crates/llamaforge-api/tests/build_tests.rs` |
| 6.19 | Integration tests: build timeout | Create a mock cmake that hangs. Assert `failed` event with timeout message after per-phase timeout. | `crates/llamaforge-api/tests/build_tests.rs` |
| 6.20 | Integration tests: idempotent POST | POST build twice with same backend. Assert same build_id returned. | `crates/llamaforge-api/tests/build_tests.rs` |
| 6.21 | Integration tests: disk space check | Mock disk check to return < 5 GB. Assert 409 with `insufficient_disk`. | `crates/llamaforge-api/tests/build_tests.rs` |
| 6.22 | Integration tests: SSE reconnect with terminal replay | Start build, complete it, then connect to stream. Assert `complete` event replayed. | `crates/llamaforge-api/tests/build_tests.rs` |

### PR6 Acceptance Criteria

- [ ] `POST /api/setup/build` creates a build job with UUID
- [ ] `GET /api/setup/build/{id}/stream` returns SSE stream with `phase`, `progress`, `log`, `complete`/`failed` events
- [ ] Log events throttled to max 10/s via batching
- [ ] Per-phase timeouts: clone 5m, configure 5m, build 20m
- [ ] Jobs capped at `min(requested, num_cpus / 2)`
- [ ] Process group management: SIGTERM -> 5s -> SIGKILL on cancel/timeout
- [ ] Binary backup-before-overwrite with `replaced_existing` + `backup_path`
- [ ] Temp directory cleaned up on all exit paths (success, failure, cancel, disconnect)
- [ ] Build log persisted to `$XDG_DATA_HOME/llamaforge/build-logs/`
- [ ] Terminal event replay on SSE reconnect
- [ ] Disk space check (5 GB minimum) returns 409 on insufficient space
- [ ] Idempotent POST returns existing build_id
- [ ] All 6 integration test suites pass
- [ ] `cargo clippy` clean, `cargo test` passes

### PR6 Creation Steps

```bash
git checkout -b setup/06-build-orchestrator main
# ... implement tasks 6.1–6.22 ...
git add -A && git commit -m "setup: add build orchestrator with SSE streaming, process groups, and per-phase timeouts"
git push -u origin setup/06-build-orchestrator
# Open PR: "Setup Wizard PR6: Build Orchestrator"
# Description: "Implements the build system (08_SETUP_WIZARD.md Section 11.4).
# Job pattern: POST create + GET status + GET stream + DELETE cancel.
# git clone + cmake as child processes in process groups. SSE streaming with
# phase/progress/log/complete/failed events. Per-phase timeouts (5m/5m/20m).
# Log throttling (10/s). Build log persistence. Binary backup. Temp cleanup.
# Terminal event replay. Disk space check. 6 integration test suites."
```

---

## PR7: CLI Changes + First-Run Mode

**Branch:** `setup/07-cli-first-run`  
**Merge Target:** `main`  
**Depends on:** PR6 merged to `main`  
**Size:** Medium (~500 lines)  
**Estimated Time:** 7–9 hours  

### Purpose

Add `--interactive` flag and `--timeout` option to `llamaforge init`. Implement the standalone wizard server lifecycle (Section 12): minimal axum server on ephemeral port, idle timeout, browser auto-open, periodic URL reminder, clean shutdown. Implement first-run detection in `llamaforge serve` (Section 13): `ConfigLoadError` triage, `Mode` state machine (`Setup -> Transitioning -> Running`), `SetupModeGuard` middleware for 503 on inference endpoints, `GET /api/status` with `setup_required` field, shared `setup_router()` integration.

Design doc references: Section 12 (Standalone Wizard Mode), Section 13 (First-Run Detection and Mode State Machine).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 7.1 | Add `--interactive` flag to Init subcommand | Add `interactive: bool` and `timeout: humantime::Duration` (default 30m) to the `Init` enum variant. When `interactive` is false, preserve existing template behavior. | `crates/llamaforge/src/main.rs` |
| 7.2 | Implement standalone wizard server | Bind `127.0.0.1:0` (ephemeral port). Mount `ui_router().merge(setup_router())`. No gRPC, no inference, no config handle. Print URL to stdout. Attempt browser open (`open`/`xdg-open`). | `crates/llamaforge/src/init.rs` |
| 7.3 | Implement idle timeout | `Arc<AtomicU64>` last-activity timestamp. Middleware on setup router resets on every `/api/setup/*` call. Tokio interval (60s check). Default 30m. Print reminder every 5m. Exit 0 on expiry. | `crates/llamaforge/src/init.rs` |
| 7.4 | Implement standalone logging posture | Default `WARN` to stderr via `EnvFilter`. Single INFO startup message always emitted regardless of `RUST_LOG`. Hint for verbose: `RUST_LOG=debug`. | `crates/llamaforge/src/init.rs` |
| 7.5 | Implement save-complete shutdown | After `save-config` + `validate-launch` succeed in standalone mode, shut down the server and exit 0. | `crates/llamaforge/src/init.rs` |
| 7.6 | Define `Mode` enum | `Mode { Setup, Transitioning, Running }`. Wrapped in `Arc<ArcSwap<Mode>>`. | `crates/llamaforge/src/serve.rs` |
| 7.7 | Implement `ConfigLoadError` triage | On `llamaforge serve`: `NotFound` -> setup mode. `EmptyFile` -> setup mode. `ParseError` -> `process::exit(78)` with error message. Normal load -> running mode. | `crates/llamaforge/src/serve.rs` |
| 7.8 | Implement `SetupModeGuard` middleware | Tower middleware. When mode is `Setup` or `Transitioning`: inference endpoints return 503 (HTTP: JSON with `setup_url`; gRPC: `UNAVAILABLE` with `setup-url` metadata). `Retry-After: 60` header. | `crates/llamaforge/src/serve.rs` (or `crates/llamaforge-api/src/setup/guard.rs`) |
| 7.9 | Implement setup mode boot sequence | Create empty `AppConfig`. Boot full transport (HTTP + gRPC). Compose `full_app_router().merge(setup_router())`. Set mode `Setup`. | `crates/llamaforge/src/serve.rs` |
| 7.10 | Implement mode transitions | `Setup -> Transitioning` on save + validate success. `Transitioning -> Running` on hot-reload success. `Transitioning -> Setup` on hot-reload failure. | `crates/llamaforge/src/serve.rs` |
| 7.11 | Implement `setup_required` on /api/status | Add `setup_required: bool` to the existing status endpoint response. True when mode is `Setup` or `Transitioning`. | `crates/llamaforge-api/src/setup/` or existing status handler |
| 7.12 | Implement `X-Setup-Reconfigure` guard | In `Running` mode, setup endpoints require `X-Setup-Reconfigure: true` header. Without it, return 403. In `Setup`/`Transitioning`, no header required. | `crates/llamaforge-api/src/setup/mod.rs` |
| 7.13 | Add `humantime` to workspace dependencies | Add `humantime = "2"` to `[workspace.dependencies]` in root `Cargo.toml`. Add `humantime = { workspace = true }` to `llamaforge/Cargo.toml`. | `Cargo.toml`, `crates/llamaforge/Cargo.toml` |
| 7.14 | Integration tests: standalone wizard lifecycle | Start wizard server. Hit `/api/setup/hardware`. Verify response. Ctrl+C. Verify exit 130. | `crates/llamaforge/tests/wizard_lifecycle.rs` |
| 7.15 | Integration tests: idle timeout | Start wizard with `--timeout=3s`. Make no API calls. Verify exit 0 after ~3 seconds. | `crates/llamaforge/tests/wizard_lifecycle.rs` |
| 7.16 | Integration tests: first-run mode | Start `llamaforge serve` with no config file. Verify setup mode: status shows `setup_required: true`, inference endpoints return 503. | `crates/llamaforge/tests/wizard_lifecycle.rs` |
| 7.17 | Integration tests: mode transition | In setup mode: save config + validate launch. Verify mode transitions to Running. Verify `setup_required: false`. | `crates/llamaforge/tests/wizard_lifecycle.rs` |
| 7.18 | Integration tests: 503 during setup | In setup mode: hit `/v1/chat/completions`. Verify 503 with `setup_url`. | `crates/llamaforge/tests/wizard_lifecycle.rs` |
| 7.19 | Integration tests: reconfigure guard | In running mode: hit `/api/setup/hardware` without reconfigure header. Verify 403. With header: verify 200. | `crates/llamaforge/tests/wizard_lifecycle.rs` |

### PR7 Acceptance Criteria

- [ ] `llamaforge init --interactive` starts wizard server on ephemeral port
- [ ] URL printed to stdout, browser auto-open attempted
- [ ] Idle timeout works: default 30m, configurable via `--timeout`
- [ ] Periodic URL reminder every 5 minutes
- [ ] Standalone server exits 0 after successful save + validate
- [ ] `llamaforge serve` with no config enters setup mode
- [ ] `ConfigLoadError::ParseError` causes `exit(78)` (not setup mode)
- [ ] Mode state machine: `Setup -> Transitioning -> Running` (or `-> Setup` on failure)
- [ ] `SetupModeGuard` returns 503 with `setup_url` on inference endpoints during setup
- [ ] `GET /api/status` includes `setup_required` boolean
- [ ] `X-Setup-Reconfigure` guard on setup endpoints in Running mode
- [ ] All 6 integration test suites pass
- [ ] `cargo clippy` clean, `cargo test` passes

### PR7 Creation Steps

```bash
git checkout -b setup/07-cli-first-run main
# ... implement tasks 7.1–7.19 ...
git add -A && git commit -m "setup: add llamaforge init --interactive, first-run mode, and Mode state machine"
git push -u origin setup/07-cli-first-run
# Open PR: "Setup Wizard PR7: CLI Changes + First-Run Mode"
# Description: "Implements standalone wizard (08_SETUP_WIZARD.md Section 12) and first-run mode
# (Section 13). --interactive flag on init. Standalone server: ephemeral port, idle timeout,
# browser auto-open, periodic URL reminder. First-run: ConfigLoadError triage, Mode state machine
# (Setup/Transitioning/Running), SetupModeGuard (503 on inference), setup_required on /api/status,
# X-Setup-Reconfigure guard. 6 integration test suites."
```

---

## VC-2: Build System + CLI Verification Checkpoint

**Gate:** PR6–PR7 merged to `main`. Standalone wizard and build SSE work end-to-end.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-2.1 | Standalone wizard starts | `llamaforge init --interactive` | URL printed, browser opens, API responds |
| VC-2.2 | Idle timeout | `llamaforge init --interactive --timeout=30s`, wait | Server exits after ~30s |
| VC-2.3 | Build SSE works | Start build via API, connect to SSE stream | Phase/progress/log events arrive |
| VC-2.4 | Build cancellation | Start build, DELETE it | Process killed, temp dir cleaned |
| VC-2.5 | First-run detection | `llamaforge serve` with no config file | Server enters setup mode, status shows `setup_required: true` |
| VC-2.6 | 503 during setup | Hit `/v1/chat/completions` in setup mode | Returns 503 with `setup_url` |
| VC-2.7 | Mode transition | Save config + validate in setup mode | Mode transitions to Running, `setup_required: false` |
| VC-2.8 | Ctrl+C cleanup | Kill wizard during build | Build process group killed, no orphans |

---

## Phase 3: Frontend Wizard

> **Pre-Phase 3 setup:** Rebase `frontend` branch onto `main` to pick up all Phase 1 + Phase 2 backend changes:
> ```bash
> git checkout frontend && git pull
> git rebase main
> git push --force-with-lease origin frontend
> ```

---

## PR8: Wizard Scaffold + Step Navigation

**Branch:** `ui/setup-08-scaffold`  
**Merge Target:** `frontend`  
**Depends on:** Phase 1 + Phase 2 merged to `main`, `frontend` rebased onto `main`  
**Size:** Medium (~400 lines)  
**Estimated Time:** 6–8 hours  

### Purpose

Create the wizard route structure under `ui/src/routes/setup/`, the `SetupLayout` orchestrator with `StepIndicator` and `StepNavigation`, the `SetupWizardStore` (lean: `selectedModelIds`, `userOverrides`, `savePath`), the step state machine with `STEP_TRANSITIONS`, and session ID generation. This PR creates the skeleton — step content is placeholder. The WelcomeStep is fully implemented as it is static content.

Design doc references: Section 3 (Rendering Strategy), Section 4 (Component Architecture), Section 5 (State Management), Appendix: New Route Structure, Appendix: New TypeScript Types.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 8.1 | Create route structure | `setup/+layout.svelte`, `setup/+layout.ts` (ssr=false), `setup/+page.svelte` (Welcome), `setup/hardware/+page.svelte`, `setup/binary/+page.svelte`, `setup/models/+page.svelte`, `setup/configure/+page.svelte`, `setup/review/+page.svelte`. Each step is a placeholder except Welcome. | `ui/src/routes/setup/**` |
| 8.2 | Implement `SetupLayout` | Orchestrator: step indicator + content area + navigation footer. Provides `SetupWizardStore` via Svelte context (`setContext`/`getContext`). Generates UUID v4 session ID. `beforeunload` guard when step >= configure. | `ui/src/routes/setup/+layout.svelte` |
| 8.3 | Implement `StepIndicator.svelte` | Presentational. Props: `steps: { label, status }[]`. `<ol>` with `aria-current="step"`. Arrow key navigation between completed steps. Below 640px: numbered circles only. | `ui/src/lib/components/setup/StepIndicator.svelte` |
| 8.4 | Implement `StepNavigation.svelte` | Presentational. Props: `canGoBack`, `canAdvance`, `canSkip`, `nextLabel`, `isLoading`. Events: `onback`, `onnext`, `onskip`. | `ui/src/lib/components/setup/StepNavigation.svelte` |
| 8.5 | Implement `SetupWizardStore` | Svelte 5 rune class in `.svelte.ts`. Fields: `selectedModelIds: Set<string>`, `userOverrides: Map<string, Partial<ServerConfigDraft>>`, `savePath: string`. Instantiated in `SetupLayout`, provided via context. | `ui/src/lib/stores/setup.svelte.ts` |
| 8.6 | Implement step state machine | `STEP_TRANSITIONS` record: each step has `canAdvance(qc, store)` guard and `invalidatesOnReentry: WizardStep[]`. `completedSteps` derived from iterating the transition table. | `ui/src/lib/stores/setup.svelte.ts` |
| 8.7 | Implement downstream invalidation | On re-entry to a completed step: remove TanStack Query data for invalidated steps, clear relevant client state. | `ui/src/routes/setup/+layout.svelte` |
| 8.8 | Implement session ID generation + header | Generate UUID v4 on `SetupLayout` init. Attach as `X-Setup-Session-Id` header on all wizard API calls (extend `apiFetch` or create a wrapper). | `ui/src/lib/stores/setup.svelte.ts`, `ui/src/lib/api/client.ts` |
| 8.9 | Implement WelcomeStep | Static content: welcome message, "Start Setup" button. "Start" navigates to `/ui/setup/hardware`. | `ui/src/routes/setup/+page.svelte` |
| 8.10 | Implement step focus management | On step transition, focus moves to the new step's `<h1>`. Uses `afterNavigate` hook. | `ui/src/routes/setup/+layout.svelte` |
| 8.11 | Add setup TypeScript types | All types from Appendix: New TypeScript Types — `HardwareInfo`, `BinaryScanResult`, `DiscoveredModel`, `ModelScanResult`, `BuildStatus`, `ConfigRecommendation`, `ServerConfigDraft`, `WizardStep`, etc. | `ui/src/lib/api/types.ts` |
| 8.12 | Add setup TanStack Query definitions | `createHardwareQuery()`, `createBinaryScanMutation()`, `createModelScanMutation()`, `createRecommendationMutation()`, `createPreviewConfigMutation()`, `createSaveConfigMutation()`. Each with correct query key and `staleTime: Infinity`. | `ui/src/lib/api/queries.ts` |
| 8.13 | Unit tests: StepIndicator | Render with 5 steps, 2 complete, 1 current. Assert `aria-current="step"` on current. Assert completed steps are clickable. | tests |
| 8.14 | Unit tests: StepNavigation | Render with `canAdvance: false`. Assert Next button disabled. Render with `canSkip: true`. Assert Skip button present. | tests |
| 8.15 | Unit tests: step state machine | Assert `canAdvance` returns correct boolean for each step. Assert `invalidatesOnReentry` lists correct downstream steps. | tests |
| 8.16 | Unit tests: session ID | Assert UUID v4 generated on store init. Assert it is attached to API requests. | tests |

### PR8 Acceptance Criteria

- [ ] All 6 wizard routes render with placeholder content and correct `<h1>`
- [ ] `StepIndicator` shows step progress with `aria-current="step"`
- [ ] `StepNavigation` Next button disabled when `canAdvance` is false
- [ ] `SetupWizardStore` provides `selectedModelIds`, `userOverrides`, `savePath` via context
- [ ] `STEP_TRANSITIONS` enforces advancement guards
- [ ] Downstream invalidation works on re-entry
- [ ] Session ID generated and attached to `X-Setup-Session-Id` header
- [ ] `beforeunload` fires when step >= configure
- [ ] Focus moves to `<h1>` on step transition
- [ ] All setup TypeScript types defined
- [ ] All 4 unit test suites pass
- [ ] `npm run check && npm run lint` clean

### PR8 Creation Steps

```bash
git checkout -b ui/setup-08-scaffold frontend
# ... implement tasks 8.1–8.16 ...
git add -A && git commit -m "ui: add wizard scaffold with StepIndicator, step state machine, and SetupWizardStore"
git push -u origin ui/setup-08-scaffold
# Open PR: "Setup Wizard PR8: Wizard Scaffold + Step Navigation"
# Description: "Creates the wizard route structure (08_SETUP_WIZARD.md Sections 3-5, Appendices).
# SetupLayout with StepIndicator + StepNavigation. SetupWizardStore (lean: selectedModelIds,
# userOverrides, savePath). STEP_TRANSITIONS state machine with advancement guards and downstream
# invalidation. Session ID generation. WelcomeStep. Setup TypeScript types and TanStack Query
# definitions. 4 unit test suites."
```

---

## PR9: Hardware Step + Binary Step

**Branch:** `ui/setup-09-hardware-binary`  
**Merge Target:** `frontend`  
**Depends on:** PR8 merged to `frontend`  
**Size:** Large (~550 lines)  
**Estimated Time:** 8–10 hours  

### Purpose

Implement the HardwareStep and BinaryStep pages. HardwareStep is an orchestrator that fires the hardware detection query on mount, shows hardware cards, and provides a manual entry fallback on failure. BinaryStep is an orchestrator with inline `{#if}` blocks for found/not-found/manual states — it includes `BuildConfigurator` (backend selector) and `BuildProgress` (streaming log via SSE `EventSource` connected to the build job stream endpoint) with cancel support.

Design doc references: Section 2.1 (User Flows 1 and 2), Section 2.2 (Hardware Detection and Binary Locator screen states), Section 4 (Component Architecture — HardwareStep, BinaryStep, BuildProgress), Section 6.2 (Error Handling Strategy).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 9.1 | Implement `HardwareStep` (orchestrator) | `TanStack Query` for `GET /api/setup/hardware`. Shows spinner while loading. 10-second frontend "Cancel and enter manually" button. Displays results in `HardwareCard` components. On error: `ErrorBanner` with "Retry" + "Enter manually" link. | `ui/src/routes/setup/hardware/+page.svelte` |
| 9.2 | Implement `HardwareCard.svelte` (presentational) | Props: `type`, `label`, `details`, `status`, `icon`. Uses `StatusBadge` for detection status. | `ui/src/lib/components/setup/HardwareCard.svelte` |
| 9.3 | Implement `HardwareManualEntry.svelte` (presentational) | Props: `onsubmit`. Form for manual GPU name, VRAM, CPU cores, RAM. Inline validation. | `ui/src/lib/components/setup/HardwareManualEntry.svelte` |
| 9.4 | Implement `BinaryStep` (orchestrator) | Fires binary scan mutation on mount. Inline `{#if}` states: scanning, found (path + version), not-found (build option + manual path input). Manages build lifecycle. | `ui/src/routes/setup/binary/+page.svelte` |
| 9.5 | Implement `BuildConfigurator.svelte` (presentational) | Props: `detectedBackend`, `onstart`. Backend selector dropdown (`cuda`/`metal`/`vulkan`/`cpu`). Install directory input. Jobs input. "Build" button. First-run confirmation dialog per design doc. | `ui/src/lib/components/setup/BuildConfigurator.svelte` |
| 9.6 | Implement `BuildProgress.svelte` (presentational) | Props: `status`, `logLines`, `progress`, `errorSummary`. Events: `onretry`, `onskip`, `onchangebackend`, `oncancel`. `LogViewer` component with auto-scroll, `role="log"`, `aria-live="polite"`. Progress display: cmake `[N/M]` + elapsed time. Error state: last lines + retry/skip/change-backend buttons. | `ui/src/lib/components/setup/BuildProgress.svelte` |
| 9.7 | Implement `LogViewer.svelte` (shared) | Scrollable `<pre>` (400px fixed height). Auto-tail on new lines. Focusable (`tabindex="0"`). Arrow key scrolling. Reusable for future server log viewing. | `ui/src/lib/components/shared/LogViewer.svelte` |
| 9.8 | Implement `ProgressBar.svelte` (shared) | Determinate/indeterminate modes. `role="progressbar"` with `aria-valuenow/min/max`. Percentage label. `prefers-reduced-motion` disables animation. | `ui/src/lib/components/shared/ProgressBar.svelte` |
| 9.9 | Implement SSE build stream connection | `EventSource` to `/api/setup/build/{id}/stream`. Parse `phase`, `progress`, `log`, `complete`, `failed` events. Update build state. Reconnect with exponential backoff + jitter (base 1s, max 8s, 3 attempts). Cleanup in `$effect` return. | `ui/src/routes/setup/binary/+page.svelte` |
| 9.10 | Implement build cancel | Click "Cancel" calls `DELETE /api/setup/build/{id}`. Close EventSource. Transition to not-found state. | `ui/src/routes/setup/binary/+page.svelte` |
| 9.11 | Unit tests: HardwareCard | Render with `status: 'detected'` -> green StatusBadge. `status: 'not_detected'` -> red. `status: 'error'` -> yellow. | tests |
| 9.12 | Unit tests: HardwareStep states | Mock successful hardware response -> assert cards rendered. Mock 500 error -> assert ErrorBanner with retry. Mock timeout -> assert "Cancel and enter manually" button. | tests |
| 9.13 | Unit tests: BinaryStep found/not-found | Mock `found: true` -> assert path + version displayed. Mock `found: false` -> assert build option shown. | tests |
| 9.14 | Unit tests: BuildProgress states | Render with `status: 'cloning'` -> assert progress. `status: 'failed'` -> assert error summary + retry button. `status: 'complete'` -> assert success. | tests |
| 9.15 | Unit tests: LogViewer | Push 100 lines -> assert auto-scroll to bottom. Assert `role="log"`. Assert focusable. | tests |
| 9.16 | Unit tests: SSE event parsing | Mock each SSE event type individually. Assert correct UI state update for each. | tests |

### PR9 Acceptance Criteria

- [ ] HardwareStep shows detection results in HardwareCards
- [ ] Manual entry fallback on detection failure
- [ ] 10-second "Cancel and enter manually" button appears during long detection
- [ ] BinaryStep shows found/not-found/manual states correctly
- [ ] Build flow works: configure -> progress -> complete (or error)
- [ ] SSE stream connects and parses all event types
- [ ] SSE reconnection with exponential backoff + jitter (3 attempts)
- [ ] Build cancel kills process and transitions to not-found
- [ ] `LogViewer` auto-scrolls, is focusable, has `role="log"`
- [ ] `ProgressBar` has `role="progressbar"` with correct ARIA attributes
- [ ] All 6 unit test suites pass
- [ ] `npm run check && npm run lint` clean

### PR9 Creation Steps

```bash
git checkout -b ui/setup-09-hardware-binary frontend
# ... implement tasks 9.1–9.16 ...
git add -A && git commit -m "ui: add HardwareStep and BinaryStep with build SSE streaming and LogViewer"
git push -u origin ui/setup-09-hardware-binary
# Open PR: "Setup Wizard PR9: Hardware Step + Binary Step"
# Description: "Implements Hardware and Binary wizard steps (08_SETUP_WIZARD.md Sections 2.1, 2.2, 4, 6.2).
# HardwareStep with TanStack Query, HardwareCards, manual entry fallback. BinaryStep with inline
# {#if} states, BuildConfigurator, BuildProgress with SSE via EventSource. LogViewer (shared,
# reusable) with auto-scroll and role=log. ProgressBar (shared). SSE reconnect with jitter.
# Build cancel. 6 unit test suites."
```

---

## PR10: Model Step + Config Step

**Branch:** `ui/setup-10-models-config`  
**Merge Target:** `frontend`  
**Depends on:** PR9 merged to `frontend`  
**Size:** Medium (~450 lines)  
**Estimated Time:** 7–9 hours  

### Purpose

Implement the ModelStep (directory selector + scan trigger + ModelTable with checkboxes) and ConfigStep (ServerConfigForm with recommendation badges and form validation). ModelStep fires the scan-models mutation and populates `selectedModelIds` in the store. ConfigStep fires the recommendation query and renders a `ServerConfigForm` per selected model, with inline "Recommended: X" hints. Includes the `beforeunload` unsaved changes guard.

Design doc references: Section 2.1 (Flows 1 and 2 — Steps 3 and 4), Section 2.2 (Model Scanner and Server Config screen states), Section 4.2 (ModelTable and ServerConfigForm contracts), Section 5.6 (Form Design with concrete validation bounds).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 10.1 | Implement `ModelStep` (orchestrator) | Directory selector with sensible defaults (`~/models`, `./models`). "Scan" button fires `scan-models` mutation. Shows `ScanProgress` during scan. Results in `ModelTable`. Empty state: "No .gguf files found." Warnings displayed. | `ui/src/routes/setup/models/+page.svelte` |
| 10.2 | Implement `DirectorySelector.svelte` (presentational) | Props: `directories: string[]`. Events: `onadd`, `onremove`, `onscan`. Text inputs with add/remove. Pre-filled with defaults. | `ui/src/lib/components/setup/DirectorySelector.svelte` |
| 10.3 | Implement `ScanProgress.svelte` (presentational) | Props: `isScanning: boolean`, `filesScanned: number`. Spinner + file count. | `ui/src/lib/components/setup/ScanProgress.svelte` |
| 10.4 | Implement `ModelTable.svelte` (presentational) | Props: `models: DiscoveredModel[]`, `selectedIds: Set<string>`. Events: `onselectionchange`. `<table>` with `<thead>`, `<th scope="col">`. Columns: checkbox, filename, size, quantization, modified. Native `<input type="checkbox">`. Space toggles checkbox. | `ui/src/lib/components/setup/ModelTable.svelte` |
| 10.5 | Implement `ConfigStep` (orchestrator) | Fires recommendation mutation with selected models. Renders `ServerConfigForm` per selected model. Stores user overrides in `SetupWizardStore.userOverrides`. Merges recommendations with overrides at render time. | `ui/src/routes/setup/configure/+page.svelte` |
| 10.6 | Implement `ServerConfigForm.svelte` (presentational) | Props: `model`, `hardware`, `recommendation`, `values`. Events: `onchange`. Fields: gpu_layers, ctx_size, parallel_slots, port, cache_type_k, cache_type_v, flash_attn. Each field is a labeled input with inline "Recommended: X" hint. Inline validation on blur. Concrete bounds per Section 5.6: gpu_layers (-1 or 0–512), ctx_size (0 or 128–1048576), parallel_slots (1–64), port (1024–65535, unique). | `ui/src/lib/components/setup/ServerConfigForm.svelte` |
| 10.7 | Implement form validation | Client-side validation for all numeric fields per concrete bounds. `binary_path` and `model_path` non-empty. Validation errors use `aria-describedby` linked to the input. | `ui/src/lib/components/setup/ServerConfigForm.svelte` |
| 10.8 | Implement unsaved changes guard | `beforeunload` listener when ConfigStep has modifications. Active when `userOverrides.size > 0`. | `ui/src/routes/setup/+layout.svelte` (update existing guard) |
| 10.9 | Unit tests: ModelTable | Render 5 models. Check 2. Assert `onselectionchange` called with Set of 2 IDs. Render 0 models. Assert `EmptyState`. | tests |
| 10.10 | Unit tests: ServerConfigForm validation | Enter `gpu_layers: -2` -> assert rejected. `gpu_layers: 513` -> rejected. `gpu_layers: -1` -> accepted. `ctx_size: 64` -> rejected. `ctx_size: 2000000` -> rejected. `parallel_slots: 0` -> rejected. `port: 1023` -> rejected. | tests |
| 10.11 | Unit tests: recommendation display | Render with recommendation. Assert "Recommended: 8192" hint near ctx_size input. | tests |
| 10.12 | Unit tests: unsaved changes | Set `userOverrides` to non-empty. Assert `beforeunload` listener active. | tests |

### PR10 Acceptance Criteria

- [ ] ModelStep: directory selector, scan trigger, model table with checkboxes
- [ ] Model scan shows progress, handles truncation warning, empty state
- [ ] `selectedModelIds` persisted in `SetupWizardStore`
- [ ] ConfigStep: renders `ServerConfigForm` per selected model
- [ ] Recommendations displayed as inline hints with "Recommended: X"
- [ ] Form validation enforces concrete bounds (gpu_layers, ctx_size, parallel_slots, port)
- [ ] Validation errors use `aria-describedby`
- [ ] `beforeunload` guard active when overrides exist
- [ ] All 4 unit test suites pass
- [ ] `npm run check && npm run lint` clean

### PR10 Creation Steps

```bash
git checkout -b ui/setup-10-models-config frontend
# ... implement tasks 10.1–10.12 ...
git add -A && git commit -m "ui: add ModelStep with table selection and ConfigStep with validation and recommendation badges"
git push -u origin ui/setup-10-models-config
# Open PR: "Setup Wizard PR10: Model Step + Config Step"
# Description: "Implements Model and Config wizard steps (08_SETUP_WIZARD.md Sections 2.1, 4.2, 5.6).
# ModelStep with DirectorySelector, ScanProgress, ModelTable with checkboxes. ConfigStep with
# ServerConfigForm per selected model, recommendation badges, inline validation with concrete
# bounds (gpu_layers -1/0-512, ctx_size 0/128-1M, parallel_slots 1-64, port 1024-65535).
# beforeunload unsaved changes guard. 4 unit test suites."
```

---

## PR11: Review Step + Completion

**Branch:** `ui/setup-11-review-completion`  
**Merge Target:** `frontend`  
**Depends on:** PR10 merged to `frontend`  
**Size:** Medium (~400 lines)  
**Estimated Time:** 6–8 hours  

### Purpose

Implement the ReviewStep with `preview-config` endpoint call, `TomlPreview` via `renderMarkdown()` (TOML code block — no `{@html}`), `SavePathSelector`, `save-config` call with backup display, and `CompletionScreen` with status polling (`setup_required` check). This closes the wizard loop: the user can see the generated config, choose a save path, save, and verify the server transitions to running mode.

Design doc references: Section 2.1 (Flows 1 and 2 — Step 5), Section 2.2 (Review and Complete screen states), Section 4.2 (TomlPreview, SavePathSelector, CompletionScreen contracts), Section 5.6 (Save path form), Section 13.7 (CompletionScreen Verification).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 11.1 | Implement `ReviewStep` (orchestrator) | Fires `preview-config` mutation on mount. Shows TOML preview. "Save & Finish" button. Save path selector. On save: calls `save-config`, shows completion screen. Handles 409 (file exists without force) with `ConfirmDialog`. Handles 403 (permission denied) with inline error. | `ui/src/routes/setup/review/+page.svelte` |
| 11.2 | Implement `TomlPreview.svelte` (presentational) | Props: `toml: string`. Renders syntax-highlighted TOML via `renderMarkdown()` with a code block (```toml ... ```). No `{@html}` — uses the existing sanitized pipeline. | `ui/src/lib/components/setup/TomlPreview.svelte` |
| 11.3 | Implement `SavePathSelector.svelte` (presentational) | Props: `path: string`, `defaultPath: string`. Events: `onchange`. Text input pre-filled with XDG default path. "Reset to default" link. | `ui/src/lib/components/setup/SavePathSelector.svelte` |
| 11.4 | Implement `CompletionScreen.svelte` (presentational) | Props: `saveResult`, `validationResult`, `reloadResult`, `isVerifying`, `isVerified`. States: saving -> validation result -> reload result -> polling -> success/failure. "Go to Dashboard" button (enabled when verified). "Go to Dashboard Anyway" link (fallback after 10s). Backup path display when `backup_path` non-null. | `ui/src/lib/components/setup/CompletionScreen.svelte` |
| 11.5 | Implement completion verification polling | Poll `GET /api/status` every 2s, up to 5 attempts. Check `setup_required == false`. On success: show green checkmark. After 5 polls: show error with retry + "Go to Dashboard Anyway." | `ui/src/routes/setup/review/+page.svelte` |
| 11.6 | Implement validation-launch result display | If `validation_launch.success: false`: display error, server name, hint. "Go Back" button to ConfigStep. "Retry" button. | `ui/src/routes/setup/review/+page.svelte` |
| 11.7 | Implement hot-reload failure display | If `reload.success: false`: display error. "Retry" button (re-calls save-config with `force: true`). | `ui/src/routes/setup/review/+page.svelte` |
| 11.8 | Implement save double-submit protection | Disable "Save & Finish" button and show spinner while save request is in flight. | `ui/src/routes/setup/review/+page.svelte` |
| 11.9 | Unit tests: TomlPreview | Pass TOML string -> assert rendered in `<pre><code>` block. Assert no `{@html}` usage (verify it goes through `renderMarkdown()`). | tests |
| 11.10 | Unit tests: CompletionScreen states | Render with `isVerifying: true` -> assert spinner. Render with `isVerified: true` -> assert success checkmark + "Go to Dashboard" enabled. Render with verification failed -> assert "Go to Dashboard Anyway" link. | tests |
| 11.11 | Unit tests: validation-launch failure | Mock `validation_launch.success: false`. Assert error display with hint and "Go Back" button. | tests |
| 11.12 | Unit tests: 409 overwrite flow | Mock save-config returning 409. Assert `ConfirmDialog` appears. Confirm -> assert re-call with `force: true`. | tests |
| 11.13 | Unit tests: backup path display | Mock save response with `backup_path`. Assert "Previous config backed up to ..." displayed. | tests |

### PR11 Acceptance Criteria

- [ ] TOML preview rendered via `renderMarkdown()` (no raw `{@html}`)
- [ ] Save path pre-filled with XDG default
- [ ] 409 overwrite handled with `ConfirmDialog`
- [ ] 403 permission denied displayed inline
- [ ] Validation-launch failure shows error + hint + "Go Back"
- [ ] Hot-reload failure shows error + "Retry"
- [ ] Completion polling: 5 attempts, 2s interval, success/failure states
- [ ] "Go to Dashboard" enabled only when `setup_required == false`
- [ ] "Go to Dashboard Anyway" link available after 10s timeout
- [ ] Backup path displayed when non-null
- [ ] Double-submit protection on "Save & Finish"
- [ ] All 5 unit test suites pass
- [ ] `npm run check && npm run lint` clean

### PR11 Creation Steps

```bash
git checkout -b ui/setup-11-review-completion frontend
# ... implement tasks 11.1–11.13 ...
git add -A && git commit -m "ui: add ReviewStep with TOML preview, save flow, and CompletionScreen verification"
git push -u origin ui/setup-11-review-completion
# Open PR: "Setup Wizard PR11: Review Step + Completion"
# Description: "Implements Review and Completion wizard steps (08_SETUP_WIZARD.md Sections 2.1, 4.2, 5.6, 13.7).
# TomlPreview via renderMarkdown() (no {@html}). SavePathSelector with XDG default. save-config
# with 409 overwrite confirmation, 403 permission display, validation-launch failure display with
# hint, hot-reload failure retry, completion verification polling (5 attempts, 2s, setup_required
# check). Backup path display. Double-submit protection. 5 unit test suites."
```

---

## VC-3: Full Wizard Flow Verification Checkpoint

**Gate:** PR8–PR11 merged to `frontend`. Full wizard flow works end-to-end (happy path).

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-3.1 | Welcome -> Hardware | Click "Start Setup" -> hardware detection runs | Hardware cards appear with detection results |
| VC-3.2 | Hardware -> Binary | Advance from hardware -> binary scan runs | Binary found/not-found state displayed |
| VC-3.3 | Build flow | Binary not found -> start build -> SSE log streams | Build progresses through phases |
| VC-3.4 | Binary -> Models | Advance from binary -> model scan UI | Directory selector + scan + model table |
| VC-3.5 | Models -> Configure | Select models, advance -> config forms appear | Recommendations pre-filled, validation works |
| VC-3.6 | Step back-navigation | Navigate back to Hardware from Configure | Hardware shows cached results, Configure invalidated |
| VC-3.7 | Keyboard-only flow | Complete welcome through configure with keyboard only | All controls keyboard-accessible |
| VC-3.8 | axe-core all wizard steps | Run axe-core on each step | Zero violations |
| VC-3.9 | Review step TOML preview | Review step shows TOML preview from preview-config endpoint | TOML rendered via renderMarkdown() |
| VC-3.10 | Save flow end-to-end | save-config -> validation-launch -> completion polling -> "Go to Dashboard" | Full save flow completes successfully |

---

## Phase 4: Integration & Testing

---

## PR12: Backend Contract Tests

**Branch:** `setup/12-contract-tests`  
**Merge Target:** `main`  
**Depends on:** PR5, PR6, PR7 merged to `main` (all backend PRs)  
**Size:** Medium (~350 lines)  
**Estimated Time:** 5–7 hours  

### Purpose

Implement JSON schema snapshot tests for all `/api/setup/*` endpoints to ensure the contract between backend and frontend stays in sync. Covers happy-path response shapes, error response shapes, and input validation boundary tests. If `ts-rs` is feasible, generate TypeScript type definitions directly from Rust structs; otherwise, use manual snapshot comparison.

Design doc references: Section 10.6 (Backend API Tests), Section 10.7 (Contract Tests).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 12.1 | Implement JSON schema snapshot capture | For each endpoint, capture the response JSON shape from happy-path integration tests. Store as `.json` snapshot files in version control. | `crates/llamaforge-api/tests/snapshots/` |
| 12.2 | Hardware detection snapshot | Capture response from `GET /api/setup/hardware` with mocked probes. Pin field names, types, nesting. | `crates/llamaforge-api/tests/contract_tests.rs` |
| 12.3 | Binary scanner snapshot | Capture both found and not-found variants. | `crates/llamaforge-api/tests/contract_tests.rs` |
| 12.4 | Model scanner snapshot | Capture response with models, truncation, and warnings. | `crates/llamaforge-api/tests/contract_tests.rs` |
| 12.5 | Recommendation snapshot | Capture response with recommendations and rationale. | `crates/llamaforge-api/tests/contract_tests.rs` |
| 12.6 | Build SSE event snapshots | Capture each SSE event type: `phase`, `progress`, `log`, `complete`, `failed`. | `crates/llamaforge-api/tests/contract_tests.rs` |
| 12.7 | Save-config / preview-config snapshot | Capture success, 409, 403, 422 responses. | `crates/llamaforge-api/tests/contract_tests.rs` |
| 12.8 | Error response envelope tests | Verify all 4xx/5xx responses have `Content-Type: application/json` and `error`/`message` fields. | `crates/llamaforge-api/tests/contract_tests.rs` |
| 12.9 | Evaluate ts-rs integration | Attempt `ts-rs` derivation from Rust response structs. If feasible, add `#[derive(TS)]` and generate `.ts` files. If not, document why and proceed with snapshots. | `crates/llamaforge-api/Cargo.toml`, response structs |
| 12.10 | TypeScript type sync verification | If using snapshots: compare snapshot shapes against `ui/src/lib/api/types.ts` setup types. If using ts-rs: verify generated types match checked-in types. | CI script or test |
| 12.11 | Input validation boundary tests | All boundary tests from Section 10.6: empty directories, path traversal, null bytes, read-only path, invalid backend, jobs 0/negative, jobs > num_cpus, empty models, gibberish command output, multi-GPU output, Apple Silicon unified memory. | `crates/llamaforge-api/tests/validation_boundary_tests.rs` |

### PR12 Acceptance Criteria

- [ ] JSON schema snapshots exist for all `/api/setup/*` endpoints
- [ ] Snapshot tests fail if response shape changes (new field, removed field, type change)
- [ ] Error responses verified: all return structured JSON with `error`/`message`
- [ ] Input validation boundary tests cover all cases from Section 10.6
- [ ] ts-rs evaluation documented (adopted or deferred with rationale)
- [ ] TypeScript types verified against backend response shapes
- [ ] All test suites pass
- [ ] `cargo clippy` clean, `cargo test` passes

### PR12 Creation Steps

```bash
git checkout -b setup/12-contract-tests main
# ... implement tasks 12.1–12.11 ...
git add -A && git commit -m "setup: add contract tests with JSON schema snapshots and input validation boundaries"
git push -u origin setup/12-contract-tests
# Open PR: "Setup Wizard PR12: Backend Contract Tests"
# Description: "Implements contract tests (08_SETUP_WIZARD.md Sections 10.6, 10.7).
# JSON schema snapshot tests for all /api/setup/* endpoints. Error envelope verification.
# Input validation boundary tests (empty dirs, path traversal, null bytes, invalid backend,
# jobs bounds, multi-GPU parsing, Apple Silicon). ts-rs evaluation. TypeScript type sync check."
```

---

## PR13: Frontend Test Suite + CI

**Branch:** `ui/setup-13-tests`  
**Merge Target:** `frontend`  
**Depends on:** PR11 merged to `frontend`  
**Size:** Large (~600 lines of tests + CI config)  
**Estimated Time:** 8–10 hours  

### Purpose

Implement the comprehensive frontend test suite for the wizard: component tests (5 states each), integration tests (full wizard flow, build failure recovery, hardware failure manual entry, SSE streaming tests), visual regression (light + dark), axe-core, and bundle size verification for the wizard chunk (< 40 KB gzipped). Also includes standalone mode tests and the CI pipeline configuration for wizard-specific checks.

Design doc references: Section 10 (Testing Strategy — all subsections).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 13.1 | Component tests: StepIndicator | 5 states: all upcoming, 1 current, 2 complete + 1 current, all complete, 1 skipped. | `ui/tests/components/StepIndicator.test.ts` |
| 13.2 | Component tests: HardwareCard | 5 states: detected, not_detected, error, loading (skeleton), timed_out. | `ui/tests/components/HardwareCard.test.ts` |
| 13.3 | Component tests: ModelTable | 5 states: empty, 1 model, 5 models (2 selected), truncated, with warnings. | `ui/tests/components/ModelTable.test.ts` |
| 13.4 | Component tests: ServerConfigForm | 5 states: default values, custom values, validation errors, all recommended, recommendation override. | `ui/tests/components/ServerConfigForm.test.ts` |
| 13.5 | Component tests: BuildProgress | 5 states: idle, cloning, building (50%), complete, failed. | `ui/tests/components/BuildProgress.test.ts` |
| 13.6 | Component tests: TomlPreview | Render TOML string -> assert syntax highlighting applied via renderMarkdown. | `ui/tests/components/TomlPreview.test.ts` |
| 13.7 | Integration test: full wizard flow (happy path) | Mock all API responses. Walk through all 6 steps. Verify state accumulates. Verify save request contains expected config. | `ui/tests/integration/wizard-flow.test.ts` |
| 13.8 | Integration test: build failure recovery | Mock build SSE with `failed` event. Verify error UI. Click retry. Mock success on second attempt. Verify transition to binary-found. | `ui/tests/integration/build-recovery.test.ts` |
| 13.9 | Integration test: build failure skip | Mock build `failed`. Click "Skip." Verify manual path input. Enter path. Advance. Verify path carried to review. | `ui/tests/integration/build-skip.test.ts` |
| 13.10 | Integration test: hardware failure manual entry | Mock 500 from hardware endpoint. Verify error banner + manual entry form. Fill in hardware. Advance. Verify manual values used in recommendations. | `ui/tests/integration/hardware-fallback.test.ts` |
| 13.11 | Integration test: SSE reconnect during build | Mock SSE disconnect mid-stream. Verify reconnect. Mock successful resume. Verify log continues without duplicates. | `ui/tests/integration/sse-reconnect.test.ts` |
| 13.12 | Integration test: SSE reconnect exhaustion | Mock 3 consecutive failed reconnects. Verify "Connection lost" UI. Click retry. Verify fresh connection. | `ui/tests/integration/sse-exhaustion.test.ts` |
| 13.13 | Integration test: SSE malformed event | Mock invalid JSON in SSE data. Verify graceful handling: logged, stream continues. | `ui/tests/integration/sse-malformed.test.ts` |
| 13.14 | Integration test: navigation back/forward | Complete through Configure, navigate back to Hardware, verify data cached. Navigate forward, verify Configure invalidated. | `ui/tests/integration/wizard-navigation.test.ts` |
| 13.15 | Integration test: keyboard-only flow | Playwright: complete full wizard using only Tab, Enter, Space, Arrow keys. | `ui/tests/e2e/wizard-keyboard.spec.ts` |
| 13.16 | Visual regression: wizard steps | Playwright screenshots for each step in light + dark themes at 1280x800. | `ui/tests/visual/wizard.spec.ts` |
| 13.17 | axe-core: wizard steps | Run axe-core on each wizard step in both themes. | `ui/tests/a11y/wizard-axe.spec.ts` |
| 13.18 | Bundle size check | Verify wizard chunk < 40 KB gzipped. Update `bundle-baseline.json` if it exists. | `ui/scripts/check-bundle-size.js` (update) |
| 13.19 | Standalone mode test: save failure + retry | Mock filesystem error on first save. Verify error displayed. Retry with valid path. Verify success. | `ui/tests/integration/standalone-save.test.ts` |

### PR13 Acceptance Criteria

- [ ] Component tests: 6 components tested with 5 states each (30 test cases)
- [ ] Integration tests: 8 flow tests covering happy path, failure recovery, SSE edge cases
- [ ] Keyboard-only E2E test passes
- [ ] Visual regression screenshots captured for 6 steps x 2 themes = 12 screenshots
- [ ] axe-core: zero violations on all wizard steps in both themes
- [ ] Wizard chunk < 40 KB gzipped
- [ ] All test suites pass
- [ ] `npm run check && npm run lint` clean

### PR13 Creation Steps

```bash
git checkout -b ui/setup-13-tests frontend
# ... implement tasks 13.1–13.19 ...
git add -A && git commit -m "ui: add wizard test suite — components, integration, visual regression, axe-core, bundle check"
git push -u origin ui/setup-13-tests
# Open PR: "Setup Wizard PR13: Frontend Test Suite + CI"
# Description: "Implements the full wizard test suite (08_SETUP_WIZARD.md Section 10).
# Component tests (6 components x 5 states). Integration tests: full flow, build failure
# recovery, build skip, hardware fallback, SSE reconnect, SSE exhaustion, SSE malformed,
# navigation, standalone save. Keyboard E2E test. Visual regression (12 screenshots).
# axe-core (12 pages). Bundle size check (< 40 KB). 19 test files."
```

---

## PR14: Final Polish + End-to-End Verification

**Branch:** `setup/14-final-polish`  
**Merge Target:** `main`  
**Depends on:** PR12 (backend contract tests) and PR13 (frontend tests) merged  
**Size:** Small–Medium (~300 lines)  
**Estimated Time:** 5–7 hours  

### Purpose

The capstone PR: first-run mode end-to-end test (boot with no config -> wizard -> save -> running mode transition -> dashboard), reconfigure flow test, cross-branch verification, update Open Questions in the design doc with resolutions, and update the Decision Log. This PR touches both the backend (end-to-end test) and the frontend (any polish items found during verification).

Design doc references: Section 10.8 (Standalone Wizard Mode Tests), Section 14 (Open Questions), Appendix: Decision Log.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 14.1 | E2E test: first-run mode full lifecycle | Start `llamaforge serve` with no config. Navigate to `/ui/setup`. Walk through all wizard steps with mocked hardware/models. Save config. Verify mode transitions to Running. Navigate to dashboard. Verify dashboard loads. | `crates/llamaforge-e2e/tests/wizard_e2e.rs` |
| 14.2 | E2E test: reconfigure flow | Start in Running mode. Navigate to setup wizard with `X-Setup-Reconfigure` header. Re-scan hardware. Add a model. Save. Verify config updated (merged, not replaced). | `crates/llamaforge-e2e/tests/wizard_e2e.rs` |
| 14.3 | E2E test: standalone save + shutdown | Start `llamaforge init --interactive`. Complete wizard. Save config. Verify server exits 0. Verify config file written. | `crates/llamaforge-e2e/tests/wizard_e2e.rs` |
| 14.4 | E2E test: Ctrl+C during build | Start wizard. Initiate build. Send SIGINT. Verify process group killed. Verify no orphan processes. Verify exit 130. | `crates/llamaforge-e2e/tests/wizard_e2e.rs` |
| 14.5 | E2E test: concurrent tabs | Open wizard in two sessions. Complete and save from both. Verify first saves, second gets 409 or waits. Verify no config corruption. | `crates/llamaforge-e2e/tests/wizard_e2e.rs` |
| 14.6 | Update Open Questions | Verify Open Questions #1-#5 were implemented as decided in Pre-Implementation Decisions. Update design doc Section 14 to mark each as resolved with the implementation PR reference. | `docs/component-designs/08_SETUP_WIZARD.md` |
| 14.7 | Update Decision Log | Add implementation decisions to the Appendix: Decision Log table. | `docs/component-designs/08_SETUP_WIZARD.md` |
| 14.8 | Cross-branch type sync audit | Verify all TypeScript types in `ui/src/lib/api/types.ts` match the backend response structs. Document any discrepancies. | manual verification |

### PR14 Acceptance Criteria

- [ ] First-run E2E: no config -> wizard -> save -> running mode -> dashboard
- [ ] Reconfigure E2E: running mode -> wizard -> add model -> merge config
- [ ] Standalone E2E: init --interactive -> complete -> exit 0
- [ ] Ctrl+C E2E: no orphan processes, exit 130
- [ ] Concurrent tabs E2E: no config corruption
- [ ] Open Questions resolved in design doc
- [ ] Decision Log updated
- [ ] TypeScript types match backend structs
- [ ] All E2E test suites pass

### PR14 Creation Steps

```bash
# Backend E2E tests
git checkout -b setup/14-final-polish main
# ... implement tasks 14.1–14.5 ...
git add -A && git commit -m "setup: add end-to-end tests for first-run, reconfigure, standalone, and concurrent flows"
git push -u origin setup/14-final-polish

# Doc updates (same branch or separate, depending on review preference)
# ... implement tasks 14.6–14.8 ...
git add -A && git commit -m "setup: resolve open questions, update decision log, type sync audit"
git push origin setup/14-final-polish

# Open PR: "Setup Wizard PR14: Final Polish + End-to-End Verification"
# Description: "Capstone PR. E2E tests: first-run lifecycle (no config -> wizard -> running -> dashboard),
# reconfigure flow, standalone save+shutdown, Ctrl+C build cleanup, concurrent tab safety.
# Open Questions #1-#5 resolved. Decision Log updated. Cross-branch type sync audit."
```

---

## VC-4: Final Sign-Off Checkpoint

**Gate:** All 14 PRs merged. Setup Wizard is complete.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-4.1 | All backend tests pass | `cargo test` in workspace root | Zero failures |
| VC-4.2 | All frontend tests pass | `npm run test && npm run test:e2e` in `ui/` | Zero failures |
| VC-4.3 | Zero lint warnings | `cargo clippy` + `npm run check && npm run lint` | Exit code 0 |
| VC-4.4 | Wizard bundle < 40 KB | `node scripts/check-bundle-size.js` | Wizard chunk passes |
| VC-4.5 | axe-core clean | All wizard steps, both themes | Zero violations |
| VC-4.6 | First-run E2E | Boot with no config -> wizard -> save -> dashboard | Full lifecycle works |
| VC-4.7 | Standalone E2E | `llamaforge init --interactive` -> complete -> exit 0 | Clean lifecycle |
| VC-4.8 | Build flow E2E | Binary not found -> build -> SSE streams -> success | Full build lifecycle |
| VC-4.9 | Hardware detection accuracy | Run on test machine with known GPU | Correct GPU/VRAM/CPU/RAM |
| VC-4.10 | Model scan accuracy | Scan directory with known .gguf files | All files found, metadata correct |
| VC-4.11 | Config validity | Generated config passes `llamaforge validate` | Zero errors |
| VC-4.12 | Keyboard-only wizard | Complete all steps without mouse | All flows possible |
| VC-4.13 | Screen reader audit | VoiceOver walkthrough of full wizard flow | Step announcements, error announcements, build progress correct |
| VC-4.14 | Dark mode audit | Visual inspection of every wizard step in dark mode | No contrast issues |
| VC-4.15 | Design doc compliance | Cross-check every Section in `08_SETUP_WIZARD.md` against implementation | Checklist complete |
| VC-4.16 | No raw fetch | `grep -rn "fetch(" ui/src/ --include="*.ts" --include="*.svelte"` — only in `client.ts` | Zero matches outside `client.ts` |
| VC-4.17 | No bare {@html} | `grep -rn "{@html" ui/src/ --include="*.svelte"` — only with `renderMarkdown()` | Zero bare `{@html}` usages |
| VC-4.18 | No Svelte 4 API | `grep -rn "onMount\|onDestroy\|writable\|readable" ui/src/ --include="*.ts" --include="*.svelte"` | Zero matches |

---

## Summary: PR -> File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `crates/llamaforge-server/src/hardware.rs`, tests | ~400 |
| PR2 | `crates/llamaforge-server/src/model_scanner.rs`, tests | ~350 |
| PR3 | `crates/llamaforge-server/src/binary_scanner.rs`, tests | ~200 |
| PR4 | `crates/llamaforge-server/src/recommendation.rs`, tests | ~350 |
| PR5 | `crates/llamaforge-api/src/setup/*.rs` (8 handler files + mod + state), tests | ~600 |
| PR6 | `crates/llamaforge-server/src/build.rs`, `crates/llamaforge-api/src/setup/build_*.rs`, tests | ~700 |
| PR7 | `crates/llamaforge/src/init.rs`, `crates/llamaforge/src/serve.rs`, `crates/llamaforge-api/src/setup/guard.rs`, tests | ~500 |
| PR8 | `ui/src/routes/setup/**`, `ui/src/lib/stores/setup.svelte.ts`, `ui/src/lib/components/setup/StepIndicator.svelte`, `ui/src/lib/components/setup/StepNavigation.svelte`, `ui/src/lib/api/types.ts`, `ui/src/lib/api/queries.ts` | ~400 |
| PR9 | `ui/src/routes/setup/hardware/+page.svelte`, `ui/src/routes/setup/binary/+page.svelte`, `ui/src/lib/components/setup/HardwareCard.svelte`, `ui/src/lib/components/setup/BuildProgress.svelte`, `ui/src/lib/components/shared/LogViewer.svelte`, `ui/src/lib/components/shared/ProgressBar.svelte` | ~550 |
| PR10 | `ui/src/routes/setup/models/+page.svelte`, `ui/src/routes/setup/configure/+page.svelte`, `ui/src/lib/components/setup/ModelTable.svelte`, `ui/src/lib/components/setup/ServerConfigForm.svelte`, `ui/src/lib/components/setup/DirectorySelector.svelte` | ~450 |
| PR11 | `ui/src/routes/setup/review/+page.svelte`, `ui/src/lib/components/setup/TomlPreview.svelte`, `ui/src/lib/components/setup/SavePathSelector.svelte`, `ui/src/lib/components/setup/CompletionScreen.svelte` | ~400 |
| PR12 | `crates/llamaforge-api/tests/contract_tests.rs`, `crates/llamaforge-api/tests/validation_boundary_tests.rs`, `crates/llamaforge-api/tests/snapshots/*.json` | ~350 |
| PR13 | `ui/tests/components/*.test.ts`, `ui/tests/integration/*.test.ts`, `ui/tests/e2e/wizard-keyboard.spec.ts`, `ui/tests/visual/wizard.spec.ts`, `ui/tests/a11y/wizard-axe.spec.ts` | ~600 |
| PR14 | `crates/llamaforge-e2e/tests/wizard_e2e.rs`, `docs/component-designs/08_SETUP_WIZARD.md` (updates) | ~300 |
| **Total** | | **~6,150** |

> Note: Table totals ~6,150 lines of core implementation. With type definitions, configuration, test fixtures, and CI scripts, the total reaches ~8,200. Breakdown: ~3,550 backend production + ~2,050 frontend production + ~2,600 tests.

---

## Timeline (Single Engineer)

| Day | Work | PRs |
|-----|------|-----|
| Day 1 | Hardware detection module | PR1 |
| Day 2 | Model scanner module | PR2 |
| Day 3 | Binary scanner module (morning) + start recommendation engine | PR3, PR4 start |
| Day 4 | Finish recommendation engine | PR4 |
| Day 5–6 | Setup API endpoints | PR5 |
| Day 7–8 | Build orchestrator (SSE, process groups, timeouts) | PR6 |
| Day 9 | CLI changes + first-run mode | PR7 |
| Day 10 | VC-1 + VC-2 verification, fix any issues | — |
| Day 11 | Wizard scaffold + step navigation | PR8 |
| Day 12–13 | Hardware step + Binary step (build SSE UI) | PR9 |
| Day 14 | Model step + Config step | PR10 |
| Day 15 | Review step + Completion | PR11 |
| Day 16 | VC-3 verification, fix any issues | — |
| Day 17 | Backend contract tests | PR12 |
| Day 18–19 | Frontend test suite + CI | PR13 |
| Day 20–21 | Final polish + E2E verification | PR14 |
| Day 22 | VC-4 final sign-off, code review fixes | — |

## Timeline (Two Engineers)

| Day | Engineer A (Backend-focused) | Engineer B (Frontend-focused) |
|-----|-----|-----|
| Day 1 | PR1 (hardware detection) | PR2 (model scanner) |
| Day 2 | PR3 (binary scanner) | PR4 (recommendation engine) |
| Day 3–4 | PR5 (setup API endpoints) | Reviews PR1–PR4, writes mock fixtures |
| Day 5–6 | PR6 (build orchestrator) | Reviews PR5, prepares frontend TypeScript types |
| Day 7 | PR7 (CLI + first-run mode) | PR8 (wizard scaffold) — starts after frontend rebase |
| Day 8 | Reviews PR8, VC-1 + VC-2 verification | PR9 (hardware + binary steps) |
| Day 9 | PR12 (backend contract tests) | PR10 (model + config steps) |
| Day 10 | Reviews PR9, PR10 | PR11 (review + completion) |
| Day 11 | PR14 part 1 (E2E backend tests) | PR13 (frontend test suite) |
| Day 12 | PR14 part 2 (doc updates, audits) | Reviews PR12, PR14 |
| Day 13 | VC-3 + VC-4 verification | Code review fixes |

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `nvidia-smi` / `system_profiler` output format changes between versions | Medium | Medium | Parse with flexible regex. Unit tests with fixture outputs from multiple OS/driver versions. Graceful fallback to manual entry on parse failure. |
| Build process takes > 20 minutes on slow hardware | Medium | Low | Per-phase timeouts prevent infinite hangs. UI shows factual progress (target count + elapsed), not ETAs. User can cancel and provide binary manually. |
| SSE `EventSource` reconnection behavior differs across browsers | Medium | Low | Custom reconnection logic (not browser-native retry). Exponential backoff with jitter. Terminal event replay on reconnect. Integration tests with mock disconnect. |
| GGUF filename heuristics fail for non-standard naming | High | Low | Heuristic is best-effort — `"unknown"` for unrecognized patterns. GGUF header parser is a documented fast-follow (Open Question #1). |
| Wizard bundle exceeds 40 KB gzipped | Low | Medium | Wizard has no heavy dependencies (no ECharts, no TanStack Table). LogViewer and ProgressBar are lightweight. CI check enforces budget. |
| Process group cleanup fails on some Linux kernels | Low | High | `setsid` + `kill(-pgid)` is POSIX standard. Integration test verifies no orphan processes. Fallback: individual PID kill of known children. |
| Port availability check races with other processes | Medium | Low | Port check is advisory (documented in design doc). Validation-launch is the authoritative test — it actually binds the port. |
| macOS Gatekeeper blocks built binary | Medium | Medium | Document: built-from-source binaries may require `xattr -d com.apple.quarantine`. Wizard could run this automatically post-build. |
| Wizard state lost on page refresh | Low | Low | By design — the design doc explicitly states wizard state is session-scoped. TanStack Query re-fetches on mount. User re-scans, which is correct behavior (hardware/files may have changed). |
| `walkdir` follows symlink cycles despite detection | Low | Medium | Track canonical paths in a `HashSet`. Skip already-visited paths. Report as warning. Integration test with symlink cycle fixture. |
| `humantime` Duration parsing rejects valid inputs | Low | Low | Pin `humantime` version. Test common formats (`30m`, `1h`, `45m`, `2h30m`). |
| First-run mode hot-reload fails for non-obvious reasons | Medium | Medium | Detailed error messages in `reload.error`. Mode reverts to `Setup`, not crash. User can retry or adjust config. |

---

## Integration Points (for other components)

After the Setup Wizard is complete, the following integration seams are ready:

1. **Config Manager** — The wizard's `save-config` endpoint assembles `AppConfig` using the existing config types from `llamaforge-core`. The saved config must pass `llamaforge validate` and be loadable by the config manager's hot-reload path.

2. **Server Manager** — The `validate-launch` endpoint (Section 13.5) spawns a `llama-server` process using the same process management primitives as the server manager (`llamaforge-server`). The wizard reuses `vram_estimate.rs` for recommendations.

3. **Transport Layer** — The first-run `SetupModeGuard` middleware integrates with the existing transport layer (axum + tonic). Setup endpoints coexist with inference endpoints on the same router, gated by the `Mode` state machine.

4. **Frontend Web UI** — The wizard routes (`/ui/setup/*`) are code-split from the main UI routes (`/ui/*`). They share: `app.css` (design tokens), shared components (`StatusBadge`, `Skeleton`, `ErrorBanner`, `EmptyState`, `Toast`, `ConfirmDialog`), `client.ts` (API client), theme system. New shared components introduced: `LogViewer`, `ProgressBar`, `StepIndicator`.

5. **Metrics & Storage** — No direct dependency. The wizard does not write metrics. Future: the setup session could be logged as a metric event for telemetry.

6. **Status Endpoint** — `GET /api/status` gains the `setup_required` boolean field. The main UI's root layout checks this on mount and redirects to the wizard when `true`.
