# LlamaForge — Phased Implementation Plan

**Version:** 2.0  
**Author:** Sumanth Gottipati  
**Date:** 2026-05-04  
**Methodology:** Incremental delivery with E2E verification checkpoints  
**Revision Note:** Revised based on adversarial review findings (32 issues addressed)

---

## Executive Summary

This plan decomposes LlamaForge into 6 implementation phases, each delivering a vertically-integrated slice of functionality that can be verified end-to-end against real `llama-server` instances. The philosophy is "narrow but deep" — each phase produces a working system that handles real inference requests, rather than building all components to 50% completion before anything works together.

Total estimated duration: **21 weeks** (single senior Rust engineer) or **13–14 weeks** (2 engineers with clear ownership boundaries, Phase 3 and Phase 4+5 running in parallel after Phase 2).

---

## Phase 0: Foundation & Skeleton (Weeks 1–2)

**Objective:** Establish the project structure, build system, CI pipeline, and the minimal viable binary that compiles and runs — proving the toolchain works before any business logic.

### Tasks

| # | Task | Description | Est. |
|---|------|-------------|------|
| 0.1 | Cargo workspace setup | Create `llamaforge` binary crate + `llamaforge-proto` lib crate (for generated protobuf types). Configure workspace-level dependencies, edition 2021, `rust-version = "1.75"`. | 2h |
| 0.2 | Proto definitions | Write `.proto` files for `InferenceService`, `QueueService`, `ServerMgmtService`, `MetricsService`. Set up `tonic-build` in `build.rs`. Verify `cargo build` generates Rust types. | 4h |
| 0.3 | Dual-port listener skeleton | Minimal `main.rs` that starts tonic on :50051 and axum on :8000. Both ports bind or process exits. Implement `GET /health` (HTTP) and `grpc.health.v1.Health/Check` (gRPC). | 3h |
| 0.4 | Configuration scaffolding | Define `Config` struct with `#[serde(default)]` on all fields. Parse `llamaforge.toml` at startup. Implement `ArcSwap` for hot-reload readiness. Add `llamaforge validate --config` CLI subcommand. Explicit fields: `per_requestor_limit`, `sse_keepalive_interval_secs`, `queue_update_interval_secs`. | 4h |
| 0.5 | Structured logging | Set up `tracing` + `tracing-subscriber` with JSON output. Configure `RUST_LOG` filtering. Add `tracing_appender::rolling::daily` for file output. | 2h |
| 0.6 | SQLite initialization | Create DB file, apply schema migrations via `rusqlite_migration`. Implement downgrade guard (`PRAGMA user_version` check). Set WAL mode, `synchronous=NORMAL`, `busy_timeout=5000`. | 3h |
| 0.7 | CI pipeline | GitHub Actions: `cargo clippy`, `cargo test`, `cargo build --release`. Add `cargo deny` for license/supply-chain checks. | 3h |
| 0.8 | Mock llama-server | Build `mock-llama` binary: configurable HTTP server returning canned SSE `/completion` responses with tunable delay, token count, and error injection. | 4h |
| 0.9 | Docker scaffold | Multi-stage Dockerfile (builder → distroless). `docker-compose.yml` with GPU passthrough. Verify container builds. | 2h |
| 0.10 | Database abstraction traits | Define `MetricsStore` and `ConfigOverlayStore` async traits with SQLite implementation behind trait boundary. Enables future backend swaps without coupling. | 3h |
| 0.11 | GGUF metadata parser | Binary format reader extracting `num_kv_heads`, `head_dim`, `num_layers`, chat template from GGUF header. Used by Phase 1 for template loading and Phase 2 for dynamic VRAM computation. | 6h |

### Verification Checkpoint 0 (VC-0)

**Gate criteria — all must pass before Phase 1 begins:**

- [ ] `cargo build --release` succeeds with zero warnings
- [ ] Binary starts, binds both ports, responds to `GET /health` with 200 and `grpc.health.v1.Health/Check` with `SERVING`
- [ ] `llamaforge validate --config test/config.toml` exits 0 for valid config, non-zero for invalid
- [ ] SQLite DB is created with correct schema (verified by `sqlite3 llamaforge.db ".schema"`)
- [ ] `mock-llama` starts and responds to `POST /completion` with a configurable SSE token stream
- [ ] Docker image builds and runs (health check passes inside container)
- [ ] CI pipeline runs green on the initial commit
- [ ] `MetricsStore` and `ConfigOverlayStore` traits compile with SQLite implementations passing basic CRUD tests
- [ ] GGUF parser extracts metadata from a real TinyLlama GGUF file (chat template, layer count, head dimensions)

---

## Phase 1: Single-Server Inference Path (Weeks 3–6)

**Objective:** A single `llama-server` instance can be spawned, health-checked, and serve inference requests through the HTTP/OpenAI shim with metrics recorded. This is the minimum viable product — one server, one model, no queue. Establishes the `Scheduler` trait boundary that Phase 2 extends.

### Tasks

