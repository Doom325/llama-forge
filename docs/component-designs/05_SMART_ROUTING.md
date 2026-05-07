# Component Design: Smart Routing

**Component:** Smart Routing  
**Phase:** 3 (Request Pipeline)  
**Estimated LOC:** ~1,500  
**Risk:** Medium  
**Dependencies:** Server Manager (slot state from /slots polling), Dispatch & Proxy (DispatchTracker counts), Config Manager (routing weights)  
**Consumers:** Request Queue (model-aware dispatch check), Dispatch & Proxy (slot selection)

---

## 1. Purpose & Scope

Smart Routing makes intelligent request placement decisions to maximize the efficiency of llama-server's built-in continuous batching. It computes a routing score per eligible server, compensates for stale /slots data via the dispatch tracker, and provides model-aware capacity checks for the queue's dispatch iterator.

**v1 scope (Tier 1 only):**
- Scoring: slot availability, KV cache usage, deferred ratio, session affinity
- Dispatch tracking compensation (stale-slot problem)
- Hard routing constraints (model, capabilities, LoRA, multimodal)
- Deterministic round-robin tie-breaking

**Explicitly NOT in v1 (Tier 2, deferred to v1.1):**
- Prompt prefix cache affinity (hierarchical hash matching)
- `id_slot` pinning for cache-warm slots
- Cross-server KV cache save/restore orchestration

