# Transport Layer — Implementation Plan

**Component:** Transport Layer (`07_TRANSPORT_LAYER.md`)  
**Estimated LOC:** ~2,400 (~1,500 production + ~900 tests)  
**Estimated Duration:** 9–10 working days (single engineer) / 7 days (two engineers, PR3/PR4/PR5 parallelizable)  
**Design Document:** `component-designs/07_TRANSPORT_LAYER.md` (R2 PASS WITH WARNINGS — all 36 findings addressed across two review passes)  
**Branch Prefix:** `transport/`  
**Target:** `main` (via squash-merge PRs)  
**Upstream Dependencies:** Config Manager (`llamaforge-config`), shared types (`llamaforge-types` with port traits), Metrics & Storage (`llamaforge-metrics`)

> **Branching workflow:** Each PR is squash-merged to `main` before the next PR branches. All `git checkout -b` commands branch from `main`, which is correct because the predecessor PR has already been merged. The "Depends on" field on each PR indicates which predecessor(s) must be merged to `main` before that PR's branch is created. Do NOT branch from a predecessor's feature branch — always branch from `main` after the predecessor is merged.

---

## Ground Rules for CoderAgents

1. **Read the design doc first.** Every PR description must reference the specific design doc sections it implements.
2. **No `unwrap()` or `expect()` in production code paths.** Use `?` propagation, `unwrap_or_else`, or explicit `match`. The one exception: `Uuid::now_v7()` is infallible (design doc §3.4 invariant 6).
3. **No blocking under `.await`.** Request parsing for bodies > 1MB uses `tokio::task::spawn_blocking`. Missing `Content-Length` defaults to spawn_blocking (assume large). See design doc §2.4 spawn_blocking contract.
4. **Every public type gets a doc comment.** Reference the design doc section number. Include `Open trait` / `Sealed trait` designation on all traits.
5. **Newtype wrappers for domain IDs.** Use `ServerId`, `RequestId` from `llamaforge-types` — never raw `String` for domain identifiers. Route parameters parse into newtypes: `Path(id): Path<ServerId>`, not `Path<String>`.
6. **Tests run with `cargo test --workspace`.** Every PR must pass before merge.
7. **`cargo clippy -- -D warnings` must pass.** Zero warnings policy.
8. **Commit messages:** `transport: <imperative summary>` (e.g., `transport: add OpenAI request parsing with null semantics`).
9. **Port traits, not concrete types.** `start_servers()` accepts `TransportDeps` — a struct of 8 `Arc<dyn Trait>` fields. No concrete upstream crate types (`ServerManager`, `RequestQueue`, `PrometheusMetrics`, `ConfigHandle`) may appear in the transport crate.
10. **DTO/Domain separation.** `OpenAiChatRequest` and `AnthropicMessagesRequest` are `pub(crate)` serde DTOs. `ChatCompletionRequest` and `ChatCompletionChunk` are domain types — they MUST NOT derive `serde::Deserialize`. All deserialization targets DTO types with explicit conversion functions.
11. **Visibility discipline.** Internal types (`SseEvent`, `AnthropicState`, `AnthropicStreamPhase`, `AnthropicProtocolError`, `LargeBodyConcurrencyLimit`, all DTOs, all handlers) are `pub(crate)`. Only types listed as `pub` in design doc §2.5 are exported.
12. **CatchPanicLayer on both servers.** HTTP and gRPC middleware stacks both have `CatchPanicLayer` as the outermost layer. No middleware panic crashes the process.

---

## Dependency Graph (PR merge order)

```
PR1 ─── PR2 ──┬── PR3 ──┬── PR6 ─── PR7 ─── PR8 ─── PR9
              ├── PR4 ──┤
              └── PR5 ──┘
```

PR3, PR4, and PR5 can be developed concurrently after PR2 merges. PR6 depends on PR3, PR4, and PR5 all being merged. PR7 → PR8 → PR9 are strictly sequential after PR6.

---

## PR1: Crate Scaffold, Proto Definitions & Port Traits

**Branch:** `transport/01-scaffold`  
**Merge Target:** `main`  
**Size:** Small (~250 lines)  
**Estimated Time:** 3–4 hours

### Purpose

Create the `llamaforge-transport` library crate, define protobuf files for gRPC, set up `build.rs` for tonic codegen, add all new workspace dependencies, and register the new port traits (`ConfigReader`, `ConfigWriter`, `ServerControl`, `QueueSubmitter`, `MetricsExporter`) in `llamaforge-types`.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 1.1 | Create `llamaforge-transport` lib crate | `cargo init --lib crates/llamaforge-transport`. This crate owns all Transport Layer code. | `crates/llamaforge-transport/` |
| 1.2 | Add workspace member | Add `crates/llamaforge-transport` to workspace `Cargo.toml` `[workspace]` members list. | `Cargo.toml` |
| 1.3 | Add workspace-level dependencies | `axum = { version = "0.8", features = ["macros"] }`, `tower = "0.5"`, `tower-http = { version = "0.6", features = ["limit", "cors", "trace", "catch-panic"] }`, `tonic = { version = "0.13", features = ["tls", "gzip"] }`, `tonic-build = "0.13"`, `prost = "0.13"`, `hyper = { version = "1.0", features = ["server", "http1", "http2"] }`, `serde_json = "1"`, `tokio-stream = "0.1"`, `reqwest = { version = "0.12", features = ["json"] }`. Note: `uuid`, `pin-project-lite`, `futures-core` already added by earlier components. | `Cargo.toml` |
| 1.4 | Wire crate dependencies | `llamaforge-transport/Cargo.toml` inherits workspace deps per design doc §1.1. Dev-deps: `tokio` (with `test-util`, `macros`), `tower` (with `util`), `reqwest` (with `json`) — include comment: `# reqwest: used for full-stack integration tests (real HTTP client → real server). Unit/handler tests use axum::test helpers instead.`. Build-deps: `tonic-build`. Include dependency rationale comments in `Cargo.toml` per design doc §1.1 dependency rationale block: explain why `llamaforge-config` and `llamaforge-metrics` are direct dependencies despite all cross-crate communication being via port traits. | `crates/llamaforge-transport/Cargo.toml` |
| 1.5 | Create proto files | `proto/llamaforge/inference.proto`: define `InferenceService` with `rpc StreamInference(InferenceRequest) returns (stream InferenceResponse)`. `proto/llamaforge/management.proto`: define `ManagementService` with RPCs for server control, queue status, config reload. Include `grpc.health.v1.Health` via tonic's built-in. | `proto/llamaforge/` |
| 1.6 | Implement `build.rs` | `tonic_build::configure().build_server(true).build_client(false).compile_protos(...)` per design doc §1.1. | `crates/llamaforge-transport/build.rs` |
| 1.7 | Create module structure | `lib.rs` declares modules: `pub(crate) mod http;`, `pub(crate) mod grpc;`, `pub(crate) mod openai;`, `pub(crate) mod anthropic;`, `pub(crate) mod management;`, `pub(crate) mod middleware;`, `mod proto;`, `pub(crate) mod error;`. All modules are `pub(crate)` because they contain only internal items — public types are re-exported from `lib.rs`. Stub each module file. Re-export `start_servers`, `TransportDeps`, `TransportHandles`, `TransportError`, `InferenceEngine`, `ApiError`, and all port traits from `lib.rs`. | `crates/llamaforge-transport/src/` |
| 1.8 | Add port traits to `llamaforge-types` | `ConfigReader` (sync, open), `ConfigWriter` (async, open), `ServerControl` (async, open), `QueueSubmitter` (sync, open — no `#[async_trait]`), `MetricsExporter` (sync, open). All with doc comments specifying "Open trait" and crate placement. Associated types (`ServerSummary`, `ServerError`, `QueueTicket`, `QueueSnapshot`, `AdmissionError`, `QueueError`, `IncomingRequest`) added as placeholder structs if not already defined. | `crates/llamaforge-types/src/traits.rs` |
| 1.9 | Implement `TransportDeps` struct | Named fields for all 8 `Arc<dyn Trait>` dependencies per design doc §2.1. | `crates/llamaforge-transport/src/lib.rs` |
| 1.10 | Implement `TransportHandles` struct | Private fields (`http_handle`, `grpc_handle`, `shutdown_tx`). Public `shutdown()` and `wait()` methods. `wait()` returns `Result<(), TransportError>` — propagates `JoinError`, does not discard it. | `crates/llamaforge-transport/src/lib.rs` |
| 1.11 | Implement error types | `TransportError` (bootstrap errors, `ServerTaskFailed` variant wrapping `JoinError`), `ApiError` (request-level errors with HTTP/gRPC dual mapping), `AnthropicProtocolError` (`InvalidTransition` variant with `from`/`to` fields of type `AnthropicStreamPhase`). All via `thiserror`. | `crates/llamaforge-transport/src/error.rs` |
| 1.12 | Define `AnthropicStreamPhase` enum | `pub(crate)` enum with variants: `Init`, `PreambleSent`, `ContentBlock`, `BlockStopped`, `MessageDone`, `Complete`. Derive `Debug, Clone, Copy, PartialEq, Eq`. This enum is defined here (not in PR2) because `AnthropicProtocolError::InvalidTransition` references it. PR2 task 2.7 uses this existing enum rather than redefining it. | `crates/llamaforge-transport/src/error.rs` |
| 1.13 | Verify build | `cargo build --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`. Proto codegen produces valid Rust. | — |

