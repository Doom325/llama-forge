# Component Design: Server Manager

**Component:** Server Manager (Process Lifecycle)  
**Phase:** 2 (Process Management)  
**Estimated LOC:** ~3,000  
**Risk:** High  
**Dependencies:** Config Manager (server definitions, reload diffs), Metrics & Storage (event logging, PID tracking), OS/GPU APIs (NVML, IOKit, rocm-smi)  
**Consumers:** Dispatch & Proxy (server state, capabilities), Smart Routing (slot state from /slots polling), Transport Layer (server status for health endpoints)

---

## 1. Purpose & Scope

The Server Manager owns the lifecycle of all `llama-server` child processes: spawning, health monitoring, graceful shutdown, restart with backoff, VRAM reservation, deadlock detection, and performance degradation tracking. It provides a real-time view of each server's state (slots, health, capabilities) to the routing and dispatch layers.

**What this component explicitly does NOT do:**

- It does not decide which server to route a request to (that's Smart Routing).
- It does not manage the request queue or priority system (that's Request Queue).
- It does not proxy HTTP streams to backends (that's Dispatch & Proxy).
- It does not expose APIs directly (Transport Layer wraps its state).
- It does not own the reqwest::Client configuration for proxying (Dispatch & Proxy manages per-server clients via `ArcSwap<reqwest::Client>`; Server Manager signals when to swap).

---

## 2. Public API Surface

### 2.1 Core Interface

```rust
/// The primary handle for interacting with the Server Manager.
/// Shared via Arc across components.
pub struct ServerManager {
    servers: Arc<DashMap<ServerId, ServerState>>,
    vram_ledger: Arc<Mutex<HashMap<GpuId, u64>>>,
    gpu_telemetry: Arc<dyn GpuTelemetry>,
    config: Arc<ConfigHandle>,
    metrics_ingest: MetricsIngest,
    metrics_store: Arc<dyn MetricsStore>,
    shutdown_token: CancellationToken,
}

impl ServerManager {
    /// Create and start managing all servers defined in config.
    pub async fn start(
        config: Arc<ConfigHandle>,
        metrics_ingest: MetricsIngest,
        metrics_store: Arc<dyn MetricsStore>,
        gpu_telemetry: Arc<dyn GpuTelemetry>,
    ) -> Result<Arc<Self>, ServerManagerError>;

    /// Get current state of a specific server (non-blocking snapshot).
    pub fn server_state(&self, id: &ServerId) -> Option<ServerStateSnapshot>;

    /// Get all servers in Running state with their slot info.
    pub fn running_servers(&self) -> Vec<(ServerId, ServerStateSnapshot)>;

    /// Get slot-level detail for a server (from last /slots poll).
    pub fn slot_states(&self, id: &ServerId) -> Option<Vec<SlotState>>;

    /// Signal that a server's reqwest::Client should be refreshed.
    /// Called internally after restart completes health check.
    /// Consumers (Dispatch & Proxy) subscribe to this via broadcast.
    pub fn subscribe_client_refresh(&self) -> broadcast::Receiver<ClientRefreshEvent>;

    /// Handle a config diff (called by the config reload subscriber).
    pub async fn apply_config_diff(&self, diff: &ConfigDiff) -> Result<(), ServerManagerError>;

    /// Initiate graceful shutdown of a specific server.
    pub async fn stop_server(&self, id: &ServerId) -> Result<(), ServerManagerError>;

    /// Initiate graceful shutdown of all servers (called on SIGTERM).
    pub async fn shutdown_all(&self) -> Result<(), ServerManagerError>;

    /// Get cumulative request counter for id_task overflow warning.
    pub fn cumulative_requests(&self, id: &ServerId) -> Option<u64>;

    /// Increment cumulative request counter (called by Dispatch on each dispatch).
    pub fn increment_requests(&self, id: &ServerId);
}
```

### 2.2 GPU Telemetry Trait

```rust
/// Safe abstraction over GPU FFI (NVML, IOKit, rocm-smi-lib).
/// Implementations are behind Cargo feature flags.
pub trait GpuTelemetry: Send + Sync + 'static {
    /// Query free VRAM for a specific GPU device.
    fn free_vram_bytes(&self, device: &GpuId) -> Result<u64, TelemetryError>;

    /// Query GPU compute utilization (0-100%).
    fn utilization_percent(&self, device: &GpuId) -> Result<u8, TelemetryError>;

    /// List available GPU devices.
    fn list_devices(&self) -> Result<Vec<GpuDevice>, TelemetryError>;
}

/// Feature-gated implementations:
/// - `NvmlTelemetry` (feature = "gpu-nvml") — CUDA/NVML
/// - `MetalTelemetry` (feature = "gpu-metal") — macOS IOKit/sysctl
/// - `RocmTelemetry` (feature = "gpu-rocm") — rocm-smi-lib FFI
/// - `StubTelemetry` (default) — returns TelemetryError::Unavailable

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("GPU telemetry unavailable for this backend")]
    Unavailable,

    #[error("device not found: {device}")]
    DeviceNotFound { device: GpuId },

    #[error("FFI call failed: {message}")]
    FfiError { message: String },
}
```

### 2.3 Server State Types

```rust
/// The lifecycle state machine for a managed server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerLifecycle {
    Stopped,
    Starting,
    Running,
    Draining,     // graceful shutdown in progress
    Restarting,
    Failed,
}

/// Snapshot of a server's current state (cheap to clone).
#[derive(Debug, Clone)]
pub struct ServerStateSnapshot {
    pub id: ServerId,
    pub lifecycle: ServerLifecycle,
    pub pid: Option<u32>,
    pub port: u16,
    pub model_alias: String,
    pub parallel_slots: u32,
    pub ctx_size: u32,
    pub slots: Vec<SlotState>,
    pub last_health_check: Option<Instant>,
    pub restart_count: u32,
    pub cumulative_requests: u64,
    pub capabilities: ServerCapabilities,
}

#[derive(Debug, Clone)]
pub struct SlotState {
    pub id: u32,
    pub is_processing: bool,
    pub n_past: u32,
    pub n_decoded: u32,
    pub prompt_hash: Option<u64>,  // for prefix affinity routing
}

#[derive(Debug, Clone)]
pub struct ServerCapabilities {
    pub models: Vec<String>,       // aliases this server can serve
    pub lora_adapters: Vec<String>,
    pub multimodal: bool,
    pub speculative: SpeculativeMethod,
    pub architecture: ModelArchitecture,
}

/// Event emitted when a server restart completes and consumers should refresh clients.
#[derive(Debug, Clone)]
pub struct ClientRefreshEvent {
    pub server_id: ServerId,
    pub new_port: u16,  // in case port changed on config reload
}
```

### 2.4 Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum ServerManagerError {
    #[error("server {id} not found")]
    ServerNotFound { id: ServerId },

    #[error("insufficient VRAM on {gpu}: need {needed_mb} MB, have {free_mb} MB")]
    InsufficientVram { gpu: GpuId, needed_mb: u64, free_mb: u64 },

    #[error("server {id} failed to start: {reason}")]
    StartFailed { id: ServerId, reason: String },

    #[error("server {id} exceeded max restart attempts ({max})")]
    MaxRestartsExceeded { id: ServerId, max: u32 },

    #[error("process spawn failed: {source}")]
    SpawnError { source: std::io::Error },

    #[error("config validation failed: {message}")]
    ConfigInvalid { message: String },
}
```

---

## 3. Internal Architecture

### 3.1 Per-Server Task Model

```
┌─────────────────────────────────────────────────────────────────┐
│                    ServerManager (shared Arc)                     │
│                                                                   │
│  ┌───────────────────┐  ┌───────────────────┐                   │
│  │ Server Task:      │  │ Server Task:      │  ... per server   │
│  │ llama-8b-gpu0     │  │ codellama-gpu1    │                   │
│  │                   │  │                   │                   │
│  │  tokio::select! { │  │  tokio::select! { │                   │
│  │    child.wait()   │  │    child.wait()   │  ← crash detect   │
│  │    health_tick    │  │    health_tick    │  ← 2s /health     │
│  │    slots_tick     │  │    slots_tick     │  ← 5s /slots      │
│  │    metrics_tick   │  │    metrics_tick   │  ← 10s /metrics   │
│  │    shutdown_rx    │  │    shutdown_rx    │  ← graceful stop  │
│  │  }               │  │  }               │                   │
│  └───────────────────┘  └───────────────────┘                   │
│                                                                   │
│  ┌───────────────────────────────────────────────────────────┐   │
│  │ VRAM Reservation Ledger: Arc<Mutex<HashMap<GpuId, u64>>>  │   │
│  │ - Entries for servers in spawn-to-healthy state ONLY      │   │
│  │ - Running servers reflected in NVML directly              │   │
│  │ - Removed on process exit                                 │   │
│  └───────────────────────────────────────────────────────────┘   │
│                                                                   │
│  ┌───────────────────────────────────────────────────────────┐   │
│  │ Restart Coordinator                                        │   │
│  │ - Stagger: restart_stagger_secs between sequential starts │   │
│  │ - Backoff: 10±2s, 20±4s, 40±8s (exponential + jitter)    │   │
│  │ - Concurrency: Semaphore(1) per GPU for spawn serialization│   │
│  └───────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 Process Lifecycle State Machine

```
                    ┌──────────┐
          ┌────────►│ Stopped  │◄─────────────────┐
          │         └────┬─────┘                   │
          │              │ start()                  │ stop() from any state
          │              ▼                          │
          │    ┌────────────────┐                   │
          │    │ VRAM Validate  │ ← pre-spawn check │
          │    └───┬────────┬───┘                   │
          │  fail  │        │ pass                  │
          │        ▼        ▼                       │
          │  ┌──────┐  ┌──────────┐                │
          │  │Failed│  │ Starting │                 │
          │  └──────┘  └────┬─���───┘                │
          │                  │ health_check OK       │
          │                  ▼                       │
          │            ┌──────────┐                  │
          │            │ Running  │──── stop() ─────►│ Draining │
          │            └────┬─────┘                  └────┬─────┘
          │                 │ crash / health fail          │ drain complete
          │                 ▼                              ▼
          │           ┌───────────┐                  SIGTERM → SIGKILL
          │           │Restarting │
          │           └─────┬─────┘
          │                 │ max_retries exceeded
          │                 ▼
          └─────────── ┌──────────┐
                       │  Failed  │
                       └──────────┘
```

### 3.3 VRAM Pre-Spawn Sequence

```rust
async fn pre_spawn_vram_check(
    &self,
    server_config: &ServerConfig,
) -> Result<(), ServerManagerError> {
    let gpu_id = &server_config.devices[0]; // primary GPU

    // 1. Acquire ledger lock (synchronous, sub-microsecond)
    let mut ledger = self.vram_ledger.lock().unwrap();

    // 2. Query free VRAM via GPU telemetry trait
    let nvml_free = self.gpu_telemetry.free_vram_bytes(gpu_id)
        .unwrap_or(u64::MAX); // if unavailable, skip check

    // 3. Subtract reservations for servers in spawn-to-healthy state
    let reserved = *ledger.get(gpu_id).unwrap_or(&0);
    let effective_free = nvml_free.saturating_sub(reserved);

    // 4. Compute estimated need
    let estimated = compute_vram_estimate(server_config);

    // 5. Compare
    if effective_free < estimated {
        return Err(ServerManagerError::InsufficientVram {
            gpu: gpu_id.clone(),
            needed_mb: estimated / 1_048_576,
            free_mb: effective_free / 1_048_576,
        });
    }

    // 6. Record reservation
    *ledger.entry(gpu_id.clone()).or_insert(0) += estimated;

    Ok(())
    // Lock released here
}
```

### 3.4 Deadlock Detection Logic

```rust
/// Returns true if the server is diagnosed as deadlocked.
fn is_deadlocked(
    &self,
    server: &ServerState,
    slots: &[SlotState],
    tracker_count: u32,
) -> bool {
    // Condition 1: Server has active work
    let has_active_work = slots.iter().any(|s| s.is_processing)
        || tracker_count > 0;
    if !has_active_work { return false; }

    // Condition 2: No decode progress
    let no_progress = slots.iter().all(|s| s.n_decoded == server.last_n_decoded[s.id]);
    if !no_progress { return false; }

    // Condition 3: GPU utilization < 10% (backend-dependent)
    let gpu_idle = match self.gpu_telemetry.utilization_percent(&server.gpu_id) {
        Ok(util) => util < 10,
        Err(TelemetryError::Unavailable) => {
            // Fallback: CPU usage check for Metal/CPU/Vulkan
            server.process_cpu_usage() < 50
        }
        Err(_) => false, // can't determine — don't kill
    };
    if !gpu_idle { return false; }

    // Condition 4: Not in loading state
    let has_been_healthy = server.first_health_ok_at.is_some();
    if !has_been_healthy { return false; }

    // All four conditions met → deadlock
    true
}
```

### 3.5 Key Invariants

1. **VRAM ledger consistency:** Ledger entries exist only for servers in spawn-to-healthy state. Running servers are reflected in NVML directly. Entries removed on ANY process exit.
2. **Single spawn per GPU at a time:** Spawn operations for the same GPU are serialized by a per-GPU semaphore to prevent VRAM races.
3. **Process always reaped:** `child.wait()` is always running in the server task. No zombie processes.
4. **Graceful before forceful:** SIGTERM always precedes SIGKILL (with 10s timeout between).
5. **Staggered restarts:** Concurrent restarts for servers on the same GPU are delayed by `restart_stagger_secs`.
6. **Health check stagger:** Initial health check loops have random jitter to prevent synchronized bursts.
7. **id_task overflow warning:** Warning logged at `i32::MAX / 2` cumulative requests. Operator advised to rolling-restart.

---

## 4. Boundary Contracts

### 4.1 What This Component REQUIRES

| Dependency | Contract |
|-----------|----------|
| Config Manager | `ServerConfig` definitions available via snapshot. Config diffs identify added/removed/changed servers. |
| Metrics & Storage | `MetricsIngest::send_telemetry()` for event logging. `MetricsStore::upsert_managed_process()` for PID persistence (orphan recovery). |
| GPU Telemetry | `free_vram_bytes()` and `utilization_percent()` best-effort. Graceful fallback on `Unavailable`. |
| OS | `tokio::process::Command::spawn()` works. Signals deliverable (SIGTERM, SIGKILL). `/proc` or equivalent for process liveness. |
| llama-server binary | Path provided via each server's required `binary_path` config field. Auto-discovery on `PATH` is deferred (see issue #115). Exposes `/health`, `/slots`, `/metrics` HTTP endpoints. |

### 4.2 What This Component GUARANTEES to Consumers

| Consumer | Guarantee |
|----------|-----------|
| Smart Routing | `slot_states()` returns data ≤5 seconds old (polling interval). Returns `None` for non-Running servers. |
| Dispatch & Proxy | `server_state()` accurately reflects lifecycle. `Running` means health check has passed and process is alive. |
| Dispatch & Proxy | `ClientRefreshEvent` emitted within 1 second of restart completing health check. Consumers must swap their `reqwest::Client` on receipt. |
| Transport Layer | `running_servers()` never includes servers in `Starting`, `Failed`, or `Stopped` states. |
| All consumers | Server transitions are logged as events to Metrics & Storage before state change is visible to consumers. |

### 4.3 Failure Propagation

| Failure | Propagation |
|---------|-------------|
| Server process crash | Detected in <1ms via `child.wait()`. State → Restarting. Event logged. Consumers see lifecycle change on next `server_state()` call. |
| Health check timeout (transient) | Logged as warning. No state change. Consumers unaffected. |
| Health check fail (3 consecutive) | State → Restarting. Active connections fail naturally (Dispatch sees lifecycle change, stops routing). |
| VRAM insufficient | Server stays in `Failed`. Error returned from `start()`. Other servers unaffected. |
| GPU telemetry unavailable | VRAM check skipped (with warning). Deadlock detection reduced to 3 signals. |
| Max restarts exceeded | State → Failed permanently until operator intervention (config reload or manual start). |

---

## 5. Failure Modes

### FM-1: Orphan Processes After LlamaForge Crash

- **Scenario:** LlamaForge killed by OOM/SIGKILL. Child processes keep running.
- **Detection:** On restart, read `managed_processes` table from SQLite. Check each PID with `kill(pid, 0)`.
- **Recovery:** For alive orphans: issue `GET /health` to confirm it's our process (match model on port). If match, adopt into Running state. If mismatch, log warning and leave alone.
- **State preserved:** SQLite PID records survive crashes (`synchronous = NORMAL`).

### FM-2: llama-server Hangs (Deadlock)

- **Scenario:** CUDA kernel deadlock (Xid 79), Metal command buffer hang.
- **Detection:** Four-signal detection (active work + no decode progress + GPU idle + previously healthy).
- **Recovery:** SIGKILL (not SIGTERM — deadlocked process can't handle signals). Transition to Restarting. Backoff applies.

### FM-3: VRAM Exhaustion During Spawn

- **Scenario:** External process (another user's CUDA program) consumed VRAM between validation and actual allocation.
- **Detection:** llama-server fails to start (OOM in CUDA malloc). Health check never passes.
- **Recovery:** Remove ledger entry after restart cooldown. Log event with VRAM mismatch details.

### FM-4: Thundering Herd After Host GPU Reset

- **Scenario:** All servers on a GPU die simultaneously (GPU reset, driver crash).
- **Detection:** Multiple server tasks fire restart within milliseconds.
- **Recovery:** Per-GPU semaphore + `restart_stagger_secs` ensures sequential restarts. Jitter prevents lock-step retries.

### FM-5: Config Reload Changes Server's GPU Assignment

- **Scenario:** Operator moves a server from gpu:0 to gpu:1 via TOML edit + SIGHUP.
- **Detection:** Config diff shows `devices` field changed → `requires_restart = true`.
- **Recovery:** Graceful shutdown on old GPU → remove old ledger entry → VRAM check on new GPU → spawn on new GPU.

---

## 6. Configuration

| Knob | Location | Default | Valid Range | Hot-Reload |
|------|----------|---------|-------------|------------|
| `auto_restart` | Per-server | true | bool | Yes |
| `max_restart_attempts` | Per-server | 3 | 1-100 | Yes |
| `restart_cooldown_secs` | Per-server | 10 | 1-3600 | Yes |
| `restart_on_degradation` | Per-server | false | bool | Yes |
| `restart_stagger_secs` | Global | 10 | 1-300 | Yes |
| `estimated_vram_mb` | Per-server | Auto-computed | 0-999999 | Yes (on restart) |
| `shutdown_drain_timeout_secs` | Per-server | Auto: `max(60, min(ctx_size/30, 600))` | 0-3600 | Yes |
| Health check interval | Hardcoded | 2 seconds | — | No |
| /slots poll interval | Hardcoded | 5 seconds | — | No |
| /metrics scrape interval | Hardcoded | 10 seconds | — | No |
| Deadlock health timeouts | Hardcoded | 3 consecutive | — | No |
| SIGTERM→SIGKILL timeout | Hardcoded | 10 seconds | — | No |

**What changes without restart:**
- `auto_restart`, `max_restart_attempts`, `restart_cooldown_secs`, `restart_on_degradation`: Effective immediately on next restart attempt.
- `restart_stagger_secs`: Effective on next staggered restart.
- Adding/removing servers: Handled by `apply_config_diff()`.

---

## 7. Observability

### 7.1 Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `llamaforge_server_status` | Gauge | `server_id` | 0=stopped, 1=starting, 2=running, 3=failed |
| `llamaforge_server_restarts_total` | Counter | `server_id, reason` | Restart count (reason: crash, health_fail, degradation, config) |
| `llamaforge_server_uptime_seconds` | Gauge | `server_id` | Time since last successful start |
| `llamaforge_vram_reserved_bytes` | Gauge | `gpu_id` | Current ledger reservations (spawn-to-healthy only) |
| `llamaforge_vram_free_bytes` | Gauge | `gpu_id` | NVML-reported free VRAM |
| `llamaforge_gpu_utilization_pct` | Gauge | `gpu_id` | GPU compute utilization |
| `llamaforge_server_cumulative_requests` | Counter | `server_id` | For id_task overflow monitoring |
| `llamaforge_pending_dispatches` | Gauge | `server_id` | Dispatched but not slot-confirmed |
| `llamaforge_health_check_duration_seconds` | Histogram | `server_id` | Health check latency |

**Label cardinality:** `server_id` bounded by config (typically 2-8). `reason` bounded enum (4 values). `gpu_id` bounded by hardware (typically 1-8).

### 7.2 Log Events

| Event | Level | Frequency | Content |
|-------|-------|-----------|---------|
| `server.started` | INFO | On spawn | Server ID, PID, port, model |
| `server.stopped` | INFO | On exit | Server ID, exit code/signal, uptime |
| `server.health_ok` | DEBUG | Sampled 1/10 | Server ID, response time |
| `server.health_fail` | WARN | On failure | Server ID, failure count, error |
| `server.deadlock_detected` | ERROR | Rare | Server ID, last slot state, GPU util, duration |
| `server.performance_degraded` | WARN | On degradation | Server ID, current TPS, rolling avg |
| `server.vram_insufficient` | ERROR | On spawn fail | GPU ID, needed vs. free |
| `server.restarting` | INFO | On restart | Server ID, reason, attempt number |
| `server.failed` | ERROR | On max retries | Server ID, total attempts |
| `server.orphan_reclaimed` | INFO | On restart | PID, server ID |
| `server.id_task_overflow_warning` | WARN | At i32::MAX/2 | Server ID, cumulative count, recommend restart |

### 7.3 Health Signals

- **Ready:** All configured servers have reached Running state (or Failed — failed servers don't block readiness).
- **Live:** Server Manager task is alive (heartbeat via health endpoint).
- **Degraded:** Any server in Failed state OR cumulative requests > `i32::MAX / 2` for any server.

---

## 8. Testing Strategy

### 8.1 Unit Test Boundaries

**Pure-logic tests:**
- VRAM estimation formula (various quant types, ctx sizes, architectures)
- Deadlock detection logic (all four conditions, edge cases)
- Backoff with jitter computation
- Config diff → lifecycle action mapping (what requires restart vs. hot-reload)
- Drain timeout formula
- Cumulative request overflow threshold

**Mockable:**
- `GpuTelemetry` trait → `StubTelemetry` returning configurable values
- `MetricsStore` trait → in-memory mock for PID persistence
- `tokio::process` → cannot mock directly; use integration tests with real processes

### 8.2 Integration Test Requirements

- **Real processes:** Spawn a minimal HTTP server (e.g., `python -m http.server` or a custom Rust binary) to simulate llama-server's `/health`, `/slots`, `/metrics` endpoints.
- **Lifecycle transitions:** Test full state machine: Stopped → Starting → Running → Draining → Stopped.
- **Crash detection:** Kill child process → assert state transition within 100ms.
- **Graceful shutdown:** Start server, dispatch fake requests, initiate shutdown → assert drain then SIGTERM.
- **Tokio runtime:** Multi-threaded for concurrent server management.

### 8.3 Property-Based Tests

- **Backoff monotonicity:** `∀ attempt n: backoff(n) <= backoff(n+1)` (modulo jitter)
- **Ledger consistency:** `∀ spawn/exit sequence: ledger entries == servers in Starting state`
- **Stagger correctness:** `∀ concurrent restarts on same GPU: time between starts >= stagger_secs`

### 8.4 Specific Test Cases

```rust
#[tokio::test]
async fn test_vram_ledger_removed_on_exit() {
    // Start server → ledger entry created.
    // Kill server → assert ledger entry removed within 1s.
}

#[tokio::test]
async fn test_concurrent_spawn_same_gpu_serialized() {
    // Request two spawns on gpu:0 simultaneously.
    // Assert: second spawn waits for first to complete health check.
}

#[tokio::test]
async fn test_deadlock_detection_all_conditions() {
    // Mock: is_processing=true, n_decoded stalled, GPU util=0, was_healthy.
    // Assert: deadlocked=true, SIGKILL sent.
}

#[tokio::test]
async fn test_deadlock_false_positive_during_prefill() {
    // Mock: is_processing=true, n_decoded=0, GPU util=95%.
    // Assert: deadlocked=false (GPU is busy → legitimate prefill).
}

#[tokio::test]
async fn test_health_timeout_not_counted_as_failure() {
    // Health check times out once → assert: still Running, no restart.
}

#[tokio::test]
async fn test_three_consecutive_failures_trigger_restart() {
    // Three consecutive connection refused → assert: state=Restarting.
}

#[tokio::test]
async fn test_config_reload_adds_new_server() {
    // Apply diff with new server → assert: new server task spawned, health checked.
}

#[tokio::test]
async fn test_config_reload_removes_server() {
    // Apply diff removing server → assert: graceful shutdown, task cleaned up.
}

#[tokio::test]
async fn test_orphan_recovery_on_startup() {
    // Pre-populate managed_processes table with a PID of a running process.
    // Start ServerManager → assert: process adopted into Running state.
}

#[tokio::test]
async fn test_id_task_overflow_warning() {
    // Set cumulative_requests to i32::MAX/2 - 1.
    // Increment → assert: warning logged.
}
```

---

## 9. Open Questions

### Deferred to Implementation

1. **Process group for orphan detection:** Should we use `setsid()` to create a process group, allowing `kill(-pgid, 0)` for batch liveness check? Decision: yes — simplifies orphan recovery and enables clean group kill on SIGTERM if drain times out.

2. **Health check endpoint validation:** Should we validate that `/health` response JSON matches expected llama-server format, or just check HTTP 200? Decision: check HTTP 200 only for v1. Format validation is fragile across llama.cpp versions.

3. **GGUF metadata parsing for VRAM estimation:** Implement in Rust (parsing the GGUF header) or shell out to `llama-server --info`? Decision: Rust implementation — GGUF format is simple and well-documented. Avoids process spawn overhead.

4. **Per-server reqwest::Client ownership:** Server Manager creates the client, or Dispatch & Proxy creates it and Server Manager just signals "please recreate"? Decision: Server Manager owns creation (knows port/TLS config), publishes via `ClientRefreshEvent`. Dispatch & Proxy subscribes and swaps its local `ArcSwap<Client>`.

5. **Rolling restart strategy (for id_task overflow):** Automatic rolling restart at `i32::MAX * 0.8`, or warning-only? Decision: warning-only for v1. Automatic rolling restart requires drain coordination that's complex to get right. Document the recommended cron approach.

### Alternatives Considered

- **External supervision (systemd/K8s):** Rejected for v1 (single-binary constraint). Migration path documented for v2.
- **Per-server sidecar:** Rejected (doubles process count, introduces distributed coordination).
- **watchdog pattern instead of polling:** Considered having llama-server send heartbeats to LlamaForge. Rejected — requires llama-server modification. External polling is sufficient at 2-4 server scale.

---

## Cross-Cutting Concerns

### Cancellation Safety

- `pre_spawn_vram_check()`: Synchronous mutex lock (no `.await` while held). Safe.
- `child.wait().await`: If cancelled, the `Child` handle is dropped. On Unix, this does NOT kill the child — it becomes orphaned. The server task must ensure it either completes `child.wait()` or explicitly kills the child before being dropped.
- Health check `reqwest::get().await`: If cancelled, the HTTP request is aborted. No state change. Next tick retries.
- Graceful shutdown drain: Uses `CancellationToken` — cooperative. If the drain is cancelled (parent shutdown), escalate immediately to SIGTERM.

**Critical:** The per-server Tokio task must use `tokio::select!` with a shutdown branch that runs cleanup (kill child, remove ledger entry) before the task terminates.

### Graceful Shutdown

On SIGTERM:
1. Cancel all per-server monitoring tasks (they enter their shutdown branch).
2. Each server task: stop routing → drain (with shortened timeout) → SIGTERM → SIGKILL.
3. All server tasks run in a `JoinSet` — wait for all to complete.
4. Clear VRAM ledger (all entries).
5. Remove managed_processes entries from SQLite (clean state for next startup).

### Panic Safety

- **Per-server task panic:** JoinSet detects it. Server enters Failed state. Other servers unaffected.
- **VRAM ledger mutex poison:** `Mutex` is poisoned. All subsequent VRAM checks fail → all spawns fail. This is catastrophic. Mitigation: the critical section is trivial arithmetic — panic is near-impossible. If it happens, log and use `into_inner()` to recover the data.
- **GPU telemetry FFI panic:** Caught by the `GpuTelemetry` impl wrapper. Returns `TelemetryError::FfiError`. No propagation.

### Clock Assumptions

- **Health check interval:** `tokio::time::interval` (monotonic). Not affected by clock skew.
- **Backoff calculation:** `Instant`-based. Monotonic.
- **Performance degradation rolling average:** Window based on `Instant` timestamps. Monotonic.
- **id_task overflow:** Counter-based, no clock dependency.

### Memory Bounds

| Resource | Bound | Worst Case |
|----------|-------|------------|
| ServerState per server | Fixed | ~4KB × 8 servers = 32KB |
| SlotState per slot per server | Fixed | ~200B × 32 slots × 8 servers = 50KB |
| VRAM ledger entries | Bounded by GPU count | ~100B × 8 GPUs = 800B |
| Per-server Tokio task stack | Fixed | ~8KB × 8 servers = 64KB |
| Rolling TPS window (5 min) | Fixed | ~50 samples × 8B × 8 servers = 3.2KB |

**Total:** < 200KB. Trivially bounded.

### Backpressure

- **Config reload with many servers:** `apply_config_diff()` processes servers sequentially (staggered). A config adding 10 servers takes `10 × restart_stagger_secs` to fully apply. The reload API returns early with the diff; server starts happen asynchronously.
- **Health check burst after network partition:** If network recovers and all health checks succeed simultaneously, the slot state updates flood Smart Routing. This is fine — DashMap writes are O(1) and routing reads a snapshot.
- **Restart storms:** Per-GPU semaphore + stagger prevents overwhelming GPU memory allocator.
