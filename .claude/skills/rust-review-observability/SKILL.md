---
name: rust-review-observability
description: >
  Reviews Rust code for observability and telemetry architecture. Checks 6 rules covering
  structured tracing (not println/log), multi-layer subscriber architecture, keeping
  instrumentation out of domain logic (decorator pattern), distributed trace context propagation,
  telemetry transport efficiency at scale, and actionable symptom-based alerting design.
  Use this skill whenever reviewing Rust code for logging, tracing, metrics, observability,
  OpenTelemetry integration, or monitoring strategy. Trigger on any Rust review involving
  tracing, logging, telemetry, distributed tracing, or observability architecture — even if
  the user just says "check the logging" or "review our tracing setup."
---

# Rust Observability & Telemetry Review

You are a sub-agent reviewer. Your job is to audit Rust code against 6 rules governing observability architecture. Good observability is what separates a service you can debug in production from one you can only pray about. These rules ensure telemetry is structured, composable, decoupled from domain logic, and operationally useful.

## How to review

1. Search for `tracing::`, `log::`, `println!`, `eprintln!`, `#[instrument]`, `info!`, `warn!`, `error!`, `debug!` to map all instrumentation points.
2. Find the subscriber/telemetry initialization code (usually in `main.rs` or a telemetry module).
3. Check whether instrumentation appears inside domain-layer code or only at boundary/adapter layers.
4. For distributed services, check HTTP/gRPC middleware for trace context injection/extraction.
5. Write `review_report.md` and `inline_comments.md` per `references/output_format.md`.

## The Rules

### Rule 24 — Structured Tracing, Not Printf Logging

All logging must use the `tracing` crate's span-based, structured model. `println!` debugging and flat `log::info!` calls that produce unstructured text are violations. Every logical operation should open a tracing span with contextual fields, and log events (`info!`, `warn!`, `error!`) should be emitted within spans so the observability backend can reconstruct execution hierarchies.

**What to look for:**
- `println!` or `eprintln!` used for operational logging (debug prints gated behind `#[cfg(debug_assertions)]` are marginal but acceptable).
- `log::info!`, `log::warn!`, etc. from the `log` crate instead of `tracing` equivalents.
- `tracing::info!("Processing user {}", user_id)` — positional formatting loses structure. Should be `tracing::info!(user_id = %user_id, "Processing user")` with named fields.
- Missing spans around logical operations — if a request handler just emits events without wrapping them in a span, you lose the hierarchy.

### Rule 25 — Multi-Layer Subscriber Architecture

The `tracing-subscriber` configuration must use the composable Layer system to route telemetry to multiple destinations simultaneously. At minimum:

- A human-readable output layer (`fmt::Layer` with `EnvFilter` for stdout during development)
- A structured export layer (OpenTelemetry layer emitting OTLP to Jaeger, Datadog, Honeycomb, etc. for production)

Monolithic subscriber setups that only write to stdout OR only export to a single backend are violations. The Layer architecture exists to avoid this false choice.

**What to look for:**
- `tracing_subscriber::fmt::init()` as the only subscriber setup — this gives you stdout only.
- A single subscriber with no layering — the code should use `Registry::default().with(layer1).with(layer2)`.
- No conditional configuration between development and production environments.
- Missing `EnvFilter` for dynamic log-level control.

### Rule 26 — Observability Must Not Leak Into Domain Logic

Tracing instrumentation (`#[instrument]`, `tracing::info!`, OpenTelemetry metric calls) must never appear inside domain layer code. Observability is an infrastructure concern and should be applied via the Decorator pattern at application boundary layers.

The correct pattern: a wrapper struct (e.g., `TracingCustomerRepository<T: CustomerRepository>`) that implements the domain trait, adds instrumentation, and delegates to the inner implementation.

**What to look for:**
- `#[instrument]` on methods in domain-layer modules.
- `tracing::info!` or `tracing::error!` inside domain entities, value objects, or domain services.
- Metric recording (`counter!`, `histogram!`) in domain logic.
- The domain crate's `Cargo.toml` depending on `tracing`, `metrics`, or `opentelemetry`.

### Rule 27 — Distributed Trace Context Propagation

In any multi-service architecture, every outbound HTTP/gRPC call must inject the current trace context (W3C Trace Context headers) into request headers, and every inbound handler must extract it. An unbroken distributed trace across service boundaries is a non-negotiable operational requirement.

**What to look for:**
- HTTP client calls (`reqwest`, `hyper`) without trace context injection middleware.
- gRPC clients without metadata propagation.
- Inbound HTTP handlers that don't extract `traceparent` / `tracestate` headers.
- Custom trace header formats instead of W3C standard — interoperability matters.
- Services that start new root spans on every inbound request instead of continuing the caller's trace.

### Rule 28 — Telemetry Transport Efficiency at Scale

For high-volume services, the telemetry export pipeline must account for transport efficiency. Naive JSON-over-HTTP OTLP export is acceptable for low-volume services but becomes a cost and performance liability at scale.

**What to look for:**
- High-throughput services exporting telemetry as uncompressed JSON.
- No mention of batching in the exporter configuration (`BatchSpanProcessor` vs `SimpleSpanProcessor`).
- Ultra-low-latency paths where even standard `tracing` overhead is measurable — check for specialized low-allocation tracing (e.g., `fastrace`) rather than disabling instrumentation entirely.
- Missing gzip/zstd compression on OTLP export endpoints.

### Rule 29 — Actionable, Symptom-Based Alerting

Observability outputs must be designed to support symptom-based alerting — alerts that fire when user experience degrades, not when individual component metrics spike. This means telemetry must have sufficient dimensionality.

**What to look for:**
- Metrics that only track component internals (queue depth, thread count) without corresponding user-facing metrics (request latency percentiles, error rates by endpoint).
- "Log and pray" approaches — logging that exists but with no clear consumption strategy (who reads these logs? what dashboard shows them? what alert fires on them?).
- Missing latency histograms (only averages — p50/p95/p99 matter more).
- Error metrics without endpoint/status-code dimensionality.

## Severity calibration

- **HIGH:** Rule 24 (`println!` as operational logging in production), Rule 26 (tracing inside domain crate with tracing in its Cargo.toml), Rule 27 (dropped trace headers in multi-service setup)
- **MEDIUM:** Rule 25 (single-layer subscriber), Rule 24 (unstructured tracing fields), Rule 29 (no latency percentiles)
- **LOW:** Rule 28 (transport efficiency in low-volume services), Rule 29 (alerting design suggestions)

## Output

After reviewing, produce both files per `references/output_format.md`. Read that file first if you haven't already.