### PR1 Acceptance Criteria

- [ ] `cargo build --workspace` succeeds — proto codegen completes without errors
- [ ] All port traits compile with `Arc<dyn Trait>` (object-safe)
- [ ] `TransportDeps` struct compiles with all 8 fields
- [ ] `TransportHandles::wait()` returns `Result<(), TransportError>`
- [ ] `AnthropicProtocolError::InvalidTransition` has `from` and `to` fields of type `AnthropicStreamPhase`
- [ ] `cargo clippy --workspace -- -D warnings` passes clean
- [ ] No concrete upstream types (`ServerManager`, `RequestQueue`, `PrometheusMetrics`, `ConfigHandle`) appear in the crate

### PR1 Creation Steps

```bash
git checkout -b transport/01-scaffold main
# ... implement tasks 1.1–1.13 ...
git add -A && git commit -m "transport: bootstrap llamaforge-transport crate with proto, port traits, and error types"
git push -u origin transport/01-scaffold
# Open PR: "Transport Layer PR1: Crate Scaffold, Proto & Port Traits"
# Description: "Creates llamaforge-transport crate with tonic proto codegen,
# 5 new port traits (ConfigReader, ConfigWriter, ServerControl, QueueSubmitter,
# MetricsExporter), TransportDeps dependency bundle, TransportHandles with
# graceful shutdown, and error types. Design doc Sections 1.1, 2.1, 2.5."
```

---

## PR2: OpenAI & Anthropic Protocol Translation

**Branch:** `transport/02-protocol`  
**Depends on:** PR1 merged to `main`  
**Size:** Medium (~400 lines)  
**Estimated Time:** 6–8 hours

### Purpose

Implement the protocol translation layer: OpenAI and Anthropic request DTOs, parsing functions with null semantics, chunk/event formatting, the Anthropic SSE state machine with runtime-checked transitions, and the SseEvent wrapper. This is pure logic with no async — ideal for thorough unit testing.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 2.1 | Implement `OpenAiChatRequest` DTO | `pub(crate)` struct with `#[derive(Deserialize)]`. Fields: `model`, `messages`, `max_tokens` (with `deserialize_null_as_none`), `temperature`, `stream`, `logit_bias` (HashMap<String, f64>), `tools`, `tool_choice`. Custom deserializer for `Option<T>` fields to handle explicit `null`. | `src/openai.rs` |
| 2.2 | Implement `deserialize_null_as_none` | Custom serde deserializer that parses `null` as `None` rather than erroring. Applied to all `Option<T>` fields in OpenAI DTOs. Design doc §3.4 invariant 3. | `src/openai.rs` |
| 2.3 | Implement `parse_openai_request()` | `pub(crate) fn(OpenAiChatRequest) -> Result<ChatCompletionRequest, ApiError>`. Handles: logit_bias keys parsed as `i32` (not `u32` — negative BOS/EOS valid, design doc §3.4 invariant 4), `tool_calls` only (reject `function_call` with 400 + migration hint), stream boolean. Doc comment includes `spawn_blocking` contract with Content-Length threshold and chunked-body fallback. | `src/openai.rs` |
| 2.4 | Implement `format_openai_chunk()` | `pub(crate) fn(&ChatCompletionChunk) -> String`. Handles: empty-delta queue updates with `llamaforge` extension field, `[DONE]` sentinel, `finish_reason` mapping. Doc comment notes v2 `BytesMut` buffer consideration. | `src/openai.rs` |
| 2.5 | Implement `AnthropicMessagesRequest` DTO | `pub(crate)` struct with `#[derive(Deserialize)]`. Fields: `model`, `messages`, `system`, `max_tokens`, `stream`, `tools`, `tool_choice`. System prompt is a separate field (not in messages array). | `src/anthropic.rs` |
| 2.6 | Implement `parse_anthropic_request()` | `pub(crate) fn(AnthropicMessagesRequest) -> Result<ChatCompletionRequest, ApiError>`. Handles: system prompt extraction into first message, content block structure translation, `tool_use` format conversion. Includes `spawn_blocking` contract. | `src/anthropic.rs` |
| 2.7 | Use `AnthropicStreamPhase` enum (defined in PR1) | The enum is already defined in `error.rs` (PR1 task 1.12). Import it into `anthropic.rs` via `use crate::error::AnthropicStreamPhase;`. Do NOT redefine it. | `src/anthropic.rs` |
| 2.8 | Implement `AnthropicState` struct | `pub(crate)`. Fields: `phase: AnthropicStreamPhase`, `block_index: u32`. Constructor `new()` starts at `Init`. `transition(&mut self, to) -> Result<(), AnthropicProtocolError>` with explicit match on all valid `(from, to)` pairs. Invalid transitions return `Err` — NEVER panic, no `unreachable!()`. Design doc §3.2. | `src/anthropic.rs` |
| 2.9 | Implement `SseEvent` struct | `pub(crate)`. Fields: `event_type: &'static str`, `data: String`. `impl From<SseEvent> for axum::response::sse::Event`. Doc comment notes v2 BytesMut consideration. | `src/anthropic.rs` |
| 2.10 | Implement `format_anthropic_event()` | `pub(crate) fn(&ChatCompletionChunk, &mut AnthropicState) -> Result<Vec<SseEvent>, AnthropicProtocolError>`. Calls `state.transition()` — propagates `Err`, does not swallow it. Emits preamble events (message_start, content_block_start) synchronously before token loop. Design doc §3.2 (G-11 critical invariant). | `src/anthropic.rs` |
| 2.11 | Unit tests: OpenAI null max_tokens | `"max_tokens": null` → parsed as `None`, not 400 error. | tests in `src/openai.rs` |
| 2.12 | Unit tests: OpenAI logit_bias i32 | `{"-1": 5.0}` (BOS token) → parsed correctly, negative key preserved. | tests |
| 2.13 | Unit tests: OpenAI function_call rejection | Request with `function_call` field → 400 with migration hint message. | tests |
| 2.14 | Unit tests: OpenAI chunk formatting | Verify empty-delta queue update format, `[DONE]` sentinel, `finish_reason` on final chunk. | tests |
| 2.15 | Unit tests: Anthropic system prompt extraction | System string → first message in domain type. Messages array untouched. | tests |
| 2.16 | Unit tests: Anthropic state machine valid paths | Walk through full Init→PreambleSent→ContentBlock→BlockStopped→MessageDone→Complete. Assert all transitions succeed. | tests |
| 2.17 | Unit tests: Anthropic state machine invalid transition | Attempt Init→ContentBlock (skipping PreambleSent). Assert `Err(InvalidTransition)`. | tests |
| 2.18 | Unit tests: Anthropic event ordering | Stream 3 tokens → verify event sequence: `message_start`, `content_block_start`, 3× `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`. | tests |
| 2.19 | Unit tests: Anthropic no token before preamble | Attempt to emit `content_block_delta` from Init state → assert `Err`. | tests |

