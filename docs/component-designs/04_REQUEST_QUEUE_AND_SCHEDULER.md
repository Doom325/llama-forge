# Component Design: Request Queue & Scheduler

**Component:** Request Queue & Scheduler  
**Phase:** 3 (Request Pipeline)  
**Estimated LOC:** ~2,000  
**Risk:** Medium-High  
**Dependencies:** Config Manager (priority rules, queue limits), Smart Routing (model-aware dispatch check), Dispatch & Proxy (hand off dispatched requests)  
**Consumers:** Transport Layer (enqueue), Dispatch & Proxy (dequeue)

---

## 1. Purpose & Scope

The Request Queue is the central nervous system of LlamaForge. Every inference request passes through it. It implements a multi-level priority queue with Deficit Round Robin fair share, logarithmic starvation prevention, model-aware dispatch (no head-of-line blocking), admission control (depth limits, per-requestor limits, timeouts), and queue position feedback to clients.

**What this component explicitly does NOT do:**

- It does not decide which specific server/slot to use (that's Smart Routing — it only checks "is there ANY server for this model with capacity").
- It does not proxy HTTP streams to backends (that's Dispatch & Proxy).
- It does not manage server processes (that's Server Manager).
- It does not handle protocol translation (that's Transport Layer).
- It does not persist queued requests (the queue is intentionally in-memory — crash loses queue contents).

---

## 2. Public API Surface

### 2.1 Queue Interface

```rust
/// The primary queue interface. Shared via Arc.
pub struct RequestQueue {
    inner: std::sync::Mutex<MultiLevelQueue>,
    config: Arc<ConfigHandle>,
    starvation_notify: tokio::sync::Notify,
    dispatch_notify: tokio::sync::Notify,
    admission_semaphores: DashMap<String, Arc<Semaphore>>, // per-requestor
}

impl RequestQueue {
    pub fn new(config: Arc<ConfigHandle>) -> Self;

    /// Enqueue a request. Returns queue position info or rejection.
    pub fn enqueue(&self, request: QueuedRequest) -> Result<QueuePosition, AdmissionError>;

    /// Try to dispatch the next eligible request for available capacity.
    /// Called by the scheduler loop when a slot becomes available.
    /// Returns None if no request can be dispatched (all models full).
    pub fn try_dispatch(
        &self,
        available_capacity: &AvailableCapacity,
    ) -> Option<DispatchCandidate>;

    /// Cancel a queued request by ID. Returns the request if found.
    pub fn cancel(&self, id: &RequestId) -> Option<QueuedRequest>;

    /// Boost a request's priority (for manual operator intervention).
    pub fn boost(&self, id: &RequestId, new_priority: u32) -> Result<(), QueueError>;

    /// Get current queue depth and per-tier breakdown.
    pub fn depth(&self) -> QueueDepthInfo;

    /// Get position and estimated wait for a specific request.
    pub fn position(&self, id: &RequestId) -> Option<QueuePosition>;

    /// Wait for a dispatch opportunity (used by scheduler loop).
    pub async fn wait_for_dispatch_signal(&self);

    /// Signal that a slot has become available (called by Dispatch on completion).
    pub fn notify_slot_available(&self);
}
```

### 2.2 Data Types

```rust
/// A request waiting in the queue.
#[derive(Debug)]
pub struct QueuedRequest {
    pub id: RequestId,
    pub payload: Arc<InferencePayload>,  // Arc for cheap clone during starvation scan
    pub task_type: TaskType,
    pub requestor: RequestorId,
    pub requestor_class: RequestorClass,         // bounded Prometheus label
    pub model_target: ModelTarget,       // which model(s) can serve this
    pub effective_priority: u32,
    pub base_priority: u32,
    pub requestor_bonus: i32,
    pub explicit_override: Option<u32>,
    pub enqueued_at: Instant,            // monotonic for boost computation
    pub enqueued_at_utc: DateTime<Utc>,  // for serialization/display only
    pub queue_timeout: Duration,
    pub response_tx: oneshot::Sender<DispatchResult>, // or streaming channel
    pub cancel_token: CancellationToken,
}

/// What model(s) can serve this request.
#[derive(Debug, Clone)]
pub enum ModelTarget {
    Exact(ModelAlias),                // specific model alias
    AnyOf(Vec<ModelAlias>),          // any of these aliases
    AnyWithCapability(Capability), // any model with this capability
}

/// Result of dispatch selection.
#[derive(Debug)]
pub struct DispatchCandidate {
    pub request: QueuedRequest,
    pub dispatch_priority: u32,   // effective_priority at dispatch time
    pub queue_wait: Duration,
}

/// Available capacity information (from Server Manager + Smart Routing).
pub struct AvailableCapacity {
    pub servers: Vec<(ServerId, Vec<ModelAlias>, u32)>, // (id, model_aliases, free_slots)
}

/// Queue position feedback.
#[derive(Debug, Clone)]
pub struct QueuePosition {
    pub position: u32,           // 1-indexed position in priority order
    pub estimated_wait_secs: f64,
    pub tier: PriorityTier,
}

/// Queue depth breakdown.
#[derive(Debug, Clone)]
pub struct QueueDepthInfo {
    pub total: u32,
    pub by_tier: HashMap<PriorityTier, u32>,
    pub by_model: HashMap<ModelAlias, u32>,
}

/// Port traits.
pub trait CapacityChecker: Send + Sync {
    fn has_capacity(&self, target: &ModelTarget) -> bool;
}

#[allow(async_fn_in_trait)]
pub trait SlotDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        server_id: &ServerId,
        request: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    ) -> Result<(ResponseStream, Box<dyn DispatchToken>), DispatchError>;
}

pub trait ServerSelector: Send + Sync {
    fn select_server(&self, target: &ModelTarget) -> Option<RoutingDecision>;
}

pub trait AdmissionAccounting: Send + Sync {
    fn record_completion(&self, requestor: &RequestorId);
    fn record_dequeue(&self, requestor: &RequestorId, reason: DequeueReason);
}

pub trait SlotNotifier: Send + Sync {
    fn notify_slot_available(&self);
}
```

### 2.3 Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    #[error("queue full ({depth}/{max}), retry after {retry_after_secs}s")]
    QueueFull { depth: u32, max: u32, retry_after_secs: u32 },

    #[error("requestor '{requestor}' at concurrent limit ({current}/{max})")]
    RequestorLimit { requestor: RequestorId, current: u32, max: u32 },

    #[error("no server configured for model '{model}'")]
    NoServerForModel { model: ModelAlias },
}

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("request {id} not found in queue")]
    NotFound { id: RequestId },
}
```

---

## 3. Internal Architecture

### 3.1 Data Structure

```rust
/// The core multi-level priority queue.
struct MultiLevelQueue {
    /// Outer: tier selection (4 tiers). Inner: priority-ordered requests.
    buckets: BTreeMap<PriorityTier, BTreeMap<Reverse<u32>, VecDeque<QueuedRequest>>>,

    /// O(1) lookup by request ID → (tier, effective_priority) for cancel/boost.
    index: HashMap<RequestId, (PriorityTier, u32)>,

    /// DRR state per tier.
    drr: DrrState,

    /// Total depth (cached to avoid iterating buckets).
    total_depth: u32,
}

/// Deficit Round Robin tracking state.
struct DrrState {
    /// Tokens dispatched per tier in sliding 60s window.
    tier_tokens: HashMap<PriorityTier, SlidingWindow>,

    /// Configured fair share percentages.
    fair_share: FairShareConfig,

    /// Deficit counters (DRR core state).
    deficit: HashMap<PriorityTier, u64>,

    /// Quantum tokens per round.
    quantum: u64,

    /// Track when tiers became empty (for redistribution).
    tier_empty_since: HashMap<PriorityTier, Option<Instant>>,
}

/// Fixed-size sliding window for token counting.
struct SlidingWindow {
    /// Ring buffer of (timestamp, tokens) pairs.
    entries: VecDeque<(Instant, u64)>,
    window_size: Duration,
    total: u64,
}
```

### 3.2 Dispatch Flow

```
    Transport Layer calls enqueue()
                │
                ▼
    ┌───────────────────────────┐
    │ Admission Control         │
    │ - Check queue depth       │
    │ - Check per-requestor     │
    │ - Check model exists      │
    └────────────┬──────────────┘
                 │ pass
                 ▼
    ┌───���────────────────────���──┐
    │ Compute effective_priority│
    │ - base + bonus + override │
    │ - Determine tier          │
    ���────────────┬──────────────┘
                 │
                 ▼
    ┌───��───────────────────────┐
    �� Insert into MultiLevelQueue│
    │ - BTreeMap at tier        │
    │ - Index update            │
    │ - notify_slot_available() │
    └───────────��───────────────┘

    ─────── Scheduler Loop (separate task) ───────

    wait_for_dispatch_signal()
                │
                ▼
    ┌───��───────────────────────────────────┐
    │ DRR: Determine which tier to serve    │
    │ - Check fair share vs actual share    │
    │ - If total_tokens==0: pure priority   │
    │ - Pick starved tier first             │
    └────────────┬────────────────────────��─┘
                 │
                 ▼
    ┌───────────────────────────────────────┐
    │ Model-Aware Peek & Iterate            ��
    │ - For each request in priority order: │
    │   - Does any server have capacity?    │
    │   - If yes: record coordinates, break │
    │   - If no: skip, continue             │
    └────────────┬────���─────────────────────┘
                 │ found candidate
                 ▼
    ┌─��─────────────────────────────────────┐
    │ Remove from queue (coordinates-based) │
    │ - Update index                        │
    │ - Update DRR counters                 │
    │ - Return DispatchCandidate            │
    └──────────────────���────────────────────┘
                 │
                 ▼
         Hand off to Dispatch & Proxy
```

### 3.3 Starvation Boost Cycle (every 5 seconds)

```
    ┌───────────────────────────────────────┐
    │ Phase 1: Snapshot (under lock)        │
    │ - Collect Vec<(RequestId, Instant)>   │
    │ - Release lock immediately            │
    └────────────┬���──────────────────���──────┘
                 │
    ┌────────────▼───��──────────────────────┐
    │ Phase 2: Compute (no lock)            │
    │ - For each request:                   │
    │   - wait_intervals = (now - enqueued  │
    │     - 30s) / 5s                       │
    │   - boost = min(50 * ln(1 + wait),    │
    │     400)                              │
    │ - Filter: only requests where         │
    │   computed boost > 0                  │
    └──────��─────┬──────────────────────────┘
                 │
    ┌────────────▼──────────────────────���───┐
    │ Phase 3: Apply (under lock)           │  [spawn_blocking if depth > 200]
    │ - For each candidate:                 │
    │   - Skip if no longer in index        │
    │   - RECOMPUTE boost from enqueued_at  │  ← G-9 fix: authoritative at apply
    │   - Skip if new_prio == current_prio  │  ← preserve FIFO position
    │   - Re-key in BTreeMap                │
    │   - Update index                      │
    └────────────���──────────────────────────┘
```

### 3.4 Key Invariants

1. **FIFO within same priority:** Requests with identical `effective_priority` are served in arrival order (VecDeque front-pop).
2. **Index consistency:** `index` always reflects current (tier, priority) of every request in the queue. Updated atomically with BTreeMap mutations.
3. **Model-aware, work-conserving:** No slot sits idle while any dispatchable request exists (regardless of higher-priority requests blocking on other models).
4. **DRR never wastes capacity:** Fair share is a soft floor. If a tier's queue is empty, its share is available to other tiers.
5. **Monotonic time for boost:** `enqueued_at` uses `Instant` (monotonic). NTP corrections cannot produce negative wait or incorrect boost.
6. **Lock never held across .await:** `std::sync::Mutex` only. Critical section is pure computation (insert, remove, re-key). No I/O, no channel sends under lock.
7. **DRR division-by-zero guard:** `if total_tokens == 0 { pure priority }` — unconditional, no time dependency.

---

## 4. Boundary Contracts

### 4.1 What This Component REQUIRES

| Dependency | Contract |
|-----------|----------|
| Config Manager | `PriorityRulesConfig` (task types, bonuses, starvation params, fair share). Reloads update the rules for new requests (in-flight requests keep their original base priority). |
| Smart Routing / Server Manager | `AvailableCapacity` — which servers have free slots, what models they serve. Must be fresh (≤5 seconds old). |
| Dispatch & Proxy | `notify_slot_available()` called when a slot is freed. Without this, scheduler loop sleeps indefinitely after queue drains. |

### 4.2 What This Component GUARANTEES to Consumers

| Consumer | Guarantee |
|----------|-----------|
| Transport Layer | `enqueue()` is O(log K) and non-blocking (no .await). Returns immediately with position or rejection. |
| Transport Layer | Queue position updates available via `position()` (polling) or watcher channel (streaming). |
| Dispatch & Proxy | `DispatchCandidate` contains a fully-formed request ready for dispatch. No further queue interaction needed. |
| Dispatch & Proxy | `cancel()` is O(log K). Request is removed atomically — no double-dispatch possible. |
| All consumers | `depth()` is O(1) (cached counter). Safe to poll for metrics without contention. |

### 4.3 Failure Propagation

| Failure | Propagation |
|---------|-------------|
| Queue full | `AdmissionError::QueueFull` → Transport Layer returns HTTP 429 with `Retry-After`. |
| Per-requestor limit | `AdmissionError::RequestorLimit` → Transport Layer returns HTTP 429. |
| Queue timeout | Request evicted, `response_tx` sent `Err(Timeout)` → client sees 408. |
| Starvation boost phase panic | Boost cycle fails. Existing priorities unchanged. Next cycle retries. Logged as error. |
| No capacity for any model | `try_dispatch()` returns None. Scheduler sleeps until next `notify_slot_available()`. |
| Client disconnect during queue wait | `cancel_token` fires → queue removes the request (if still present). No orphaned entries. |

---

## 5. Failure Modes

### FM-1: All Servers for a Model Go Down

- **Scenario:** The only server for "llama-3.1-8b" crashes. Requests queue up.
- **Impact:** Requests for that model wait (up to `queue_timeout`). Other model requests dispatch normally (model-aware iteration skips blocked requests).
- **Recovery:** Server Manager restarts the server. On health check pass, `notify_slot_available()` fires. Scheduler dispatches queued requests.

### FM-2: Starvation Boost Causes Priority Inversion Burst

- **Scenario:** Background requests boosted to emergency-tier range after long wait.
- **Mitigation:** Logarithmic curve saturates at `max_boost=400`. Background(100) + 400 = 500, still below RealtimeChat(800). True priority inversion requires multi-minute waits, which DRR prevents independently.

### FM-3: Queue Depth Exceeds 200 During Starvation Scan

- **Scenario:** High admission rate fills queue to 500+ during a starvation cycle.
- **Impact:** Apply phase takes longer. Wrapped in `spawn_blocking` to avoid Tokio starvation.
- **Recovery:** Self-recovering. Once apply completes, queue processes normally.

### FM-4: VecDeque Large-D Pathology

- **Scenario:** Hundreds of requests at identical `effective_priority` (misconfigured priority rules).
- **Impact:** O(D) removal per re-key during starvation boost.
- **Detection:** `llamaforge_starvation_boost_duration_seconds` histogram shows elevated p99.
- **Mitigation:** Operator should differentiate priorities. At D>50, consider switching to a `HashMap<usize, _>` + swap_remove approach (loses FIFO — documented tradeoff for pathological cases).

### FM-5: Client Disconnects After Enqueue, Before Dispatch

- **Scenario:** Client closes TCP connection while request sits in queue.
- **Detection:** `cancel_token.cancelled()` fires (set by Transport Layer on connection drop).
- **Recovery:** Queue's timeout sweep (every 1 second) checks `cancel_token.is_cancelled()` and removes dead requests. Alternatively, an immediate removal on cancel notification via `cancel()`.

---

## 6. Configuration

| Knob | Location | Default | Valid Range | Hot-Reload |
|------|----------|---------|-------------|------------|
| `queue_max_depth` | `[global]` | 1000 | 1-100000 | Yes |
| `queue_default_timeout_secs` | `[global]` | 300 | 1-3600 | Yes |
| `task_types.*` | `priority_rules.toml` | See TOML | 0-10000 | Yes |
| `requestor_bonuses.*` | `priority_rules.toml` | See TOML | -1000 to 1000 | Yes |
| `starvation.check_interval_secs` | `priority_rules.toml` | 5 | 1-60 | Yes |
| `starvation.boost_scale` | `priority_rules.toml` | 50 | 1-500 | Yes |
| `starvation.boost_start_after_secs` | `priority_rules.toml` | 30 | 0-300 | Yes |
| `starvation.max_boost` | `priority_rules.toml` | 400 | 0-5000 | Yes |
| `fair_share.high_tier_min_pct` | `priority_rules.toml` | 60 | 0-100 | Yes |
| `fair_share.standard_tier_min_pct` | `priority_rules.toml` | 30 | 0-100 | Yes |
| `fair_share.low_tier_min_pct` | `priority_rules.toml` | 10 | 0-100 | Yes |
| Per-requestor max concurrent | Hardcoded | 50 | — | No |
| DRR sliding window | Hardcoded | 60s (30s for <10 slots) | — | No |

**What changes without restart:**
- All `priority_rules.toml` values take effect on next enqueue/starvation cycle. In-flight requests keep their base priority but get updated starvation params.
- `queue_max_depth` / `queue_default_timeout_secs` effective immediately for new requests.

---

## 7. Observability

### 7.1 Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `llamaforge_queue_depth` | Gauge | `priority_tier` | Current depth per tier |
| `llamaforge_queue_wait_seconds` | Histogram | `priority_tier` | Wait time for dispatched requests |
| `llamaforge_queue_admissions_total` | Counter | `result={accepted,rejected_full,rejected_limit}` | Admission outcomes |
| `llamaforge_queue_timeouts_total` | Counter | — | Requests evicted by timeout |
| `llamaforge_queue_cancellations_total` | Counter | `reason={client,operator,timeout}` | Cancel reasons |
| `llamaforge_starvation_boost_applied_total` | Counter | `crossed_tier={yes,no}` | Boost applications |
| `llamaforge_starvation_boost_duration_seconds` | Histogram | — | Time to complete boost cycle |
| `llamaforge_drr_override_total` | Counter | `tier` | Times DRR overrode priority ordering |
| `llamaforge_dispatch_scan_depth` | Histogram | — | How many requests scanned before finding dispatchable one |

**Label cardinality:** `priority_tier` (4 values), `result` (3 values), `reason` (3 values). All bounded.

### 7.2 Log Events

| Event | Level | Frequency | Content |
|-------|-------|-----------|---------|
| `queue.admitted` | DEBUG | Per-request | Request ID, model, tier, position |
| `queue.rejected` | WARN | On rejection | Reason, queue depth, requestor |
| `queue.dispatched` | DEBUG | Per-dispatch | Request ID, wait time, tier, DRR override? |
| `queue.timeout` | INFO | On timeout | Request ID, waited duration, tier |
| `queue.cancelled` | DEBUG | On cancel | Request ID, reason |
| `queue.starvation_boost` | DEBUG | Sampled | Requests boosted, tier promotions, cycle duration |
| `queue.drr_override` | INFO | On override | Starved tier, actual vs target share |
| `queue.unknown_task_type` | WARN | First occurrence per type | Task type string, defaulting to priority 0 |

### 7.3 Health Signals

- **Ready:** Queue initialized and scheduler loop running.
- **Live:** Scheduler loop responsive (heartbeat within 10s).
- **Degraded:** Queue > 80% of `queue_max_depth` for > 30s.

---

## 8. Testing Strategy

### 8.1 Unit Test Boundaries

**Pure-logic tests (no async, no I/O):**
- Priority computation: base + bonus + override + starvation
- Tier classification from effective priority
- BTreeMap insertion/removal correctness
- FIFO ordering within same priority
- Model-aware dispatch: skips model-blocked requests
- DRR share computation and override logic
- DRR division-by-zero guard (total_tokens == 0)
- Starvation boost formula (logarithmic, capped)
- Queue position calculation
- Admission control (depth limit, per-requestor)
- Fair share normalization (percentages sum to 100)
- Tier empty redistribution with grace period

### 8.2 Integration Test Requirements

- **Tokio runtime:** For scheduler loop, notify signals, timeout handling.
- **No external processes:** Fully testable with mock `AvailableCapacity`.

### 8.3 Property-Based Tests

- **Priority ordering:** `∀ (a, b) where a.priority > b.priority and same model: a dispatched before b`
- **FIFO within priority:** `��� (a, b) where a.priority == b.priority: a.enqueued_at < b.enqueued_at ⟹ a dispatched first`
- **Starvation bounded:** `∀ request: boost ≤ max_boost`
- **Work-conserving:** `∀ state where queue non-empty AND capacity available for that model: try_dispatch returns Some`
- **Index consistency:** `∀ operations: index.len() == sum of all VecDeque lengths across all BTreeMaps`
- **DRR fairness:** Over 1000 dispatches with mixed tiers, each tier's actual share is within 10% of its configured minimum (when all tiers have demand).

### 8.4 Specific Test Cases

```rust
#[test]
fn test_model_aware_no_hol_blocking() {
    // Queue: [High-priority ModelA, Low-priority ModelB]
    // Capacity: ModelA=0 slots, ModelB=1 slot
    // Assert: ModelB request dispatched (not blocked by ModelA).
}

#[test]
fn test_fifo_within_same_priority() {
    // Insert 5 requests at priority 600.
    // Dispatch all 5 → assert: dispatched in insertion order.
}

#[test]
fn test_starvation_boost_logarithmic() {
    // Request waiting 60s (6 intervals after 30s start).
    // boost = min(50 * ln(1+6), 400) = min(50 * 1.945, 400) = 97.
    // Assert: effective_priority increased by 97.
}

#[test]
fn test_drr_override_starved_tier() {
    // Standard tier at 10% share (target: 30%).
    // Next dispatch: assert DRR selects standard tier despite emergency having requests.
}

#[test]
fn test_drr_division_by_zero() {
    // total_tokens == 0 (no dispatches yet).
    // Assert: falls back to pure priority ordering without panic.
}

#[test]
fn test_boost_skip_if_same_priority() {
    // Request with boost that doesn't change effective_priority (below rounding).
    // Assert: request NOT re-keyed (FIFO position preserved).
}

#[test]
fn test_cancel_during_starvation_scan() {
    // Start starvation scan phase 1 (snapshot).
    // Cancel a snapshotted request before phase 3.
    // Assert: phase 3 skips it silently (not in index).
}

#[tokio::test]
async fn test_queue_timeout_eviction() {
    // Enqueue with 1s timeout. Wait 2s. Assert: request evicted, response_tx sent Timeout.
}

#[tokio::test]
async fn test_client_disconnect_cleanup() {
    // Enqueue with a CancellationToken. Cancel the token.
    // Assert: request removed from queue within 1s.
}

#[test]
fn test_admission_queue_full() {
    // Fill to max_depth. Next enqueue → AdmissionError::QueueFull.
}
```

---

## 9. Open Questions

### Deferred to Implementation

1. **Queue timeout sweep mechanism:** Periodic Tokio task (every 1s) scanning for expired requests, or per-request `tokio::time::sleep` that fires and calls `cancel()`? Decision: periodic sweep — simpler, fewer spawned tasks, O(N) scan of timestamps is trivial at N<1000.

2. **Queue position calculation cost:** Computing position requires iterating from the front. For streaming position updates, should we cache positions and invalidate on mutation? Decision: compute on-demand for polling; for streaming SSE, send updates only on dispatch events (position decrements by 1).

3. **DRR quantum size:** Fixed 4096 tokens or adaptive based on average request size? Decision: fixed 4096 for v1 — simple, well-understood. Adaptive adds complexity for marginal benefit at 2-4 GPU scale.

4. **Per-requestor limit storage:** `DashMap<String, AtomicU32>` or tracked via the queue index? Decision: separate `DashMap` — the index tracks queued requests, but we also need to count in-flight (dispatched) requests against the limit. The limit is `queued + dispatched` per requestor.

5. **Priority rules hot-reload scope:** Do in-flight requests get their starvation params updated, or only new arrivals? Decision: `boost_scale` and `max_boost` apply globally on next starvation cycle (affects all queued requests). `base_priority` for existing requests is frozen (they keep the priority rules from enqueue time).

### Alternatives Considered

- **BinaryHeap:** Rejected — `iter()` is unordered, LIFO tie-breaking, no peek-and-skip.
- **Sorted Vec with binary search:** Viable for <100 requests. Rejected for generality — doesn't scale to 500+ without rewrite.
- **Skip list:** O(log N) insert/remove with iteration. More complex than BTreeMap, no standard library implementation.
- **Channel-based (mpsc):** Rejected — no priority ordering, no cancel-by-ID, no position queries.
- **Priority mutation for DRR:** Rejected — destroys semantic meaning of priority score, corrupts metrics, requires re-keying. Bucket-level selection achieves same outcome.

---

## Cross-Cutting Concerns

### Cancellation Safety

- `enqueue()`: Synchronous (no .await). Cannot be cancelled.
- `try_dispatch()`: Synchronous. Cannot be cancelled.
- `wait_for_dispatch_signal()`: Single `.await` on `Notify`. If cancelled, scheduler loop simply restarts. No state change.
- `starvation_boost_cycle()`: If cancelled between phases, no harm — partial boost is never applied (phase 3 is atomic under lock).

### Graceful Shutdown

On SIGTERM:
1. Stop admitting new requests (enqueue returns error).
2. Let scheduler continue dispatching until queue is empty or drain timeout expires.
3. For remaining requests: send `Err(ShuttingDown)` to each `response_tx`. Client sees 503.
4. Drop queue state (all Arc<InferencePayload> freed).

### Panic Safety

- **Mutex panic (inside `enqueue`/`try_dispatch`):** Mutex is poisoned. All subsequent queue operations fail. System is dead. Mitigation: critical section is pure data structure manipulation — panics are logic bugs, not recoverable.
- **Starvation task panic:** Only the boost cycle is affected. Queue continues functioning without boosts. Logged as error. Consider restarting the starvation task.

### Clock Assumptions

- **`enqueued_at`:** `Instant` (monotonic). Immune to NTP.
- **Starvation boost:** Computed from `Instant` difference. Monotonic.
- **DRR sliding window:** `Instant`-based entries. Monotonic.
- **Queue timeout:** `enqueued_at + timeout` compared against `Instant::now()`. Monotonic.

### Memory Bounds

| Resource | Bound | Worst Case |
|----------|-------|------------|
| QueuedRequest | ~500 bytes (payload is Arc) | 1000 × 500B = 500KB |
| Index HashMap | ~100 bytes per entry | 1000 × 100B = 100KB |
| DRR sliding window | 60s × ~10 entries/sec | 600 × 16B = 10KB |
| Per-requestor semaphores | ~100 bytes per requestor | 50 requestors × 100B = 5KB |

**Total worst case:** < 1MB at max queue depth. Fully bounded by `queue_max_depth`.

### Backpressure

- **Admission → Transport Layer:** Queue full returns 429 with `Retry-After`. Transport Layer passes this to client. Client implements exponential backoff.
- **Dispatch → Scheduler loop:** If no capacity, `try_dispatch` returns None. Scheduler sleeps on `Notify` until `notify_slot_available()`. No busy-spinning.
- **Starvation → Queue:** If boost cycle is slow (>200 entries), it runs on blocking thread pool. Main scheduler loop unaffected.
