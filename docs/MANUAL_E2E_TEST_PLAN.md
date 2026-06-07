# LlamaForge — Manual End-to-End Test Plan

**Target build:** `v0.1.0-draft` (workspace at `crates/`)
**Primary environment (run first):** Framework Desktop — AMD **Strix Halo** APU (Ryzen AI Max+, Radeon 8060S / gfx1151), 128 GB unified RAM, **Linux (Fedora / Bazzite, x86_64)**, `llama.cpp` built with the **Vulkan** backend.
**Secondary environment (run second):** macOS (Apple Silicon, Metal).
**Author/owner:** Sumanth Gottipati  **Run date:** 2026-05-26 – 2026-05-28  **Build SHA:** 73641a4

This is a manual, operator-driven E2E test runbook. It walks the system from a cold install all the way through every major and minor use case: CLI, configuration, process supervision, the OpenAI + Anthropic HTTP shims, the canonical gRPC core, the management API, routing/scheduling, observability, security, hot-reload, and failure/chaos behavior. Each test has a stable **Test ID**, preconditions, exact commands, and a code-grounded **expected outcome**.

Every expected outcome below was derived from the source (handlers, error mapping, config validation, lifecycle code), not from the design docs alone. Where v1.0 deliberately differs from a "textbook" expectation, it is called out as a **Known behavior** so you don't file it as a bug.

---

## Table of Contents