### PR2 Acceptance Criteria

- [ ] `parse_openai_request` handles `null` as `None` on all Option fields
- [ ] logit_bias keys parse as `i32` (negative values preserved)
- [ ] `function_call` rejected with 400 and migration hint
- [ ] `AnthropicState::transition()` returns `Result` — no panics, no `unreachable!()`
- [ ] `format_anthropic_event()` propagates transition errors (not swallowed)
- [ ] Preamble events emitted synchronously before any token (G-11 invariant)
- [ ] No `serde::Deserialize` on `ChatCompletionRequest` or `ChatCompletionChunk`
- [ ] All 9 unit test suites pass
- [ ] No `unwrap()` in production code

### PR2 Creation Steps

```bash
git checkout -b transport/02-protocol main
# ... implement tasks 2.1–2.19 ...
git add -A && git commit -m "transport: add OpenAI and Anthropic protocol translation with state machine"
git push -u origin transport/02-protocol
# Open PR: "Transport Layer PR2: Protocol Translation"
# Description: "Implements OpenAI and Anthropic request parsing + response formatting
# (design doc Sections 2.4, 3.2, 3.3, 3.4). OpenAI null semantics, logit_bias i32,
# function_call rejection. Anthropic SSE state machine with runtime-checked transitions.
# SseEvent wrapper. 9 unit test suites."
```

---

## VC-1: Protocol Translation Verification Checkpoint

