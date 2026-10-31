# LlamaForge

**A Rust orchestrator for fleets of `llama-server` processes.** Priority scheduling, KV-cache-aware routing, OpenAI + Anthropic + gRPC on the same core, SQLite-backed per-request metrics, and a web UI — in one single binary with no Python or Docker.

> **Status:** `v0.1.0-draft`. In active single-maintainer development. Backend components (config, metrics, server manager, queue, routing, dispatch, dual transport) and the first-run setup wizard are implemented and under E2E test. The dashboard frontend is in integration. **Not production-ready yet.**

---

## Why this exists

I run local models every day and the fleet-management story is bad. `llama-server` is excellent at serving *one* model on *one* port — but as soon as you want two models, two priorities, or two users sharing an 80 GB card, you end up gluing shell scripts, nginx, and spreadsheets together. Existing proxies (LiteLLM) handle *cloud* APIs; existing high-throughput engines (vLLM) replace llama.cpp entirely and only run on NVIDIA. There was no production-shaped orchestrator for the llama.cpp ecosystem itself — one that owns process lifecycle, priority, routing, and metrics while *keeping* GGUF, Metal, and the quantization zoo.

LlamaForge is that layer. It sits where LiteLLM sits for cloud APIs, but owns the full local inference stack.

---

## What it does

- **Supervises `llama-server` processes.** Spawn, health-check, restart-with-backoff, graceful drain, full flag surface (context, GPU layers, parallel slots, flash-attn, KV cache types).
- **Routes by slot availability *and* KV-cache (prompt) affinity.** Multiple instances, cache-aware slot pinning via `id_slot`, cross-instance cache lifecycle (erase/save/restore) — orchestrator-level benefits that bare `llama-server` can't coordinate.
- **Priority-aware queue with queue-level preemption.** Higher-priority requests jump ahead of queued ones; in-flight requests run to completion in v1 (prefill-only eviction is a v2 target).
- **One core, three protocols.** gRPC on `:50051` is canonical (strongly-typed `request_id`, `task_type`, `requestor`, `priority`). HTTP shims on `:8000` give you OpenAI-compatible `/v1/chat/completions` and `/v1/completions` and Anthropic-compatible `/v1/messages` — plug in existing SDKs unmodified.
- **Per-request observability.** TTFT, tokens/sec, queue wait, which server/slot served each request — persisted to SQLite, exported via Prometheus, surfaced in a web UI.
- **Single Rust binary.** No Python, no CUDA toolkit beyond what `llama-server` already needs, no container runtime. Apple Silicon / Metal supported out of the box.

---

## Architecture

```
   OpenAI / Anthropic SDKs        Native gRPC client         Web UI
         │ HTTP/JSON                  │ gRPC/H2                │ HTTP/JSON
         ▼                            ▼                        ▼
   ┌─────────────────────────────────────────────────────────────────┐
   │                   LlamaForge core (Rust)                        │
   │                                                                 │
   │   Transport (axum + tonic)                                      │
   │        │                                                        │
   │        ▼                                                        │
   │   Priority Queue  ──►  Smart Routing  ──►  Dispatch Proxy       │
   │   (per-requestor      (slot availability +   (stream tokens,    │
   │    limits, admission   KV-cache affinity +    cancel on client  │
   │    control, ETAs)      model match)          disconnect)        │
   │        │                                       │                │
   │        └──────────► Server Manager ◄───────────┘                │
   │                     (spawn, health, restart,                    │
   │                      drain, VRAM ledger)                        │
   │                            │                                    │
   │   Metrics Store (SQLite) ◄─┴─► Config Manager (ArcSwap hot-     │
   │                                 reload, UI overlays)            │
   └─────────────────────────────────────────────────────────────────┘
                                │
                     ┌──────────┼──────────┐
                     ▼          ▼          ▼
              llama-server  llama-server  llama-server
                 (7B Q5)      (34B Q4)     (embedding)
```

Full architecture, failure boundaries, data models, and the request lifecycle live in [`docs/design.md`](docs/design.md).

---

## Positioning

| | **vLLM** | **LiteLLM** | **Bare llama-server** | **LlamaForge** |
|---|---|---|---|---|
| Target hardware | NVIDIA datacenter | N/A (proxy) | Anything llama.cpp runs on | Anything llama.cpp runs on |
| GGUF + quantization zoo | Partial | N/A | ✓ | ✓ |
| Apple Silicon / Metal | ✗ | N/A | ✓ | ✓ |
| Multi-model | ✗ | ✓ | ✗ | ✓ |
| Priority queue | ✓ | ✗ | ✗ | ✓ |
| Process lifecycle | ✗ | ✗ | — | ✓ |
| Web UI | ✗ | ✓ | ✗ | ✓ |
| gRPC API | ✗ | ✗ | ✗ | ✓ |
| Deployment | Python + CUDA | Python | Single binary | Single binary |