| # | Task | Description | Est. |
|---|------|-------------|------|
| 1.1 | Server Manager — process spawn | Implement `LocalServerManager` with `tokio::process::Child`. Map TOML server config to `llama-server` CLI flags. Record PID in `managed_processes` table. | 6h |
| 1.2 | Server Manager — health monitoring | Tokio task per server: `GET /health` every 2s, `GET /slots` every 5s, `GET /metrics` every 10s. Implement state machine (Stopped→Starting→Running→Failed). Process exit watch via `Child::wait()`. | 8h |
| 1.3 | Server Manager — restart with backoff | 3 consecutive health failures → restart. Exponential backoff with ±20% jitter. `max_restart_attempts` enforcement. | 4h |
| 1.4a | Chat template loading | GGUF metadata extraction (using Task 0.11 parser), template directory fallback, `minijinja` rendering of chat templates. | 5h |
| 1.4b | Downstream `/completion` proxy with SSE parsing | `reqwest` streaming client, SSE line parser for llama-server format, error mapping from backend HTTP errors to internal error types. | 6h |
| 1.4c | `async_stream` ownership pattern | `+'static` constraint handling, `DispatchToken` RAII in generator, `id_slot` injection into downstream requests. | 6h |
| 1.4d | Bounded response channel with backpressure | Bounded channel (capacity 64) between proxy task and HTTP response. 5-second backpressure timeout for slow clients. Slot release on timeout. Defense against GPU slot starvation. | 6h |
| 1.5 | HTTP/OpenAI shim — streaming | `POST /v1/chat/completions` with `stream: true`. Translate OpenAI JSON → internal types → `/completion` payload. Proxy SSE tokens back. Handle `X-Request-ID`, `X-Task-Type` headers. | 8h |
| 1.6 | HTTP/OpenAI shim — non-streaming | `POST /v1/chat/completions` with `stream: false`. Collect all tokens, build complete response JSON. Implement absolute execution timeout. | 4h |
| 1.7 | Per-request metrics capture | Instrument the proxy task: `received_at`, `dispatched_at`, `first_token_at`, `completed_at`. Compute TTFT, generation TPS, total latency. | 4h |
| 1.8 | SQLite audit writer | Dedicated `std::thread` with dual mpsc channels (telemetry lossy + audit backpressured). Batch inserts (100 events / 100ms). Write `RequestMetrics` to `request_metrics` table. Uses `MetricsStore` trait from Task 0.10. | 6h |
| 1.9 | Prometheus endpoint | Implement `/metrics` with counters (`requests_total`, `tokens_total`), gauges (`server_status`, `active_requests`), and histograms (`ttft_seconds`, `request_duration_seconds`) with LLM-tuned buckets. | 4h |
| 1.10 | Client disconnect handling | Cooperative disconnect detection via `CancellationToken`. Drop fallback with `try_send`. Bounded cancellation channels. Slot cooldown states. Erase commands. Multi-path correlation using `slot_dispatch_map`. | 12h |
| 1.11 | SSE keepalives | Emit `: ping\n\n` comments during prefill silence at `sse_keepalive_interval_secs`. | 2h |
| 1.12 | Text completions endpoint | `POST /v1/completions` — simpler than chat (no template rendering). | 3h |
| 1.13 | Request body limit | Dynamic `DefaultBodyLimit` based on multimodal capability. Custom 413 error response with limit info. | 1h |
| 1.14 | DirectDispatcher trait | Define `Scheduler` trait with `submit()` and `cancel()`. Implement `DirectDispatcher` that immediately dispatches without queuing. Phase 2 replaces with `QueuedDispatcher`. Provides clean trait seam for Phase 2 extension. | 4h |
| 1.15 | Basic slot_dispatch_map | `HashMap<(ServerId, SlotId), (RequestId, Instant)>`. Populate on dispatch, clear on completion/disconnect. Used for client disconnect slot reclamation. | 4h |
| 1.16 | LLAMAFORGE_MANAGED=1 env var | Set on all child processes spawned by LocalServerManager. Check during orphan scan to avoid killing non-LlamaForge servers. | 1h |
| 1.17 | Health check stagger | `rand::thread_rng().gen_range(0..poll_interval)` initial delay per server to prevent synchronized health-check bursts. | 1h |

### Verification Checkpoint 1 (VC-1)

**E2E test: real inference through the full stack**

- [ ] LlamaForge spawns a real `llama-server` with TinyLlama-1.1B (Q4_0)
- [ ] `curl -X POST http://localhost:8000/v1/chat/completions -d '{"model":"tinyllama","messages":[{"role":"user","content":"Hello"}],"stream":true}'` returns streaming SSE tokens
- [ ] Same request with `"stream": false` returns a complete JSON response
- [ ] OpenAI Python SDK (`openai.ChatCompletion.create()`) works without errors for both streaming and non-streaming
- [ ] `GET /metrics` returns valid Prometheus text format with non-zero `llamaforge_requests_total`
- [ ] SQLite `request_metrics` table contains the completed request with valid TTFT, TPS, and latency
- [ ] Killing `llama-server` with `kill -9` triggers auto-restart within 15 seconds
- [ ] Client disconnect mid-stream: `llama-server` slot is freed within 10 seconds (verified via `GET /slots` and `slot_dispatch_map` correlation)
- [ ] SSE keepalive comments visible during a long prefill (use a 4K token prompt on TinyLlama)
- [ ] Container deployment: same tests pass when running via `docker-compose up`
- [ ] Slow client test: client that reads 1 byte/second causes slot release after backpressure timeout (5s), not indefinite hold
- [ ] Bounded response channel: 100 concurrent requests where 5 clients stall — verify remaining 95 requests complete normally
- [ ] DirectDispatcher trait: verify `submit()` immediately dispatches and `cancel()` returns appropriate error for already-dispatched request

