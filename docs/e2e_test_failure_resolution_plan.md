# E2E test failure resolution plan

**Source report:** `docs/e2e_test_report_2026-05-17.md`
**Commit under test:** `be33fd8`
**Author:** CoderAgent_llamaforge-test
**Status:** Draft — pending review

Maps each of the 16 bugs (B-CRIT + B-1..B-15) and the cancellation-knob recommendation to a concrete fix, locks each in with a test, and groups the work into PR-sized chunks. PR ordering is dictated by the dependency graph in §"Ordering" at the end of this document.

Severity carries over from the report. "Effort" is rough engineering judgement: S ≤ 1 hr, M 1–4 hr, L 4–8 hr.

---

## PR-1 — Transport routing keystone (unblocks the report)

**Bugs:** **B-CRIT** (model hardcoded to `"dummy"`), plus the error-mapping defect baked into the same code path.
**Scope:** `crates/llamaforge-transport/src/http.rs` only (OpenAI handler at `:101-141`, Anthropic handler at `:339-377`).
**Effort:** M. **Severity:** P0.

### Root cause

`handle_openai_request` and `anthropic_messages_handler` both build a placeholder `IncomingRequest` for `validate_routable`:

```rust
// http.rs:112-125 (OpenAI) and :350-363 (Anthropic)
.validate_routable(&IncomingRequest {
    model_target: ModelTarget::Exact(ModelAlias::new("dummy".into())),  // <-- hardcoded
    messages: vec![],
    params: Default::default(),
    …
})
```

The `req: ChatCompletionRequest` already on the stack carries the real model, messages, and params from the parsed body — but they're discarded for the validator call. The validator's failure is then `.map_err(|_| ApiError::QueueFull{…})`'d at `:126-137` and `:364-376`, so a model-mismatch ends up as `429 queue_full` with the wrong `Retry-After`. The downstream call `inference_engine.chat_completion(req)` uses the *real* model, which is how some tests (those that aliased the server `"dummy"`) accidentally worked.

### Fix

1. Build the `IncomingRequest` from the parsed `req`: `model_target = ModelTarget::Exact(ModelAlias::new(Arc::from(req.model.as_str())))`. Carry `messages: req.messages.clone()` and `params` from the parsed request.
2. Stop collapsing every validator error into `QueueFull`. Pattern-match on `AdmissionError`:
   - `NoServerForModel` → `ApiError::ModelNotFound` (404, OpenAI `model_not_found` / Anthropic `not_found_error`).
   - `QueueFull` → existing `429` path.
   - Everything else → `ApiError::Internal`.
3. Drop the workaround comment at `crates/llamaforge-e2e/src/lib.rs:127-133` ("server alias must be `dummy`") — no longer true.

### Tests

- **New unit test** in `transport/src/http.rs::tests`: send `{"model": "real-alias"}`; mock validator asserts the received `model_target` equals `"real-alias"` (currently impossible because the validator sees `"dummy"`).
- **Adjust** `crates/llamaforge-e2e/src/lib.rs:131` — `start_full_with_alias` no longer needs to force `"dummy"`. Default to a stable but real alias (e.g. `"mock"`) and update tests that asserted on the alias.
- **New e2e** `crates/llamaforge-e2e/tests/openai_routing.rs` sending the configured alias and asserting `200` with non-empty content; this is the report's repro from B-CRIT.

### Dependencies / unblocks

- **Unblocks B-7** (the same code path is the place to map `ModelNotFound → 404`).
- Indirectly **unblocks B-12** verification: with the dummy-hardcode gone, B-12's "rejects burst traffic" repro is reproducible against arbitrary aliases.

---

## PR-2 — Queue admission: enqueue when slots are full, not reject

**Bugs:** **B-12** (burst rejected as `NoServerForModel`).
**Scope:** `crates/llamaforge-routing/src/router.rs::has_capacity` (lines 251-319), `crates/llamaforge-queue/src/admission.rs:152-169` (the model-validation gate).
**Effort:** M. **Severity:** P1.

### Root cause

`Router::has_capacity` (router.rs:251-319) folds two distinct questions into one boolean: "does any server serve this model?" *and* "does any of those have a free slot right now?" When servers are all-running-but-all-busy, the first answer is *yes* but the second is *no*, and the function returns `false`. Admission reads that as `NoServerForModel` (admission.rs:153-168) and synchronously rejects. The queue never sees the request, defeating its purpose.

