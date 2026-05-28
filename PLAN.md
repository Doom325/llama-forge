# PLAN.md

This plan addresses the [E2E] bugs filed during testing, as tracked on GitHub.

## Phase 1 — Transport & API Mis-mappings (PR-1)
**Goal:** Fix critical error mapping bugs and shim inconsistencies (#231, #226, #223, #224, #225).
- [ ] **1.1** Fix #231: Map `DispatchError::RateLimited` to `ApiError::QueueFull` in HTTP handlers and `RESOURCE_EXHAUSTED` in gRPC.
- [ ] **1.2** Fix #226: Emit WARN log for missing `anthropic-version` header in Anthropic shim.
- [ ] **1.3** Fix #223: Include `"object":"chat.completion.chunk"` in OpenAI streaming chunks.
- [ ] **1.4** Fix #224: Correct `finish_reason` mapping (e.g., 'length' vs 'stop').
- [ ] **1.5** Fix #225: Ensure 413 Payload Too Large uses canonical JSON envelope and has request-id.
- [ ] **1.6** Open PR-1.

## Phase 2 — Service Health & Readiness (PR-2)
**Goal:** Implement missing health/readiness endpoints (#227, #234).
- [ ] **2.1** Fix #227: Register `grpc.health.v1.Health` service.
- [ ] **2.2** Fix #234: Implement `/ready` and `/ready/inference` endpoints.
- [ ] **2.3** Open PR-2.

## Phase 3 — Metrics & Observability (PR-3)
**Goal:** Fix metrics accumulation and missing fields (#232, #230).
- [ ] **3.1** Fix #232: Ensure Prometheus `*_total` counters accumulate correctly.
- [ ] **3.2** Fix #230: Populate `dispatched_at` and `slot_id` in metrics rows.
- [ ] **3.3** Open PR-3.

## Phase 4 — Lifecycle & Server Stability (PR-4)
**Goal:** Fix state transition bugs and OOM handling (#233, #222, #219).
- [ ] **4.1** Fix #233: Prevent illegal 'Starting -> Restarting' state transition.
- [ ] **4.2** Fix #222: Improve error message for concurrent serve rejection.
- [ ] **4.3** Fix #219: Add pre-flight memory check and graceful OOM handling.
- [ ] **4.4** Open PR-4.

## Phase 5 — CLI & Config Ergonomics (PR-5)
**Goal:** Improve CLI and config validation (#220, #221).
- [ ] **5.1** Fix #220: Auto-detect `llama-server` binary in `llamaforge init`.
- [ ] **5.2** Fix #221: Map invalid ServerId slug to `CONFIG_VALIDATION_ERROR`.
- [ ] **5.3** Open PR-5.

## Phase 6 — SSE Flake & Performance (PR-6)
**Goal:** Resolve intermittent failures and timeouts (#218, #148, #133).
- [ ] **6.1** Fix #218: Audit and fix SSE close-propagation (Tier 2 fix).
- [ ] **6.2** Fix #148: Address 504 Gateway Timeout on back-to-back streaming/non-streaming.
- [ ] **6.3** Fix #133: Stop hardcoding requestor/task_type to 'anonymous'/'default'.
- [ ] **6.4** Open PR-6.
