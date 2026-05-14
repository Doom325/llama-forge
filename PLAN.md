# Integration & Composition Root — Implementation Plan

**Phase:** 8 (Final integration; closes v1)
**Estimated LOC:** ~3,200 (~2,000 production + ~1,200 tests)
**Estimated Duration:** 8–10 working days (single engineer)
**Design Reference:** `docs/design.md` §4 (Architecture), §5 (Component Deep Dives — composition flow), §6 (Config schema), §8 (Testing strategy)
**Branch Prefix:** `integration/`
**Target:** `main` (via squash-merge PRs)
**Upstream Dependencies:** All 7 component crates merged (✅ as of 2026-05-14):
- `llamaforge-config`, `llamaforge-metrics`, `llamaforge-server-manager`, `llamaforge-queue`, `llamaforge-routing`, `llamaforge-dispatch`, `llamaforge-transport`
- Plus `llamaforge-types`, `llamaforge-proto`, `llamaforge` (binary, currently stub).

**Primary CoderAgent:** `AssistantCoderAgent_llamaforge-transport_final` (per role handoff 2026-05-14; see `~/.claude/projects/-Users-sumanthgottipati-git-llama-forge/memory/project_relay_agents.md`).
**Primary Reviewer:** `ReviewerAgent_llamaforge-transport_final`.
**Relay key prefix:** `llamaforge#integration-*`.

---

## Why this phase exists

The seven component crates are individually merged and pass their own tests, but `cargo run -p llamaforge` produces a stub message. Three structural gaps prevent the system from actually running end-to-end:

1. **Domain types are still empty stubs.** `IncomingRequest {}`, `QueueTicket {}`, `ServerSummary {}` are unit structs with no fields. The port traits (`QueueSubmitter::submit(IncomingRequest) -> Result<QueueTicket, ...>`, `ServerControl::list_servers() -> Vec<ServerSummary>`, etc.) pass empty data — even after wiring, nothing meaningful flows.
2. **Only one of five port traits has a production impl.** `RouteValidator` is implemented on `Router` in `llamaforge-routing`. `QueueSubmitter`, `ServerControl`, `MetricsExporter`, and `InferenceEngine` have *zero* non-test implementations. Each upstream crate exports its own surface (e.g., `RequestPipeline`, `ServerManager`) but never adapts to the port the transport layer consumes.
3. **`InferenceEngine` is the keystone and doesn't exist.** Transport's `start_servers` calls `inference_engine.chat_completion(req).await` returning a stream of chunks. That trait is the contract that glues queue → routing → dispatch into one protocol-agnostic API for transport to call. It has no production implementation anywhere — including the binary crate.

Beyond those three, several smaller items are deferred behind `// TODO`, `v1.1`, or hardcoded constants — config knobs, TLS for gRPC, query-string filter parsing for the management API, true token-usage reporting, and the `ConfigOverlayStore` trait. These are mopping-up work, not load-bearing.

The phase output is a `cargo run -p llamaforge -- --config config/llamaforge.toml` that actually:
- Loads config, builds 7 component instances, wires them via port traits
- Spawns HTTP (`:8000`) and gRPC (`:50051`) servers
- Accepts a request via OpenAI-compatible API, routes/queues/dispatches it through `mock-llama` (or a real `llama-server`), and streams tokens back
- Shuts down gracefully on SIGTERM

That's VC-4 + VC-5 ("final sign-off") combined.

---

## Ground Rules for CoderAgents