1. [How to use this plan](#1-how-to-use-this-plan)
2. [Prerequisites & environment](#2-prerequisites--environment)
3. [Build & artifacts](#3-build--artifacts)
4. [Reference: surfaces, ports, error model](#4-reference-surfaces-ports-error-model)
5. [Phase A — Installation & CLI](#phase-a--installation--cli)
6. [Phase B — Configuration & validation](#phase-b--configuration--validation)
7. [Phase C — Launch & process lifecycle](#phase-c--launch--process-lifecycle)
8. [Phase D — OpenAI HTTP API](#phase-d--openai-http-api)
9. [Phase E — Anthropic HTTP API](#phase-e--anthropic-http-api)
10. [Phase F — gRPC core](#phase-f--grpc-core)
11. [Phase G — Management API](#phase-g--management-api)
12. [Phase H — Routing & scheduling](#phase-h--routing--scheduling)
13. [Phase I — Observability](#phase-i--observability)
14. [Phase J — Security & auth](#phase-j--security--auth)
15. [Phase K — Failure & chaos](#phase-k--failure--chaos)
16. [Phase L — Config hot-reload (end-to-end)](#phase-l--config-hot-reload-end-to-end)
17. [Phase M — Teardown & data maintenance](#phase-m--teardown--data-maintenance)
18. [Known v1.0 behaviors & limitations](#18-known-v10-behaviors--limitations)
19. [Sign-off summary](#19-sign-off-summary)

---

## 1. How to use this plan

- Work top-to-bottom. Phases A→C must pass before later phases are meaningful (a broken launch invalidates everything downstream).
- **Priority** tags help you triage: **P0** = blocker/smoke, **P1** = core, **P2** = edge/nice-to-have.
- Record results in the companion tracker `MANUAL_E2E_TEST_TRACKER.xlsx` (Status = Pass / Fail / Blocked / N/A, plus notes and the observed value).
- Keep **two terminals**: one running `llamaforge serve` in the foreground (so you can read structured logs), one for issuing requests.
- Set these shell variables once per session so commands are copy-pasteable:

```sh
export HTTP=http://127.0.0.1:8000
export GRPC=127.0.0.1:50051
export CFG="$HOME/.config/llamaforge/llamaforge.toml"
export RULES="$HOME/.config/llamaforge/priority_rules.toml"
export LF=./target/release/llamaforge
```

- `jq` is used to pretty-print JSON; drop the `| jq` if you don't have it.
- Throughout, "the orchestrator" = the `llamaforge` process; "a server" / "instance" = a child `llama-server` process it supervises.

---

## 2. Prerequisites & environment

### 2.1 Toolchain & tooling (Linux / Fedora / Bazzite — primary)

| Item | Requirement | Verify |
|------|-------------|--------|
| OS | Linux x86_64 (Fedora / Bazzite) | `uname -sm` → `Linux x86_64` |
| Rust toolchain | stable, per `rust-toolchain.toml` (with `rustfmt`, `clippy`) | `rustc --version` |
| **mold linker** | **Required** on x86_64 Linux — `.cargo/config.toml` forces `-C link-arg=-fuse-ld=mold` for `x86_64-unknown-linux-gnu`. Build fails to link without it. | `mold --version` |
| protoc | Protocol Buffers compiler (needed by `llamaforge-proto/build.rs`) | `protoc --version` |
| `llama.cpp` | a working `llama-server` built with **Vulkan** (see §2.3) | `llama-server --version` |
| Model files | at least **two** GGUF models (enables multi-model routing tests) | `ls *.gguf` |
| HTTP tooling | `curl`, optionally `jq` | `curl --version` |
| gRPC tooling | `grpcurl` (reflection-based, no `.proto` needed) | `grpcurl -version` |
| SQLite | `sqlite3` CLI (to inspect the metrics DB) | `sqlite3 --version` |
| Vulkan loader | `vulkan-loader` + Mesa RADV driver (AMD) | `vulkaninfo --summary` |

**Install (standard Fedora — `dnf`):**

```sh
sudo dnf install -y mold protobuf-compiler protobuf-devel jq sqlite \
    vulkan-loader vulkan-tools mesa-vulkan-drivers
# Rust via rustup (rust-toolchain.toml auto-selects the pinned stable):
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# grpcurl: download a release binary, or `go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest`
```

> **Bazzite / atomic-Fedora note.** Bazzite is an immutable `rpm-ostree` base — don't install the build toolchain on the host. Either (a) work inside a **toolbox/distrobox** Fedora container (recommended): `toolbox create lf-dev && toolbox enter lf-dev`, then `sudo dnf install ...` and `cargo build` inside it; or (b) layer packages with `rpm-ostree install mold protobuf-compiler ...` followed by a reboot. The Vulkan **runtime/driver** is already on the Bazzite host, so the *running* orchestrator + `llama-server` can use the GPU even if you *built* inside a container (the container shares `/dev/dri`). Verify GPU visibility inside the container with `vulkaninfo --summary`.

### 2.2 Toolchain & tooling (macOS — secondary pass)

| Difference vs Linux | macOS behavior |
|---------------------|----------------|
| Linker | `mold` is **not** used — `.cargo/config.toml` only targets `x86_64-unknown-linux-gnu`, so Apple's `ld-prime` is used. No mold install needed. |
| GPU | Metal backend, automatic. |
| Installs | `brew install protobuf grpcurl jq` ; Rust via rustup. |
| State dir default | `~/Library/Application Support/llamaforge` (Linux uses `~/.local/state/llamaforge`). |

### 2.3 Building `llama.cpp` for Strix Halo (Vulkan)

If `llama-server` isn't built yet:

```sh
git clone https://github.com/ggml-org/llama.cpp && cd llama.cpp
cmake -B build -DGGML_VULKAN=ON -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release -j
# resulting binary: build/bin/llama-server
./build/bin/llama-server --list-devices   # should list the Radeon 8060S (gfx1151) via Vulkan
```

### 2.4 Strix Halo memory & GPU-offload guidance (affects config + routing tests)

Strix Halo uses **unified LPDDR5x** — the iGPU has no dedicated VRAM; "GPU memory" is carved from the 128 GB shared pool. This is a big advantage for large models, but it changes how you set the per-server config fields:

- **`gpu_layers = -1`** offloads all layers to the iGPU (recommended). On Linux the GPU can address memory via the **GTT** heap dynamically (often well beyond the BIOS UMA/“VRAM” buffer), so large models fit even with a modest UMA setting. If you see allocation failures, raise the **BIOS UMA / dedicated graphics memory** buffer, or rely on GTT.
- **`estimated_vram_mb`** is an operator *hint* LlamaForge uses for VRAM-fit reasoning, not an enforced limit. On unified memory, set it to a realistic per-model footprint so multi-instance fit/balancing tests (Phase H) are meaningful; it does not cap actual usage.
- With 128 GB you can run **multiple large instances** concurrently — ideal for the multi-instance balancing (RT-02) and capacity tests. Watch total system memory (`free -h`, `radeontop`/`amdgpu_top`) rather than a discrete VRAM figure.
- `flash_attn = "auto"` and `cache_type_k/v` quantization behave per `llama-server`'s Vulkan support; if a model misbehaves with KV quantization on Vulkan, fall back to `f16`.

### 2.5 Build-independent notes

- gRPC **reflection** is compiled in by default (the `dev-reflection` feature is in `default`), so `grpcurl` works against a plain `cargo build --release` with no `.proto` files.
- The repo ships example configs at `examples/llamaforge.toml` and `examples/priority_rules.toml`; `llamaforge init` writes these exact files.
- **Default writable state dir:** Linux `~/.local/state/llamaforge` (or `$XDG_STATE_HOME/llamaforge`); macOS `~/Library/Application Support/llamaforge`. Override with `--state-dir` or `$LLAMAFORGE_STATE_DIR`.

---

## 3. Build & artifacts

| ID | Priority | Step | Expected outcome |
|----|----------|------|------------------|
| BLD-01 | P0 | `cargo build --release` from the repo root | Builds all 8 workspace crates with no errors. **Linux:** `mold` must be on `PATH` (the build forces `-fuse-ld=mold` for x86_64 Linux) — a missing mold fails at link. On **Bazzite**, run this inside a toolbox/distrobox (see §2.1). Note: `.cargo/config.toml` also sets `-D warnings`, so any warning fails the build. |
| BLD-02 | P1 | `cargo build --bin mock-llama` (optional; for chaos tests without burning a GPU) | Produces `target/debug/mock-llama` — an in-process `llama-server` stand-in supporting `--alias`, `--port`, `--parallel`, plus `health_fail_after=` / `startup_503_ms=` knobs. |
| BLD-03 | P2 | `ls -l target/release/llamaforge` | The orchestrator binary exists and is executable. |
| BLD-04 | P2 | `cargo test --workspace` (optional sanity, not part of manual E2E) | Unit/integration suites pass; e2e suite additionally needs `protoc`. |

After this phase, all later commands assume `LF=./target/release/llamaforge`.

---

## 4. Reference: surfaces, ports, error model

**Default ports** (`[global]` in config): HTTP/shim + management on **:8000**, canonical gRPC on **:50051**, bind `0.0.0.0`.

**HTTP inference + discovery routes:** `POST /v1/chat/completions`, `POST /v1/completions`, `POST /v1/messages`, `GET /v1/models`, `GET /v1/servers`, `GET /health`, `GET /metrics`.

**Management routes:** `GET /api/servers`, `POST /api/servers/{id}/start`, `POST /api/servers/{id}/stop`, `GET /api/queue`, `DELETE /api/queue/{id}`, `POST /api/config/reload`, `GET /api/metrics/requests`, `GET /api/metrics/aggregated`, `POST /api/maintenance/wal-checkpoint`.

**gRPC services:** `llamaforge.inference.InferenceService/StreamInference` (server-streaming), `llamaforge.management.ManagementService` (`ListServers`, `StartServer`, `StopServer`, `QueueStatus`, `CancelRequest`, `ReloadConfig`), plus `grpc.health.v1.Health` and server reflection.

**Canonical error body (HTTP):** `{"error":"<code>","message":"<text>","request_id":"<uuid>"}` (the `request_id` is injected on any 4xx/5xx). Responses also carry `x-request-id` and `x-llamaforge-request-id` headers.

| Condition | HTTP status | `error` code | gRPC status |
|-----------|-------------|--------------|-------------|
| Malformed/invalid JSON body | 400 | `invalid_body` | `INVALID_ARGUMENT` (3) |
| Auth required & missing/bad key | 401 | `auth_failure` | `UNAUTHENTICATED` (16) |
| Unknown model alias | 404 | `model_not_found` | `NOT_FOUND` (5) |
| Queue full / admission backpressure | 429 (+`Retry-After`) | `queue_full` | `RESOURCE_EXHAUSTED` (8) |
| Backend (dispatch) error | 502 | `backend_error` | `INTERNAL` (13) |
| Backend warming / no `Stop` chunk | 503 (+`Retry-After`) | `service_unavailable` | `UNAVAILABLE` (14) |
| Request timed out | 504 | `timeout` | `DEADLINE_EXCEEDED` (4) |
| Reload touches restart-required field | 412 | `precondition_failed` | `FAILED_PRECONDITION` (9) |
| Body exceeds `max_request_body_bytes` (default 50 MiB) | 413 | `request_body_too_large` | — |
| Unhandled internal error | 500 | `internal_error` | `INTERNAL` (13) |

**Boot/CLI error JSON (stderr):** `{"timestamp","level":"ERROR","code","message"}`. Codes: `CONFIG_NOT_FOUND`, `CONFIG_EMPTY`, `CONFIG_PARSE_ERROR`, `CONFIG_VALIDATION_ERROR`, `CONFIG_IO_ERROR`, `STATE_DIR_NOT_WRITABLE`, `RUNTIME_INIT_FAILED`, `FATAL_ERROR`. Exit codes: **78** (config/state usage error), **70** (runtime init), **1** (fatal at run).

**Server lifecycle states** (shown in `/api/servers` and `/v1/servers` `state` field): `Stopped`, `Starting`, `Running`, `Draining`, `Restarting`, `Failed`.

---

## Phase A — Installation & CLI

Goal: confirm the binary's command surface, help/version output, the `init` scaffolding, and every CLI-level error path.

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| CLI-01 | P0 | Binary runs & shows help | `$LF --help` | Usage printed listing global flags `--config`, `--priority-rules`, `--state-dir` and subcommands `init`, `validate`, `serve`. Exit 0. |
| CLI-02 | P2 | Version string | `$LF --version` | Prints `0.1.0-draft` (workspace version). Exit 0. |
| CLI-03 | P1 | Per-subcommand help | `$LF init --help` ; `$LF serve --help` ; `$LF validate --help` | Each lists its own flags (`init`: `--out --rules --state-dir --force`; `serve`/`validate`: `--config --rules` etc.). Exit 0. |
| CLI-04 | P0 | `init` writes default config | `$LF init` (no flags) | Creates `$XDG_CONFIG_HOME/llamaforge/llamaforge.toml` (i.e. `~/.config/llamaforge/...`) **and** `priority_rules.toml` alongside it; creates the state dir at mode `0750` (Linux default `~/.local/state/llamaforge`; macOS `~/Library/Application Support/llamaforge`). Prints `Wrote default config to <path>. Edit \`binary_path\` ...`. Exit 0. |
| CLI-05 | P1 | `init` content matches examples | `diff ~/.config/llamaforge/llamaforge.toml examples/llamaforge.toml` | No differences — `init` embeds the repo's `examples/*.toml` verbatim. |
| CLI-06 | P1 | `init` refuses to clobber | Run `$LF init` again without `--force` | Error to stderr: `error: existing config at <path> last modified <date>. Use --force to overwrite.` Exit **78**. Existing file untouched. |
| CLI-07 | P1 | `init --force` overwrites | `$LF init --force` | Overwrites config + rules, prints the success line, exit 0. |
| CLI-08 | P2 | `init --out` custom path | `$LF init --out /tmp/lf/llamaforge.toml --state-dir /tmp/lf/state` | Writes config to `/tmp/lf/llamaforge.toml`, rules to `/tmp/lf/priority_rules.toml` (sibling), creates `/tmp/lf/state`. Exit 0. |
| CLI-09 | P1 | Default search order honored | Unset `LLAMAFORGE_CONFIG`; with a config at `~/.config/llamaforge/llamaforge.toml`, run `$LF validate` | Resolves the XDG path automatically (search order: `--config` → `$LLAMAFORGE_CONFIG` → `$XDG_CONFIG_HOME/llamaforge/` → `/etc/llamaforge/`). Validates that file. |
| CLI-10 | P1 | `$LLAMAFORGE_CONFIG` env override | `LLAMAFORGE_CONFIG=/tmp/lf/llamaforge.toml $LF validate` | Uses the env-pointed file. |
| CLI-11 | P0 | Serve with missing config | `$LF serve --config /nope/missing.toml` | stderr JSON `{"code":"CONFIG_NOT_FOUND", ...}` with remediation text ("Run \`llamaforge init\`..."). Exit **78**. Does *not* silently fall back to a default. |
| CLI-12 | P1 | State dir not writable | `$LF serve --config $CFG --state-dir /proc/cannot-write` (or any non-writable path) | stderr JSON `{"code":"STATE_DIR_NOT_WRITABLE", ...}` advising `mkdir -p` / `--state-dir`. Exit **78**. |
| CLI-13 | P2 | Bare invocation == serve | `$LF` (no subcommand, with a valid config discoverable) | Behaves identically to `serve` (boots the orchestrator). |

---

## Phase B — Configuration & validation

Goal: confirm `validate` accepts good configs and rejects each business-rule violation with the right code/message, without starting any servers.

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| CFG-01 | P0 | Valid config passes | Point `model_path`/`binary_path` at real files, then `$LF validate --config $CFG --rules $RULES` | stdout: `config OK: <N> servers, <M> task types, validation passed`. Exit 0. No `llama-server` is spawned. |
| CFG-02 | P0 | Parse error | Introduce a TOML syntax error (e.g. unmatched bracket), `$LF validate --config <bad>` | stderr JSON `{"code":"CONFIG_PARSE_ERROR", ...}` naming the file + parse detail. Exit 78. |
| CFG-03 | P1 | Empty file | `: > /tmp/empty.toml ; $LF validate --config /tmp/empty.toml` | stderr JSON `{"code":"CONFIG_EMPTY", ...}` (mentions possible interrupted writeback; suggests `init --force`). Exit 78. |
| CFG-04 | P1 | Duplicate ports | Set two `[servers.*]` to the same `port`, validate | `CONFIG_VALIDATION_ERROR`, message `duplicate server port: <port>`. Exit 78. |
| CFG-05 | P1 | Empty `binary_path` | Set `binary_path = ""` on a server, validate | Validation error `empty binary_path for server '<id>'`. Exit 78. |
| CFG-06 | P1 | Empty `model_path` | Set `model_path = ""`, validate | Validation error `empty model_path for server '<id>'`. Exit 78. |
| CFG-07 | P1 | `parallel_slots = 0` | Set `parallel_slots = 0`, validate | Validation error `parallel_slots must be > 0 for server '<id>'`. Exit 78. |
| CFG-08 | P2 | Duplicate GPU id | Add two `[[servers.x.devices]]` with same `gpu_id`, validate | Validation error `duplicate gpu_id <n> for server '<id>'`. Exit 78. |
| CFG-09 | P2 | `queue_default_timeout_secs = 0` | Set it to 0, validate | Validation error `queue_default_timeout_secs must be > 0`. Exit 78. |
| CFG-10 | P2 | Invalid server id slug | Name a server table with an illegal key (empty / too long / bad chars), validate | `CONFIG_VALIDATION_ERROR` referencing the slug rule (`empty ServerId`, length, or charset). Exit 78. |
| CFG-11 | P2 | `validate` honors `--config` verbatim | `$LF validate --config /typo/does-not-exist.toml` | `CONFIG_NOT_FOUND` (does **not** fall back to the default search path — a typo must not silently validate a different file). Exit 78. |
| CFG-12 | P2 | Fair-share weights auto-normalize | In `priority_rules.toml` set weights summing to ≠100; validate | Validation still passes; weights are auto-normalized to 100 internally (no error). |

---

## Phase C — Launch & process lifecycle

Goal: confirm a clean boot, that the child `llama-server` fleet actually comes up, and that the boot is "fail-closed" (transports don't start if the fleet fails).

Edit `$CFG` so it has **two** real servers (give them distinct ports and aliases), e.g. `llama-3-8b` (alias `Llama-3-8B-Instruct`, port 8080) and `tinyllama` (port 8081). Then start the orchestrator in terminal 1.

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| LIF-01 | P0 | Clean startup | `$LF serve --config $CFG --rules $RULES` (foreground) | Logs in order: `lifecycle.startup.begin` (with `bind_address`, `http_port=8000`, `grpc_port=50051`) → server-manager spawns each `llama-server` → `lifecycle.startup.ready`. Process stays running. |
| LIF-02 | P0 | Child processes spawned | In terminal 2: `pgrep -fl llama-server` | One OS process per configured server, each invoked with `--model`, `--port`, etc. from your config. |
| LIF-03 | P0 | Liveness endpoint | `curl -s $HTTP/health` | `{"status":"ok"}`, HTTP 200. (Note: this is the orchestrator's own liveness, independent of downstream health.) |
| LIF-04 | P0 | Fleet status surface | `curl -s $HTTP/v1/servers \| jq` | JSON array; each entry `{id, model, state, slots_total, slots_busy}`. After warmup, `state` should read `"Running"` and `slots_total` matches `parallel_slots`. |
| LIF-05 | P1 | Same payload on management route | `curl -s $HTTP/api/servers \| jq` | Identical shape/content to `/v1/servers` (they share the DTO). |
| LIF-06 | P1 | Fail-closed on bad fleet | Point one server's `binary_path` at a non-executable, start serve | Server-manager `start()` errors propagate; the binary refuses to start the HTTP/gRPC transports and exits non-zero with a `FATAL_ERROR` log. (No half-up gateway.) |
| LIF-07 | P1 | Startup → Running transition visible | Poll `curl -s $HTTP/v1/servers` immediately after boot, then again after a few seconds | First poll may show `"Starting"`; later polls show `"Running"`. Legal transitions: Stopped→Starting→Running. |
| LIF-08 | P2 | Concurrent restart staggering | With ≥2 servers, observe restart timing (see Phase K) | Restarts are spaced by `restart_stagger_secs` (default 5s) to avoid simultaneous VRAM reload contention. |

---

## Phase D — OpenAI HTTP API

Goal: exercise the OpenAI-compatible shim — discovery, chat (streaming + non-streaming), legacy completions, parameter passthrough, and all client-error paths. Use a model name equal to one of your configured **aliases** (e.g. `Llama-3-8B-Instruct`).

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| OAI-01 | P0 | Model discovery | `curl -s $HTTP/v1/models \| jq` | `{"object":"list","data":[ {"id":"<alias>","object":"model","created":<unix>,"owned_by":"llamaforge"}, ... ]}`. One entry per distinct non-empty alias; servers without an alias are omitted. |
| OAI-02 | P0 | Chat, non-streaming | `curl -s $HTTP/v1/chat/completions -H 'content-type: application/json' -d '{"model":"<alias>","messages":[{"role":"user","content":"Say hello in one word."}]}' \| jq` | HTTP 200. Body has full OpenAI shape: `id` = `chatcmpl-<uuid>`, `object":"chat.completion"`, `created`, `model` echoes your alias, `choices[0].index=0`, `choices[0].message.{role:"assistant",content:"..."}`, `choices[0].finish_reason`, and a `usage` block `{prompt_tokens, completion_tokens, total_tokens}`. |
| OAI-03 | P0 | Chat, streaming (SSE) | add `"stream":true`; `curl -N -s $HTTP/v1/chat/completions -H 'content-type: application/json' -d '{...,"stream":true}'` | A stream of `data: {…"object":"chat.completion.chunk"…}` lines with incremental `delta` content, terminated by a final stop chunk and then `data: [DONE]`. Connection closes cleanly. |
| OAI-04 | P1 | Legacy completions | `curl -s $HTTP/v1/completions -H 'content-type: application/json' -d '{"model":"<alias>","prompt":"2+2=","max_tokens":16}' \| jq` | 200. Internally mapped to a single user message; returns the same chat-completion JSON shape with generated content. |
| OAI-05 | P1 | Parameter passthrough | Send `"max_tokens":8` and `"temperature":0` | Response is bounded by `max_tokens`; `temperature` is forwarded to the backend (deterministic-ish output at 0). |
| OAI-06 | P0 | Unknown model → 404 (not 429) | `curl -i -s $HTTP/v1/chat/completions -d '{"model":"does-not-exist","messages":[{"role":"user","content":"hi"}]}' -H 'content-type: application/json'` | **HTTP 404**, body `{"error":"model_not_found","message":"Model not found","request_id":"..."}`. No `Retry-After`. (Routing distinguishes "no server for model" from "queue full".) |
| OAI-07 | P1 | Malformed JSON → 400 | `curl -i -s $HTTP/v1/chat/completions -H 'content-type: application/json' -d '{not json'` | **HTTP 400**, `{"error":"invalid_body","message":"...","request_id":"..."}`. |
| OAI-08 | P2 | Missing required field → 400 | POST `{"model":"<alias>"}` (no `messages`) | 400 `invalid_body` (deserialization failure). |
| OAI-09 | P2 | Request body too large → 413 | POST a body > 50 MiB (`head -c 60000000 /dev/zero \| ...`) | **HTTP 413**, body `{"error":"request_body_too_large","message":"Payload too large","max_bytes":52428800,"received_bytes":<n>,"request_id":"..."}`. |
| OAI-10 | P2 | Request-id correlation | `curl -i` any request and inspect headers | Response carries `x-request-id` and `x-llamaforge-request-id` (same UUID); on errors the same id appears in the JSON `request_id`. |
| OAI-11 | P2 | Streaming cancel on disconnect | Start OAI-03 streaming then Ctrl-C the curl mid-stream; watch server logs | Orchestrator logs `Client disconnected, cancelling request <id>` (debug) and cancels the in-flight request — no zombie generation. |

---

## Phase E — Anthropic HTTP API

Goal: exercise the Anthropic Messages shim, including its deliberately **permissive** version-header behavior.

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| ANT-01 | P0 | Messages, non-streaming | `curl -s $HTTP/v1/messages -H 'content-type: application/json' -H 'anthropic-version: 2023-06-01' -d '{"model":"<alias>","max_tokens":64,"messages":[{"role":"user","content":"Hello"}]}' \| jq` | 200. Body: `{"id":"...","type":"message","role":"assistant","content":[{"type":"text","text":"..."}],"model":"<alias>","stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":<n>,"output_tokens":<n>}}`. Response echoes back `anthropic-version` header. |
| ANT-02 | P1 | Messages, streaming | add `"stream":true`, `curl -N` | Anthropic SSE event sequence (`message_start` → `content_block_start` → `content_block_delta`… → `content_block_stop` → `message_delta` → `message_stop`). Stream closes cleanly. |
| ANT-03 | P1 | **Missing version header is allowed** | Repeat ANT-01 **without** `-H 'anthropic-version: ...'` | **Still HTTP 200** and a normal completion. Server logs a WARN `api.missing_anthropic_version_header` but admits the request. _Known v1.0 behavior — not a bug; strict-reject is a deferred v1.1 knob._ |
| ANT-04 | P1 | Unknown model → 404 | POST with `"model":"nope"` | 404 `model_not_found` (same routing path as OpenAI). |
| ANT-05 | P2 | Malformed body → 400 | POST invalid JSON | 400 `invalid_body`. |

---

## Phase F — gRPC core

Goal: the gRPC service on `:50051` is the canonical API; the HTTP shims translate into it. Use `grpcurl` (works via reflection — no `.proto` files needed because `dev-reflection` is on by default). Use `-plaintext` unless you've configured TLS.

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| GRP-01 | P0 | Reflection lists services | `grpcurl -plaintext $GRPC list` | Lists `llamaforge.inference.InferenceService`, `llamaforge.management.ManagementService`, `grpc.health.v1.Health`, and `grpc.reflection.*`. |
| GRP-02 | P1 | Describe inference service | `grpcurl -plaintext $GRPC describe llamaforge.inference.InferenceService` | Shows `StreamInference(InferenceRequest) returns (stream InferenceResponse)`. |
| GRP-03 | P0 | gRPC health check | `grpcurl -plaintext $GRPC grpc.health.v1.Health/Check` | `{"status":"SERVING"}`. |
| GRP-04 | P0 | Streaming inference | `grpcurl -plaintext -d '{"model":"<alias>","messages":[{"role":"user","content":"hi"}]}' $GRPC llamaforge.inference.InferenceService/StreamInference` | A stream of `InferenceResponse` messages with token content, ending in a terminal stop. (Exact field names per `proto/llamaforge/inference.proto`.) |
| GRP-05 | P1 | Management over gRPC | `grpcurl -plaintext $GRPC llamaforge.management.ManagementService/ListServers` | Returns the server list (mirrors `GET /api/servers`). |
| GRP-06 | P1 | gRPC ReloadConfig (hot) | After an alias-only edit: `grpcurl -plaintext $GRPC llamaforge.management.ManagementService/ReloadConfig` | Success response; the alias change is applied to the running fleet. |
| GRP-07 | P1 | gRPC unknown model → NOT_FOUND | `StreamInference` with `"model":"nope"` | gRPC status `NOT_FOUND` (5), message "Model not found". |
| GRP-08 | P2 | gRPC queue-full mapping | Drive saturation (see Phase H) and issue a gRPC call | gRPC status `RESOURCE_EXHAUSTED` (8). |

---

## Phase G — Management API

Goal: operate the fleet and inspect queue/metrics over HTTP. (All under `:8000`.)

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| MGT-01 | P0 | List servers | `curl -s $HTTP/api/servers \| jq` | Array of `{id, model, state, slots_total, slots_busy}`. Note an `id` for the next steps. |
| MGT-02 | P0 | Stop a server | `curl -i -s -X POST $HTTP/api/servers/<id>/stop` | HTTP 200. `pgrep -fl llama-server` shows that child gone; `/api/servers` shows its `state` transitioning to `Draining`→`Stopped`. |
| MGT-03 | P0 | Start a server | `curl -i -s -X POST $HTTP/api/servers/<id>/start` | HTTP 200. The child `llama-server` respawns; `state` returns to `Starting`→`Running`. |
| MGT-04 | P1 | Bad server id slug → 400 | `curl -i -s -X POST "$HTTP/api/servers/Bad%20Id!/start"` | HTTP 400 `invalid_body` (slug validation rejects it before touching the fleet). |
| MGT-05 | P1 | Queue status | `curl -s $HTTP/api/queue \| jq` | `{"depth":<n>,"estimated_throughput_req_sec":<f>}`. Idle: `depth` 0. |
| MGT-06 | P1 | Cancel by request id (queue-only) | Submit a request that sits **queued** (e.g. fill all slots so the next one waits), then `curl -i -s -X DELETE $HTTP/api/queue/<request-uuid>` with its `x-request-id` | HTTP 200 if the request is still **queued**. Cancellation is queue-only in v1 — in-flight (dispatched/generating) requests run to completion (design.md G3); cancelling an in-flight or unknown id returns **HTTP 404 `request_not_cancellable`** (not 500). In-flight requests are cancelled only by client disconnect (OAI-11/CHA-05). |
| MGT-07 | P2 | Cancel with non-UUID → 400 | `curl -i -s -X DELETE $HTTP/api/queue/not-a-uuid` | HTTP 400 `invalid_body` ("Invalid RequestId"). |
| MGT-08 | P0 | Reload (hot-reloadable) | Edit only `alias` of a server in `$CFG`, then `curl -i -s -X POST $HTTP/api/config/reload` | HTTP 200. New alias appears in `/v1/models` and `/v1/servers`. Logs `lifecycle.config.reload.applying` / `applied`. |
| MGT-09 | P0 | Reload (restart-required) → 412 | Edit a restart-required field (e.g. `ctx_size`, `port`, `parallel_slots`, `model_path`), then POST `/api/config/reload` | **HTTP 412** `{"error":"precondition_failed","message":"config diff touches restart-required server fields ([...]); restart the orchestrator to apply", ...}`. The running fleet is unchanged. |
| MGT-10 | P1 | Per-request metrics | `curl -s "$HTTP/api/metrics/requests?limit=10" \| jq` | JSON array of recent request rows (TTFT, tokens, timing, server_id, model). Supports `offset` and filter params. |
| MGT-11 | P1 | Aggregated metrics | `curl -s "$HTTP/api/metrics/aggregated?group_by=server_id,model_name" \| jq` | Aggregates grouped by the requested dimensions. Valid `group_by` fields: `server_id`, `hour`, `model_name`, `requestor`. An invalid field → 500 `internal_error` naming the bad field. |
| MGT-12 | P2 | WAL checkpoint (default mode) | `curl -i -s -X POST $HTTP/api/maintenance/wal-checkpoint` | HTTP 200. Uses the configured `wal_checkpoint_mode` (default `passive`). |
| MGT-13 | P2 | WAL checkpoint (truncate) | `curl -i -s -X POST "$HTTP/api/maintenance/wal-checkpoint?mode=truncate"` | HTTP 200; the `-wal` file is truncated (verify size drop with `ls -l llamaforge.db-wal`). |

---

## Phase H — Routing & scheduling

Goal: confirm requests land on the right server and that the priority queue + admission control behave under contention. Best done with a multi-server config and (for saturation) a small `queue_max_depth`.

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| RT-01 | P0 | Alias routing | Send a chat request to alias A and another to alias B; check `/api/metrics/requests` | Each request is served by the server whose alias matches `model`; metrics rows show the correct `server_id` per model. |
| RT-02 | P1 | Multi-instance balancing | Configure two instances of the **same** model/alias; fire several concurrent requests | Load spreads across both instances by slot availability (inspect `slots_busy` in `/api/servers` mid-load, and `server_id` distribution in metrics). |
| RT-03 | P2 | KV-cache (prefix) affinity | Send two requests sharing a long identical prompt prefix to a multi-instance alias | Routing prefers the instance that already holds the matching prefix (KV-cache affinity) to maximize goodput. Verify both hit the same `server_id` when affinity weight dominates. |
| RT-04 | P1 | Queue saturation → 429 | Set `queue_max_depth` low (e.g. 2) and `parallel_slots` to 1; fire many concurrent long requests (`for i in $(seq 1 20); do curl ... & done`) | Once the queue is full, excess requests get **HTTP 429** `{"error":"queue_full",...}` with a `Retry-After` header (derived from depth/throughput, fallback 30s). In-slot requests still complete. |
| RT-05 | P2 | Priority ordering | With `priority_rules.toml`, submit a low-priority (`default`) and a high-priority request (e.g. requestor with a bonus, or `emergency` task type) under contention | Higher effective-priority requests are dequeued first. _Note: in v1.0 the HTTP path attributes every request to requestor `anonymous` / task `default` (see Known behaviors), so priority differentiation is best validated via gRPC/queue internals rather than the HTTP shim._ |
| RT-06 | P2 | Starvation boost | Leave a low-priority request queued beyond `starvation.threshold_secs` (default 30s) under sustained higher-priority load | The aged request receives a priority boost (`boost_per_interval`) and eventually runs rather than starving. |
| RT-07 | P2 | Preemption (if enabled) | Set `[preemption] enabled = true`, saturate with low-priority work, then submit an `emergency` request | A lower-priority request that has produced ≥ `min_tokens_before_preempt` tokens may be preempted to free a slot. (Default config has preemption **disabled** — verify the off case too: no preemption occurs.) |

---

## Phase I — Observability

Goal: metrics are exported (Prometheus), persisted (SQLite), and logs are structured with correlation ids.

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| OBS-01 | P0 | Prometheus endpoint | `curl -s $HTTP/metrics \| head -40` | Prometheus text exposition. Expect series like `llamaforge_http_requests_total{method,path,status}`, `llamaforge_http_request_duration_seconds`, `llamaforge_active_connections`, and (after an auth failure) `llamaforge_auth_failures_total`. |
| OBS-02 | P1 | Counters move | Hit `/v1/chat/completions` a few times, re-scrape `/metrics` | `llamaforge_http_requests_total` for that path/status increments accordingly. |
| OBS-03 | P1 | SQLite metrics persisted | After traffic: `sqlite3 llamaforge.db '.tables'` then `sqlite3 llamaforge.db 'select count(*) from request_metrics;'` (table name per schema) | Metrics DB exists at the configured `metrics_db_path`; request rows accumulate. (DB path is honored from config.) |
| OBS-04 | P1 | DB path honored | Set `metrics_db_path` to a custom path, restart, send traffic | The DB is created at the custom path, not the default. |
| OBS-05 | P2 | Structured logs | Run with `log_format = "json"`; observe stdout during requests | Each line is a JSON event with a stable event name (`lifecycle.*`, `transport.request`, etc.); request lifecycle carries `request_id`. With `log_format="pretty"` the same data is human-formatted. |
| OBS-06 | P2 | Retention setting present | Confirm `metrics_retention_days` (default 30) is read | Old rows beyond the retention window are pruned by the maintenance path (long-running observation, or inspect retention logic). |
| OBS-07 | P2 | TTFT / throughput captured | Inspect a metrics row from MGT-10 | Row includes time-to-first-token and tokens/sec style fields for the completed request. |

---

## Phase J — Security & auth

Goal: confirm API-key enforcement across HTTP and gRPC, and CORS behavior. Set `api_key = "test-secret"` under `[global]` and restart.

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| SEC-01 | P0 | No key → 401 (HTTP) | With `api_key` set: `curl -i -s $HTTP/v1/chat/completions -d '{"model":"<alias>","messages":[{"role":"user","content":"hi"}]}' -H 'content-type: application/json'` | **HTTP 401** `{"error":"auth_failure","message":"Authentication failed",...}`. Server logs `transport.auth_failed` (with masked key prefix + client ip) and increments `llamaforge_auth_failures_total`. |
| SEC-02 | P0 | Correct bearer → 200 | Same request with `-H 'authorization: Bearer test-secret'` | HTTP 200, normal completion. |
| SEC-03 | P1 | Wrong key → 401 | `-H 'authorization: Bearer wrong'` | HTTP 401 `auth_failure`. |
| SEC-04 | P1 | Auth covers ALL routes | With `api_key` set, `curl -i -s $HTTP/health` and `curl -i -s $HTTP/metrics` **without** a key | **HTTP 401** for both. _Known behavior: auth is a global layer — even `/health` and `/metrics` require the key when one is configured. Plan your liveness probes accordingly._ |
| SEC-05 | P1 | gRPC auth | With `api_key` set: `grpcurl -plaintext $GRPC llamaforge.management.ManagementService/ListServers` (no creds) → then add `-H 'authorization: Bearer test-secret'` | Without creds: gRPC `UNAUTHENTICATED` (16). With correct bearer metadata: success. |
| SEC-06 | P2 | Empty key disables auth | Set `api_key = ""`, restart | Auth is bypassed (empty key treated as "no auth"); requests succeed without a header. (Same as omitting `api_key`.) |
| SEC-07 | P2 | CORS preflight | `curl -i -s -X OPTIONS $HTTP/v1/chat/completions -H 'Origin: http://localhost:3000' -H 'Access-Control-Request-Method: POST'` | CORS headers returned per `[cors] allowed_origins` (default permissive `*`); allowed methods include GET/POST/DELETE/OPTIONS. |

_Remember to remove/blank `api_key` again before continuing with unauthenticated phases, or keep adding the bearer header._

---

## Phase K — Failure & chaos

Goal: confirm failures are "boring and predictable" — restart-with-backoff, warming guards, timeouts, cancellation, and clean shutdown with no orphans. The bundled **mock-llama** (BLD-02) is ideal here because it can inject health failures and slow starts deterministically; with real `llama.cpp` use `kill` to simulate crashes.

> Optional mock-based setup: point a server's `binary_path` at `target/debug/mock-llama` and add args via alias knobs, e.g. an instance that fails health after N checks (`--alias mock,health_fail_after=3`) or is slow to come up (`--alias mock,startup_503_ms=4000`). `MOCK_LLAMA_CHUNK_DELAY_MS` slows token output for cancellation/timeout windows.

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| CHA-01 | P0 | Crash → restart with backoff | While running, `pkill -f 'llama-server.*8080'` (kill one child) | Server-manager detects the exit, marks the server `Failed`→`Restarting`, and respawns it (`Starting`→`Running`) with backoff. `/api/servers` reflects the transitions; new requests to that alias resume once healthy. |
| CHA-02 | P1 | Health-check failure handling | (mock) configure `health_fail_after=N`; let it fail health | Server transitions out of `Running` (Failed/Restarting) once health checks fail; the orchestrator stops routing to it and attempts recovery. |
| CHA-03 | P1 | Warming window → 503 | Send a request to a server still in `Starting` (catch it right at boot, or use `startup_503_ms`) | Non-streaming request that ends without a terminal stop chunk returns **HTTP 503** `{"error":"service_unavailable",...}` with `Retry-After` — not a fake empty 200. |
| CHA-04 | P1 | Request timeout → 504 | Make the backend stall longer than `queue_default_timeout_secs` (lower it to e.g. 5s; with mock raise `MOCK_LLAMA_CHUNK_DELAY_MS`) | Non-streaming: **HTTP 504** `{"error":"timeout",...}`. Streaming: the SSE closes with a logged `openai.sse.upstream_stalled` after the per-chunk timeout. |
| CHA-05 | P1 | Client cancellation frees slot | Start a long streaming request, kill the client mid-stream, then immediately check `/api/queue` / `slots_busy` | The dropped request is cancelled (log `Client disconnected, cancelling request ...`); its slot frees and queued work proceeds. |
| CHA-06 | P0 | Graceful shutdown (SIGINT) | In terminal 1 press **Ctrl-C** | Logs `lifecycle.shutdown.started` → transports drain → `shutdown_all` stops children → `lifecycle.shutdown.completed`, then process exits 0. |
| CHA-07 | P0 | No orphaned children | After CHA-06: `pgrep -fl llama-server` | **No** `llama-server` processes remain — all children were terminated with the orchestrator. |
| CHA-08 | P1 | SIGTERM also graceful | Start again; `kill -TERM <llamaforge-pid>` | Same graceful drain as SIGINT (SIGTERM and Ctrl-C share the shutdown path). |
| CHA-09 | P2 | Backend 5xx → 502 | Force the backend to error on a request (mock error mode, or a model that rejects the input) | Gateway returns **HTTP 502** `{"error":"backend_error",...}`. |

---

## Phase L — Config hot-reload (end-to-end)

Goal: confirm the three reload triggers (HTTP, gRPC, SIGHUP) all share the same gate: hot-reloadable fields apply live; restart-required fields are refused.

**Hot-reloadable fields:** `alias`, `lora_adapters`, `estimated_vram_mb`, and global/gateway/cors settings.
**Restart-required fields:** `model_path`, `ctx_size`, `parallel_slots`, `devices`, `backend`, `gpu_layers`, `cache_type_k`, `cache_type_v`, `flash_attn`, `port`, `speculative_method`, `model_architecture`.

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| REL-01 | P0 | SIGHUP hot reload | Edit a server `alias` in `$CFG`, then `kill -HUP <llamaforge-pid>` | Logs `lifecycle.config.reload.signaled` → `...completed` with `changes=N`. New alias live in `/v1/models`. No process restart. |
| REL-02 | P0 | SIGHUP restart-required is refused | Edit `ctx_size`, `kill -HUP <pid>` | Logs `lifecycle.config.reload.restart_required` (WARN) with the offending fields. The change is **not** applied; the fleet keeps running on the old value. |
| REL-03 | P1 | HTTP reload parity | Repeat REL-01/02 via `POST /api/config/reload` | Hot field → 200; restart-required → 412 (matches MGT-08 / MGT-09). |
| REL-04 | P1 | gRPC reload parity | Repeat via `ManagementService/ReloadConfig` | Hot field → success; restart-required → `FAILED_PRECONDITION` (9). |
| REL-05 | P2 | Mid-request snapshot stability | Start a long request, hot-reload during it | The in-flight request finishes against the config snapshot it started with; the new snapshot applies to subsequent requests only. |
| REL-06 | P2 | Restart applies restart-required change | After REL-02, stop the orchestrator, restart it | The previously-refused `ctx_size` change now takes effect on boot. |

---

## Phase M — Teardown & data maintenance

| ID | Priority | Objective | Steps | Expected outcome |
|----|----------|-----------|-------|------------------|
| TD-01 | P1 | Clean stop leaves consistent DB | Shut down (CHA-06), then `sqlite3 llamaforge.db 'pragma integrity_check;'` | `ok`. Metrics from the session are persisted and readable. |
| TD-02 | P2 | Lock file lifecycle | Inspect `state_dir` for `config.lock` during run; confirm a second `serve` against the same state dir behaves per the flock policy | The lock file exists while running; concurrent writers contend via the documented flock with timeout rather than corrupting state. |
| TD-03 | P2 | Restart resumes cleanly | Start again after teardown | Boots normally; prior metrics still queryable via `/api/metrics/requests`. |
| TD-04 | P2 | WAL growth controlled | After heavy traffic, run MGT-13 (`?mode=truncate`) | WAL file shrinks; DB remains consistent. |

---

## 18. Known v1.0 behaviors & limitations

Treat these as **expected** — verifying them confirms intended behavior, and they should not be filed as defects:

1. **Anthropic version header is permissive.** A missing/malformed `anthropic-version` header logs a WARN and the request proceeds (ANT-03). A hard-reject mode is a deferred v1.1 config knob.
2. **Restart-required config changes are refused, not staged.** Reloads touching restart-required fields return 412 / `FAILED_PRECONDITION` / WARN and require an orchestrator restart (MGT-09, REL-02). The drain-and-restage protocol is v1.1.
3. **Auth is global.** When `api_key` is set, `/health` and `/metrics` also require the key (SEC-04). External liveness/scrape probes must supply it.
4. **Request attribution is coarse on the HTTP path.** v1.0 attributes HTTP-shim requests to requestor `anonymous` / task `default`; persisted metrics collapse onto that pair, and HTTP-side priority differentiation is limited (RT-05). Per-principal attribution and per-server config overlays (`ConfigOverlayStore`) are deferred to v1.1.
5. **Reflection is a default-on dev feature.** `grpcurl` works out of the box; a production build could disable `dev-reflection`, after which reflection-based tooling would need `.proto` files.

> If you are validating against a different commit, re-confirm these against `docs/v1.1-deferred.md` and the handlers — at least one item there (the OpenAI non-streaming response shape) has **already been completed** in this build, so the non-streaming response now includes the full `usage` / `finish_reason` fields (OAI-02).

---

## 18a. Plan-revision notes (from the 2026-05-27/28 run)

Issues found **in this plan itself** (not product defects) — fix before the next run:

1. **Phase A / Phase B overlap.** Several Configuration & validation checks duplicate Installation & CLI checks (noted live at CFG-02: "why are these duplicate tasks"). Consolidate the overlapping CLI-vs-config validation cases so each behavior is exercised once.
2. **RT-03 is mis-scoped for v1.0.** It expects prompt-prefix KV-cache affinity to influence routing, but `05_SMART_ROUTING.md` lists that as **Tier 2, deferred to v1.1** (`w_affinity × 0.0` in v1). Two identical-prefix requests landing on different instances is *correct* for this build. Re-label RT-03 as a v1.1 test (or assert the v1 behavior: affinity does not influence routing).
3. **RT-05/06/07 priority tests aren't client-API-exercisable in v1.0.** Priority keys off `requestor`/`task_type`, but neither the HTTP shim nor the gRPC `InferenceRequest` carries those fields — every request collapses to `anonymous`/`default`. Mark these as queue-internal/unit-test scope, or add the note that only RT-07's *off-case* (preemption disabled by default) is observable end-to-end.
4. **LIF-06's expectation contradicts `design.md:4353`.** It expects fail-closed (process exits non-zero) on a single bad server, but partial-fleet startup is **intentional** — the gateway stays up while any server is `Running`; the design's bad-fleet mechanism is the `/ready` all-failed circuit breaker (which is itself unimplemented, see #234). Fail-closed only applies to port-bind failure (`design.md:325`) and empty/unparseable config (`design.md:1226`). Rewrite LIF-06 to assert: (a) partial fleet → gateway up + failed server reflected; (b) all-failed → `/ready` 503 (once #234 lands).
5. **CHA-09 (backend 5xx → 502) has no harness.** `mock-llama` has no completion-error/5xx mode and a real `llama-server` won't reliably emit a 5xx. Either add a 5xx-injection knob to `mock-llama` or move CHA-09 to the error-translation unit tests.
6. **`tee`-based log capture loses shutdown logs on Ctrl-C.** Ctrl-C signals the whole foreground pipeline, killing `tee` before the orchestrator's graceful-shutdown lines are written (CHA-06). For log-observation tests, prefer `kill -INT/-TERM <pid>` against the orchestrator PID (leaves `tee` alive) or redirect to a file instead of piping.

---

## 19. Sign-off summary

| Phase | Area | Tests | Pass | Fail | N/A | Notes |
|-------|------|-------|------|------|-----|-------|
| BLD | Build & artifacts | 4 | 3 | 1 | 0 | BLD-04 flaky anthropic streaming test (#218) |
| A | Installation & CLI | 13 | 13 | 0 | 0 | |
| B | Configuration & validation | 12 | 12 | 0 | 0 | |
| C | Launch & lifecycle | 8 | 6 | 1 | 1 | LIF-06 Fail (#234, + plan defect); LIF-07 N/A |
| D | OpenAI HTTP API | 11 | 10 | 1 | 0 | OAI-09 413 returns plain text, not canonical JSON |
| E | Anthropic HTTP API | 5 | 5 | 0 | 0 | |
| F | gRPC core | 8 | 6 | 2 | 0 | GRP-03 health svc not registered (#227); GRP-08 saturation → Internal not RESOURCE_EXHAUSTED (#231) |
| G | Management API | 13 | 13 | 0 | 0 | MGT-06 resolved in #229: cancel is queue-only (v1), in-flight/unknown id → 404 `request_not_cancellable` not 500 |
| H | Routing & scheduling | 7 | 3 | 1 | 3 | RT-04 Fail (#231); RT-03/05/06 N/A (Tier-2 / no v1 priority API) |
| I | Observability | 7 | 6 | 1 | 0 | OBS-02 counters don't accumulate (#232) |
| J | Security & auth | 7 | 7 | 0 | 0 | |
| K | Failure & chaos | 9 | 8 | 0 | 1 | CHA-09 N/A (no mock 5xx-on-completion mode) |
| L | Config hot-reload | 6 | 6 | 0 | 0 | |
| M | Teardown & maintenance | 4 | 4 | 0 | 0 | |
| **Total** | | **114** | **101** | **8** | **5** | GitHub issues #218–#234 filed across the run (incl. feature requests); RCA comment added to open #148 (scheduler wedge under saturation) |

**Overall verdict:** ☐ Ship  ☑ Ship with caveats  ☐ Hold — _no P0 crashers in the happy path; the open Fails are error-mapping/observability/lifecycle-edge issues (see issues). #148 scheduler-wedge-under-saturation and #232 counter non-accumulation are the most operationally significant._

**Tester signature:** _Sumanth Gottipati (with Claude as test companion)_  **Date:** _2026-05-28_

---

_Generated as a code-grounded manual E2E plan for LlamaForge `v0.1.0-draft`. Expected outcomes were derived from the CLI (`crates/llamaforge`), transport handlers (`crates/llamaforge-api`), config validation (`crates/llamaforge-core`), and lifecycle code (`serve.rs`). Companion tracker: `MANUAL_E2E_TEST_TRACKER.xlsx`._