**LlamaForge is not trying to be vLLM.** vLLM wins on raw throughput because its scheduler lives *inside* the inference engine. LlamaForge's scheduler is external — it routes whole requests to `llama-server` slots and delegates batching to llama.cpp. The trade-off is deliberate: hardware diversity, llama.cpp's ecosystem, and operational simplicity, in exchange for a modest batching-efficiency gap at high concurrency. See [design.md §3](docs/design.md#3-related-work--positioning) for the full argument.

---

## Quickstart

```sh
cargo build --release
./target/release/llamaforge init                   # writes config/llamaforge.toml.example
./target/release/llamaforge serve --config <path>  # starts the orchestrator
```

Point an OpenAI SDK at `http://localhost:8000/v1`, or an Anthropic SDK at `http://localhost:8000/v1/messages`, or a native gRPC client at `localhost:50051`. The web UI is at `http://localhost:8000/ui/`.

**First-run setup wizard.** `llamaforge serve` with no config launches a browser-based wizard that detects your hardware, scans `~/models/` for GGUF files, and generates a starter `llamaforge.toml` with VRAM-aware `gpu_layers` and `ctx_size` recommendations.

---

## End-to-end tests

```sh
cargo build --bin mock-llama                       # required by the e2e harness
cargo test -p llamaforge-e2e --tests               # also needs `protoc`
                                                   # Linux also needs `mold` (fast linker; macOS uses the system linker)
```

Integration tests run against `mock-llama` (an in-process `llama-server` stand-in with deterministic delays, errors, and token streams), so no GPU or real model is required. A separate manual E2E tracker lives at [`docs/MANUAL_E2E_TEST_TRACKER.xlsx`](docs/MANUAL_E2E_TEST_TRACKER.xlsx).

**Benchmarks:** TBA. The harness exists in `crates/llamaforge-e2e`; representative numbers (TTFT, tokens/s, routing-decision overhead vs. bare `llama-server`) will land here once the dashboard integration stabilizes.

---

## Repository layout

```
crates/
  llamaforge          # the binary — init, serve, cli
  llamaforge-core     # config, types, routing, queue
  llamaforge-server   # llama-server process lifecycle + health + telemetry
  llamaforge-api      # transport, OpenAI/Anthropic adapters, dispatch
  llamaforge-metrics  # SQLite-backed metrics store
  llamaforge-proto    # generated tonic/prost bindings (source: proto/)
  llamaforge-e2e      # end-to-end test harness
  mock-llama          # in-process llama-server mock for deterministic tests
proto/                # .proto definitions — source of truth for all gRPC types
docs/                 # design, component designs, implementation plan
ui/                   # SvelteKit dashboard
```

---

## How this was built

LlamaForge is a solo project, but it wasn't built by one agent working linearly. Each component (config manager, server manager, queue, routing, dispatch, transport, setup wizard, frontend) was driven to completion by a **pair of AI agents** — one *coder*, one *reviewer* — communicating over an MCP message bus I built for this purpose, using **GitHub PRs as the handshake artifact**.

- The coder opens a PR for a phased task from `PLAN.md`.
- The reviewer fetches it, runs a 32-rule architecture rubric (traits, boundaries, crate hygiene, safety, observability, CI) against the diff, posts inline findings, and sends a verdict back on the message bus.
- The coder iterates until approval, then squash-merges via `gh pr merge --auto`.
- I act as the operator: I set the design, scope each phase, break deadlocks, ratify design-choice calls, and merge the agent-authored PRs after independent verification.

The result is 316 reviewed PRs on a solo repo, every change gated by an adversarial second pair of eyes, every design decision traceable to a numbered rule or a design-doc section. The orchestration layer that produced this code is itself a small system worth looking at — the agent protocol, reviewer skills, and relay are all in `.claude/skills/` and `~/.claude/CLAUDE.md`.

If this project interests you, that pipeline is as much of the signal as the code it produced.

---

## Documentation

- **Master design:** [`docs/design.md`](docs/design.md) — the system constitution. Goals/non-goals, architecture, API spec, request lifecycle, priority system, routing strategy, failure modes, security model.
- **Component designs:** [`docs/component-designs/`](docs/component-designs) — per-component specs (one file per subsystem).
- **Implementation plan:** [`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md) — phased plan with verification checkpoints VC-0 through VC-6.
- **Frontend architecture:** [`FRONTEND_ARCHITECTURE.md`](FRONTEND_ARCHITECTURE.md) and visual references in [`design_reference/v1/`](design_reference/v1/).
- **Deferred to v1.1:** [`docs/v1.1-deferred.md`](docs/v1.1-deferred.md).

---

## Status & roadmap

**Shipped (v0.1.0-draft):**
Config manager with hot-reload · SQLite metrics store · Server manager (spawn/health/restart/drain + VRAM ledger) · Priority queue with per-requestor limits · KV-cache-affinity routing · Dispatch proxy (streaming, cancellation) · Dual-protocol transport (gRPC + OpenAI/Anthropic HTTP shims) · First-run setup wizard · Prometheus metrics endpoint · SvelteKit dashboard (in integration).

**Next (v1.0):**
Dashboard polish and release · representative benchmarks · hardened E2E suite.

**v2 targets:**
Prefill-only in-flight eviction · multi-node routing · response caching · richer auth.

---

## License

Apache-2.0. See [LICENSE](LICENSE).

---

*Maintained by [Sumanth Gottipati](https://github.com/gsumanth). Built with Rust, Tokio, tonic, axum, tracing, rusqlite — and a lot of agent-pair review cycles.*