**Performance baseline (record, don't gate):**
- TTFT overhead (LlamaForge vs direct `llama-server`): target < 5ms
- Streaming token overhead per chunk: target < 0.1ms

---

## Phase 2: Priority Queue & Multi-Server Routing (Weeks 7–10)

**Objective:** Multiple `llama-server` instances with different models, priority-aware queuing, smart routing, and queue position feedback to clients. Replaces `DirectDispatcher` with `QueuedDispatcher` implementing the same `Scheduler` trait.

### Tasks

| # | Task | Description | Est. |
|---|------|-------------|------|
| 2.1 | MultiLevelQueue implementation | Bucket-based `BTreeMap<PriorityTier, BTreeMap<Reverse<u32>, VecDeque<QueuedRequest>>>` with `HashMap` index. `std::sync::Mutex` protection. Insert, pop, cancel, boost operations. Deeply nested structure with two-phase starvation scan, borrow-checker-compliant dispatch coordinates, and cross-tier re-keying. | 14h |
| 2.2 | Priority computation | `compute_effective_priority()` function. Parse `priority_rules.toml`. Task type base priorities + requestor bonuses + explicit overrides. | 3h |
| 2.3 | Starvation prevention | Background task every 5s: two-phase snapshot-then-boost pattern. Logarithmic boost formula. Cross-tier promotion with re-keying. | 5h |
| 2.4 | Fair share DRR | Deficit Round Robin at inter-tier level. Sliding 60s window. Tier redistribution on empty queues. Startup grace period. | 6h |
| 2.5 | Model-aware dispatch | Peek-and-iterate from highest priority. Skip requests with no available capacity for their model. Record coordinates, drop iterator, then remove (borrow checker compliance). | 5h |
| 2.6 | Admission control | Hard queue limit (1000). Per-requestor limit (50, using `per_requestor_limit` config). HTTP 429 / gRPC `RESOURCE_EXHAUSTED` with `Retry-After`. | 3h |
| 2.7 | Queue timeout | Background task scanning for expired `queue_timeout_secs`. Remove and notify client with 408 / `DEADLINE_EXCEEDED`. | 2h |
| 2.8 | Multi-server routing algorithm | Scoring formula: `w_slots × free_slot_ratio + w_kv × (1-kv_usage) + w_queue × (1-deferred_ratio)`. Tie-breaking via deterministic round-robin. Stubs `w_affinity=0` for prompt cache affinity term (implemented in Phase 5). | 5h |
| 2.9 | Dispatch Tracker (v1: Mutex+HashSet) | `Arc<Mutex<HashSet<RequestId>>>` per server. RAII `DispatchToken`. Stale-slot compensation in routing formula. | 4h |
| 2.10 | Forward reconciliation | On `/slots` poll: snapshot tracked IDs, compare with active slots, remove stale entries (with 10s grace period). | 4h |
| 2.11 | Reverse reconciliation (orphan erase) | For each backend-active slot unknown to LlamaForge: track `first_seen_processing`, erase after grace period. `orphan_slots_erased_total` counter. | 4h |
| 2.12 | Queue position feedback (SSE) | Emit `QueueUpdate` chunks via empty-delta format for OpenAI path. Implement queue-wait `tokio::select!` (dispatch vs disconnect vs timeout). Refactors Phase 1 streaming handler to support SSE stream spanning both queue-wait and generation phases. | 10h |
| 2.13 | Queue position feedback (gRPC) | `QueueUpdate` message in `oneof response_type`. Same select! logic. | 3h |
| 2.14 | Hard routing constraints | Model match, server state, multimodal capability, LoRA adapter, embedding, reranking — checked before scoring. Clear error responses for each constraint failure. | 4h |
| 2.15 | VRAM pre-spawn validation | Query NVML/sysctl. Compute estimated VRAM from GGUF metadata. Uses GGUF parser from Task 0.11 for dynamic VRAM computation. Falls back to `estimated_vram_mb` config override. VRAM Reservation Ledger with lifecycle (spawn → first inference → ledger removal). | 6h |
| 2.16 | `/v1/models` endpoint | List all models across all servers. Format response based on inbound request path (OpenAI vs Anthropic format). | 2h |
| 2.17 | Dispatch tracker burst test infrastructure | Test harness verifying 20 concurrent requests within one /slots polling window distribute across servers. | 3h |
| 2.18 | VRAM ledger concurrent spawn race guard | Mutex-protected ledger preventing double-allocation when 2 servers start simultaneously on same GPU. | 3h |

### Verification Checkpoint 2 (VC-2)

**E2E test: multi-model priority scheduling**

- [ ] Two `llama-server` instances running different models (TinyLlama + a second small model)
- [ ] Request for model A routes to server A; request for model B routes to server B
- [ ] Saturate all slots on server A (4 concurrent requests), then submit a 5th → client receives queue position updates via SSE
- [ ] Submit an `emergency` priority request while queue has `background` requests → emergency dispatches first
- [ ] Submit 10 `background` requests, wait 35s → verify starvation boost increases effective_priority (check SQLite audit)
- [ ] Request for nonexistent model → HTTP 404 with clear error
- [ ] Request exceeding per-requestor limit (50) → HTTP 429 with `Retry-After`
- [ ] Request exceeding queue depth (1000) → HTTP 429
- [ ] Request timeout: set `X-Queue-Timeout: 5`, saturate slots for 10s → client receives 408
- [ ] Client disconnects during queue wait → request removed from queue (verify queue depth decreases)
- [ ] Kill one server: its in-flight requests get `503`; queued requests for the same model wait for restart
- [ ] After server restart: new requests route successfully; orphan slots are erased (check `orphan_slots_erased_total` counter)
- [ ] gRPC client receives `QueueUpdate` messages followed by `TokenChunk` messages on same stream
- [ ] VRAM validation: configure `estimated_vram_mb` higher than available → server refuses to start with clear error
- [ ] Burst routing: 20 concurrent requests within 1 polling window (5s) distribute across available servers (not all to one)
- [ ] VRAM ledger race: 2 servers configured to start on same GPU simultaneously — second one rejects with clear VRAM error
- [ ] Starvation boost saturation: request at max_boost=400 cap doesn't exceed tier boundary
- [ ] DRR tier re-activation: tier empty for 30s, new traffic arrives — deficit_tokens reset correctly, no priority inversion

**Scheduling correctness tests (automated, run in CI with mock-llama):**
- [ ] 1000 requests with mixed priorities: verify dispatch order matches effective_priority (FIFO within same priority)
- [ ] Fair share: 60% emergency + 40% background traffic → background tier gets ≥10% of dispatches over 60s window
- [ ] Model-aware dispatch: blocked model A request does NOT block lower-priority model B request

---

## Phase 3: gRPC API, Anthropic Shim & Advanced Features (Weeks 11–13.5)

**Objective:** Full gRPC service implementation, Anthropic SDK compatibility, tool use, grammar constraints, embeddings, and reranking.

### Tasks

| # | Task | Description | Est. |
|---|------|-------------|------|
| 3.1 | gRPC InferenceService | `ChatCompletion` (streaming), `TextCompletion` (streaming), `Embed` (unary), `Rerank` (unary). Wire to same `InferenceEngine`. | 8h |
| 3.2 | gRPC QueueService | `GetQueueStatus`, `CancelRequest`, `BoostPriority`, `WatchQueue` (streaming). | 5h |
| 3.3 | gRPC ServerMgmtService | `ListServers`, `GetServer`, `StartServer`, `StopServer`, `RestartServer`, `ListModels`, `GetSlots`. | 5h |
| 3.4 | gRPC MetricsService | `QueryRequestMetrics`, `GetAggregatedMetrics`, `WatchMetrics` (streaming). | 4h |
| 3.5 | Anthropic `/v1/messages` shim | Translate Anthropic Messages format → internal types. System prompt mapping. SSE with named events (`message_start`, `content_block_delta`, etc.). Strict state machine compliance. | 10h |
| 3.6 | Tool use (unary) | Parse Jinja-templated tool call output from `llama-server`. Construct `ToolCall` protobuf messages. Fallback to raw content on parse failure. | 6h |
| 3.7 | Tool use (streaming) | Bounded-lookahead buffered-then-emit mode. 20-token lookahead buffer. Tool call prefix detection. Buffered accumulation → structured `ToolCallChunk` emission. | 8h |
| 3.8 | Grammar / response_format | Pass GBNF grammar to `/completion`. JSON schema pass-through for `response_format`. Mutual exclusion validation (grammar vs response_format vs tools). | 4h |
| 3.9 | Embeddings endpoint | `POST /v1/embeddings`. Route to embedding-capable servers. Coerce single string → array. | 3h |
| 3.10 | Reranking endpoint | `POST /v1/rerank`. Route to reranking-capable servers. | 3h |
| 3.11 | Extended sampling params | `top_k`, `min_p`, `repeat_penalty`, `frequency_penalty`, `presence_penalty`, `seed`, `logprobs`, `logit_bias`. Map to `/completion` parameters. | 4h |
| 3.12 | `stream_options.include_usage` | Dedicated trailing chunk with usage stats (empty choices array). Two-chunk pattern for SDK compatibility. | 2h |
| 3.13 | Stop sequence validation | Max 10 sequences, each max 256 bytes. String-to-array coercion. | 1h |
| 3.14 | `null` semantics & coercion | Custom serde deserializer for OpenAI `null` fields. `logit_bias` string-key coercion. `stop` string/array coercion. | 3h |
| 3.15 | gRPC TLS | `tonic` TLS config from `[grpc.tls]` section. Certificate file loading. | 2h |
| 3.16 | API key authentication | tonic interceptor for gRPC. axum middleware for HTTP. Skip for health/ready endpoints. | 3h |
| 3.17 | `n` parameter validation | Reject `n=0` and `n>1` with `INVALID_ARGUMENT` / HTTP 400 in v1. | 1h |
| 3.18 | Mid-stream error format through real SDK | Verify error chunk format when backend crashes mid-stream doesn't cause SDK exceptions. Test with both OpenAI and Anthropic Python SDKs. Ensure clean exception propagation. | 4h |

### Verification Checkpoint 3 (VC-3)

**E2E test: full API surface**

- [ ] gRPC Python client (generated stubs) completes a streaming chat request and receives `QueueUpdate` → `TokenChunk` → `CompletionSummary`
- [ ] gRPC `CancelRequest` removes a queued request; `BoostPriority` moves a request up
- [ ] gRPC `WatchQueue` streams live queue events as requests flow through
- [ ] Anthropic Python SDK (`anthropic.Anthropic().messages.create()`) works for streaming and non-streaming
- [ ] Anthropic SSE event ordering is correct: `message_start` → `content_block_start` → `content_block_delta` → `content_block_stop` → `message_delta` → `message_stop`
- [ ] Tool use round-trip: send messages with `tools` defined → model produces tool call → response parses correctly in OpenAI SDK
- [ ] Streaming tool use: model produces tool call via stream → client receives `ToolCallChunk` deltas in correct format
- [ ] Grammar constraint: send GBNF grammar → output conforms to grammar
- [ ] `response_format: {"type": "json_schema", ...}` → output is valid JSON matching schema
- [ ] Grammar + tools simultaneously → HTTP 400 with clear error
- [ ] Embeddings: `openai.Embedding.create()` returns valid embedding vectors
- [ ] gRPC with TLS enabled: client connects with TLS certificate
- [ ] API key: requests without valid key → 401; with valid key → success
- [ ] `logprobs: true` → response includes per-token log probabilities
- [ ] `seed` parameter produces deterministic output (same seed → same output on same model)
- [ ] Mid-stream crash: kill llama-server during token generation → OpenAI SDK raises clean exception (not protocol error), Anthropic SDK raises clean exception

**SDK compatibility suite (automated, no GPU required — uses mock-llama):**
- [ ] All 10+ scenarios from Section 18.6 of design doc pass
- [ ] Anthropic SDK does not throw `ProtocolError` for any valid request pattern

---

## Phase 4: Server Lifecycle, Config Management & Observability (Weeks 14–16)

**Objective:** Production-grade server management, config hot-reload, graceful shutdown, deadlock detection, performance degradation monitoring, and the full observability stack.

### Tasks

| # | Task | Description | Est. |
|---|------|-------------|------|
| 4.1 | Graceful shutdown (full sequence) | Stop routing → re-queue undispatched → drain timeout → CancellationToken abort → SIGTERM → SIGKILL. Structured error payloads to clients. | 8h |
| 4.2 | Config hot-reload (SIGHUP) | `tokio::signal::unix::signal(SignalKind::hangup())`. Spawn-blocking for file I/O. Diff-based reload: changed servers restart, new servers start, removed servers drain. `ArcSwap::store()` for atomic config publish. | 6h |
| 4.3 | Config writeback (LoRA) | `POST /api/servers/{id}/lora-adapters`. File lock in state directory. Atomic rename (phase 1) with in-place truncation fallback (phase 2). | 5h |
| 4.4 | Deadlock detection | 4-signal classifier: active work check (slots + tracker), decode stall (`n_decoded`), GPU utilization (NVML/IOKit), loading state. SIGKILL on confirmed deadlock. | 6h |
| 4.5 | Performance degradation detection | 5-min rolling TPS average. 10% threshold for 60s. `restart_on_degradation` option. `server.performance_degraded` event. | 4h |
| 4.6 | Staggered restarts | `restart_stagger_secs` between concurrent server restarts. Prevent thundering-herd VRAM contention. | 2h |
| 4.7 | Orphan process recovery | 4-step startup procedure: PID liveness + `/proc/comm` check → health check → reclaim (erase all slots) or kill → cleanup stale DB entries. Check `LLAMAFORGE_MANAGED=1` env var before killing processes. | 5h |
| 4.8 | WAL management | Periodic `PASSIVE` checkpoint every 60s. WAL size gauge. `POST /api/maintenance/wal-checkpoint` endpoint (passive + truncate modes). | 3h |
| 4.9 | Writer thread liveness monitor | Background task polling `JoinHandle::is_finished()` every 10s. Gateway degraded flag. `writer_thread_alive` gauge. | 2h |
| 4.10 | Event batching pipeline | Upgrades the Phase 1 SQLite writer with a ring buffer (10,000 capacity) for high-throughput telemetry. The dual-mpsc architecture from 1.8 remains; this task adds the ring buffer as a front-end to the lossy telemetry channel. Flush every 100ms or 100 events. `events_dropped_total` counter. Channel gauge polling task (5s interval). | 4h |
| 4.11 | GPU telemetry | NVML integration (Linux/CUDA): VRAM, utilization, temperature. IOKit/sysctl (macOS/Metal). Prometheus gauges. Graceful degradation for unsupported backends. | 6h |
| 4.12 | Full Prometheus metrics | All counters, gauges, histograms from design doc Section 5.4. Cardinality protection (`requestor_class`, `task_type_class` bounded enums). | 5h |
| 4.13 | Structured event logging | All event types from Section 5.6 table. Priority breakdown and routing score decomposition in dispatch events. Log sampling (1-per-10 health_ok). | 4h |
| 4.14 | `GET /ready` endpoint | Lock-free readiness via `AtomicU8`. Circuit breaker for all-servers-failed. `/ready/inference` stricter variant. | 3h |
| 4.15 | Management API endpoints | `GET/POST /api/servers/*`, `GET /api/queue/*`, `GET /api/metrics/*`, `GET /api/models`, `POST /api/config/reload`, `POST /api/servers/{id}/slots/{slot}/erase`. | 6h |
| 4.16 | `llamaforge dry-run` mode | Start API surface without spawning backends. Validate config, serve Web UI, accept queue insertions. | 2h |
| 4.17 | Log rotation support | `tracing_appender::rolling::daily` default. SIGHUP reload handle with safe `WorkerGuard` swap via `spawn_blocking`. | 3h |
| 4.18 | Metrics retention pruning | Background task enforcing `metrics_retention_days=30`. `prune_events_older_than()` runs daily. | 3h |
| 4.19 | 10-rapid-SIGHUP stress test | Integration test verifying thread count stability and no Tokio panics under rapid config reloads. | 2h |
| 4.20 | ROCm and Vulkan GPU backends | rocm-smi-lib FFI for AMD, `VK_EXT_memory_budget` for Vulkan. Extend GPU telemetry task. | 6h |
| 4.21 | Tokio blocking detection | `tokio::runtime::Builder::on_thread_park` hook or `tokio-console` integration to detect blocking calls on worker threads. | 3h |

### Verification Checkpoint 4 (VC-4)

**E2E test: production resilience**

- [ ] Graceful shutdown: `SIGTERM` LlamaForge → all in-flight clients receive structured error with `finish_reason` → `llama-server` processes receive SIGTERM → clean exit
- [ ] Config reload: modify TOML (change `parallel_slots`), send SIGHUP → server restarts with new config, new requests use updated slot count
- [ ] Deadlock simulation: freeze `llama-server` with `kill -STOP` → deadlock detected within 20s → process SIGKILLed → restarted
- [ ] Performance degradation: throttle `llama-server` (e.g., cgroups CPU limit) → `server.performance_degraded` event logged within 90s
- [ ] Orphan recovery: kill LlamaForge (not servers), restart → servers reclaimed without re-spawn, slots erased, inference resumes within 10s
- [ ] WAL management: generate 10,000 requests → WAL checkpoint runs → WAL size stays under 100MB → `POST /api/maintenance/wal-checkpoint {"mode":"truncate"}` reclaims disk
- [ ] Writer thread crash: inject panic in writer → `writer_thread_alive` gauge drops to 0 → `/health` still responds → inference continues (metrics lost)
- [ ] GPU telemetry: `GET /metrics` includes `llamaforge_gpu_vram_free_bytes` and `llamaforge_gpu_utilization_pct` (CUDA systems)
- [ ] Readiness: all servers failed → `GET /ready` returns 503 → one server recovers → `GET /ready` returns 200
- [ ] Management API: `POST /api/servers/llama-8b/stop` → server stops → `POST /api/servers/llama-8b/start` → server starts
- [ ] Manual slot erase: `POST /api/servers/llama-8b/slots/2/erase` → slot cleared (verify via `/slots`)
- [ ] 1-hour sustained load test: no memory leak (RSS stable ±10%), no WAL growth beyond checkpoint cycle, no metric drift
- [ ] 10 rapid SIGHUPs in 1 second: thread count stable, no Tokio panics, final config correct
- [ ] Tokio worker starvation: no blocking call detected on worker threads under 1-hour load test (verified via tokio-console or instrumentation)
- [ ] Config writeback atomicity: POST LoRA update + concurrent SIGHUP + cross-filesystem fallback — config never corrupted
- [ ] WAL TRUNCATE during active read transaction: no deadlock (resolves within 5s)
- [ ] Metrics retention: after 30+ days, old events pruned automatically
- [ ] Model file corruption: mark as Failed, do NOT auto-restart (no crash loop)

**Chaos tests (automated):**
- [ ] `kill -9` server mid-request 100 times → all clients receive errors, all restarts succeed, audit log complete
- [ ] Fill disk (tmpfs) → metrics fallback to stdout, inference continues
- [ ] Corrupt config file + SIGHUP → error logged, old config retained, no crash

---

## Phase 5: Web UI & Prompt Cache Affinity (Weeks 14–16, parallel with Phase 4)

**Objective:** Browser-based management dashboard and prompt cache affinity for intelligent request placement.

### Tasks

| # | Task | Description | Est. |
|---|------|-------------|------|
| 5.1 | Web UI — static asset embedding | Embed htmx, Alpine.js, Tailwind CSS, ECharts into binary. Serve at `/ui/*`. CSP headers. | 4h |
| 5.2 | Web UI — Dashboard page | Real-time overview: throughput, goodput, queue depth, error rate, TTFT percentiles, per-server slot utilization bars. Auto-refresh via htmx polling. | 8h |
| 5.3 | Web UI — Server Management page | Server list with status badges. Start/stop/restart buttons. Slot status grid (processing/idle/stuck). "Erase Slot" button for stuck slots. | 6h |
| 5.4 | Web UI — Queue Inspector page | Live queue table: request_id, task_type, priority, position, wait time, model. Cancel and Boost buttons. | 5h |
| 5.5 | Web UI — Request Explorer page | Historical request query: filter by model, task_type, requestor, time range, status. Per-request detail view with timing breakdown. | 6h |
| 5.6 | Web UI — Configuration page | Read-only config display. "Reload Config" button. Divergence banner for failed writebacks. | 3h |
| 5.7 | Web UI — Security | `minijinja` auto-escaping. `htmx.config.allowEval = false`. No user data in `hx-*` attributes. `tower::catch_panic` for UI handlers. | 3h |
| 5.8 | Session affinity (Tier 1) | `HashMap<SessionId, ServerId>`. Route same-session requests to same server. Invalidate on server drain/fail. | 4h |
| 5.9 | System prompt hash (Tier 1) | 64-token character-count hash of system prompt. Lightweight affinity signal for cross-session cache reuse. | 4h |
| 5.10 | Model Registry Web UI page | View all loaded models, their server assignments, and performance characteristics. Live TPS display per model. | 4h |
| 5.11 | Slot cache lifecycle API | `POST /api/servers/{id}/slots/{slot}/cache/save`, `POST .../cache/restore`. Proxy to `llama-server` `/slots/{id}?action=save|restore`. | 3h |
| 5.12 | Grafana dashboard JSON | Export provisioning JSON with all panels from Section 11.2. | 3h |
| 5.13 | Prompt cache affinity — upgrade routing score | Integrate `w_affinity` term into routing formula (replacing Phase 2 stub). Hierarchical 4-level prefix hash for Tier 2. | 6h |
| 5.14 | Speculative decoding config + metrics | `draft_model_path`, `speculative_method` config. 4 Prometheus metrics (`speculative_draft_tokens_total`, `speculative_accepted_tokens_total`, `speculative_acceptance_rate`, gauge unregistration on server stop). | 4h |

### Verification Checkpoint 5 (VC-5)

**E2E test: operational UX**

- [ ] Web UI loads at `http://localhost:8000/ui/` — dashboard shows live metrics
- [ ] Server Management: click "Stop" → server status changes to "Stopped" → click "Start" → server comes back
- [ ] Queue Inspector: submit 10 requests with saturated slots → requests visible in queue → click "Cancel" → request disappears
- [ ] Request Explorer: query requests from last hour → filter by model → view per-request TTFT breakdown
- [ ] Stuck slot detection: intentionally stall a slot → highlighted red in UI → click "Erase Slot" → slot cleared
- [ ] XSS prevention: inject `<script>alert(1)</script>` in requestor field → rendered as escaped text in UI
- [ ] Session affinity: 5 sequential requests with same session → all route to same server (verify in audit log)
- [ ] System prompt affinity: 10 requests from different sessions sharing same system prompt → majority route to same server
- [ ] Cache save/restore: save slot 0 cache → erase slot 0 → restore → next request with same prefix has reduced TTFT (measurable improvement)
- [ ] Grafana: import provided JSON → all panels render with live data from Prometheus
- [ ] Model Registry page: shows all loaded models with server assignments and live TPS

---

## Phase 6: Hardening, Performance & Release (Weeks 17–19.5)

**Objective:** Production hardening, performance optimization, comprehensive test suite, documentation, and release packaging.

### Tasks

| # | Task | Description | Est. |
|---|------|-------------|------|
| 6.1 | SDK compatibility test suite | Python pytest: OpenAI SDK + Anthropic SDK black-box tests against mock-llama. All scenarios from Section 18.6. | 6h |
| 6.2 | Property-based tests | `proptest` for priority queue: ordering stability, starvation boost bounds, fair share invariants, admission control. | 5h |
| 6.3 | Micro-benchmarks | `criterion` benchmarks: queue insert/dispatch, routing score computation, starvation scan, prefix hash. CI regression gate (>20% p95 regression fails). | 4h |
| 6.4 | Load test harness | Custom Rust binary: configurable request rate, mixed task_type distributions. 1-hour sustained run. Validate LlamaForge metrics match client-observed metrics. | 6h |
| 6.5 | Chaos test suite | Automated fault injection: server kills, disk fills, queue floods, config corruption, slow backends. All scenarios from Section 18.5. | 5h |
| 6.6 | Connection pool tuning | Verify `reqwest` pool sizing (`pool_max_idle_per_host`). Test ephemeral port exhaustion under burst. `pool_idle_timeout` validation after server restart. | 3h |
| 6.7 | Memory profiling | Run 1-hour load test with `jemalloc` profiling. Verify no unbounded growth. Fix any leaks. | 4h |
| 6.8 | Binary size optimization | `strip`, LTO, `codegen-units=1`. Target < 30MB static binary (musl). | 2h |
| 6.9 | Cross-compilation | `cross` builds for: linux-x86_64-musl, linux-aarch64-musl, darwin-aarch64. Verify health check on each target. | 3h |
| 6.10 | Documentation | README, CONFIGURATION.md, DEPLOYMENT.md, CONTRIBUTING.md. Inline rustdoc for public APIs. | 6h |
| 6.11 | Systemd unit file | Production-ready unit with `TimeoutStopSec=630`, `Restart=on-failure`, environment file. | 1h |
| 6.12 | Release automation | GitHub Actions: tag-triggered release with multi-arch binaries + Docker images pushed to GHCR. | 3h |
| 6.13 | Security audit | Review: API key storage, bind address defaults, config file permissions, Web UI CSP, input validation on all endpoints. | 4h |
| 6.14 | Protobuf round-trip unit tests | Verify all protobuf message types serialize/deserialize correctly. Edge cases: empty repeated fields, oneof variants, default values. | 3h |
| 6.15 | LlamaForge crash persistence boundary test | Kill LlamaForge itself: queued requests lost (not zombies), audit records pre-crash survive, dispatch tracker rebuilds in <5s. | 3h |
| 6.16 | `llamaforge export-config` CLI command | Dump effective config (merged TOML + env + defaults) for debugging. | 2h |
| 6.17 | Additional Prometheus gauges | `llamaforge_schema_version`, `llamaforge_sqlite_wal_size_bytes`, `llamaforge_context_shifts_total`. | 2h |

### Verification Checkpoint 6 (VC-6) — Release Gate

**All previous VCs must still pass (regression).**

- [ ] SDK compatibility: 100% pass rate for OpenAI + Anthropic test suites
- [ ] Property tests: 10,000 iterations with no failures
- [ ] Micro-benchmarks: all within 20% of baseline
- [ ] 1-hour load test: RSS stable (< 10% growth), P99 TTFT < 2× P50, zero dropped requests, zero audit events lost
- [ ] Chaos tests: 100% recovery from all fault scenarios
- [ ] Cross-platform: health check passes on all 3 targets
- [ ] Binary size: < 30MB (stripped, musl)
- [ ] `cargo clippy -- -D warnings`: zero warnings
- [ ] `cargo deny check`: no blocked licenses, no known vulnerabilities
- [ ] All documentation reviewed and accurate
- [ ] Protobuf round-trip: all message types survive encode/decode with no data loss
- [ ] Crash persistence: kill LlamaForge, restart — no zombie requests, audit intact, tracker rebuilds <5s

---

## Risk Register & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `llama-server` API changes between versions | Medium | High | Pin to specific llama.cpp release tag. Abstract downstream API behind trait. Integration test against pinned version in CI. |
| Jinja template rendering mismatches | Medium | Medium | Use `minijinja` (same engine as llama.cpp uses internally). Test against all common chat templates (ChatML, Llama3, Mistral). |
| SQLite WAL corruption under crash | Low | High | WAL mode + `synchronous=NORMAL` provides crash safety for process crashes. Document host-level crash risk. Periodic `VACUUM INTO` backups. |
| Tokio task starvation from blocking ops | Medium | High | All blocking I/O via `spawn_blocking`. Enforce via clippy lint (`clippy::await_holding_lock`). Review all `.await` chains for hidden blocking. |
| `reqwest` connection pool exhaustion | Low | Medium | Set `pool_max_idle_per_host` to max parallel slots. Load test validates no `TIME_WAIT` accumulation. |
| Prometheus cardinality explosion | Low | Medium | Bounded enum mapping for all user-supplied labels. CI test validates max cardinality per metric. |
| gRPC/HTTP body size limits too restrictive | Medium | Low | Dynamic limits based on multimodal capability. Clear 413 error with current limit. Configurable override in TOML. |
| Bounded response channel starvation | Medium | Critical | Implemented in Phase 1 with 5s timeout. VC-1 tests slow-client scenario. |
| Phase 2 streaming refactor scope | Medium | High | DirectDispatcher trait seam from Phase 1 provides clean extension point. VC-1 validates the trait boundary works. |

---

## Dependency Graph (Critical Path)

```
Phase 0 ──► Phase 1 ──► Phase 2 ──┬──► Phase 3 ──┐
                                   │               ├──► Phase 6 (Release)
                                   ├──► Phase 4 ──┘    │
                                   │                    │
                                   └──► Phase 5 ───────┘
```

- Phases 0→1→2 are strictly sequential (each builds on the prior)
- Phase 3 (gRPC + advanced features) depends on Phase 2 (queue exists)
- Phases 4 and 5 can proceed in parallel after Phase 2 is complete
- Phase 6 requires all prior phases (3, 4, and 5)

---

## Revised Time Table

| Phase | Original | Revised | Delta | Reason |
|-------|----------|---------|-------|--------|
| 0 | 2 weeks | 2 weeks | — | GGUF parser + traits absorbed into scaffold |
| 1 | 3 weeks | 4 weeks | +1 week | Task 1.4 split (23h), 1.10 doubled, new tasks 1.14-1.17 |
| 2 | 3 weeks | 4 weeks | +1 week | Task 2.1 increased, streaming refactor, burst/race guards |
| 3 | 3 weeks | 3.5 weeks | +0.5 week | Mid-stream error SDK testing, Anthropic state machine edge cases |
| 4 | 3 weeks | 3 weeks | — | New tasks offset by clearer scope from Phase 1 foundations |
| 5 | 2 weeks | 2 weeks | — | slot_dispatch_map moved out, replaced by model registry + spec decoding |
| 6 | 2 weeks | 2.5 weeks | +0.5 week | Additional tests, CLI command, Prometheus gauges |
| **Total** | **18 weeks** | **21 weeks** | **+3 weeks** | |

---

## Success Metrics (Post-Launch)

| Metric | Target | Measurement |
|--------|--------|-------------|
| TTFT overhead vs direct llama-server | < 5ms P99 | Load test comparison |
| Token streaming latency overhead | < 0.5ms per chunk P99 | Instrumented proxy task |
| Gateway crash recovery time | < 10s (orphan reclaim + first request served) | Chaos test |
| Queue dispatch latency (enqueue → dispatch) | < 1ms when slots available | Prometheus histogram |
| Memory footprint (idle, 4 servers configured) | < 50MB RSS | Runtime measurement |
| Binary size | < 30MB (stripped, musl) | Build output |
| SDK compatibility | 100% for OpenAI Python + Anthropic Python | Automated test suite |

---

## Review Findings Addressed

All 32 findings from the adversarial review have been resolved in this revision.

### Category A: Missing Features & Tasks

| # | Finding | Resolution |
|---|---------|------------|
| A1 | Chat template acquisition | Task 0.11 (GGUF parser) + Task 1.4a (chat template loading) |
| A2 | GGUF metadata parser | Task 0.11 |
| A3 | Database abstraction traits | Task 0.10 |
| A4 | `server_configs` historical snapshot table | Addressed within Task 4.2 (config hot-reload writes snapshot on each diff-based reload) |
| A5 | Multimodal content handling | Task 2.14 expanded scope + routing constraints; full multimodal pipeline deferred to post-v1 |
| A6 | Speculative decoding config + metrics | Task 5.14 |
| A7 | Context Shift Rate metric | Task 6.17 (`llamaforge_context_shifts_total`) |
| A8 | ROCm and Vulkan backend support | Task 4.20 |
| A9 | `/v1/models/{model_id}` endpoint | Absorbed into Task 2.16 scope |
| A10 | `GET /api/models` management endpoint | Added to Task 4.15 |
| A11 | `LLAMAFORGE_MANAGED=1` env var | Task 1.16 (set) + Task 4.7 (check) |
| A12 | Model Registry Web UI page | Task 5.10 |
| A13 | Metrics retention pruning | Task 4.18 |
| A14 | `llamaforge export-config` CLI command | Task 6.16 |
| A15 | `llamaforge_schema_version` gauge | Task 6.17 |
| A16 | `llamaforge_sqlite_wal_size_bytes` gauge | Task 6.17 |
| A17 | 10-rapid-SIGHUP integration test | Task 4.19 |
| A18 | Protobuf round-trip unit tests | Task 6.14 |
| A19 | `ctx_shift` config option | Absorbed into Task 1.1 (server config → CLI flag mapping) |
| A20 | `sse_keepalive_interval_secs` config field | Task 0.4 (explicit config field) |
| A21 | `queue_update_interval_secs` config field | Task 0.4 (explicit config field) |
| A22 | Bounded response channel | Task 1.4d |
| A23 | `per_requestor_limit` config field | Task 0.4 (explicit config field) |
| A24 | CI `dry-run` mode gate | Task 4.16 + CI pipeline references in VC-0 |
| A25 | Health check stagger | Task 1.17 |

### Category B: Sequencing Errors & Underestimated Complexity

| # | Finding | Resolution |
|---|---------|------------|
| B1 | Phase 1 needs DirectDispatcher trait | Task 1.14 |
| B2 | Task 1.10 depends on slot_dispatch_map | Task 1.15 (moved from Phase 5) |
| B3 | Phase 2 streaming handler refactor | Task 2.12 revised (estimate 5h → 10h, explicit refactoring scope) |
| B4 | Task 1.4 underestimated | Split into Tasks 1.4a/1.4b/1.4c/1.4d (total 23h) |
| B5 | Task 1.10 underestimated | Estimate revised 6h → 12h with expanded description |
| B6 | Task 2.1 underestimated | Estimate revised 8h → 14h |
| B7 | Phase 2 routing stubs w_affinity | Task 2.8 explicit note about stub |
| B8 | Tasks 1.8 and 4.10 overlap | Task 4.10 clarified as upgrade (ring buffer front-end), not replacement |
| B9 | VRAM validation needs GGUF parser | Task 2.15 clarified: uses Task 0.11, falls back to config override |

### Category C: Verification Checkpoint Gaps

| # | Gap | Resolution |
|---|-----|------------|
| C1 | Slow-client backpressure | VC-1: slow client test + bounded channel test |
| C2 | Dispatch tracker burst race | VC-2: burst routing test + Task 2.17 |
| C3 | VRAM ledger concurrent spawn race | VC-2: VRAM ledger race test + Task 2.18 |
| C4 | Mid-stream crash error format | VC-3: mid-stream crash test + Task 3.18 |
| C5 | Starvation boost saturation | VC-2: starvation boost saturation test |
| C6 | DRR tier re-activation | VC-2: DRR tier re-activation test |
| C7 | DRR startup window | Covered by VC-2 scheduling correctness tests (first-60s behavior) |
| C8 | Config writeback atomicity | VC-4: config writeback atomicity test |
| C9 | WAL TRUNCATE blocking | VC-4: WAL TRUNCATE during active read test |
| C10 | LlamaForge crash persistence | VC-6: crash persistence test + Task 6.15 |
| C11 | Tokio worker thread starvation | VC-4: Tokio worker starvation test + Task 4.21 |
| C12 | Model file corruption no auto-restart | VC-4: model file corruption test |