`design.md` §5.2 and `04_REQUEST_QUEUE_AND_SCHEDULER.md` both place the "no server serves this model" rejection at admission *and* the "all servers busy → queue" path *into* the queue.

### Fix

1. Split `Router`'s capacity API into two methods:
   - `serves_model(&self, target, capabilities) -> bool` — true if any *running* server matches the model and capability set, regardless of slot occupancy.
   - `has_idle_slot(&self, target, capabilities) -> bool` — the existing per-slot check (kept for the scheduler hot path / fast-fail).
2. `AdmissionController::submit` calls `serves_model` for the synchronous reject path. Slot occupancy stops mattering at admission.
3. `has_idle_slot` stays available for the routing-decision hot path inside the scheduler — admission no longer reads it.

### Tests

- **Unit** in `routing/src/router.rs`: with 1 running server (1 slot) and `in_flight_count = 1`, `serves_model("alias")` returns `true`, `has_idle_slot` returns `false`. Mirror for "no servers serve the model" (both false).
- **Unit** in `queue/src/admission.rs`: with `has_capacity` mocked as currently coupled vs the new split shape, admission admits when `serves_model && !has_idle_slot`.
- **New e2e** `crates/llamaforge-e2e/tests/queue_burst.rs`: 12 parallel POSTs against a 2-slot server, assert 12 × 200, no 5xx. Mirrors the report's B-12 repro.

### Dependencies