**What this component explicitly does NOT do:**
- It does not manage the priority queue (that's Request Queue).
- It does not maintain server process state (that's Server Manager — it reads snapshots).
- It does not proxy HTTP connections (that's Dispatch & Proxy).
- It does not implement the dispatch tracker itself (that's Dispatch & Proxy via DispatchToken).

---

## 2. Public API Surface

### 2.1 Router Interface

```rust
/// The routing engine. Shared via Arc.
pub struct Router {
    config: Arc<ConfigHandle>,
    server_manager: Arc<ServerManager>,
}

impl Router {
    pub fn new(config: Arc<ConfigHandle>, server_manager: Arc<ServerManager>) -> Self;

    /// Check if any server has capacity for a given model target.
    /// Used by the queue's model-aware dispatch iterator.
    /// O(S) where S = number of servers for the model (typically 1-4).
    pub fn has_capacity(&self, target: &ModelTarget) -> bool;

    /// Select the best server for a request. Returns scored candidates.
    /// Called by Dispatch & Proxy after dequeue.
    pub fn select_server(
        &self,
        request: &QueuedRequest,
    ) -> Result<RoutingDecision, RoutingError>;

    /// Validate that a request CAN be served (hard constraints).
    /// Called at admission time (before enqueue).
    pub fn validate_routable(
        &self,
        model: &ModelTarget,
        capabilities: &RequestCapabilities,
    ) -> Result<(), RoutingError>;
}
```

### 2.2 Types

```rust
/// The routing decision — which server and slot to target.
#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub server_id: ServerId,
    pub score: f64,
    pub id_slot: i32,          // -1 for auto-assignment (v1 default)
    pub reason: RoutingReason,
}

#[derive(Debug, Clone)]
pub enum RoutingReason {
    HighestScore { score: f64 },
    SessionAffinity { session_id: String },
    RoundRobin { counter: u64 },  // tie-breaker
}

/// Capabilities required by a request (for hard constraint filtering).
#[derive(Debug, Clone, Default)]
pub struct RequestCapabilities {
    pub lora_adapter: Option<String>,
    pub multimodal: bool,
    pub embedding: bool,
    pub reranking: bool,
}

/// Routing weights (from config).
#[derive(Debug, Clone)]
pub struct RoutingWeights {
    pub w_slots: f64,     // default 0.4
    pub w_kv: f64,        // default 0.2
    pub w_queue: f64,     // default 0.2
    pub w_affinity: f64,  // default 0.2 (unused in v1, reserved for Tier 2)
}

#[derive(Debug, thiserror::Error)]
pub enum RoutingError {
    #[error("model not found: {model}")]
    ModelNotFound { model: String },

    #[error("no capable server: constraint={constraint}, requested={requested}")]
    NoCapableServer { constraint: String, requested: String },

    #[error("model unavailable: all servers in non-Running state")]
    ModelUnavailable { model: String, server_states: Vec<(ServerId, ServerLifecycle)> },
}
```

### 2.3 Session Affinity (v1 - simple LRU)

```rust
/// Session affinity cache. Maps session_id → server_id.
/// LRU with configurable max size (default: 10,000 sessions).
struct SessionAffinityCache {
    cache: lru::LruCache<String, ServerId>,
}

impl SessionAffinityCache {
    /// Look up affinity. Returns None if no affinity or server is unavailable.
    fn get(&mut self, session_id: &str, available_servers: &[ServerId]) -> Option<ServerId>;

    /// Record affinity after successful dispatch.
    fn set(&mut self, session_id: String, server_id: ServerId);

    /// Invalidate all entries for a server (called when server enters Draining/Failed).
    fn invalidate_server(&mut self, server_id: &ServerId);
}
```

---

## 3. Internal Architecture

### 3.1 Scoring Flow

```
    Request arrives for dispatch
                │
                ▼
    ┌───────────────────────────────────┐
    │ Hard Constraint Filter             │
    │ - Model match                      │
    │ - Server in Running state          │
    │ - LoRA adapter available           │
    │ - Multimodal capability            │
    │ - Architecture match (if needed)   │
    └────────────┬──────────────────────┘
                 │ eligible servers S[]
                 ▼
    ┌───────────────────────────────────┐
    │ Session Affinity Check            │
    │ - If session_id present in cache  │
    │   AND target server is in S[]     │
    │   → return immediately            │
    └────────────┬──────────────────────┘
                 │ no affinity hit
                 ▼
    ┌───────────────────────────────────┐
    │ Score Each Server                  │
    │                                    │
    │ For S[i]:                          │
    │  free_ratio = max(0,              │
    │    idle_slots - tracker.count())  │
    │    / total_slots                   │
    │                                    │
    │  score = w_slots × free_ratio     │
    │        + w_kv × (1 - kv_usage)    │
    │        + w_queue × (1 - deferred) │
    │        + w_affinity × 0.0 (v1)    │
    └────────────┬──────────────────────┘
                 │
                 ▼
    ┌───────────────────────────────────┐
    │ Select Best                        │
    │ - Highest score wins               │
    │ - Tie: deterministic round-robin   │
    │   (per-model AtomicU64 counter)   │
    └────────────┬──────────────────────┘
                 │
                 ▼
    RoutingDecision { server_id, score, id_slot: -1 }
```

### 3.2 Recurrent Model Handling

For servers with `model_architecture = "recurrent"` (Mamba, RWKV):
- `kv_cache_usage` component → 0.0 (no KV cache concept)
- `prompt_cache_affinity` component → 0.0 (no prefix caching)
- Score reduces to: `w_slots × free_ratio + w_queue × (1 - deferred)`

### 3.3 Key Invariants

1. **Hard constraints never violated:** A request is never routed to a server that doesn't serve its model or lacks required capabilities.
2. **Stale-slot compensation:** `free_ratio` accounts for in-flight dispatches via `tracker.count()`. Never goes below 0.
3. **Session affinity is advisory:** If the affinity server is full/unavailable, scoring takes over. Affinity never blocks dispatch.
4. **Tie-breaking is deterministic:** Round-robin counter ensures even spread on idle clusters between /slots polls.
5. **Scoring is read-only:** Router never mutates server state. It reads snapshots.

---

## 4. Boundary Contracts

### 4.1 What This Component REQUIRES

| Dependency | Contract |
|-----------|----------|
| Server Manager | `running_servers()` and `slot_states()` with ≤5s staleness. Server lifecycle accurately reported. |
| Dispatch & Proxy | `tracker.count()` per server available as O(1) read. Accurately reflects in-flight dispatches. |
| Config Manager | `RoutingWeights` available via config snapshot. |

### 4.2 What This Component GUARANTEES to Consumers

| Consumer | Guarantee |
|----------|-----------|
| Request Queue | `has_capacity()` returns true IFF at least one server has `free_ratio > 0` for the model. O(S) worst case (S = servers for model). |
| Dispatch & Proxy | `select_server()` returns the best available server OR a typed error explaining why no server is eligible. Never blocks. |
| Transport Layer | `validate_routable()` at admission prevents enqueuing unserviceable requests. |

### 4.3 Failure Propagation

| Failure | Propagation |
|---------|-------------|
| No server for model | `RoutingError::ModelNotFound` → 404 at admission. |
| All servers full | `has_capacity()` returns false → request stays in queue. |
| All servers down | `RoutingError::ModelUnavailable` → 503 at admission (or enqueue with 30s grace if Starting). |
| Stale slot data | Compensated by dispatch tracker. Worst case: brief over-dispatch (one extra request before next /slots poll corrects). |

---

## 5. Failure Modes

### FM-1: Dispatch Tracker Count Higher Than Reality

- **Scenario:** Proxy task panics after tracker insert but before /slots confirms. Tracker shows 1 fewer free slot than reality.
- **Impact:** Under-utilization (one phantom occupied slot) until reconciliation removes the stale entry.
- **Recovery:** Reconciliation cycle (every /slots poll) detects entries not matching any slot's processing state → removes them.

### FM-2: Session Affinity to a Degraded Server

- **Scenario:** Server passes health checks but has high KV cache usage. Session affinity pins requests to it.
- **Impact:** Higher latency for affinity-routed requests.
- **Mitigation:** Session affinity is checked only if target server's `free_ratio > 0`. If server is full, affinity is bypassed and scoring selects a better server.

### FM-3: All Scores Zero (Cluster Saturated)

- **Scenario:** All servers at 100% slot utilization.
- **Impact:** `has_capacity()` returns false. Requests queue. No dispatch occurs.
- **Recovery:** Self-recovering when any request completes and frees a slot → `notify_slot_available()`.

---

## 6. Configuration

| Knob | Location | Default | Valid Range | Hot-Reload |
|------|----------|---------|-------------|------------|
| `routing.w_slots` | `llamaforge.toml` | 0.4 | 0.0-1.0 | Yes |
| `routing.w_kv` | `llamaforge.toml` | 0.2 | 0.0-1.0 | Yes |
| `routing.w_queue` | `llamaforge.toml` | 0.2 | 0.0-1.0 | Yes |
| `routing.w_affinity` | `llamaforge.toml` | 0.2 | 0.0-1.0 | Yes (unused in v1) |
| `routing.session_affinity_max_entries` | `llamaforge.toml` | 10000 | 100-1000000 | No |
| `routing.startup_wait_secs` | `llamaforge.toml` | 30 | 0-300 | Yes |

---

## 7. Observability

### 7.1 Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `llamaforge_routing_decisions_total` | Counter | `reason={score,affinity,round_robin}` | How decisions were made |
| `llamaforge_routing_score` | Histogram | `server_id` | Score distribution per server |
| `llamaforge_routing_rejections_total` | Counter | `reason={not_found,no_capable,unavailable}` | Admission rejections |
| `llamaforge_session_affinity_hits_total` | Counter | — | Affinity cache hits |
| `llamaforge_session_affinity_misses_total` | Counter | — | Affinity cache misses |

### 7.2 Log Events

| Event | Level | Frequency | Content |
|-------|-------|-----------|---------|
| `routing.selected` | DEBUG | Per-dispatch | Server ID, score, reason |
| `routing.rejected` | WARN | On admission reject | Model, constraint, error |
| `routing.weights_non_unit` | WARN | On config load | Sum of weights, suggestion to normalize |

---

## 8. Testing Strategy

### 8.1 Unit Test Boundaries

- Scoring formula: various slot/KV/deferred combinations
- Hard constraint filtering: model match, LoRA, multimodal, architecture
- Tie-breaking: round-robin distributes evenly
- Session affinity: hit, miss, invalidation on server failure
- `has_capacity()`: true/false with various tracker counts
- Recurrent model zeroing of KV/affinity components
- Stale-slot compensation (tracker.count > idle_slots → free_ratio = 0)

### 8.2 Specific Test Cases

```rust
#[test]
fn test_routing_prefers_server_with_more_free_slots() {
    // Server A: 3/4 slots free. Server B: 1/4 slots free.
    // Assert: A selected.
}

#[test]
fn test_dispatch_tracker_compensation() {
    // Server: 4 slots, /slots shows 3 idle. Tracker count = 2.
    // Effective free = max(0, 3 - 2) = 1. free_ratio = 1/4 = 0.25.
}

#[test]
fn test_session_affinity_bypassed_when_full() {
    // Affinity → Server A. Server A has free_ratio = 0.
    // Assert: scoring selects Server B instead.
}

#[test]
fn test_model_not_found_at_admission() {
    // Request for "nonexistent-model".
    // Assert: RoutingError::ModelNotFound.
}

#[test]
fn test_round_robin_on_tie() {
    // Two servers with identical scores.
    // Dispatch 4 requests → assert 2 to each server.
}
```

---

## 9. Open Questions

### Deferred to Implementation

1. **Session affinity key:** Use `requestor + session_id` composite, or just `session_id`? Decision: composite — prevents cross-requestor affinity collision.
2. **Routing weights normalization:** Warn on non-unit sum, or auto-normalize? Decision: warn only. Auto-normalization changes user-visible behavior silently.
3. **Recurrent model detection:** From config (`model_architecture` field) or from GGUF metadata? Decision: config field — explicit, no ambiguity.

---

## Cross-Cutting Concerns

### Cancellation Safety
All methods are synchronous (no `.await`). Cannot be cancelled mid-operation.

### Graceful Shutdown
No background tasks. No cleanup needed. State is ephemeral (session cache is advisory).

### Panic Safety
Pure computation — panics are logic bugs. No shared mutable state beyond the LRU cache (behind Mutex). Mutex poisoning is fatal for affinity but scoring still works.

### Memory Bounds
- Session affinity LRU: 10,000 entries × ~150 bytes = 1.5 MB max.
- Scoring computation: stack-only. No heap allocation per routing decision.
- Total: < 2 MB.
