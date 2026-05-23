# Design ↔ implementation map

The authoritative mapping from numbered sections in [`design.md`](design.md) to the crates and modules that implement them. Maintained as part of every PR that adds or moves implementation surface — if you change a numbered design section, update this table (or vice versa).

For per-component detail, see [`docs/component-designs/`](component-designs).
For deferred items not yet implemented, see [`docs/v1.1-deferred.md`](v1.1-deferred.md).

## Section map

| Design § | Topic | Implementing crate(s) / module(s) |
|---|---|---|
| §1 | Executive Summary | (narrative — no code) |
| §2 | Goals and Non-Goals | (narrative — no code) |
| §3 | Related Work & Positioning | (narrative — no code) |
| §4 | System Architecture Overview | `crates/llamaforge/src/lib.rs`, `crates/llamaforge/src/build.rs` (composition root) |
| §4.1 | Failure Boundaries & Durability | `crates/llamaforge-metrics/src/sqlite/` (WAL, atomic writeback), `crates/llamaforge-server/src/manager.rs` (orphan recovery) |
| §4.2 | Hexagonal Layering | `crates/llamaforge-adapters/src/` (port-trait adapters) |
| §5.1 | Transport Layer | `crates/llamaforge-transport/`, `crates/llamaforge-dispatch/` |
| §5.2 | Request Queue & Scheduler | `crates/llamaforge-queue/` |
| §5.3 | Server Manager | `crates/llamaforge-server/` |
| §5.4 | Metrics Collector & Aggregator | `crates/llamaforge-metrics/src/{ingest,sqlite,filter}.rs` |
| §5.5 | Config Manager & Web UI | `crates/llamaforge-config/`, management routes in `crates/llamaforge-transport/src/management.rs` |
| §5.6 | Audit Log & Event Store | `crates/llamaforge-metrics/src/sqlite/{batch_collector,writer}.rs`, `crates/llamaforge-adapters/src/audit.rs` |
| §6.1 | Core Types (Rust) | `crates/llamaforge-types/` |
| §6.2 | SQLite Schema | `crates/llamaforge-metrics/src/sqlite/{migrations,store}.rs` |
| §7.0 | Protocol Buffers Definition | `proto/`, `crates/llamaforge-proto/` |
| §7.1 | gRPC Streaming: Chat Completion | `crates/llamaforge-transport/src/grpc.rs` |
| §7.2 | OpenAI HTTP Shim | `crates/llamaforge-transport/src/openai.rs` (Anthropic shim: `anthropic.rs`) |
| §7.3 | Queue Status | `crates/llamaforge-transport/src/management.rs` |
| §7.4 | Server Management | `crates/llamaforge-transport/src/management.rs`, `crates/llamaforge-adapters/src/server_control.rs` |
| §8 | Request Lifecycle | `crates/llamaforge-adapters/src/inference_engine.rs`, `crates/llamaforge-dispatch/src/proxy.rs` |
| §9.1 | Priority Computation | `crates/llamaforge-queue/src/admission.rs` (priority bonus, task-type base) |
| §9.2 | Queue-Only Preemption (v1) | `crates/llamaforge-queue/src/starvation.rs`, `crates/llamaforge-queue/src/queue.rs` |
| §9.3 | Preemptive Eviction (v2) | **Deferred to v1.1+** — see `docs/v1.1-deferred.md` |
| §10.1 | `llama-server` Control Surfaces | `crates/llamaforge-server/src/{process,task}.rs`, `crates/llamaforge-dispatch/src/client.rs` (`SlotErasure`) |
| §10.2 | Routing Algorithm | `crates/llamaforge-routing/src/router.rs`, `weights.rs`, `affinity.rs` |
| §10.3 | Prompt Cache Affinity | `crates/llamaforge-routing/src/affinity.rs` |
| §10.4 | Request Coalescing | **Deferred** (design doc: "Future") |
| §11.1 | Dashboard Views | `/api/metrics/*` endpoints in `crates/llamaforge-transport/src/management.rs`; dashboard frontend not in v1.0 |
| §11.2 | Grafana Integration | Prometheus exporter at `/metrics` (`crates/llamaforge-adapters/src/metrics_exporter.rs`) |
| §13.1 | Trait Definitions (Persistence) | `crates/llamaforge-metrics/src/traits.rs` |
| §13.2 | SQLite Implementation | `crates/llamaforge-metrics/src/sqlite/store.rs` |
| §13.3 | Config Overlay Semantics | `crates/llamaforge-metrics/src/traits.rs::ConfigOverlayStore` trait (no impl in v1.0) — see `docs/v1.1-deferred.md` |
| §13.4 | Event Batching Architecture | `crates/llamaforge-metrics/src/sqlite/batch_collector.rs` |
| §13.5 | Path to Postgres (v2) | **Deferred** (design doc: "v2") |
| §13.6 | Rejected Alternatives | (narrative — no code) |
| §18.1 | Unit Tests | per-crate `mod tests` |
| §18.2 | Integration Tests (Mock Backend) | `crates/mock-llama/`, `crates/llamaforge-e2e/` |
| §18.3 | End-to-End Tests | `crates/llamaforge-e2e/tests/` (5 protocol tests from PR10a) |
| §18.4 | Load and Stress Tests | **Deferred** to a post-v1.0 perf pass |
| §18.5 | Chaos Tests | **Deferred** to v1.1 |
| §18.6 | SDK Compatibility Tests | **Deferred** to v1.1 |
| §18.7 | Micro-Benchmarks | **Deferred** to v1.1 (no Criterion suite in v1.0) |
| §18.8 | CI Pipeline | `.github/workflows/ci.yml` |

## How to keep this file honest

When a PR touches numbered design content:

1. If it implements a new section: add the row.
2. If it moves an implementation between crates: update the row.
3. If it defers a section: move the row to a "Deferred" line linking `v1.1-deferred.md`.

The reviewer rubric R11–R16 leans on this table to spot drift between design and code.