**Gate:** PR2 merged. All protocol translation logic correct.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-1.1 | DTO/domain boundary | Attempt `#[derive(Deserialize)]` on `ChatCompletionRequest` → verify it fails (type doesn't derive it) | Does not compile |
| VC-1.2 | Null semantics | JSON `{"max_tokens": null}` round-trips through `parse_openai_request` → `None` | Assertion passes |
| VC-1.3 | State machine safety | `AnthropicState::transition(Init, ContentBlock)` returns `Err` | Assertion passes |
| VC-1.4 | Visibility enforcement | Run `cargo doc --no-deps -p llamaforge-transport` and confirm `OpenAiChatRequest` does NOT appear in the public API documentation. Alternatively, add `trybuild` as a dev-dependency and create a compile-fail test. | Not in public docs |

---

## PR3: HTTP Server & Middleware Stack

**Branch:** `transport/03-http`  
**Depends on:** PR2 merged to `main`  
**Size:** Medium (~350 lines)  
**Estimated Time:** 6–8 hours

### Purpose

Build the axum HTTP server: route registration, the Tower middleware stack (CatchPanicLayer, body limit, large body concurrency limiter, auth, request ID injection, metrics), and the inference endpoint handlers that wire protocol translation to `InferenceEngine`.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 3.1 | Implement `LargeBodyConcurrencyLimit` middleware | Tower `Layer` + `Service` implementation. `Semaphore`-based, configured by `max_concurrent_large_requests` and `large_body_threshold_mb`. Returns 429 when semaphore exhausted. Design doc §Memory Bounds. | `src/middleware.rs` |
| 3.2 | Implement auth middleware | Tower layer. Takes `Arc<dyn ConfigReader>` as constructor parameter (extracted from `TransportDeps` in PR6). If `api_key` configured: check `Authorization: Bearer <key>` header. If empty: pass-through. Returns 401 on mismatch. Hot-reloadable (calls `config.transport_config()` on each request to get current `api_key`). Record `llamaforge_auth_failures_total` metric. PR3 tests pass a mock `ConfigReader` directly. Design doc §3.4 invariant 7. | `src/middleware.rs` |
| 3.3 | Implement request ID middleware | Tower layer. Generate `RequestId::from(Uuid::now_v7())` — infallible construction, no `unwrap()`. Set both `x-request-id` and `x-llamaforge-request-id` response headers. Store in request extensions for downstream handlers. Design doc §3.4 invariant 6, §9 decision 5. | `src/middleware.rs` |
| 3.4 | Implement metrics middleware | Tower layer. Record `llamaforge_http_requests_total` (counter, labels: method/path/status), `llamaforge_http_request_duration_seconds` (histogram, labels: method/path), `llamaforge_active_connections` (gauge). `path` label bounded to ~15 known routes (use route pattern, NOT client-supplied path). Design doc §7.1. | `src/middleware.rs` |
| 3.5 | Implement `openai_chat_completion_handler` | Axum handler. Extract `Json<OpenAiChatRequest>`. For bodies > 1MB: use `spawn_blocking` for `parse_openai_request`. Early rejection via `route_validator.validate_routable()`. Call `engine.chat_completion()`. Non-streaming: wrap in `tokio::time::timeout(non_streaming_timeout_secs)` (design doc §3.4 invariant 9). Streaming: SSE response with `format_openai_chunk`, no outer timeout. | `src/http.rs` |
| 3.6 | Implement `openai_completion_handler` | Axum handler. Wraps text prompt as single user message → delegates to `chat_completion()`. Design doc §2.3. | `src/http.rs` |
| 3.7 | Implement `anthropic_messages_handler` | Axum handler. Similar to 3.5 but uses `parse_anthropic_request` and `format_anthropic_event`. Preamble events emitted synchronously before token loop. State machine errors converted to `ApiError::Internal`. | `src/http.rs` |
| 3.8 | Implement health and metrics endpoints | `GET /health` → 200 with `{"status": "ok"}`. `GET /metrics` → `metrics_exporter.gather_text()`, always returns 200 (empty on degraded). Design doc §4.2. | `src/http.rs` |
| 3.9 | Build axum Router with middleware stack | Layer ordering (outermost → innermost): `CatchPanicLayer` → body limit → `LargeBodyConcurrencyLimit` → auth → request ID → metrics. NO `TimeoutLayer`. Register all routes from design doc §2.3. | `src/http.rs` |
| 3.10 | Implement HTTP server startup | Bind to `config.bind_address:config.http_port`. Use axum's graceful shutdown with watch channel from `TransportHandles`. Log `transport.started` at INFO. | `src/http.rs` |
| 3.11 | Implement `ApiError` → HTTP response mapping | JSON error bodies: `{"error": "<code>", "message": "<detail>"}`. Status mapping per design doc §4.3. `Queue full` includes `Retry-After` header. | `src/error.rs` |
| 3.12 | Unit tests: auth middleware pass-through | Empty `api_key` config → request without auth header succeeds. | tests |
| 3.13 | Unit tests: auth middleware rejection | `api_key` set → request without header returns 401. | tests |
| 3.14 | Unit tests: request ID injection | Request → verify both `x-request-id` and `x-llamaforge-request-id` headers in response. | tests |
| 3.15 | Unit tests: body limit rejection | 300MB body → 413 with structured error. | tests |
| 3.16 | Unit tests: large body concurrency limit | Semaphore full → 429. | tests |
| 3.17 | Axum integration test: full streaming path | Mock `InferenceEngine` returning 3 chunks. POST `/v1/chat/completions` with `stream: true` → verify SSE events in order, `[DONE]` terminal. | tests |
| 3.18 | Axum integration test: non-streaming timeout | Mock engine that sleeps 400s. `non_streaming_timeout_secs = 1`. Assert 504 Gateway Timeout. | tests |

### PR3 Acceptance Criteria

- [ ] Middleware stack ordering: CatchPanic (outermost) → body limit → large body limiter → auth → request ID → metrics
- [ ] NO `TimeoutLayer` in middleware stack
- [ ] Non-streaming timeout applied at handler level via `tokio::time::timeout`
- [ ] Streaming responses have no outer timeout
- [ ] `path` metric label uses route pattern, not client path
- [ ] Auth middleware hot-reloads (reads config on each request)
- [ ] Request ID generated via infallible `Uuid::now_v7()`
- [ ] All 7 test suites pass

### PR3 Creation Steps

```bash
git checkout -b transport/03-http main
# ... implement tasks 3.1–3.18 ...
git add -A && git commit -m "transport: implement HTTP server with middleware stack and inference handlers"
git push -u origin transport/03-http
# Open PR: "Transport Layer PR3: HTTP Server & Middleware"
# Description: "Implements axum HTTP server (design doc Sections 3.1, 3.4, 7.1).
# 6-layer middleware stack with CatchPanicLayer, LargeBodyConcurrencyLimit,
# auth, request ID, and metrics. OpenAI and Anthropic inference handlers with
# spawn_blocking for large bodies. Non-streaming timeout at handler level.
# 7 test suites including streaming integration test."
```

---

## PR4: gRPC Server & Service Implementations

**Branch:** `transport/04-grpc`  
**Depends on:** PR2 merged to `main` (can be developed concurrently with PR3)  
**Size:** Medium (~300 lines)  
**Estimated Time:** 5–7 hours

### Purpose

Build the tonic gRPC server: `InferenceService` with server-streaming RPC, `ManagementService` with control RPCs, health checking, interceptors (auth, CatchPanicLayer), and gRPC-specific middleware configuration (keepalive, max message size, concurrent streams).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 4.1 | Implement `InferenceService` | Tonic service implementing `StreamInference` RPC. Translates proto request → `ChatCompletionRequest` → calls `engine.chat_completion()` → streams proto response chunks. | `src/grpc.rs` |
| 4.2 | Implement `ManagementService` | Tonic service implementing RPCs: `ListServers`, `StartServer`, `StopServer`, `QueueStatus`, `CancelRequest`, `ReloadConfig`. Delegates to port traits (`ServerControl`, `QueueSubmitter`, `ConfigWriter`). | `src/grpc.rs` |
| 4.3 | Implement auth interceptor | Tonic interceptor. Checks `authorization` metadata key. Same logic as HTTP auth but adapted for gRPC metadata. Returns `UNAUTHENTICATED` on failure. | `src/grpc.rs` |
| 4.4 | Implement `ApiError` → gRPC status mapping | Map `ApiError` variants to gRPC status codes per design doc §4.3: `InvalidBody` → `INVALID_ARGUMENT`, `AuthFailure` → `UNAUTHENTICATED`, `ModelNotFound` → `NOT_FOUND`, `QueueFull` → `RESOURCE_EXHAUSTED`, `BackendError` → `INTERNAL`, `Timeout` → `DEADLINE_EXCEEDED`. | `src/error.rs` |
| 4.5 | Build tonic Server with middleware | `Server::builder().layer(CatchPanicLayer::new())` (outermost). Configure using exact config key names from design doc §6: `grpc.max_message_size_mb` (default 16), `grpc.keepalive_interval_secs` (default 30), `grpc.keepalive_timeout_secs` (default 10), `grpc.max_concurrent_streams` (default 1000). Optional TLS. Add all services + health service. Design doc §3.1 gRPC section. | `src/grpc.rs` |
| 4.6 | Implement gRPC server startup | Bind to `config.bind_address:config.grpc_port`. Graceful shutdown via watch channel. GOAWAY frame on shutdown. | `src/grpc.rs` |
| 4.7 | Add gRPC metrics | `llamaforge_grpc_requests_total` (counter, labels: method/status), `llamaforge_grpc_request_duration_seconds` (histogram, labels: method), `llamaforge_active_connections{transport=grpc}` (gauge). Design doc §7.1. | `src/grpc.rs` |
| 4.8 | Unit tests: gRPC auth interceptor | Request without metadata key → `UNAUTHENTICATED`. | tests |
| 4.9 | Unit tests: gRPC error status mapping | Verify all `ApiError` variants map to correct gRPC status codes. | tests |
| 4.10 | Integration test: gRPC streaming | Compile proto client. Stream 3 chunks from mock engine → verify proto response messages. | tests |
| 4.11 | Integration test: gRPC health check | Call `grpc.health.v1.Health/Check` → verify `SERVING`. | tests |

### PR4 Acceptance Criteria

- [ ] `CatchPanicLayer` is outermost gRPC middleware layer
- [ ] `grpc.max_message_size_mb`, `grpc.keepalive_interval_secs`, `grpc.keepalive_timeout_secs`, `grpc.max_concurrent_streams` configured per design doc §6 using exact config key names
- [ ] All `ApiError` variants mapped to both HTTP and gRPC status codes
- [ ] Health checking uses standard `grpc.health.v1.Health` protocol
- [ ] GOAWAY sent on graceful shutdown
- [ ] All 4 test suites pass

### PR4 Creation Steps

```bash
git checkout -b transport/04-grpc main
# ... implement tasks 4.1–4.11 ...
git add -A && git commit -m "transport: implement gRPC server with inference and management services"
git push -u origin transport/04-grpc
# Open PR: "Transport Layer PR4: gRPC Server & Services"
# Description: "Implements tonic gRPC server (design doc Sections 3.1, 4.3).
# InferenceService (streaming), ManagementService (control RPCs), health check,
# auth interceptor, CatchPanicLayer, keepalive/message-size/concurrent-streams config.
# gRPC error status mapping. 4 test suites."
```

---

## PR5: Management REST API

**Branch:** `transport/05-management`  
**Depends on:** PR2 merged to `main` (can be developed concurrently with PR3 and PR4)  
**Size:** Small (~200 lines)  
**Estimated Time:** 3–4 hours

### Purpose

Implement the management REST API handlers: server control, queue inspection, config reload, metrics queries, and WAL checkpoint. These are simple CRUD-style handlers delegating to port traits.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 5.1 | Implement `list_servers` handler | `GET /api/servers` → `server_control.list_servers()` → JSON array of `ServerSummary`. | `src/management.rs` |
| 5.2 | Implement `start_server` handler | `POST /api/servers/{id}/start` → `Path(id): Path<ServerId>` → `server_control.start_server(&id)`. Returns 200 or error. | `src/management.rs` |
| 5.3 | Implement `stop_server` handler | `POST /api/servers/{id}/stop` → `Path(id): Path<ServerId>` → `server_control.stop_server(&id)`. | `src/management.rs` |
| 5.4 | Implement `queue_status` handler | `GET /api/queue` → `queue.status()` → JSON `QueueSnapshot`. | `src/management.rs` |
| 5.5 | Implement `cancel_request` handler | `DELETE /api/queue/{id}` → `Path(id): Path<RequestId>` → `queue.cancel(&id)`. | `src/management.rs` |
| 5.6 | Implement `reload_config` handler | `POST /api/config/reload` → `config_writer.reload()`. Returns 200 on success. | `src/management.rs` |
| 5.7 | Implement `query_request_metrics` and `query_aggregated` handlers | `GET /api/metrics/requests` and `GET /api/metrics/aggregated` → `metrics_store.query_*()`. | `src/management.rs` |
| 5.8 | Implement `wal_checkpoint` handler | `POST /api/maintenance/wal-checkpoint` → `metrics_store.wal_checkpoint()`. If `wal_checkpoint()` does not yet exist on the `MetricsStore` trait, add it to `llamaforge-types` as part of PR1 task 1.8: `fn wal_checkpoint(&self) -> Result<(), MetricsError>;`. | `src/management.rs` |
| 5.9 | Build standalone management sub-router | Build a `pub(crate) fn management_routes(deps: ...) -> Router` function returning a sub-router with all `/api/*` routes. This sub-router is merged into the main HTTP Router in PR6 task 6.1. Do NOT attempt to register routes on the main axum Router here — PR3's HTTP server infrastructure may not be merged yet. | `src/management.rs` |
| 5.10 | Unit tests: route parameter parsing | `/api/servers/{id}/start` with valid `ServerId` → 200. Invalid ID format → 400. | tests |
| 5.11 | Unit tests: config reload | Mock `ConfigWriter`. Call reload → verify `reload()` called. | tests |

### PR5 Acceptance Criteria

- [ ] All route parameters parse into domain newtypes (`ServerId`, `RequestId`)
- [ ] All handlers delegate to port traits — no concrete upstream types
- [ ] Config reload uses `ConfigWriter` (not `ConfigReader` — interface segregation)
- [ ] All 2 test suites pass

### PR5 Creation Steps

```bash
git checkout -b transport/05-management main
# ... implement tasks 5.1–5.11 ...
git add -A && git commit -m "transport: implement management REST API handlers"
git push -u origin transport/05-management
# Open PR: "Transport Layer PR5: Management REST API"
# Description: "Implements /api/* management handlers (design doc Sections 2.3, 4.1).
# Server control, queue inspection, config reload via ConfigWriter, metrics queries.
# All route parameters parse into newtypes. 2 test suites."
```

---

## VC-2: Server Components Verification Checkpoint

**Gate:** PR3 + PR4 + PR5 merged. Both servers and management API work independently.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-2.1 | HTTP streaming end-to-end | POST `/v1/chat/completions` with mock engine → full SSE stream received | All events present |
| VC-2.2 | Anthropic SSE ordering | POST `/v1/messages` → verify strict event ordering (message_start before any delta) | Event order correct |
| VC-2.3 | gRPC streaming end-to-end | `StreamInference` RPC with mock engine → all proto response chunks received | All chunks present |
| VC-2.4 | Auth enforcement | Both HTTP and gRPC reject requests without valid key when configured | 401/UNAUTHENTICATED |
| VC-2.5 | Middleware panic recovery | Inject panic in auth middleware → verify 500 returned, process survives | No process crash |
| VC-2.6 | Large body concurrency | 20 concurrent 15MB requests with `max_concurrent_large_requests=5` → exactly 5 proceed, 15 get 429 | Counts match |
| VC-2.7 | Management API | GET/POST/DELETE through all management endpoints → correct responses | All endpoints respond |

---

## PR6: Server Bootstrap & Wiring

**Branch:** `transport/06-bootstrap`  
**Depends on:** PR3, PR4, PR5 all merged to `main`  
**Size:** Small (~200 lines)  
**Estimated Time:** 3–5 hours

### Purpose

Implement `start_servers()` — the composition root function that wires HTTP and gRPC servers together with the shared `TransportDeps`, starts both on their configured ports, and returns `TransportHandles`. This also wires in the management routes to the HTTP server.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 6.1 | Implement `start_servers()` | Accept `TransportDeps`. Read `TransportConfig` from `deps.config.transport_config()`. Build axum Router (inference + management + health + metrics routes, full middleware stack). Build tonic Server (inference + management + health services, interceptors). Spawn both as `tokio::spawn` tasks. Return `TransportHandles` with join handles and shutdown watch channel. | `src/lib.rs` |
| 6.2 | Wire shared state into handlers | Pass `Arc<dyn Trait>` references into axum handlers via `axum::Extension` or `State`. Both HTTP and gRPC handlers share the same `TransportDeps` references. | `src/lib.rs` |
| 6.3 | Implement graceful shutdown coordination | `TransportHandles::shutdown()` sends `()` on watch channel. Both servers receive it and stop accepting new connections. HTTP: axum graceful shutdown. gRPC: GOAWAY frame. In-flight requests drain until server-level timeout. Design doc §Cross-Cutting: Graceful Shutdown. | `src/lib.rs` |
| 6.4 | Add structured log events | `transport.started` at INFO: ports, bind address, TLS status. `transport.request` at DEBUG: method, path, request_id, content-length. `transport.auth_failed` at WARN: IP, masked key prefix. `transport.body_too_large` at WARN: received bytes, limit. Design doc §7.2. | `src/http.rs`, `src/grpc.rs` |
| 6.5 | Integration test: dual-server startup | Call `start_servers()` with mock deps. Verify both HTTP and gRPC ports accept connections. Shutdown → verify both exit cleanly. | tests |
| 6.6 | Integration test: graceful shutdown | Start servers. Send in-flight request. Call `shutdown()`. Verify in-flight completes. New requests rejected. `wait()` returns `Ok(())`. | tests |
| 6.7 | Integration test: prometheus endpoint | `GET /metrics` → 200 with Prometheus text format. Even when `metrics_exporter.gather_text()` returns empty string. Design doc §4.2. | tests |

### PR6 Acceptance Criteria

- [ ] `start_servers()` accepts only `TransportDeps` — no other parameters
- [ ] Both HTTP and gRPC servers start on configured ports
- [ ] Graceful shutdown drains in-flight requests
- [ ] `TransportHandles::wait()` returns `Ok(())` on clean shutdown, `Err` on task panic
- [ ] All structured log events use exact field names from design doc §7.2
- [ ] All 3 test suites pass

### PR6 Creation Steps

```bash
git checkout -b transport/06-bootstrap main
# ... implement tasks 6.1–6.7 ...
git add -A && git commit -m "transport: implement start_servers bootstrap and graceful shutdown"
git push -u origin transport/06-bootstrap
# Open PR: "Transport Layer PR6: Server Bootstrap & Wiring"
# Description: "Implements start_servers() composition root (design doc Sections 2.1, 3.1).
# Wires HTTP + gRPC servers with shared TransportDeps. Graceful shutdown with drain.
# Structured log events. 3 integration tests including dual-server and shutdown."
```

---

## PR7: Error Response Formatting & Edge Cases

**Branch:** `transport/07-errors`  
**Depends on:** PR6 merged to `main`  
**Size:** Small (~200 lines)  
**Estimated Time:** 3–4 hours

### Purpose

Polish error response formatting for all failure modes, implement CORS configuration, and handle edge cases: mid-stream errors, client disconnects, body limit responses, queue full with Retry-After.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 7.1 | Implement structured JSON error bodies | All HTTP error responses: `{"error": "<code>", "message": "<detail>", "request_id": "<uuid>"}`. Include `request_id` when available. Design doc §4.3. | `src/error.rs` |
| 7.2 | Implement `Retry-After` header for 429 | Queue full → HTTP 429 with `Retry-After: <seconds>` header. Compute from queue depth / throughput estimate. Design doc §4.2. | `src/error.rs`, `src/http.rs` |
| 7.3 | Implement mid-stream error events | Backend error during streaming → emit SSE error event (OpenAI: error object in data field; Anthropic: `error` event type). Do NOT close connection without signaling. | `src/http.rs` |
| 7.4 | Implement client disconnect detection | Response body write error → log at DEBUG, cancel upstream via `cancel_token`. Design doc §FM-1. | `src/http.rs` |
| 7.5 | Implement CORS middleware | Tower CORS layer from `tower-http`. v1 CORS config: `cors.allowed_origins` (default `["*"]`). Methods default to `["GET", "POST", "DELETE", "OPTIONS"]`. Headers default to `["Content-Type", "Authorization"]`. Max-age 3600s. No config knobs for methods/headers/max-age in v1 — hardcoded defaults. Only `cors.allowed_origins` is configurable and hot-reloadable. Design doc §9 decision 4. Note: design doc §6 does not yet include `[cors]` entries — this is a known gap; the config knob is defined here as the authoritative spec for v1. | `src/middleware.rs` |
| 7.6 | Implement body limit error response | 413 with `{"error": "request_body_too_large", "max_bytes": N, "received_bytes": M}`. Design doc §FM-2. | `src/middleware.rs` |
| 7.7 | Unit tests: JSON error format | Verify all error responses have `error`, `message`, `request_id` fields. | tests |
| 7.8 | Unit tests: Retry-After computation | Queue at 80% capacity → verify `Retry-After` header present and reasonable. | tests |
| 7.9 | Unit tests: mid-stream error event | Mock engine returns error after 2 chunks → verify error event emitted before stream closes. | tests |
| 7.10 | Unit tests: CORS headers | Request with `Origin` header → verify `Access-Control-Allow-Origin` in response. | tests |

### PR7 Acceptance Criteria

- [ ] All HTTP error responses include structured JSON with `error`, `message`, `request_id`
- [ ] Queue full returns 429 with `Retry-After` header
- [ ] Mid-stream errors produce SSE error events (not silent disconnect)
- [ ] Client disconnect triggers cancel_token
- [ ] CORS configured per `[cors]` config section
- [ ] All 4 test suites pass

### PR7 Creation Steps

```bash
git checkout -b transport/07-errors main
# ... implement tasks 7.1–7.10 ...
git add -A && git commit -m "transport: add error formatting, CORS, mid-stream errors, and Retry-After"
git push -u origin transport/07-errors
# Open PR: "Transport Layer PR7: Error Formatting & Edge Cases"
# Description: "Polishes error responses (design doc Sections 4.2, 4.3, 5.0).
# Structured JSON error bodies, Retry-After for 429, mid-stream error events,
# client disconnect handling, CORS middleware. 4 test suites."
```

---

## VC-3: Full Transport Layer Verification Checkpoint

**Gate:** PR1–PR7 merged. The complete transport layer works end-to-end.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-3.1 | OpenAI streaming lifecycle | POST `/v1/chat/completions` stream=true → queue update events → token events → [DONE] | All events present in order |
| VC-3.2 | Anthropic streaming lifecycle | POST `/v1/messages` stream=true → strict event ordering verified | message_start before any delta |
| VC-3.3 | gRPC streaming lifecycle | `StreamInference` RPC → all response chunks received | Complete stream |
| VC-3.4 | Non-streaming with timeout | POST `/v1/chat/completions` stream=false, engine sleeps 400s, timeout=1s → 504 | GatewayTimeout |
| VC-3.5 | Queue full → 429 | Mock queue returns `AdmissionError::QueueFull` → 429 + Retry-After header | Header present |
| VC-3.6 | Auth on both transports | HTTP without key → 401. gRPC without metadata → UNAUTHENTICATED | Both reject |
| VC-3.7 | Client disconnect cleanup | Start streaming. Close client TCP. Verify cancel_token fires. | Cancellation detected |
| VC-3.8 | Mid-stream backend error | Engine yields 2 chunks then error → SSE error event emitted before stream closes | Error event present |
| VC-3.9 | Prometheus always 200 | `GET /metrics` when metrics_exporter returns empty → 200 with empty text | Status 200 |
| VC-3.10 | Management API round-trip | List servers → start server → stop server → config reload → queue status | All return success |
| VC-3.11 | Large body concurrency limit | 20 × 15MB concurrent, semaphore=5 → 5 proceed, 15 get 429 | Counts match |
| VC-3.12 | Graceful shutdown drain | Start streaming request. Call shutdown. Verify stream completes. New request rejected. | Stream completes |
| VC-3.13 | Panic recovery HTTP | Inject panic in handler → 500 returned, next request succeeds | Process survives |
| VC-3.14 | Panic recovery gRPC | Inject panic in service method → INTERNAL returned, next RPC succeeds | Process survives |
| VC-3.15 | Memory bounds | 16 concurrent 200MB requests + 10000 connections → total < 3.5GB | Memory assertion |

**Integration test file:** `tests/vc3_transport.rs`

---

## PR8: Design Doc Test Cases & Integration Tests

**Branch:** `transport/08-tests`  
**Depends on:** PR7 merged to `main`  
**Size:** Medium (~350 lines of tests)  
**Estimated Time:** 5–7 hours

### Purpose

Implement all specific test cases from design doc Section 8.3, the 15 VC-3 integration tests, and edge case tests for all failure modes. This PR is pure test code — no production logic changes.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 8.1 | Implement design doc test: `test_openai_null_max_tokens_accepted` | Design doc §8.3. | `tests/design_doc_tests.rs` |
| 8.2 | Implement design doc test: `test_openai_logit_bias_negative_keys` | Design doc §8.3. | `tests/design_doc_tests.rs` |
| 8.3 | Implement design doc test: `test_anthropic_state_machine_ordering` | Design doc §8.3. 3 tokens → verify full event sequence. | `tests/design_doc_tests.rs` |
| 8.4 | Implement design doc test: `test_anthropic_no_token_before_preamble` | Design doc §8.3. Fast dispatch (TTFT < 1ms). | `tests/design_doc_tests.rs` |
| 8.5 | Implement design doc test: `test_queue_full_returns_429_with_retry_after` | Design doc §8.3. | `tests/design_doc_tests.rs` |
| 8.6 | Implement design doc test: `test_auth_required_when_configured` | Design doc §8.3. | `tests/design_doc_tests.rs` |
| 8.7 | Implement design doc test: `test_prometheus_endpoint_always_200` | Design doc §8.3. Degraded metrics store. | `tests/design_doc_tests.rs` |
| 8.8 | Implement design doc test: `test_streaming_no_timeout_layer` | Design doc §8.3. Stream for 120 seconds — no timeout. | `tests/design_doc_tests.rs` |
| 8.9 | Implement all 15 VC-3 integration tests | Per VC-3 table above. Each as a separate `#[tokio::test]` function. | `tests/vc3_transport.rs` |
| 8.10 | Implement mock trait infrastructure | `MockInferenceEngine`, `MockServerControl`, `MockQueueSubmitter`, `MockConfigReader`, `MockConfigWriter`, `MockMetricsExporter`, `MockRouteValidator`, `MockMetricsStore`. Shared across test files. | `tests/mocks.rs` |
| 8.11 | Tag slow tests with `#[ignore]` | Long-running tests (streaming 120s, memory bounds, concurrent stress) tagged for separate CI run. | — |
| 8.12 | Final CI check | `cargo test --workspace`, `cargo test --workspace -- --ignored`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`. | — |

### PR8 Acceptance Criteria

- [ ] All 8 design doc test cases pass
- [ ] All 15 VC-3 integration tests pass
- [ ] Mock infrastructure covers all 8 port traits
- [ ] `cargo test --workspace` passes (excluding `#[ignore]` tests)
- [ ] `cargo test --workspace -- --ignored` passes (including slow tests)
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] No production code changes in this PR — test code only. Panic injection tests (VC-3.13/3.14) use a mock `InferenceEngine` implementation that panics on `chat_completion()`. The panic propagates through the real middleware stack and is caught by `CatchPanicLayer`. No `#[cfg(test)]` annotations or production-side test hooks needed.

