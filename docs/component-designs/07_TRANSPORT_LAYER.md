# Component Design: Transport Layer

**Component:** Transport Layer
**Phase:** 4 (Client Interface)
**Estimated LOC:** ~2,000
**Risk:** Medium
**Dependencies:** All upstream components (Request Queue, Smart Routing, Dispatch & Proxy, Server Manager, Metrics & Storage, Config Manager)
**Consumers:** External clients (OpenAI SDK, Anthropic SDK, gRPC clients, Prometheus scrapers)
**Crate location:** `crates/llamaforge-api/src/transport/` (merged in Phase 9 PR3 with `dispatch/` and `adapters/`). Pre-merge: `crates/llamaforge-transport/`. See `docs/component-designs/06_DISPATCH_AND_PROXY.md` for the adapter-collapse opportunities that the merge unlocked.

---

## 1. Purpose & Scope

The Transport Layer is LlamaForge's client-facing edge. It translates between external API protocols (OpenAI HTTP, Anthropic HTTP, gRPC) and LlamaForge's internal types, handles SSE streaming with protocol-specific state machines, exports Prometheus metrics, implements authentication, and provides the management REST API.

**v1 scope:**
- OpenAI-compatible HTTP API (`/v1/chat/completions`, `/v1/completions`)
- Anthropic-compatible HTTP API (`/v1/messages`)
- gRPC API (proto-defined, tonic codegen)
- Prometheus `/metrics` endpoint
- Management REST API (`/api/*`)
- Authentication (API key, optional)

**Explicitly NOT in v1:**
- Web UI (deferred to Phase 5)
- WebSocket transport
- Server-Sent Events for management subscriptions (polling only)

**What this component explicitly does NOT do:**