- Depends on PR-1 (otherwise the test can't send a non-dummy model). Lands second.

---

## PR-3 — Server lifecycle warming guard (cold-start empty 200)

**Bugs:** **B-6** (cold-start `200` with empty content) and the **anthropic-streaming cold-start flake** the report attributes to the same root cause. Plus **B-13** (`started_at` / `last_restart_at` null) since both touch `ServerStateView` projection.
**Scope:** the routing layer's startup-grace heuristic (`crates/llamaforge-routing/src/router.rs:303-309`) and the `ServerStateAdapter` projection (`crates/llamaforge/src/build.rs:226-275`).
**Effort:** M. **Severity:** P1 (B-6) / P3 (B-13).

### Root cause B-6

Today `Router::has_capacity` returns `true` while servers are still `Starting`, gated by the `startup_wait_secs` window — admission lets the request through, dispatch finds no `Running` server, the proxy stream closes empty, the shim aggregator emits `finish_reason="stop", completion_tokens=0`. The error here is silent success: a degraded boot returns "stop with 0 tokens" instead of `503 Retry-After` or a wait-and-retry on the server side.

### Root cause B-13

`ServerStateAdapter::all_servers` in `crates/llamaforge/src/build.rs:236-275` projects `server-manager::ServerStateSnapshot → routing::ServerSnapshot` but drops timestamp fields. `ServerStateSnapshot` carries `first_health_ok_at` (`server-manager/src/manager.rs:124-127`) and the manager has `restart_count` — the management API just isn't reading them.

### Fix B-6

1. The non-stream HTTP shim aggregator (transport `:206-245`) gains a "stream ended without a terminal `Stop` chunk" guard: track `saw_stop: bool` across the aggregation loop and return `503 ServiceUnavailable` with `Retry-After: 1` when the stream closes with `!saw_stop`. This is the bug shape — a stream that closes without ever observing `ChatCompletionChunk::Stop { .. }` means the backend went away mid-response. A legitimately empty completion (`Stop { output_tokens: 0, .. }` after no `Token` chunks) is still a successful `200` with `content: ""`. Mirror on the Anthropic path.
2. *Defence in depth*: in admission, when the startup-grace window is active and no server has reached `Running` yet, return `AdmissionError::QueueFull` with `retry_after_secs` set from the startup-wait config rather than the throughput estimate. Maps to a more honest `503` than a `429` with a wrong delay.

### Fix B-13

1. Extend `routing::ServerSnapshot` with `started_at: Option<DateTime<Utc>>` and `last_restart_at: Option<DateTime<Utc>>`.
2. Wire from `server-manager::ServerStateSnapshot::first_health_ok_at` (existing) and the spawn timestamp tracked in `manager.rs::ServerState`.
3. `transport::management::ServerSummary` already has the JSON shape; surface the new fields.

### Tests

- **New e2e** `crates/llamaforge-e2e/tests/cold_start_503.rs`: boot orchestrator with `start_full_no_wait` (a new harness method that doesn't `wait_for_running_server`), POST non-stream immediately, assert `503` + `Retry-After`.
- **Unit (both branches)** on the non-stream aggregator (transport::http::tests):
  - `MockEngine` returns no chunks at all (stream closes without `Stop`) → handler returns `503`.
  - `MockEngine` returns `Stop { finish_reason: "stop", output_tokens: 0 }` with no preceding `Token` chunks → handler returns `200` with `content: ""` (legitimately empty completion).
- **Unit** in management routes: `started_at` is `Some` after the first health-check passes; reuses the existing `MockServerControl` shape.

### Dependencies

- Independent. Can land before or after PR-2.

---

## PR-4 — Lifecycle signals: SIGHUP hot-reload + ReloadConfig RPC

**Bugs:** **B-14** (SIGHUP kills orchestrator), **B-15** (ReloadConfig RPC OKs but doesn't apply).
**Scope:** `crates/llamaforge/src/serve.rs:60-81` (signal handlers), `crates/llamaforge-transport/src/grpc.rs:155-161` (ReloadConfig RPC), `crates/llamaforge-config/src/handle.rs::reload` (the actual reload entry point).
**Effort:** M. **Severity:** P1 (both).

### Root cause B-14

`serve.rs:60-81` registers handlers for `SignalKind::terminate()` and `SIGINT` only; SIGHUP propagates to default-terminate. The integration test `crates/llamaforge-config/tests/design_doc_tests.rs::test_concurrent_sighups_serialized` documents the intent — the runtime never wired it.

### Root cause B-15

`GrpcManagementService::reload_config` (grpc.rs:155-161) calls `self.config_writer.reload()` and returns `Ok(())`. `ConfigHandle::reload` (handle.rs:77+) calls `source.load()` and swaps the `ArcSwap`, *but* server-manager / scheduler subscribers may not be picking up `ConfigDiff` events. The report's evidence (no `config.reloaded` log lines, alias still old) suggests either the diff is empty or downstream subscribers are silent.

### Fix B-14

1. Register a `SignalKind::hangup()` handler in `serve::wait_for_shutdown`. On HUP, spawn `config.reload()` via the `ConfigHandle` and *do not* signal shutdown.
2. Move the existing shutdown branch and the new reload branch into a single `loop { tokio::select! { … } }` so HUP is repeatable; the loop only exits on SIGTERM / SIGINT.
3. Wire `tracing::info!("lifecycle.config.reload.signaled")` plus the resulting `ConfigDiff` summary so operators see the action in the log.

### Fix B-15

**Confirmed by reviewer:** alias is restart-required.

1. `ReloadConfig` returns `Status::failed_precondition("alias change requires restart")` when the computed `ConfigDiff` touches a restart-required field. Mirror for any other restart-required field surfaced by `diff.rs` (port, model_path, ctx_size, parallel_slots, devices — anything `requires_restart()` returns true on).
2. Hot-reloadable diffs (priority_rules, gateway concurrency, log_level, etc.) apply normally; surface a `config.reloaded` tracing event keyed on the diff field set.
3. Document the restart-required field set in the management API docs (a short table near the `ReloadConfig` definition in `crates/llamaforge-proto/proto/management.proto`).
4. **v1.1-deferred entry** (landed in *this* PR, not a follow-up): "Staged-apply protocol — when a `ReloadConfig` touches a restart-required field, today we reject with `failed_precondition`; v1.1 should stage the change, drain the affected servers, and apply on restart. Tracks the placeholder removed by `ConfigHandle::apply_mutation`'s existing TODO at handle.rs:119."

### Tests

- **New unit** in `serve.rs::tests`: registers a fake `ConfigHandle` and asserts SIGHUP triggers `reload()` without process exit. (Direct signal injection is awkward; consider extracting `handle_hup(handle: Arc<ConfigHandle>)` and unit-testing that function plus a smoke shell test for the binary.)
- **New e2e** `crates/llamaforge-e2e/tests/sighup_reload.rs`: edit the temp config file (change a hot-reloadable field — pick one verified non-alias if alias is restart-only), send SIGHUP, assert the new value visible at `/api/config`.
- **Existing** `test_concurrent_sighups_serialized` should pass once SIGHUP is wired — it's been an unreachable test until now.

### Dependencies

- Independent of PR-1..PR-3.

---

## PR-5 — CLI hygiene (state-dir, validate, init defaults, README)

**Bugs:** **B-1** (`serve` subcommand doesn't exist), **B-2** (`--state-dir` ignored), **B-3** (default state dir unwritable on macOS), **B-4** (`validate --config <missing>` says OK).
**Scope:** `crates/llamaforge/src/main.rs`, `crates/llamaforge/src/validate.rs`, `README.md`.
**Effort:** S. **Severity:** P1 (B-4) / P2 (rest).

### Root cause B-1

`README.md:8` documents `llamaforge serve --config <path>`; `main.rs` only has `Init` / `Validate` subcommands and falls through to "no subcommand → serve" at `main.rs:227-258`.

### Root cause B-2

`main.rs:218` correctly reads `cli.state_dir` for the `init` flow, but the `init::run` function probably re-derives the dir from defaults. Likely the `state_dir` parameter is shadowed or not threaded.

### Root cause B-3

`find_state_dir()` (main.rs:105-110) returns `/var/run/llamaforge/` unconditionally. macOS Application Sandbox + non-root user makes it unwritable.

### Root cause B-4

`validate::run` at `main.rs:224` is invoked with `config_path = cli.config.unwrap_or_else(find_config)`. `find_config` falls back to the default search path if the user-supplied path doesn't exist — so a typo silently routes to a working default. The fix is in `find_config` / the `Validate` arm: pass the user path *as-is* and let `source.load()` produce `ConfigLoadError::NotFound`. The existing `handle_config_error` matches on it.

### Fix

1. **B-1**: add a no-op `serve` subcommand that just dispatches to the same `None` branch (`main.rs:227-258`). One-line clap addition. (**Reviewer confirmed:** add the alias rather than edit README.)
2. **B-2**: thread `state_dir: Option<PathBuf>` parameter through `init::run` end-to-end. Trace shows it's currently dropped on the floor.
3. **B-3**: use `dirs::state_dir()` (or the existing `directories` crate if already a dep) for the macOS default. Fall back to `~/.local/state/llamaforge/` on Linux only when not root.
4. **B-4**: in the `Validate` arm at `main.rs:222-227`, use the user-supplied path *without* `find_config` fallback. If `cli.config.is_none()`, that path is intended; if `Some(path)`, use `path` even if it doesn't exist — load will fail correctly and `handle_config_error` reports it.

### Tests

- **CLI smoke** tests with `assert_cmd` under `crates/llamaforge/tests/cli_*.rs` (new fixture):
  - `serve` subcommand exits non-zero with the same path as bare invocation (config missing).
  - `--state-dir /tmp/foo init` writes to `/tmp/foo` (assert files exist).
  - On macOS (`#[cfg(target_os = "macos")]`), default `init` writes under `~/Library/Application Support/llamaforge`.
  - `validate --config /nonexistent.toml` exits non-zero with `config file not found` on stderr.

### Dependencies

- Independent.

---

## PR-6 — Transport HTTP surface fillers (compat + error mapping)

**Bugs:** **B-5** (`/v1/servers`, `/v1/models`), **B-7** (unknown model `502` → `404`), **B-8** (`messages: []` accepted), **B-9** (Anthropic missing version header), **B-11** (`StopServer` gRPC `NotFound` mapping).
**Scope:** `crates/llamaforge-transport/src/http.rs` routes table at `:455-490`, `crates/llamaforge-transport/src/anthropic.rs::parse_anthropic_request`, `crates/llamaforge-transport/src/grpc.rs::stop_server` error mapping.
**Effort:** M (lots of small surface). **Severity:** P1 (B-5) / P2 (B-7) / P3 (B-8, B-9, B-11).

### Fix B-5

- Add `GET /v1/models` handler returning `{"object":"list","data":[{"id":alias,"object":"model","created":<ts>,"owned_by":"llamaforge"}, …]}` enumerating distinct aliases from `config.servers`. (Server-by-server detail belongs on `/v1/servers` if we want it.)
- Add `GET /v1/servers` that proxies the existing `/api/servers` shape (or aliases via `Router::route`). USER_JOURNEYS J3 expects it; same payload.

### Fix B-7

Once PR-1 lands, the validator returns `AdmissionError::NoServerForModel`. PR-1 already maps that to `ApiError::ModelNotFound`. Confirm `ModelNotFound` → HTTP 404 in the OpenAI shape and add an `error.type = "invalid_request_error"` + `error.code = "model_not_found"` JSON body (`crates/llamaforge-transport/src/error.rs`). Anthropic side: `error.type = "not_found_error"` with HTTP 404.

### Fix B-8

In `parse_openai_request` and `parse_anthropic_request`, validate `messages.is_empty()` → `ApiError::InvalidBody("messages must contain at least one entry")`. One assertion per shim.

### Fix B-9

**Confirmed by reviewer:** warn-only.

In `anthropic_messages_handler` (http.rs:281+), extract the `anthropic-version` header at request entry. If absent, log a `tracing::warn!(api = "anthropic", "missing_anthropic_version_header")` and continue. If present, echo as a response header (Anthropic SDK convention). No hard rejection.

**v1.1-deferred entry** (landed in *this* PR): "`gateway.anthropic_strict_version_header` — a config knob to make missing `anthropic-version` a 400 rejection instead of a warn. Useful for compliance-strict deployments; default stays permissive."

### Fix B-11

`grpc.rs::stop_server` currently `.map_err(|e| Status::internal(e.to_string()))`. Pattern-match on the `ServerError` variants: `NotFound` → `Status::not_found`, `InvalidId` → `Status::invalid_argument`, everything else → `Status::internal`.

### Carry-over from PR-1: gRPC `stream_inference` skips `validate_routable`

Surfaced during PR-1 review. `crates/llamaforge-transport/src/grpc.rs::stream_inference` (~lines 37-53) goes straight to `inference_engine.chat_completion(chat_req)` without calling `validate_routable`. Net effect: an unknown model over gRPC surfaces as whatever the inference engine returns (currently mapped via `.map_err(|_| Status::internal(...))`) — the same error-masking shape B-CRIT addressed on the HTTP side, on a different transport.

Fix: mirror PR-1's HTTP-shim shape. Build the `IncomingRequest` from the parsed gRPC `chat_req`, call `state.route_validator.validate_routable(&incoming)`, map the result so `AdmissionError::NoServerForModel` → `Status::not_found`, `AdmissionError::QueueFull` → `Status::resource_exhausted`, others → `Status::internal`. Reuse the `build_incoming_request` helper from `http.rs`; promoting it to a `pub(crate)` symbol is acceptable scope.

Test: new e2e file `crates/llamaforge-e2e/tests/grpc_unknown_model.rs` — gRPC `StreamInference` against an unknown alias → `tonic::Code::NotFound`, not `Code::Internal`.

### Tests

- **Unit** in transport tests for each of the five sub-bugs (`/v1/models` shape, unknown-model 404 JSON, empty-messages 400, anthropic-version warn, stop_server NotFound mapping).
- **e2e** smoke for `/v1/models` shape since OpenAI SDKs probe it at construction time.

### Dependencies

- B-7 needs PR-1's error mapping in place. B-5/B-8/B-9/B-11 are independent and could be split off into smaller PRs if reviewer prefers. Recommendation: keep grouped — same crate, same review pass.

---

## PR-7 — Dev/test ergonomics (gRPC reflection, mock-llama chunk delay, README clarifications)

**Bugs:** **B-10** (gRPC reflection disabled), **mock-llama cancellation knob** (report §"Notes / inconclusive"), **README mold note**.
**Scope:** `crates/llamaforge-transport/src/grpc.rs::start_grpc_server`, `crates/mock-llama/src/{lib,main}.rs`, `README.md`.
**Effort:** S. **Severity:** P3.

### Fix B-10

Add `tonic-reflection` as a workspace dep. In `grpc.rs::start_grpc_server`, behind a feature flag `dev-reflection` (default-off in release, default-on in dev), add `tonic_reflection::server::Builder::configure().register_encoded_file_descriptor_set(...)`. Generated descriptors come from `tonic-build` (`out_dir + "/file_descriptor_set.bin"`).

### Fix mock-llama chunk delay

Add `MOCK_LLAMA_CHUNK_DELAY_MS` env override, read at startup in `mock-llama/src/lib.rs::MockConfig::from_env_or_default` (new constructor). `mock-llama/src/main.rs` passes it through. Default unchanged (1 ms).

### Fix README

`README.md` end-to-end test section currently says `cargo test -p llamaforge-e2e --tests # also needs protoc and mold`. `mold` is Linux-only per `.cargo/config.toml`. Reword: "Linux only: also needs `mold` (Linux fast linker). macOS uses the system linker."

### Tests

- gRPC reflection: smoke test invoking `tonic-reflection`'s `ServerReflectionClient::list_services` against the dev profile.
- mock-llama env: unit test sets the env var, constructs `MockConfig`, asserts `chunk_delay == Duration::from_millis(N)`.

### Dependencies

- Independent.

---

## Bug → PR map (lookup table)

| Bug | Severity | PR | Status after PR |
|---|---|---|---|
| B-CRIT | P0 | PR-1 | Fixed |
| B-1 | P2 | PR-5 | Fixed |
| B-2 | P2 | PR-5 | Fixed |
| B-3 | P2 | PR-5 | Fixed |
| B-4 | P1 | PR-5 | Fixed |
| B-5 | P1 | PR-6 | Fixed |
| B-6 | P1 | PR-3 | Fixed |
| B-7 | P2 | PR-6 (depends on PR-1) | Fixed |
| B-8 | P3 | PR-6 | Fixed |
| B-9 | P3 | PR-6 | Warn-on-missing |
| B-10 | P3 | PR-7 | Dev-flag enabled |
| B-11 | P3 | PR-6 | Fixed |
| gRPC validate_routable | P2 (carry-over from PR-1 review) | PR-6 | Fixed |
| B-12 | P1 | PR-2 (depends on PR-1) | Fixed |
| B-13 | P3 | PR-3 | Fixed |
| B-14 | P1 | PR-4 | Fixed |
| B-15 | P1 | PR-4 | Fixed-or-explicit-reject |
| Cancel knob | nit | PR-7 | env override |
| README mold | nit | PR-7 | Linux-only clarification |

---

## Ordering

```
PR-1 (transport routing keystone)              ── unblocks PR-2 (model arg), PR-6 (error mapping)
   │
   ├── PR-2 (queue admission)                  ── independent after PR-1
   ├── PR-6 (transport surface fillers)        ── B-7 hangs off PR-1; B-5/B-8/B-9/B-11 independent
   │
PR-3 (lifecycle warming / B-13)                ── independent
PR-4 (SIGHUP + ReloadConfig)                   ── independent
PR-5 (CLI hygiene)                             ── independent
PR-7 (dev ergonomics)                          ── independent
```

Recommended merge sequence: **PR-1 → PR-2 → PR-3 → PR-4 → PR-5 → PR-6 → PR-7**. PR-1 first because it makes every other transport-layer e2e meaningful. PR-2 second because it's the next P1 and shares conceptual surface with PR-1 (admission validator). PR-3..PR-5 are independent P1s and can run in parallel if reviewer bandwidth allows. PR-6 batches the surface-completion P2/P3s and depends on PR-1's error mapping. PR-7 last (P3-only, ergonomics).

## Out of scope for this round

- USER_JOURNEYS J8 multi-model balancing, J9 capacity dashboards, J11 power-user UI — the report explicitly notes there is no UI yet.
- The `streaming` non-streaming response-shape work landed in PR103 already; this plan does not re-touch that surface.
- Test #37 (mid-stream cancellation) is inconclusive pending the mock-llama knob; coverage will follow PR-7.

## Process discipline (reviewer notes)

These apply across all 7 PRs:

1. **Stale `dummy` scan during PR-1.** Beyond the two `http.rs` sites, sweep `crates/llamaforge-adapters/src/{audit,inference_engine}.rs` and the e2e harness (`crates/llamaforge-e2e/src/lib.rs:131`) for comments / fixtures referencing the placeholder. Remove or update; tests should default to a real alias.
2. **Pre-push workflow-ref grep is mandatory on every PR.** New test files are the recurring surface: `git diff origin/main..HEAD -- '*.rs' | grep -nE '#[0-9]+|PR[0-9]+|Regression for'` must come back clean before push. Recurring nit from PR102/103/105 reviews — closing it at source.
3. **v1.1-deferred entries land in the PR that introduces the deferral**, not as a doc-only follow-up. PR-4 lands the "staged-apply protocol" entry; PR-6 lands the "anthropic_strict_version_header" entry.

## Open questions

(All resolved by reviewer in the first review pass — preserved here for traceability.)

1. **B-15 reload semantics → restart-required.** Alias is restart-required; `ReloadConfig` returns `Status::failed_precondition` on diffs that touch restart-required fields. Documented in mgmt API; staged-apply protocol in v1.1-deferred.
2. **B-1 `serve` subcommand → add the alias.** No README edit.
3. **B-9 anthropic-version → warn-only.** Strict-mode knob deferred to v1.1.
4. **PR-6 split → keep grouped.** Five sub-bugs in one PR pass.