### PR8 Creation Steps

```bash
git checkout -b transport/08-tests main
# ... implement tasks 8.1–8.12 ...
git add -A && git commit -m "transport: add design doc test cases, VC-3 integration tests, and mock infrastructure"
git push -u origin transport/08-tests
# Open PR: "Transport Layer PR8: Integration Tests & Design Doc Test Cases"
# Description: "Pure test PR. Adds 8 design doc §8.3 test cases, 15 VC-3
# integration tests, and mock infrastructure for all 8 port traits. Total:
# 23 new test functions. No production code changes."
```

---

## PR9: Observability Polish & Documentation

**Branch:** `transport/09-observability`  
**Depends on:** PR8 merged to `main`  
**Size:** Small (~150 lines)  
**Estimated Time:** 2–3 hours

### Purpose

Final polish: ensure all 6 Prometheus metrics are registered and recorded, all structured log events use correct field names, `cargo doc` builds cleanly, and the public API surface matches the design doc §2.5 visibility table.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 9.1 | Audit all Prometheus metrics | Cross-check design doc §7.1 table against actual metric recording sites. Verify all 6 metrics registered, labels correct, cardinality bounded. | `src/middleware.rs`, `src/grpc.rs` |
| 9.2 | Audit all structured log events | Cross-check design doc §7.2 table. Verify exact field names, levels (INFO/DEBUG/WARN), frequency. | all source files |
| 9.3 | Audit public API surface | `cargo doc --no-deps -p llamaforge-transport`. Verify re-exports match §2.5 visibility table. Internal types (`SseEvent`, `AnthropicState`, all DTOs, all handlers) NOT in public docs. | `src/lib.rs` |
| 9.4 | Add module-level doc comments | Each source file gets a top-level `//!` doc comment explaining its role and design doc reference. | all source files |
| 9.5 | Verify no `unwrap()` | `grep -rn "unwrap()" crates/llamaforge-transport/src/ --include="*.rs"` — zero bare `unwrap()`. Only allowed: `Uuid::now_v7()` (infallible). | — |
| 9.6 | Verify no concrete upstream types | `grep -rn "ServerManager\|RequestQueue\|PrometheusMetrics\|ConfigHandle" crates/llamaforge-transport/src/ --include="*.rs"` — zero matches. | — |