- It does not implement scheduling or priority logic (that's Request Queue).
- It does not select servers (that's Smart Routing).
- It does not proxy to backends (that's Dispatch & Proxy).
- It does not manage processes (that's Server Manager).
- It does not parse TOML or manage config state (that's Config Manager).

---

## 2. Public API Surface

### 2.1 Server Bootstrap

```rust
/// Starts both the gRPC and HTTP servers.
/// Returns join handles for graceful shutdown coordination.
pub async fn start_servers(
    config: Arc<ConfigHandle>,
    inference_engine: Arc<dyn InferenceEngine>,
    server_manager: Arc<ServerManager>,
    metrics_store: Arc<dyn MetricsStore>,
    prometheus_metrics: Arc<PrometheusMetrics>,
    queue: Arc<RequestQueue>,
) -> Result<TransportHandles, TransportError>;

pub struct TransportHandles {
    pub http_handle: tokio::task::JoinHandle<()>,
    pub grpc_handle: tokio::task::JoinHandle<()>,
    pub shutdown_tx: tokio::sync::watch::Sender<()>,
}
```

### 2.2 InferenceEngine Trait (protocol-agnostic core)

```rust
/// The core service trait. Protocol-agnostic.
/// Implemented by the orchestrator, consumed by both gRPC and HTTP handlers.
#[async_trait]
pub trait InferenceEngine: Send + Sync + 'static {
    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>,
        DispatchError,
    >;
}
```

### 2.3 HTTP Routes (axum)

```rust
// OpenAI-compatible endpoints
POST /v1/chat/completions    → openai_chat_completion_handler
POST /v1/completions         → openai_completion_handler

// Anthropic-compatible endpoint
POST /v1/messages            → anthropic_messages_handler

// Management API
GET  /api/servers            → list_servers
POST /api/servers/{id}/start → start_server
POST /api/servers/{id}/stop  → stop_server
POST /api/servers/{id}/restart → restart_server  // graceful stop→start (user-initiated)
GET  /api/queue              → queue_status
DELETE /api/queue/{id}       → cancel_request
POST /api/config/reload      → reload_config
GET  /api/metrics/requests   → query_request_metrics
GET  /api/metrics/aggregated → query_aggregated
GET  /api/metrics/summary    → query_summary       // dashboard {throughput_total, latency}
POST /api/maintenance/wal-checkpoint → wal_checkpoint
GET  /health                 → health_check
GET  /metrics                → prometheus_export
```

**Management DTO contract.** Management responses are consumed by the web UI, so
their JSON shapes are pinned to the frontend types in `FRONTEND_ARCHITECTURE.md`
§5.2 — not to the internal Rust structs. In particular `GET /api/servers` returns
the `ServerState` shape `{id, name, status, uptime}` (resolved #292), where
`status` is the UI's lowercase `ServerStatus` vocabulary mapped from
`ServerLifecycle` (`Running→running`, `Starting→pending`, `Draining→degraded`,
`Restarting→restarting`, `Stopped→stopped`, `Failed→failed`) — never the `Debug`
rendering of the enum, which the dashboard's `StatusBadge` cannot key off. Per-server
slot/throughput detail is exposed separately on `GET /api/servers/{id}/metrics`.

### 2.4 Protocol Translation Types

```rust
/// OpenAI request → Internal type.
/// Handles: null semantics (#[serde(deserialize_with = "deserialize_null_as_none")]),
/// logit_bias string keys (parse::<i32>), stream boolean, tool definitions.
pub fn parse_openai_request(body: OpenAiChatRequest) -> Result<ChatCompletionRequest, ApiError>;

/// Internal chunk → OpenAI SSE format.
/// Handles: empty-delta queue updates, [DONE] sentinel, finish_reason mapping.
pub fn format_openai_chunk(chunk: &ChatCompletionChunk) -> String;

/// Anthropic request → Internal type.
/// Handles: system prompt extraction, content block structure, tool_use format.
pub fn parse_anthropic_request(body: AnthropicMessagesRequest) -> Result<ChatCompletionRequest, ApiError>;

/// Internal chunk → Anthropic SSE event sequence.
/// STRICT STATE MACHINE: message_start → content_block_start → content_block_delta* →
/// content_block_stop → message_delta → message_stop.
/// Preamble events emitted synchronously before token forwarding loop.
pub fn format_anthropic_event(chunk: &ChatCompletionChunk, state: &mut AnthropicState) -> Vec<SseEvent>;
```

---

## 3. Internal Architecture

### 3.1 Dual-Server Architecture

```
    ┌─────────────────────────────────────────────────────────┐
    │                    HTTP Server (axum)                     │
    │                    Port: http_port (8000)                 │
    │                                                           │
    │  ┌─────────────────────────────────────────────────────┐ │
    │  │ Middleware Stack (Tower)                              │ │
    │  │ - Auth layer (API key check)                         │ │
    │  │ - Request body limit (50MB / 200MB for multimodal)   │ │
    │  │ - Request ID injection (UUIDv7)                      │ │
    │  │ - Metrics collection (request count, latency)        │ │
    │  │ - NO TimeoutLayer (would sever streaming!)           │ │
    │  └─────────────────────────────────────────────────────┘ │
    │                                                           │
    │  Routes:                                                  │
    │  /v1/chat/completions  → OpenAI shim → InferenceEngine  │
    │  /v1/messages          → Anthropic shim → InferenceEngine│
    │  /api/*                → Management handlers             │
    │  /metrics              → Prometheus text export           │
    │  /health               → Health check                    │
    └─────────────────────────────────────────────────────────┘

    ┌─────────────────────────────────────────────────────────┐
    │                   gRPC Server (tonic)                     │
    │                   Port: grpc_port (50051)                 │
    │                                                           │
    │  ┌─────────────────────────────────────────────────────┐ │
    │  │ Middleware:                                           │ │
    │  │ - Auth interceptor (metadata key check)              │ │
    │  │ - Max message size (16MB default)                    │ │
    │  │ - Keepalive (30s interval, 10s timeout)              │ │
    │  │ - Max concurrent streams (1000)                      │ │
    │  └─────────────────────────────────────────────────────┘ │
    │                                                           │
    ���  Services:                                                │
    │  InferenceService::StreamInference → InferenceEngine     │
    │  ManagementService::*              → Management ops      │
    │  grpc.health.v1.Health             → Standard health     │
    └─────────────────────────────────────────────────────────┘
```

### 3.2 Anthropic SSE State Machine

```
    ┌─────────────┐
    │   INIT      │ ← handler starts here
    └──────┬���─────┘
           │ emit: event: message_start\ndata: {...}\n\n
           ▼
    ┌──────────────────┐
    │ PREAMBLE_SENT    │
    └──────┬───────────┘
           │ emit: event: content_block_start\ndata: {...}\n\n
           ▼
    ┌──────────────────┐
    │ CONTENT_BLOCK    │ ← token forwarding loop runs here
    └──────┬───────────┘
           │ for each token:
           │   emit: event: content_block_delta\ndata: {...}\n\n
           │
           │ on stream end:
           │   emit: event: content_block_stop\ndata: {...}\n\n
           ▼
    ┌──────────────────┐
    │ BLOCK_STOPPED    │
    └──────┬───────────┘
           │ emit: event: message_delta\ndata: {...}\n\n
           ▼
    ┌──────────────────┐
    │ MESSAGE_DONE     │
    └──────┬───────────┘
           │ emit: event: message_stop\ndata: {}\n\n
           ▼
    ┌──────────────────┐
    │   COMPLETE       │
    └──────────────────┘

    CRITICAL (G-11): Preamble events (message_start + content_block_start)
    are emitted SYNCHRONOUSLY before entering the token forwarding loop.
    No token can be yielded until state == CONTENT_BLOCK.
```

### 3.3 OpenAI SSE Format

```
    Queue update (while queued):
    data: {"id":"...","choices":[{"delta":{},"index":0}],"llamaforge":{"status":"queued","position":3}}\n\n

    First token:
    data: {"id":"...","choices":[{"delta":{"role":"assistant"},"index":0}]}\n\n

    Subsequent tokens:
    data: {"id":"...","choices":[{"delta":{"content":"Hello"},"index":0}]}\n\n

    Final chunk:
    data: {"id":"...","choices":[{"delta":{},"index":0,"finish_reason":"stop"}],"usage":{...}}\n\n

    Stream end:
    data: [DONE]\n\n
```

### 3.4 Key Invariants

1. **No TimeoutLayer on streaming:** Streaming responses can run for minutes. Timeouts applied only to queue-wait phase and non-streaming generation.
2. **Anthropic state machine strict:** No token emitted before `content_block_start`. Violation crashes compliant SDKs.
3. **OpenAI null semantics:** `"max_tokens": null` parsed as `None`, not error. Custom deserializer on all `Option<T>` fields.
4. **logit_bias keys parsed as i32:** Negative token IDs (BOS/EOS) are valid. `parse::<u32>()` would drop them.
5. **Separate middleware stacks:** gRPC and HTTP have independent timeout, body limit, and keepalive configurations. No shared TimeoutLayer.
6. **Request ID injected once:** UUIDv7 generated at the transport edge. Propagated through all internal components. Returned in response headers.
7. **Authentication optional:** If `api_key` is empty in config, auth middleware is a pass-through.

---

## 4. Boundary Contracts

### 4.1 What This Component REQUIRES

| Dependency | Contract |
|-----------|----------|
| InferenceEngine | `chat_completion()` returns a `'static + Send` stream. Stream yields `Result<Chunk, StreamError>`. |
| Request Queue | `enqueue()` available for admission. |
| Smart Routing | `validate_routable()` for early rejection. |
| Server Manager | `running_servers()` for management API. |
| Metrics & Storage | `query_events()` / `query_request_metrics()` for API queries. |
| Config Manager | `reload()` for POST /api/config/reload. |
| PrometheusMetrics | `registry.gather()` for /metrics endpoint. |

### 4.2 What This Component GUARANTEES to Consumers (External Clients)

| Client | Guarantee |
|--------|-----------|
| OpenAI SDK | Compatible SSE format. `data: [DONE]` terminal. `finish_reason` always present on final chunk. |
| Anthropic SDK | Strict event ordering. `ping` events for queue updates. Never emit `content_block_delta` before `content_block_start`. |
| gRPC clients | Standard health checking protocol. Keepalive prevents silent dead connections. |
| Prometheus | Text exposition format at /metrics. Always returns 200 (empty metrics on degraded). |
| All clients | HTTP 429 with `Retry-After` on queue full. Structured JSON error bodies with error code. |

### 4.3 Failure Propagation

| Failure | Propagation |
|---------|-------------|
| Invalid request body | HTTP 400 / gRPC INVALID_ARGUMENT with specific field error. |
| Auth failure | HTTP 401 / gRPC UNAUTHENTICATED. |
| Model not found | HTTP 404 / gRPC NOT_FOUND. |
| Queue full | HTTP 429 with Retry-After / gRPC RESOURCE_EXHAUSTED. |
| Backend error | HTTP 502 / gRPC INTERNAL (non-streaming) or mid-stream error event. |
| Backend timeout | HTTP 504 / gRPC DEADLINE_EXCEEDED. |

---

## 5. Failure Modes

### FM-1: Client Disconnects During Streaming

- **Scenario:** Client closes TCP connection mid-stream.
- **Detection:** axum's response body write returns error. Channel receiver drops.
- **Recovery:** Proxy task's `cancel_token` fires. Proxy stops reading from backend. Token released.

### FM-2: Request Body Exceeds Limit

- **Scenario:** 300MB multimodal request exceeds configured limit.
- **Detection:** Tower's `DefaultBodyLimit` layer rejects before handler invoked.
- **Response:** HTTP 413 with `{"error": "request_body_too_large", "max_bytes": N, "received_bytes": M}`.

### FM-3: Anthropic State Machine Error (Bug)

- **Scenario:** Logic bug emits token before preamble.
- **Impact:** Anthropic SDK clients crash with `ProtocolError`.
- **Mitigation:** State machine enforced by enum — compiler prevents invalid transitions. Integration tests verify event ordering.

### FM-4: gRPC Keepalive Failure

- **Scenario:** Client network partition. Keepalive timeout fires.
- **Detection:** tonic detects keepalive miss after 10s.
- **Recovery:** Connection closed. Client must reconnect. In-flight streams cancelled.

---

## 6. Configuration

| Knob | Location | Default | Valid Range | Hot-Reload |
|------|----------|---------|-------------|------------|
| `http_port` | `[global]` | 8000 | 1-65535 | No (startup) |
| `grpc_port` | `[global]` | 50051 | 1-65535 | No (startup) |
| `bind_address` | `[global]` | 127.0.0.1 | Valid IP | No (startup) |
| `api_key` | `[global]` | None (no auth) | String | Yes |
| `grpc.max_message_size_mb` | `[grpc]` | 16 | 1-256 | No |
| `grpc.keepalive_interval_secs` | `[grpc]` | 30 | 1-3600 | No |
| `grpc.keepalive_timeout_secs` | `[grpc]` | 10 | 1-300 | No |
| `grpc.max_concurrent_streams` | `[grpc]` | 1000 | 1-100000 | No |
| `grpc.tls.enabled` | `[grpc.tls]` | false | bool | No |
| `gateway.max_request_body_bytes` | `[gateway]` | Auto (50MB/200MB) | 1MB-1GB | No |
| `prometheus.enabled` | `[prometheus]` | true | bool | No |
| `prometheus.endpoint` | `[prometheus]` | /metrics | String | No |

---

## 7. Observability

### 7.1 Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `llamaforge_http_requests_total` | Counter | `method, path, status` | HTTP request count |
| `llamaforge_http_request_duration_seconds` | Histogram | `method, path` | HTTP latency |
| `llamaforge_grpc_requests_total` | Counter | `method, status` | gRPC call count |
| `llamaforge_grpc_request_duration_seconds` | Histogram | `method` | gRPC latency |
| `llamaforge_active_connections` | Gauge | `transport={http,grpc}` | Current open connections |
| `llamaforge_auth_failures_total` | Counter | `transport` | Authentication failures |

**Label cardinality:** `path` bounded to ~15 known routes (not client-supplied paths). `method` bounded (GET/POST/DELETE). `status` bounded (200/400/401/404/429/500/502/503/504).

### 7.2 Log Events

| Event | Level | Frequency | Content |
|-------|-------|-----------|---------|
| `transport.started` | INFO | Once | Ports, bind address, TLS status |
| `transport.request` | DEBUG | Per-request | Method, path, request_id, content-length |
| `transport.auth_failed` | WARN | On failure | IP, provided key prefix (masked) |
| `transport.body_too_large` | WARN | On 413 | Received bytes, limit |

---

## 8. Testing Strategy

### 8.1 Unit Test Boundaries

**Pure-logic tests:**
- OpenAI request parsing (null handling, logit_bias i32 keys, stream boolean)
- OpenAI chunk formatting (empty-delta queue updates, [DONE], finish_reason)
- Anthropic request parsing (system prompt extraction, content blocks)
- Anthropic state machine transitions (all valid paths, error on invalid)
- Priority computation from request metadata
- Request body limit calculation (multimodal detection)

### 8.2 Integration Test Requirements

- **axum test client:** Use `axum::test` for HTTP endpoint testing.
- **tonic test client:** Compile protos, use generated client.
- **End-to-end:** Real queue + mock backend → verify full streaming path.

### 8.3 Specific Test Cases

```rust
#[tokio::test]
async fn test_openai_null_max_tokens_accepted() {
    // POST /v1/chat/completions with "max_tokens": null
    // Assert: HTTP 200, not 400.
}

#[tokio::test]
async fn test_openai_logit_bias_negative_keys() {
    // logit_bias: {"-1": 5.0}  (BOS token)
    // Assert: parsed correctly, not dropped.
}

#[tokio::test]
async fn test_anthropic_state_machine_ordering() {
    // Stream 3 tokens → verify events:
    // message_start, content_block_start, 3× content_block_delta,
    // content_block_stop, message_delta, message_stop.
}

#[tokio::test]
async fn test_anthropic_no_token_before_preamble() {
    // Fast dispatch (TTFT < 1ms). Verify content_block_start
    // is always emitted before first content_block_delta.
}

#[tokio::test]
async fn test_queue_full_returns_429_with_retry_after() {
    // Fill queue to max. Send one more.
    // Assert: HTTP 429, Retry-After header present.
}

#[tokio::test]
async fn test_auth_required_when_configured() {
    // Set api_key in config. Send request without key.
    // Assert: HTTP 401.
}

#[tokio::test]
async fn test_prometheus_endpoint_always_200() {
    // Even if metrics store is degraded.
    // Assert: /metrics returns 200 with text content.
}

#[tokio::test]
async fn test_streaming_no_timeout_layer() {
    // Stream for 120 seconds. Assert: no timeout error.
}
```

---

## 9. Open Questions

### Deferred to Implementation

1. **Proto definition location:** In-repo `proto/` directory with `tonic-build`, or separate crate? Decision: in-repo `proto/` — simpler, single compile step.

2. **OpenAI function_call vs tool_calls:** Support both the deprecated `function_call` and current `tool_calls` format? Decision: `tool_calls` only for v1. `function_call` returns 400 with migration hint.

3. **Streaming backpressure to client:** If client reads SSE slowly, does axum buffer unboundedly? Decision: use a bounded channel between proxy stream and axum response body. If channel fills, apply backpressure (this is handled by Dispatch & Proxy's 2s timeout).

4. **CORS configuration:** Needed for browser-based clients. Decision: add a `[cors]` config section with `allowed_origins` list. Default: `["*"]` for development, operator should restrict in production.

5. **Request ID header:** Which header name? `X-Request-ID` (standard) vs `x-llamaforge-request-id` (namespaced)? Decision: both — set `x-request-id` for standard tooling compatibility, also set `x-llamaforge-request-id` for explicit identification.

### Alternatives Considered

- **Single port with protocol detection:** Rejected — complicates middleware, confuses monitoring tools.
- **hyper directly (no axum):** Rejected — axum provides ergonomic routing, middleware, extractors. No performance difference for this use case.
- **warp instead of axum:** Rejected — axum is more actively maintained, better Tower integration, same async ecosystem.
- **REST-only (no gRPC):** Considered. Rejected — gRPC gives type-safe clients, efficient binary encoding for high-throughput, standard health checking protocol.

---

## Cross-Cutting Concerns

### Cancellation Safety

- **Request parsing:** Synchronous (deserialization). Not cancellable.
- **Queue wait (streaming):** If client disconnects during queue wait, their response channel drops → cancel_token fires → request removed from queue.
- **Response streaming:** If axum response body future is cancelled (client disconnect), the proxy task's cancel_token fires via the channel drop.

### Graceful Shutdown

On SIGTERM:
1. Stop accepting new TCP connections (axum's graceful shutdown).
2. In-flight requests continue until completion or server-level drain timeout.
3. gRPC: send GOAWAY frame. Clients must reconnect.
4. HTTP: finish current responses, close connections with Connection: close.

### Panic Safety

- **Handler panic:** axum catches panics in handlers and returns HTTP 500. No process crash.
- **Middleware panic:** Tower layer panics propagate — process crashes. Mitigation: middleware is simple (auth check, body limit).

### Clock Assumptions

- **UUIDv7 generation:** Uses system clock. Non-monotonic UUIDs possible on NTP jump. Acceptable — UUIDs are for uniqueness, not ordering.
- **Retry-After header:** Computed from queue depth / throughput estimate. Approximate, not clock-sensitive.

### Memory Bounds

| Resource | Bound | Worst Case |
|----------|-------|------------|
| Request body buffer | max_request_body_bytes | 200MB per request |
| Concurrent requests | max_concurrent_streams (gRPC) + system fd limit (HTTP) | 1000 gRPC + ~10000 HTTP |
| Per-connection state | ~10KB (headers, TLS state) | 10000 × 10KB = 100MB |
| Response buffer per stream | Bounded channel (32 chunks × ~1KB) | 32KB per stream |

**Total worst case:** ~300MB at extreme concurrency (10000 connections). Bounded by OS limits.

### Backpressure

- **Client → LlamaForge:** Request body limit enforces memory bound. Queue full returns 429.
- **LlamaForge → Client:** Bounded response channel. If client slow: Dispatch & Proxy handles disconnect.
- **Management API:** Read queries bounded by read pool (4 connections) + statement timeout (30s). Excess queries wait for pool.