1. **Read `docs/design.md` §4 + §5 first.** Every PR description references specific design doc sections it implements.
2. **No `unwrap()` or `expect()` in production code paths.** The one exception remains `Uuid::now_v7()`. Tests excluded.
3. **No blocking under `.await`.** Inherited from transport phase. `spawn_blocking` for CPU-heavy work; `try_recv` / `try_send` for non-blocking channel ops.
4. **Every public type and trait gets a doc comment** citing the design doc section. `Open trait` / `Sealed trait` designation on all new traits.
5. **Newtype wrappers for domain IDs.** Use `RequestId`, `ServerId`, `RequestorId`, `SessionId`, `ModelAlias` from `llamaforge-types` — never raw `String`.
6. **Port traits, not concrete types, at adapter boundaries.** The `llamaforge` binary may import concrete types from any crate (it's the composition root). Library crates must not.
7. **Workspace deps only.** New dependencies belong in `[workspace.dependencies]`. Member crates use `workspace = true`.
8. **`cargo deny check` must pass** before each merge. The CI Audit Dependencies job won't catch advisories that land mid-PR fast enough.
9. **Visibility discipline.** Composition-root types in `crates/llamaforge/src/` are `pub(crate)` unless re-exported. Domain types in `llamaforge-types` are `pub`.
10. **`unwrap_or_default()` on `Result` is not an escape hatch.** Match explicitly or propagate. See PR4 (transport) Finding 3 — silent error swallowing is a finding.
11. **No workflow refs in production code or comments.** Don't write `// Default from VC-2.6` or `// TODO(PR12)`. Use local-invariant wording: `// Defaults match design doc §6.2 until [gateway] config section lands.`
12. **5-gate local verification + CI check before every relay closure:**
    ```bash
    cargo build --workspace && \
    cargo clippy --workspace --all-targets -- -D warnings && \
    cargo test --workspace && \
    cargo fmt --check && \
    cargo deny check
    echo "all-local=$?"
    # then wait for GitHub Actions:
    gh pr checks <PR>
    ```
    Every CI check must be SUCCESS. Cite the Actions run URL in the closure message.
13. **File:line cites for each closed finding** in re-review responses. If you can't cite a line, the finding isn't resolved.

---

## Dependency Graph (PR merge order)

```
                  ┌── PR3 ──┐
PR1 ── PR2 ──────┼── PR4 ──┼── PR8 ── PR9 ── PR10 ── PR11
                  ├── PR5 ──┤              ↗
                  └── PR6 ──┘
                  PR7 (independent) ───────┘
```

- **PR1 → PR2** sequential: domain types must land before trait impls compile.
- **PR3–PR6** parallel after PR2: each adapter (queue, server-manager, metrics-exporter, inference-engine wiring) implements a different port trait against the same `llamaforge-types` surface.
- **PR7** (config schema additions) is independent — touches only `llamaforge-config`, can land any time after PR2.
- **PR8** is the composition root — needs all 5 adapters merged.
- **PR9 → PR10 → PR11** are strictly sequential end-to-end work.

---

## PR1 — Domain types: real fields for the seven stubs

**Branch:** `integration/01-domain-types`
**Depends on:** `main` only
**Size:** Medium (~350 lines)
**Estimated Time:** 4–6 hours

### Purpose

Replace the seven empty/under-populated domain stubs in `crates/llamaforge-types/` with field-complete types that satisfy the port-trait contracts. Update all crates that destructure or construct these types (transport, dispatch, queue, routing, server-manager) to compile.

### Audit of current state (file:line)

- `crates/llamaforge-types/src/traits.rs:71` — `pub struct ServerSummary {}` (empty)
- `crates/llamaforge-types/src/traits.rs:80` — `pub struct QueueTicket {}` (empty)
- `crates/llamaforge-types/src/traits.rs:101` — `pub struct IncomingRequest {}` (empty)
- `crates/llamaforge-types/src/traits.rs:74` — `pub enum ServerError { Failed }` (single uninformative variant)
- `crates/llamaforge-types/src/traits.rs:89` — `pub enum AdmissionError { QueueFull }` (missing `Cancelled`, `InvalidRequest`)
- `crates/llamaforge-types/src/traits.rs:95` — `pub enum QueueError { Failed }` (uninformative)
- `crates/llamaforge-types/src/traits.rs:104` — `pub enum MetricsError { Failed }` (uninformative)
- `crates/llamaforge-types/src/lib.rs:367` — `pub struct StreamError {}` (empty; mirrors dispatch's `DispatchError` but doesn't carry its variants)

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 1.1 | `IncomingRequest`: add fields per design doc §5.4.2 — `request_id: RequestId`, `requestor_id: RequestorId`, `requestor_class: RequestorClass`, `model_target: ModelTarget`, `capabilities: RequestCapabilities`, `messages: Vec<Message>`, `params: GenerationParams`, `stream: bool`, `session_id: Option<SessionId>`, `submitted_at: DateTime<Utc>`. Doc comment cites design doc §5.4.2. | `llamaforge-types/src/traits.rs` |
| 1.2 | Define `GenerationParams` struct in `llamaforge-types/src/lib.rs` with `max_tokens: Option<u32>`, `temperature: Option<f32>`, `top_p: Option<f32>`, `top_k: Option<u32>`, `logit_bias: Option<HashMap<i32, f64>>`, `stop: Vec<String>`, `seed: Option<u64>`. Derive `Debug, Clone, Default, Serialize, Deserialize`. | `llamaforge-types/src/lib.rs` |
| 1.3 | `QueueTicket`: add `request_id: RequestId`, `admitted_at: DateTime<Utc>`, `cancel_token: tokio_util::sync::CancellationToken`, `events: tokio::sync::mpsc::Receiver<DispatchEvent>`. `DispatchEvent` enum: `Queued { position: u32 }`, `Dispatched { server_id: ServerId, slot_id: SlotId }`, `FirstToken { ttft_ms: u64 }`, `Completed { tokens: u32 }`, `Failed { error: String }`. Doc comment notes ticket holds backpressure for cancellation. | `llamaforge-types/src/traits.rs` |
| 1.4 | `ServerSummary`: `id: ServerId`, `alias: ModelAlias`, `state: ServerLifecycle`, `port: u16`, `slots_total: u32`, `slots_busy: u32`, `started_at: Option<DateTime<Utc>>`, `last_restart_at: Option<DateTime<Utc>>`. Serialize for management JSON. | `llamaforge-types/src/traits.rs` |
| 1.5 | `StreamError`: convert to enum with `BackendDisconnect`, `Timeout`, `Cancelled`, `ChunkParse { reason: String }`, `Upstream(Box<dyn std::error::Error + Send + Sync>)`. Mirrors `llamaforge-dispatch::StreamError` (which is its actual source). | `llamaforge-types/src/lib.rs` |
| 1.6 | `ServerError` / `QueueError` / `MetricsError`: replace `Failed` with informative variants. `ServerError`: `NotFound { id: ServerId }`, `AlreadyRunning { id: ServerId }`, `SpawnFailed { reason: String }`, `Unreachable { id: ServerId }`. `QueueError`: `NotFound { id: RequestId }`, `AlreadyCancelled`, `Internal { detail: String }`. `MetricsError`: `StoreUnavailable`, `Internal { detail: String }`. | `llamaforge-types/src/traits.rs` |
| 1.7 | `AdmissionError`: add `Cancelled`, `InvalidRequest { reason: String }`, `RateLimited { retry_after_secs: u32 }` alongside existing `QueueFull`. | `llamaforge-types/src/traits.rs` |
| 1.8 | Update transport's `tonic::Status::from(ApiError)` mapping and `IntoResponse for ApiError` to surface the new error variants where applicable (e.g., `AdmissionError::RateLimited` should carry the `retry_after_secs` to the existing `ApiError::QueueFull { retry_after_secs }`). | `llamaforge-transport/src/error.rs` |
| 1.9 | Mechanical fixes in dependent crates: every site that constructed `IncomingRequest {}`, `QueueTicket {}`, etc. must compile. Search with `grep -rn 'IncomingRequest {}\|QueueTicket {}\|ServerSummary {}' crates/`. | various |
| 1.10 | Doc comments on every new field referencing design doc §5.4.2 (IncomingRequest), §5.4.3 (QueueTicket), §5.3.1 (ServerSummary). | — |
| 1.11 | Unit tests: round-trip serde on `ServerSummary`, `QueueSnapshot`, `GenerationParams` (all are public API for management JSON). | `llamaforge-types/src/lib.rs` |

### PR1 Acceptance Criteria

- [ ] `cargo build --workspace` succeeds with the new fields.
- [ ] `grep -rn 'pub struct \(IncomingRequest\|QueueTicket\|ServerSummary\) {}' crates/` returns zero hits.
- [ ] No uninformative `Failed,` variant remains in `llamaforge-types/src/traits.rs`.
- [ ] 5 local gates green + 3/3 CI checks green.
- [ ] Doc comments cite design doc sections for every new type.

---

## PR2 — Adapter scaffolding crate

**Branch:** `integration/02-adapters`
**Depends on:** PR1 merged
**Size:** Small (~200 lines)
**Estimated Time:** 2–3 hours

### Purpose

Create a new library crate `crates/llamaforge-adapters/` that will house the port-trait implementations bridging concrete component types (`RequestPipeline`, `ServerManager`, etc.) to the port traits in `llamaforge-types`. This keeps the binary crate small and makes the adapter logic unit-testable.

Why a separate crate (not the binary): the orchestrator needs the same adapter trait impls when running under workspace integration tests (PR10) and the binary crate isn't a good place for that — integration tests would have to depend on a binary's `[lib]`, which Rust supports awkwardly.

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 2.1 | `cargo init --lib crates/llamaforge-adapters`. Workspace member. | `Cargo.toml`, `crates/llamaforge-adapters/` |
| 2.2 | Dependencies: `llamaforge-types`, `llamaforge-config`, `llamaforge-queue`, `llamaforge-server-manager`, `llamaforge-routing`, `llamaforge-dispatch`, `llamaforge-metrics`, `tokio`, `tokio-util`, `async-trait`, `tracing`. All via `.workspace = true`. | `crates/llamaforge-adapters/Cargo.toml` |
| 2.3 | Module structure: `src/lib.rs` declares `pub mod queue;`, `pub mod server_control;`, `pub mod metrics_exporter;`, `pub mod inference_engine;`. Each module is a stub with one `pub struct <Name>Adapter` and a `pub fn new(...)` constructor — no logic yet (PRs 3–6 fill them). | `crates/llamaforge-adapters/src/` |
| 2.4 | Crate doc comment in `lib.rs`: "Adapters bridging concrete component types to the port traits consumed by `llamaforge-transport`. See design doc §4.2 Hexagonal layering." | `crates/llamaforge-adapters/src/lib.rs` |

### PR2 Acceptance Criteria

- [x] `cargo build -p llamaforge-adapters` succeeds (with stub impls).
- [x] No production code outside `llamaforge-adapters` depends on it yet.
- [x] 5 local gates green + 3/3 CI green.

---

## VC-A — Type & Adapter Foundation Verification

**Gate:** PR1 + PR2 merged.

| # | Check | Pass criterion |
|---|-------|-----------------|
| VC-A.1 | Domain types complete | `IncomingRequest`, `QueueTicket`, `ServerSummary` all have ≥3 fields each |
| VC-A.2 | Adapter crate scaffolded | `crates/llamaforge-adapters/src/{queue,server_control,metrics_exporter,inference_engine}.rs` all exist |
| VC-A.3 | Workspace compiles | `cargo build --workspace` clean |
| VC-A.4 | CI clean | All 3 Actions checks SUCCESS on latest `main` |

---

## PR3 — `QueueSubmitterAdapter`: bridge to `RequestPipeline`

**Branch:** `integration/03-queue-adapter`
**Depends on:** PR2 merged
**Size:** Medium (~300 lines)
**Estimated Time:** 4–6 hours

### Purpose

Implement `QueueSubmitter` for a `QueueSubmitterAdapter` that wraps `llamaforge_queue::RequestPipeline`. Owns the translation from the port trait's `IncomingRequest` to the queue's internal request envelope, and translates the queue's submit-result + cancel + status calls back through the port trait.

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 3.1 | `pub struct QueueSubmitterAdapter { pipeline: Arc<RequestPipeline>, dispatch_event_tx: ... }`. Constructor takes the pipeline. | `crates/llamaforge-adapters/src/queue.rs` |
| 3.2 | `impl QueueSubmitter for QueueSubmitterAdapter` — `submit()` translates `IncomingRequest` to queue's envelope and calls `pipeline.submit(envelope)`. Returns `QueueTicket` populated with `request_id`, `admitted_at = Utc::now()`, `cancel_token`, and an `mpsc::Receiver<DispatchEvent>` whose sender is held by the pipeline-watcher task. | `crates/llamaforge-adapters/src/queue.rs` |
| 3.3 | `cancel()` calls into the pipeline's cancel API and emits `DispatchEvent::Failed { error: "cancelled" }` on the event channel before returning. | `crates/llamaforge-adapters/src/queue.rs` |
| 3.4 | `status()` snapshots the queue depth and computes `estimated_throughput_req_sec` from the metrics layer (passed in via constructor or held via Arc). Returns a `QueueSnapshot`. | `crates/llamaforge-adapters/src/queue.rs` |
| 3.5 | Error mapping: queue's internal admission errors map to `AdmissionError::QueueFull` / `RateLimited` / `InvalidRequest`. Document each branch. | `crates/llamaforge-adapters/src/queue.rs` |
| 3.6 | Unit tests with a stub `RequestPipeline` (already supported in queue crate's `test-utils` feature, or write a minimal mock here). Cover: successful submit, queue-full rejection, cancel, status returning real depth. | tests in `queue.rs` |
| 3.7 | If `RequestPipeline` doesn't currently expose `cancel(request_id)` or a stream-of-events API, add a focused PR to `llamaforge-queue` first. **Decision required:** if the change to `llamaforge-queue` is non-trivial (>50 LOC), break it into its own preliminary PR (`integration/03a-queue-cancel-api`) and merge first. | `llamaforge-queue/src/pipeline.rs` (if needed) |

### PR3 Acceptance Criteria

- [ ] `impl QueueSubmitter for QueueSubmitterAdapter` compiles.
- [ ] Unit tests cover the four `QueueSubmitter` methods + all `AdmissionError` mappings.
- [ ] No new `unwrap()`/`expect()` in production paths.
- [ ] 5 local gates green + 3/3 CI green.

---

## PR4 — `ServerControlAdapter`: bridge to `ServerManager`

**Branch:** `integration/04-server-control-adapter`
**Depends on:** PR2 merged (parallel with PR3)
**Size:** Small (~200 lines)
**Estimated Time:** 3–4 hours

### Purpose

Implement `ServerControl` on a `ServerControlAdapter` wrapping `llamaforge_server_manager::ServerManager`. The three port methods (`list_servers`, `start_server`, `stop_server`) thinly translate to manager calls and project the manager's internal `ManagedServer` view to the public `ServerSummary` shape.

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 4.1 | `pub struct ServerControlAdapter { manager: Arc<ServerManager> }` with constructor. | `crates/llamaforge-adapters/src/server_control.rs` |
| 4.2 | `list_servers()`: read the manager's snapshot, project each entry into `ServerSummary` (id, alias, state, port, slots_total, slots_busy, started_at, last_restart_at). | `crates/llamaforge-adapters/src/server_control.rs` |
| 4.3 | `start_server(&ServerId)`: call manager's `request_start(id)`. Map `ServerManager::Error::NotFound` → `ServerError::NotFound`, `AlreadyRunning` → `ServerError::AlreadyRunning`, spawn failures → `SpawnFailed`. | `crates/llamaforge-adapters/src/server_control.rs` |
| 4.4 | `stop_server(&ServerId)`: call manager's `request_stop(id)`. Same error mapping pattern. | `crates/llamaforge-adapters/src/server_control.rs` |
| 4.5 | Unit tests with a stub manager (or real `ServerManager` driven against `mock-llama` if test-utils feature allows it). | tests in `server_control.rs` |

### PR4 Acceptance Criteria

- [x] `impl ServerControl for ServerControlAdapter` compiles.
- [x] All `ServerError` variants from PR1 mapped from at least one upstream error case.
- [x] 5 local gates green + 3/3 CI green.

---

## PR5 — `MetricsExporterAdapter`: bridge to the Prometheus registry

**Branch:** `integration/05-metrics-exporter-adapter`
**Depends on:** PR2 merged (parallel with PR3/PR4)
**Size:** Small (~150 lines)
**Estimated Time:** 2–3 hours

### Purpose

Implement `MetricsExporter` on a `MetricsExporterAdapter` that wraps the `metrics_exporter_prometheus::PrometheusBuilder` recorder (already a workspace dep). Returns the text exposition format on demand. Distinct from `MetricsStore` (which is SQLite-backed historical metrics).

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 5.1 | Build a `PrometheusHandle` at startup, hold it in `MetricsExporterAdapter`. | `crates/llamaforge-adapters/src/metrics_exporter.rs` |
| 5.2 | `gather_text()`: returns `self.handle.render()`. Per design doc §4.2 the `/metrics` endpoint must always return 200, even on degraded state. | `crates/llamaforge-adapters/src/metrics_exporter.rs` |
| 5.3 | Constructor: `MetricsExporterAdapter::install_global()` returns `(adapter, install_result)` and calls `PrometheusBuilder::new().install_recorder()` once at boot. Document that this is a process-global install. | `crates/llamaforge-adapters/src/metrics_exporter.rs` |
| 5.4 | Verify that the existing `metrics::counter!`/`gauge!`/`histogram!` calls scattered through the transport crate (and elsewhere) actually flow through this recorder. Add a single unit test that emits a counter and asserts `render()` contains the metric name. | tests in `metrics_exporter.rs` |

### PR5 Acceptance Criteria

- [x] `cargo run -p llamaforge-adapters --example metrics_smoke` (or unit test) shows `llamaforge_*` metric names in the rendered output.
- [x] No `unwrap()` in production paths.
- [x] 5 local gates green + 3/3 CI green.

---

## PR6 — `InferenceEngineAdapter`: the keystone

**Branch:** `integration/06-inference-engine-adapter`
**Depends on:** PR3 merged (needs `QueueSubmitterAdapter`); PR2 (adapter crate)
**Size:** Large (~600 lines including streaming + error mapping + tests)
**Estimated Time:** 8–12 hours

### Purpose

Implement `InferenceEngine` on a new `InferenceEngineAdapter`. This is the most substantive PR in this phase — it owns the request lifecycle:

```
ChatCompletionRequest
    → translate to IncomingRequest
    → submit via QueueSubmitter (returns QueueTicket with event stream)
    → on Dispatched event: ask SmartRouter (RouteValidator? — see Note below) for slot
    → call Dispatcher::proxy (Component 06) to get a bytes stream from llama-server
    → parse llama.cpp SSE bytes into ChatCompletionChunk
    → yield chunks (with QueueUpdate intersperced from DispatchEvent::Queued)
    → on completion/error/cancel: clean up, record metrics
```

This wires four components together. The implementation crosses the `Dispatcher → SmartRouter → Queue → Metrics` boundaries within a single async function.

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 6.1 | `pub struct InferenceEngineAdapter { queue: Arc<dyn QueueSubmitter>, router: Arc<llamaforge_routing::Router>, dispatcher: Arc<llamaforge_dispatch::Dispatcher>, metrics: Arc<llamaforge_metrics::MetricsIngest> }`. Constructor injection (Arc-of-trait for queue, Arc-of-concrete for router/dispatcher/metrics until those grow port traits). | `crates/llamaforge-adapters/src/inference_engine.rs` |
| 6.2 | `chat_completion(req: ChatCompletionRequest)` translates to `IncomingRequest`: generates `RequestId::from(Uuid::now_v7())`, maps `req.model` to `ModelTarget::Exact(ModelAlias::new(req.model))`, populates `GenerationParams` from `req.{max_tokens,temperature,...}`, sets `submitted_at = Utc::now()`. Caller-provided `RequestorId` and `RequestorClass` come from a `RequestContext` extension (defer to "anonymous" for v1 if no header). | `crates/llamaforge-adapters/src/inference_engine.rs` |
| 6.3 | Call `queue.submit(incoming_req)?`. Hold the returned `QueueTicket`. The `events` `mpsc::Receiver` is the source of state-change signals. | — |
| 6.4 | Build the output `Stream<Item = Result<ChatCompletionChunk, StreamError>>`. The stream's `poll_next` drives a state machine: `WaitingForDispatch → AwaitingFirstToken → Streaming → Completed`. While `WaitingForDispatch`, yield `ChatCompletionChunk::QueueUpdate { position }` events from the `DispatchEvent::Queued` ticks. On `DispatchEvent::Dispatched { server_id, slot_id }`, transition to dispatching. | — |
| 6.5 | Dispatching: call `dispatcher.proxy(server_id, slot_id, incoming_req_payload)`, get back a `ResponseStream` (bytes). Wrap with a parser that emits `ChatCompletionChunk::Token(s)` for each `data: {"content":"..."}` SSE line from llama.cpp, and `ChatCompletionChunk::Stop(finish_reason)` on the final chunk. Use a lightweight SSE parser (`tokio_util::codec` LinesCodec is sufficient). | — |
| 6.6 | Cancellation: the ticket's `cancel_token` is observed by a `select!` arm; on cancellation, drop the dispatcher stream (its `Drop` releases the slot per dispatch's RAII guard), emit `DispatchEvent::Failed { error: "cancelled" }`, and end the stream. | — |
| 6.7 | Error mapping: dispatch's `DispatchError` → transport's `StreamError` (`BackendDisconnect`, `Timeout`, `ChunkParse`, `Upstream`). | — |
| 6.8 | Metrics recording: at each lifecycle point, emit `metrics::counter!` / `metrics::histogram!`. Specifically TTFT histogram on first `ChatCompletionChunk::Token`, total tokens counter on `Stop`, queue-wait histogram on transition out of `WaitingForDispatch`. | — |
| 6.9 | Unit tests: mock queue + mock dispatcher (return controlled byte stream) + real router (with one mock server entry). Cover: happy-path streaming, queue cancellation, dispatch error mid-stream, client disconnect (drop the consumer side). | tests in `inference_engine.rs` |

### Note on SmartRouter integration

`Router::select(&IncomingRequest) -> Result<RoutingDecision, RoutingError>` returns the picked server. Decide in PR6 whether the adapter calls `router.select` directly *or* whether queue admission already runs routing (and the `DispatchEvent::Dispatched` carries the answer). Read `crates/llamaforge-queue/src/pipeline.rs` to confirm; document the choice in a code comment with a design doc cite.

### PR6 Acceptance Criteria

- [ ] `impl InferenceEngine for InferenceEngineAdapter` compiles and passes the 4 unit tests above.
- [ ] Streaming path parses llama.cpp SSE into `ChatCompletionChunk` variants correctly.
- [ ] No `unwrap()` outside `Uuid::now_v7()`.
- [ ] No blocking calls under `.await` (audit explicitly: no `std::sync::Mutex::lock().await` or similar).
- [ ] 5 local gates green + 3/3 CI green.

---

## PR7 — Config schema closing

**Branch:** `integration/07-config-schema`
**Depends on:** PR2 merged (independent of PR3–PR6, can land any time)
**Size:** Medium (~250 lines)
**Estimated Time:** 4–6 hours

### Purpose

Add the config knobs that were hardcoded with `// v1.1` comments during the transport phase. Wire them through `ConfigReader::snapshot()` so the transport and orchestrator read live values instead of compile-time constants.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 7.1 | Add `[gateway]` config section | New `GatewayConfig` struct in `llamaforge-config::types`. Fields: `max_request_body_bytes: u64` (default 50MB), `large_body_threshold_mb: u64` (default 1), `max_concurrent_large_requests: usize` (default 5), `wal_checkpoint_mode: CheckpointMode` (default Passive). | `llamaforge-config/src/types.rs` |
| 7.2 | Add `[routing]` → `affinity_weight: f32` (default 0.5) — wire through `Router` to replace the hardcoded `0.0` at `llamaforge-routing/src/router.rs:239`. | `llamaforge-config/src/types.rs`, `llamaforge-routing/src/router.rs` |
| 7.3 | Add `[grpc.tls]` table: `enabled: bool` (default false), `cert_path: Option<PathBuf>`, `key_path: Option<PathBuf>`. Wire into `start_grpc_server` to call `tonic::transport::ServerTlsConfig::new().identity(...)` when enabled. | `llamaforge-config/src/types.rs`, `llamaforge-transport/src/grpc.rs` |
| 7.4 | Update `llamaforge-transport/src/lib.rs:91-95` to read `LargeBodyConcurrencyLimit::new(deps.config_reader.snapshot().gateway.max_concurrent_large_requests, deps.config_reader.snapshot().gateway.large_body_threshold_mb)`. Remove the workflow-ref-style comments. | `llamaforge-transport/src/lib.rs` |
| 7.5 | Update `llamaforge-transport/src/http.rs:459` `RequestBodyLimitLayer::new(50 * 1024 * 1024)` → `RequestBodyLimitLayer::new(config.gateway.max_request_body_bytes as usize)`. | `llamaforge-transport/src/http.rs` |
| 7.6 | Update `llamaforge-transport/src/management.rs` query handlers — parse `Query<MetricsFilter>` from the URL and pass through to `RequestMetricsStore::query_*`. Replace `MetricsFilter::default()` placeholders. Same for `Query<Vec<GroupByField>>` on `query_aggregated`. | `llamaforge-transport/src/management.rs` |
| 7.7 | Update `wal_checkpoint` handler to accept `Query<{ mode: CheckpointMode }>` (optional, default Passive). | `llamaforge-transport/src/management.rs` |
| 7.8 | Update `ConfigDiff` and `compute_diff()` to emit `GatewayConfigChanged`, `RoutingAffinityChanged`, `GrpcTlsChanged` so subscribers can react to hot-reload. | `llamaforge-config/src/diff.rs` |
| 7.9 | Unit tests: round-trip serde on the new config sections, including hot-reload diff emission. | `llamaforge-config/src/types.rs`, `llamaforge-config/tests/proptest_config.rs` |

### PR7 Acceptance Criteria

- [ ] `grep -rn 'v1.1\|Default from VC\|Default from PR' crates/` returns zero hits in `src/` files.
- [ ] `crates/llamaforge-transport/src/lib.rs:91` reads from config.
- [ ] All hardcoded `50 * 1024 * 1024`, `5`, `1` (for the body-limit constants) replaced with config reads.
- [ ] 5 local gates green + 3/3 CI green.

---

## VC-B — Adapter Layer Verification

**Gate:** PR3 + PR4 + PR5 + PR6 + PR7 merged.

| # | Check | Method | Pass criterion |
|---|-------|--------|-----------------|
| VC-B.1 | All 5 port traits have production impls | `grep -rn 'impl QueueSubmitter for\|impl ServerControl for\|impl MetricsExporter for\|impl InferenceEngine for\|impl RouteValidator for' crates/ --include='*.rs'` | each grep returns ≥1 hit OUTSIDE `tests/` or `#[cfg(test)]` |
| VC-B.2 | Config knobs config-driven | `grep -c 'v1.1\|Default from' crates/*/src/*.rs` | 0 |
| VC-B.3 | All five adapters unit-tested | `cargo test -p llamaforge-adapters` | each adapter has ≥3 tests |
| VC-B.4 | CI clean | All Actions checks SUCCESS | green |

---

## PR8 — Composition root in `crates/llamaforge/src/main.rs`

**Branch:** `integration/08-composition-root`
**Depends on:** PR3 + PR4 + PR5 + PR6 + PR7 all merged
**Size:** Medium (~400 lines)
**Estimated Time:** 6–8 hours

### Purpose

Replace the stub `main.rs` with the production composition root. Parse args, load config, construct each component, build adapters, build `TransportDeps`, call `start_servers`, install signal handlers, wait for shutdown.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 8.1 | CLI: `--config <path>` (default `config/llamaforge.toml`), `--verbose`, subcommands `serve` (default), `init`, `validate`. Use `clap` (workspace dep). | `crates/llamaforge/src/main.rs`, `crates/llamaforge/src/cli.rs` |
| 8.2 | Tracing init: read `RUST_LOG`, default `info`. Configure a `tracing_subscriber::registry().with(fmt_layer).with(opentelemetry_layer_if_configured)`. | `crates/llamaforge/src/tracing_init.rs` |
| 8.3 | Build the Tokio runtime explicitly: `Runtime::new()?` with the `multi_thread` flavor. | `main.rs` |
| 8.4 | **Component construction order** (dependency-strict): (a) `ConfigHandle`, (b) `SqliteMetricsStore` + `MetricsIngest`, (c) `MetricsExporterAdapter::install_global()` — must happen before any `metrics::counter!` call site is exercised, (d) `ServerManager` (subscribes to config diffs), (e) `Router` (subscribes to `ServerManager`'s server-changed channel), (f) `Dispatcher` + `ReqwestClientStore` (subscribes to `Router`), (g) `RequestPipeline` (subscribes to `Dispatcher` for completion events), (h) the four adapter wrappers, (i) `InferenceEngineAdapter` constructed from `queue_adapter + router + dispatcher + metrics`. | `crates/llamaforge/src/build.rs` |
| 8.5 | Construct `TransportDeps` with the adapters as `Arc<dyn Trait>` and concrete types where the trait is generic (config_writer, request_metrics_store, store_maintenance). | `crates/llamaforge/src/build.rs` |
| 8.6 | Call `llamaforge_transport::start_servers(deps).await?` and hold the returned `TransportHandles`. | `main.rs` |
| 8.7 | Signal handling: `tokio::signal::ctrl_c()` and (on Unix) `signal(SignalKind::terminate())`. On either, call `handles.shutdown()` and then `handles.wait().await`. Log `lifecycle.shutdown.started` at INFO. | `main.rs` |
| 8.8 | The `init` subcommand: write `config/llamaforge.toml.example` with all the new gateway/routing/grpc.tls knobs and prose explaining each. | `crates/llamaforge/src/init.rs` |
| 8.9 | The `validate` subcommand: parse the config file, run `ConfigHandle::load()`, and exit non-zero on any error (with a printed diagnostic). | `crates/llamaforge/src/validate.rs` |
| 8.10 | Doc comment on `main()` listing the dependency order and citing design doc §4.4 (Composition root). | `main.rs` |

### PR8 Acceptance Criteria

- [ ] `cargo run -p llamaforge -- --config config/llamaforge.toml.example` starts both servers and stays running.
- [ ] `curl http://127.0.0.1:8000/health` returns 200 within 1s of startup.
- [ ] `grpcurl -plaintext 127.0.0.1:50051 grpc.health.v1.Health/Check` returns `SERVING`.
- [ ] `kill -TERM <pid>` triggers graceful shutdown that completes within `graceful_shutdown_secs` from config.
- [ ] No `unwrap()` outside `Uuid::now_v7()`.
- [ ] 5 local gates green + 3/3 CI green.

---

## PR9 — Workspace-level integration test harness against `mock-llama`

**Branch:** `integration/09-e2e-mocklama`
**Depends on:** PR8 merged
**Size:** Medium (~500 lines, mostly tests)
**Estimated Time:** 6–8 hours

### Purpose

Build the first cross-component end-to-end test. Launches a `mock-llama` subprocess on an ephemeral port, builds the full composition root in-process (or runs `cargo run --bin llamaforge -- serve` as a subprocess), POSTs a chat-completion through the HTTP and gRPC surfaces, and verifies the response chunks arrive in the correct order.

### Tasks

| # | Task | File(s) |
|---|------|---------|
| 9.1 | Move `mock-llama` from `crates/llamaforge-server-manager/src/bin/mock-llama.rs` to its own crate `crates/mock-llama/` so it can be built independently of server-manager's test-utils feature. (Or: expose it via a `mock-llama` feature on the server-manager crate that's enabled by the new e2e crate.) | `crates/mock-llama/` (or `llamaforge-server-manager/Cargo.toml`) |
| 9.2 | New workspace member `crates/llamaforge-e2e/` with `[lib]` (no [[bin]]). Integration tests live in `crates/llamaforge-e2e/tests/`. | `crates/llamaforge-e2e/` |
| 9.3 | Test harness `LlamaForgeHarness::start()`: writes a temp config TOML pointing at the spawned `mock-llama` port, then either builds the orchestrator in-process or spawns it as a subprocess. Returns the HTTP and gRPC URLs. | `crates/llamaforge-e2e/src/harness.rs` |
| 9.4 | `test_e2e_openai_chat_streaming`: POST `/v1/chat/completions` with `stream: true`. Verify the SSE stream contains `chatcmpl-<uuid>` ids consistent across chunks, `data: [DONE]` terminator, and the tokens emitted by mock-llama. | `crates/llamaforge-e2e/tests/openai.rs` |
| 9.5 | `test_e2e_anthropic_messages_streaming`: POST `/v1/messages` with `stream: true`. Verify Anthropic event ordering (`message_start` before any `content_block_delta`). | `crates/llamaforge-e2e/tests/anthropic.rs` |
| 9.6 | `test_e2e_grpc_streaming`: gRPC client streams from `/llamaforge.inference.InferenceService/StreamInference`. | `crates/llamaforge-e2e/tests/grpc.rs` |
| 9.7 | `test_e2e_graceful_shutdown`: start, fire an in-flight request, send SIGTERM, verify the in-flight finishes and a new request is rejected. | `crates/llamaforge-e2e/tests/lifecycle.rs` |
| 9.8 | `test_e2e_metrics_endpoint`: hit `/metrics` after several requests, parse the Prometheus text, assert the expected counter names and label cardinality. | `crates/llamaforge-e2e/tests/metrics.rs` |
| 9.9 | CI workflow: add a new job `e2e-mocklama` in `.github/workflows/ci.yml` that runs `cargo test -p llamaforge-e2e --features mock-llama`. Depends on the existing `test` job. | `.github/workflows/ci.yml` |

### PR9 Acceptance Criteria

- [ ] All 5 e2e tests pass locally.
- [ ] The new `e2e-mocklama` CI job is SUCCESS on the PR's own Actions run.
- [ ] `mock-llama` is buildable from `cargo build --bin mock-llama` without needing the `test-utils` feature gymnastics.
- [ ] 5 local gates green + 4/4 CI checks SUCCESS (Check & Lint, Test, e2e-mocklama, Audit Dependencies).

---

## PR10 — Final v1 cleanup pass

**Branch:** `integration/10-v1-cleanup`
**Depends on:** PR9 merged
**Size:** Small (~150 lines)
**Estimated Time:** 3–4 hours

### Purpose

Address the remaining v1 papercuts surfaced by e2e testing and design doc reconciliation. Single PR for whatever falls out of PR9.

### Tasks (template — refine after PR9)

| # | Task |
|---|------|
| 10.1 | `ConfigOverlayStore` trait: either remove (if deferred to v1.1 per design doc §2.8) with a single `// removed during PR10` note, or land a minimal SQLite impl. Decision drives whether this is +50 or +500 lines. |
| 10.2 | Anthropic `usage.output_tokens` — wire actual token-count threading from `InferenceEngineAdapter` through `ChatCompletionChunk::Stop` (extend the `Stop` variant with a `usage: Usage` field if needed). |
| 10.3 | `tracing::error!` audit: every `tracing::error!` call in `crates/*/src/` uses structured fields, not interpolated strings. Replace any `tracing::error!("Foo: {}", e)` with `tracing::error!(error = %e, "lifecycle.foo_failed")`. |
| 10.4 | Test port handling: replace any remaining hardcoded ports in tests (`50058`, `50060`, etc.) with `0`-bind and `TcpListener::local_addr()` readback. |
| 10.5 | `cargo doc --no-deps --workspace` builds without warnings. Public-API audit: every `pub` type, trait, method has a `///` doc comment with a design doc cite. |
| 10.6 | README update at workspace root: 10-line "what is llamaforge", build instructions, link to design doc, link to running e2e tests. |

### PR10 Acceptance Criteria

- [ ] `cargo doc --no-deps --workspace` clean.
- [ ] All v1 papercuts in the design-doc reconciliation checklist (see VC-C below) closed.
- [ ] 5 local gates green + all CI checks SUCCESS.

---

## PR11 — `docs/design.md` ↔ implementation reconciliation pass

**Branch:** `integration/11-design-reconcile`
**Depends on:** PR10 merged
**Size:** Small (~50 LOC + docs)
**Estimated Time:** 3–4 hours (mostly reading)

### Purpose

For every numbered section in `docs/design.md`, find the file(s) that implement it and add a comment pointing back to the design section. This is the final "no silent drift" guarantee — if a future contributor changes a `tokens_per_second` calculation without updating §7.2, the comment will surface in code review.

### Tasks

| # | Task |
|---|------|
| 11.1 | For each design doc §, find the implementing site(s). Add a one-line `//! Implements design.md §<n>.<m>` doc comment at the top of each implementing module. |
| 11.2 | Build a `docs/design-implementation-map.md` table: design section → crate/file. Maintain as part of every future PR. |
| 11.3 | Decision log: any design item still not implemented after this phase goes in `docs/v1.1-deferred.md` with the section, the reason, and the workaround. |

### PR11 Acceptance Criteria

- [ ] `docs/design-implementation-map.md` covers every numbered section in design.md.
- [ ] `docs/v1.1-deferred.md` covers every `// TODO`, `// v1.1`, or `// Pending` comment remaining in `crates/*/src/`.
- [ ] 5 local gates green + 3/3 CI green.

---

## VC-C — v1 Sign-Off Checkpoint

**Gate:** All PRs 1–11 merged. This is the actual "LlamaForge v1.0" checkpoint.

| # | Check | Method | Pass criterion |
|---|-------|--------|-----------------|
| VC-C.1 | All tests pass | `cargo test --workspace` and `cargo test -p llamaforge-e2e` | zero failures |
| VC-C.2 | Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| VC-C.3 | No `unwrap()` audit | grep `\.unwrap()` in `crates/*/src/` excluding tests | 0 (with `Uuid::now_v7()` exception if it remains; ideally zero) |
| VC-C.4 | All five port traits production-impl | manual check VC-B.1 redux | each ≥1 impl outside tests |
| VC-C.5 | E2E green | PR9 e2e suite | all 5 tests SUCCESS |
| VC-C.6 | CI all-green | `gh pr view <last PR> --json statusCheckRollup` | every check SUCCESS |
| VC-C.7 | Composition root runs | `cargo run -p llamaforge -- serve --config config/llamaforge.toml.example` runs for 60s under `mock-llama`, accepts a request, shuts down on SIGTERM | manual smoke |
| VC-C.8 | Design ↔ impl map complete | `docs/design-implementation-map.md` covers every § | manual review |
| VC-C.9 | Deferred-items map complete | `docs/v1.1-deferred.md` covers every remaining `TODO` | manual review |
| VC-C.10 | `cargo deny check` | exit 0 | green |
| VC-C.11 | Memory bound under load | run e2e harness with 100 concurrent requests, observe RSS stays under 2GB | manual smoke |
| VC-C.12 | Documentation builds | `cargo doc --no-deps --workspace` | no warnings |

When VC-C is green, `git tag v1.0.0` and announce.

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `InferenceEngineAdapter` SSE parser doesn't match real llama.cpp output | Medium | High | PR6 unit tests use canned SSE bytes from a real llama.cpp run (commit a fixture). PR9 e2e validates against `mock-llama` AND optionally against a real `llama-server` (gated by CI runner availability). |
| Domain type changes in PR1 ripple wider than expected | Medium | Medium | Search-first approach in 1.9. The transport phase's `ApiError` and `tonic::Status` mapping are the riskiest sites. Add 1.8 to surface and fix early. |
| `mock-llama` doesn't faithfully simulate `llama-server` streaming | Low | High | PR9 tests cover the contract surface (SSE event names, chunk timing). If divergence is found, fix `mock-llama` in the same PR. |
| Tokio runtime conflict between transport's `start_servers` and `main.rs` | Low | Medium | PR8 explicitly constructs the runtime once at top level and uses `Handle::current()` everywhere downstream. No nested `Runtime::new()`. |
| `metrics_exporter_prometheus::install_recorder()` is process-global | Certain | Low | PR5.3 documents it; PR9 e2e tests that need isolation use the `metrics::with_local_recorder` API. |
| Config hot-reload doesn't propagate to running components | Medium | Medium | PR7.8 emits diff events; each component should already subscribe (config phase). Verify via PR9 lifecycle test. |
| `gh pr merge --auto` ordering footgun resurfaces | Medium | High | Apply the `project_automerge_race` memory: for any PR that touches `.github/workflows/` or `Cargo.toml` at workspace root, merge without `--auto`. Sequential merges only for foundational changes. |

---

## Timeline (Single Engineer)

| Day | Work | PR |
|-----|------|----|
| Day 1 | Domain types | PR1 |
| Day 2 | Adapter crate scaffold + queue adapter | PR2, PR3 (start) |
| Day 3 | Queue adapter complete + server-control + metrics-exporter adapters | PR3, PR4, PR5 |
| Day 4–5 | `InferenceEngineAdapter` (SSE parsing is the long pole) | PR6 |
| Day 6 | Config schema + closing | PR7 |
| Day 7 | Composition root | PR8 |
| Day 8 | E2E harness + `mock-llama` integration | PR9 |
| Day 9 | v1 cleanup + design-reconciliation | PR10, PR11 |
| Day 10 | VC-C sign-off + tag v1.0.0 | — |

---

## Summary: PR → File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `llamaforge-types/src/{lib.rs,traits.rs}`, `llamaforge-transport/src/error.rs` | ~350 |
| PR2 | `crates/llamaforge-adapters/` (new crate) | ~200 |
| PR3 | `llamaforge-adapters/src/queue.rs`, possibly `llamaforge-queue/src/pipeline.rs` | ~300 |
| PR4 | `llamaforge-adapters/src/server_control.rs` | ~200 |
| PR5 | `llamaforge-adapters/src/metrics_exporter.rs` | ~150 |
| PR6 | `llamaforge-adapters/src/inference_engine.rs` | ~600 |
| PR7 | `llamaforge-config/src/types.rs`, `llamaforge-routing/src/router.rs`, `llamaforge-transport/src/{lib.rs,http.rs,management.rs}` | ~250 |
| PR8 | `llamaforge/src/{main.rs,cli.rs,build.rs,tracing_init.rs,init.rs,validate.rs}` | ~400 |
| PR9 | `crates/mock-llama/` move, `crates/llamaforge-e2e/`, `.github/workflows/ci.yml` | ~500 |
| PR10 | `crates/*/src/` cleanup, `README.md` | ~150 |
| PR11 | `docs/design-implementation-map.md`, `docs/v1.1-deferred.md`, per-module doc comments | ~50 + docs |
| **Total** | | **~3,150** |

---

## Process Notes (carried forward from transport phase memory)

- **5-gate local verification + 3/3 CI confirmation before every relay closure.** No exceptions. The pattern that caused 15 closure/diff mismatches during the transport phase was paraphrased command output rather than literal-quoted exit codes; quote exit codes verbatim.
- **`gh pr merge --squash --delete-branch` WITHOUT `--auto`** for any PR touching `.github/workflows/` or workspace `Cargo.toml`. See `project_automerge_race` memory.
- **Closing another agent's PR (`gh pr close N --comment ...`)** may be blocked by the auto-mode permission classifier when triggered by a relay instruction. Surface to user before attempting. See `project_close_other_agent_pr` memory.
- **File:line cites in closure messages** for each closed finding. If you can't cite a line, the finding isn't resolved.

---

*Plan revision 1 — 2026-05-14. Replaces the prior `PLAN.md` (Transport Layer plan, phase 7) which is complete (PRs #70–#85 on `main`).*