### PR9 Acceptance Criteria

- [ ] `cargo doc --no-deps -p llamaforge-transport` builds without warnings
- [ ] All 6 metrics from design doc §7.1 are recorded
- [ ] All 4 log events from design doc §7.2 use correct field names and levels
- [ ] Public API matches §2.5 — no internal types leaked
- [ ] Zero bare `unwrap()` in production code
- [ ] Zero references to concrete upstream types

### PR9 Creation Steps

```bash
git checkout -b transport/09-observability main
# ... implement tasks 9.1–9.6 ...
git add -A && git commit -m "transport: audit metrics, logging, docs, and public API surface"
git push -u origin transport/09-observability
# Open PR: "Transport Layer PR9: Observability Polish & Documentation"
# Description: "Final audit PR. Verifies all 6 Prometheus metrics, 4 log events,
# public API surface against design doc. Adds module doc comments.
# Zero unwrap(), zero concrete upstream type references."
```

---

## VC-4: Final Sign-Off Checkpoint

**Gate:** All 9 PRs merged. Transport Layer is complete.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-4.1 | All tests pass | `cargo test --workspace && cargo test --workspace -- --ignored` | Zero failures |
| VC-4.2 | Zero clippy warnings | `cargo clippy --workspace -- -D warnings` | Exit code 0 |
| VC-4.3 | Code coverage | `cargo llvm-cov --workspace`. Target > 85% line coverage for `llamaforge-transport`. | Coverage report generated |
| VC-4.4 | Public API audit | Review `lib.rs` re-exports. Verify only §2.5 `pub` types are exported. All DTOs, state machine types, middleware, and handlers are `pub(crate)`. | Manual review |
| VC-4.5 | Design doc compliance | Cross-check every Section in `07_TRANSPORT_LAYER.md` against the implementation. All 9 invariants (§3.4) verified. | Manual review with checklist |
| VC-4.6 | Documentation | Every public type, trait, method, and enum variant has a doc comment. `cargo doc --no-deps -p llamaforge-transport` builds without warnings. | `cargo doc` succeeds |
| VC-4.7 | Dependency audit | `cargo deny check` passes. No duplicate dependency versions. Transport crate depends on `llamaforge-types`, `llamaforge-config`, `llamaforge-metrics` only — NOT on queue, dispatch, server-mgr, or routing crates. | `cargo deny` green |
| VC-4.8 | Binary size check | `cargo build --release -p llamaforge`. Binary includes transport crate. Reasonable size. | Build succeeds |
| VC-4.9 | No `unwrap()` audit | `grep -rn "unwrap()" crates/llamaforge-transport/src/ --include="*.rs"` — zero bare `unwrap()` | Zero matches |
| VC-4.10 | No concrete upstream types | `grep -rn "ServerManager\|RequestQueue\|PrometheusMetrics\|ConfigHandle" crates/llamaforge-transport/src/ --include="*.rs"` | Zero matches |
| VC-4.11 | Proto codegen clean | `cargo build -p llamaforge-transport` from clean state — proto codegen succeeds | Build succeeds |
| VC-4.12 | Dual-server smoke test | Start both servers. HTTP and gRPC accept connections. Shutdown cleanly. | Both respond |

---

## Summary: PR → File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `crates/llamaforge-transport/src/lib.rs`, `src/error.rs`, `build.rs`, `proto/`, `llamaforge-types/src/traits.rs`, `Cargo.toml` | ~250 |
| PR2 | `src/openai.rs`, `src/anthropic.rs` | ~400 |
| PR3 | `src/http.rs`, `src/middleware.rs` | ~350 |
| PR4 | `src/grpc.rs`, `src/error.rs` (additions) | ~300 |
| PR5 | `src/management.rs` | ~200 |
| PR6 | `src/lib.rs` (additions), `src/http.rs` (additions), `src/grpc.rs` (additions) | ~200 |
| PR7 | `src/error.rs` (additions), `src/http.rs` (additions), `src/middleware.rs` (additions) | ~200 |
| PR8 | `tests/design_doc_tests.rs`, `tests/vc3_transport.rs`, `tests/mocks.rs` | ~350 |
| PR9 | All source files (doc comments, audit fixes) | ~150 |
| **Total** | | **~2,400** |

> Note: Total ~2,400 LOC = ~1,500 production + ~900 tests. Production exceeds the design doc's ~2,000 estimate when accounting for proto codegen infrastructure (~100 lines). The summary table per-PR estimates above add to ~2,400, consistent with the header.

---

## Timeline (Single Engineer)

| Day | Work | PRs |
|-----|------|-----|
| Day 1 | Crate scaffold + proto + port traits + error types | PR1 |
| Day 2 | OpenAI protocol translation + tests | PR2 (part 1) |
| Day 3 | Anthropic protocol translation + state machine + tests | PR2 (part 2) |
| Day 4 | HTTP server + middleware stack | PR3 |
| Day 5 | gRPC server + service implementations | PR4 |
| Day 6 | Management REST API + server bootstrap wiring | PR5, PR6 |
| Day 7 | Error formatting, CORS, edge cases | PR7 |
| Day 8 | Design doc tests + VC-3 integration tests | PR8 |
| Day 9 | Observability polish + documentation audit | PR9 |
| Day 10 | VC-4 verification, code review fixes | — |

## Timeline (Two Engineers)

| Day | Engineer A | Engineer B |
|-----|-----------|-----------|
| Day 1 | PR1 (scaffold + proto + traits) | Reviews PR1, writes mock trait impls for testing |
| Day 2 | PR2 (protocol translation) | Reviews PR2, sets up axum/tonic test infrastructure |
| Day 3 | PR3 (HTTP server + middleware) | PR4 (gRPC server + services) — parallel after PR2 |
| Day 4 | PR5 (management API) | Reviews PR3, PR4, PR5 |
| Day 5 | PR6 (bootstrap + wiring) — after PR3/PR4/PR5 merged | PR7 (error formatting + edge cases) |
| Day 6 | PR8 (integration tests) | PR9 (observability audit) |
| Day 7 | VC-4 verification | Code review fixes |

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Anthropic SSE state machine logic errors crash compliant SDKs | Medium | High | Exhaustive unit tests for all valid/invalid transitions. Integration test verifies exact event ordering (PR2 tasks 2.16–2.19). FM-3 documented with runtime-checked transitions. |
| `spawn_blocking` pool exhaustion during concurrent large-body parsing | Low | Medium | Default pool is 512 threads. Gated by `LargeBodyConcurrencyLimit` (16 concurrent large requests). Monitor via `llamaforge_http_requests_total`. |
| tonic `build.rs` proto codegen breaks on CI with different `protoc` versions | Medium | Low | Pin `tonic-build` version in workspace. CI installs specific `protoc` version. `build.rs` uses `tonic_build` directly (no external `protoc` dependency). |
| axum 0.8 breaking changes from minor bumps | Low | Medium | Pin exact version in workspace. axum 0.8 is stable. Test suite catches regressions. |
| Memory exhaustion from concurrent large multimodal requests | Medium | High | `LargeBodyConcurrencyLimit` semaphore caps at 16 × 200MB = 3.2GB. Monitor `llamaforge_active_connections` gauge. Config knob allows tuning at runtime. |
| gRPC keepalive false positives on slow networks | Low | Low | Configurable intervals (30s/10s defaults). Operator can increase via `[grpc]` config section. |
| OpenAI SDK null semantics edge cases not covered | Medium | Low | Custom `deserialize_null_as_none` tested on all `Option<T>` fields. `function_call` rejection tested explicitly. |
| Middleware ordering mistake breaks CatchPanicLayer coverage | Low | High | Test explicitly: inject panic in auth middleware → verify 500 returned (not process crash). Middleware ordering documented in design doc §3.1. |
| `tower-http` CatchPanicLayer does not catch all panic types | Low | Medium | `CatchPanicLayer` catches `std::panic::catch_unwind`-compatible panics. Foreign (C FFI) panics still abort. Acceptable: no FFI in middleware stack. |
| Hot-reload of `api_key` introduces TOCTOU race | Low | Low | Auth layer reads config atomically on each request. Brief window during swap is acceptable — worst case: one request uses stale key. |

---

## Integration Points (for next component)

After Transport Layer is complete, the following integration seams are ready:

1. **Composition root (`main.rs`)** will construct `TransportDeps` by injecting implementations of all 8 port traits, then call `start_servers(deps)`. The returned `TransportHandles` is held for shutdown coordination.
2. **Request Queue** implements `QueueSubmitter` trait on its `RequestPipeline`. Transport calls `submit()`, `cancel()`, `status()`.
3. **Smart Routing** implements `RouteValidator` trait on its `Router`. Transport calls `validate_routable()` at the edge for early rejection.
4. **Server Manager** implements `ServerControl` trait. Transport calls `list_servers()`, `start_server()`, `stop_server()` from management API handlers.
5. **Metrics & Storage** implements `MetricsStore` and `MetricsExporter` traits. Transport calls `query_*()` for management API and `gather_text()` for `/metrics`.
6. **Config Manager** implements `ConfigReader` on `ConfigHandle` (or wrapper) and `ConfigWriter` for reload. Transport reads `TransportConfig` snapshots on each request.
7. **External clients** (OpenAI SDK, Anthropic SDK, gRPC clients, Prometheus) connect to the HTTP/gRPC ports. The transport layer guarantees SDK-compatible protocol formatting.