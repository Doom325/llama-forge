# Component Design: Dispatch & Proxy

**Component:** Dispatch & Proxy  
**Phase:** 3 (Request Pipeline)  
**Estimated LOC:** ~2,500  
**Risk:** Critical  
**Dependencies:** Server Manager (server state, capabilities), Smart Routing (slot selection), Metrics & Storage (audit channel), Config Manager (timeout values), Request Queue (dequeued candidates)  
**Consumers:** Transport Layer (stream responses)

---

## 1. Purpose & Scope

Dispatch & Proxy is the highest-risk component in LlamaForge. It bridges the priority queue to llama-server backends: it holds the RAII `DispatchToken` for slot accounting, translates requests to llama-server's `/completion` endpoint format, proxies streaming responses back to clients, handles mid-stream errors, implements cooperative cancellation, and ensures GPU slots are released before audit writes. It owns the `slot_dispatch_map` for reverse reconciliation and the per-server dispatch tracker for stale-slot compensation.

**What this component explicitly does NOT do:**

- It does not decide priority ordering (that's Request Queue).
- It does not pick which server to use (that's Smart Routing — it receives a `RoutingDecision`).
- It does not manage server processes (that's Server Manager).
- It does not translate to/from OpenAI/Anthropic format (that's Transport Layer — it works with internal types).
- It does not persist audit records to SQLite (that's Metrics & Storage — it sends via channel).

---

## 2. Public API Surface

### 2.1 Dispatcher Interface

```rust
/// The dispatch engine. Spawns proxy tasks for dequeued requests.
pub struct Dispatcher {
    router: Arc<Router>,
    server_manager: Arc<ServerManager>,
    metrics_ingest: MetricsIngest,
    dispatch_trackers: Arc<DashMap<ServerId, Arc<DispatchTracker>>>,
    slot_dispatch_map: Arc<SlotDispatchMap>,
    config: Arc<ConfigHandle>,
    client_store: Arc<ClientStore>,
}

impl Dispatcher {
    /// Dispatch a dequeued request. Spawns a proxy task and returns immediately.
    /// The proxy task streams results back via the request's response channel.
    pub fn dispatch(
        &self,
        candidate: DispatchCandidate,
        routing: RoutingDecision,
    ) -> DispatchToken;

    /// Get the dispatch tracker for a server (for routing score computation).
    pub fn tracker(&self, server_id: &ServerId) -> Option<Arc<DispatchTracker>>;

    /// Get the slot dispatch map (for reconciliation by Server Manager).
    pub fn slot_dispatch_map(&self) -> Arc<SlotDispatchMap>;

    /// Notify that a server's client should be refreshed (on restart).
    pub fn refresh_client(&self, event: &ClientRefreshEvent);
}
```

### 2.2 DispatchToken (RAII Slot Accounting)

```rust
/// RAII guard for slot accounting. Inserting into the dispatch tracker on creation,
/// removing on drop. Ensures slots are always released, even on panic or cancellation.
#[must_use = "dropping the token immediately releases the slot"]
pub struct DispatchToken {
    tracker: Arc<DispatchTracker>,
    request_id: RequestId,
    server_id: ServerId,
    released: bool,
}

impl DispatchToken {
    pub fn new(
        tracker: Arc<DispatchTracker>,
        request_id: RequestId,
        server_id: ServerId,
    ) -> Self;

    /// Explicitly release the slot (before audit write).
    /// Subsequent drop is a no-op.
    pub fn release(&mut self);

    /// Get the server this token is for.
    pub fn server_id(&self) -> &ServerId;
}

impl Drop for DispatchToken {
    fn drop(&mut self) {
        if !self.released {
            self.tracker.remove(&self.request_id);
            // Also notify scheduler that a slot freed
        }
    }
}
```

### 2.3 DispatchTracker (v1: Simple Mutex)

```rust
/// Per-server tracking of in-flight dispatched requests.
/// v1: Arc<Mutex<HashSet>> — simple, correct, sufficient for 2-4 GPUs.
/// Scaling path: DashSet + AtomicU32 sidecar (documented in design doc).
pub struct DispatchTracker {
    inner: std::sync::Mutex<HashSet<RequestId>>,
}

impl DispatchTracker {
    /// O(1) amortized count of in-flight requests. Used by routing.
    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// Insert a request ID. Called by DispatchToken::new.
    pub fn insert(&self, id: RequestId) -> bool {
        self.inner.lock().unwrap().insert(id)
    }

    /// Remove a request ID. Called by DispatchToken::release/drop.
    pub fn remove(&self, id: &RequestId) -> bool {
        self.inner.lock().unwrap().remove(id)
    }

    /// Snapshot all IDs for reconciliation. Used by Server Manager.
    pub fn snapshot_ids(&self) -> Vec<RequestId> {
        self.inner.lock().unwrap().iter().cloned().collect()
    }
}
```

### 2.4 SlotDispatchMap (for Reconciliation)

```rust
/// Bidirectional map: (ServerId, SlotId) ↔ (RequestId, dispatched_at).
/// Used by reverse reconciliation to detect orphaned slots.
pub struct SlotDispatchMap {
    inner: std::sync::Mutex<SlotDispatchMapInner>,
}

struct SlotDispatchMapInner {
    /// Forward: request → (server, slot, dispatched_at)
    by_request: HashMap<RequestId, (ServerId, u32, Instant)>,
    /// Reverse: (server, slot) → request
    by_slot: HashMap<(ServerId, u32), RequestId>,
}

impl SlotDispatchMap {
    /// Record that request R was dispatched to server S, slot N.
    pub fn insert(&self, request_id: RequestId, server_id: ServerId, slot_id: u32);

    /// Remove entry (on completion, cancellation, or reconciliation erasure).
    pub fn remove(&self, request_id: &RequestId);

    /// Get the slot assigned to a request.
    pub fn get_slot_for_request(&self, request_id: &RequestId) -> Option<(ServerId, u32)>;

    /// Get the request assigned to a (server, slot) pair.
    pub fn get_request_for_slot(&self, server_id: &ServerId, slot_id: u32) -> Option<RequestId>;

    /// Get dispatch time for a request (used for grace period computation).
    pub fn get_dispatched_at(&self, request_id: &RequestId) -> Option<Instant>;

    /// Check if server has unresolved dispatches (dispatched but no slot assignment yet).
    pub fn has_unresolved(&self, server_id: &ServerId) -> bool;
}
```

### 2.5 ClientStore (Per-Server reqwest::Client)

```rust
/// Manages per-server HTTP clients with atomic swap on restart.
pub struct ClientStore {
    clients: DashMap<ServerId, arc_swap::ArcSwap<reqwest::Client>>,
}

impl ClientStore {
    /// Get the current client for a server (cheap ArcSwap load).
    pub fn get(&self, server_id: &ServerId) -> Option<arc_swap::Guard<Arc<reqwest::Client>>>;

    /// Swap the client for a server (on restart). Old client drains naturally.
    pub fn refresh(&self, server_id: &ServerId, new_client: reqwest::Client);

    /// Create a new client configured for a server's port/TLS.
    pub fn create_client(config: &ServerConfig) -> reqwest::Client;
}
```

### 2.6 Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("server {server_id} connection failed: {source}")]
    ConnectionFailed { server_id: ServerId, source: reqwest::Error },

    #[error("server {server_id} returned HTTP {status}: {body}")]
    BackendError { server_id: ServerId, status: u16, body: String },

    #[error("request cancelled during {phase}")]
    Cancelled { phase: String },

    #[error("dispatch timeout ({timeout:?}) exceeded")]
    Timeout { timeout: Duration },
}

/// Mid-stream error that can occur after streaming has started.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("backend disconnected mid-stream")]
    BackendDisconnect,

    #[error("generation timeout")]
    Timeout,

    #[error("request cancelled by client")]
    Cancelled,

    #[error("internal error: {0}")]
    Internal(anyhow::Error),
}
```

---

## 3. Internal Architecture

### 3.1 Proxy Task Lifecycle

```
    Dispatcher::dispatch() called
                │
    ┌───────────▼───────────────────────────┐
    │ 1. Create DispatchToken (tracker.insert) │
    │ 2. Build /completion request payload    │
    │ 3. tokio::spawn proxy_task              │
    └───────────────────────────────────────────┘

    ═══════ Inside proxy task (tokio::spawn) ═══════

    ┌───────────────────────────────────────────┐
    │ 4. POST /completion to llama-server       │
    │    (via ClientStore's ArcSwap client)     │
    └────────────┬──────────────────────────────┘
                 │
    ┌────────────▼──────────────────────────────┐
    │ 5. Stream response with select! {         │
    │      biased;                              │
    │      _ = cancel_token.cancelled() => {    │  ← AP-7: check BEFORE chunk read
    │        yield StreamError::Cancelled;       │
    │        break;                             │
    │      }                                    │
    │      chunk = response.chunk() => {        │
    │        parse chunk;                       │
    │        yield Result<Chunk, StreamError>;  │
    │      }                                    │
    │    }                                      │
    └────────────┬──────────────────────────────┘
                 │ stream complete/error
    ┌────────────▼──────────────────────────────┐
    │ 6. Release GPU slot IMMEDIATELY           │
    │    - drop(response_handle)  ← close TCP   │
    │    - dispatch_token.release() ← tracker   │
    │    - scheduler.notify_slot_available()     │
    └────────────┬──────────────────────────────┘
                 │
    ┌────────────▼──────────────────────────────┐
    │ 7. Audit write (detached tokio::spawn)    │
    │    - timeout(10s, audit_tx.send(record))  │
    │    - If timeout: drop record + counter    │
    │    - GPU slot already freed (step 6)      │
    └───────────────────────────────────────────┘
```

### 3.2 Reverse Reconciliation (Bidirectional)

```
    Every /slots poll (5 seconds) from Server Manager:
                │
    ┌───────────▼───────────────────────────────────────────────┐
    │ Forward reconciliation:                                    │
    │ For each slot with is_processing=true:                    │
    │   - Is it in slot_dispatch_map? → Known, skip.            │
    │   - Not in map? → First time seeing it processing.        │
    │     - Track in first_seen_processing with timestamp.      │
    │     - If in first_seen_processing > grace_period:         │
    │       grace = max(30s, server.ctx_size / 500)  ← AP-1     │
    │       → Orphaned slot. Erase via POST /slots/{id}?erase   │
    └───────────────────────────────────────────────────────────┘

    ┌───────────────────────────────────────────────────────────┐
    │ Reverse reconciliation:                                    │
    │ For each entry in slot_dispatch_map for this server:       │
    │   - Is the corresponding slot still is_processing?        │
    │   - If NOT processing AND dispatched_at > grace_period:   │
    │     grace = max(30s, server.ctx_size / 500)                │
    │     → Request completed without our knowledge.            │
    │     → Remove from slot_dispatch_map.                      │
    │                                                            │
    │ SKIP reverse erasure entirely if server has_unresolved()   │  ← AP-1
    │ (dispatched but not yet slot-assigned requests exist)      │
    └───────────────────────────────────────────────────────────┘

    ┌───────────────────────────────────────────────────────────┐
    │ Cleanup first_seen_processing:                             │  ← G-10
    │ Remove entries for slots that are:                         │
    │   - In slot_dispatch_map (now tracked properly), OR        │
    │   - No longer is_processing (completed naturally)          │
    └───────────────────────────────────────────────────────────┘
```

### 3.3 Client Backpressure Pattern

```rust
// Inside proxy task streaming loop:
loop {
    tokio::select! {
        biased;

        _ = cancel_token.cancelled() => {
            // Client disconnected or server draining
            yield Err(StreamError::Cancelled);
            break;
        }

        result = response_channel.send(chunk) => {
            match result {
                Ok(()) => continue,
                Err(_) => {
                    // Response channel full for 2 seconds → client too slow
                    // Stop reading from llama-server (TCP backpressure)
                    // Wait up to 2s for channel space
                    match tokio::time::timeout(
                        Duration::from_secs(2),
                        response_channel.reserve(),
                    ).await {
                        Ok(_) => continue,
                        Err(_) => {
                            // 2s backpressure timeout → disconnect
                            break;
                        }
                    }
                }
            }
        }

        chunk = backend_stream.chunk() => {
            match chunk {
                Some(Ok(data)) => { /* parse and yield */ }
                Some(Err(e)) => {
                    yield Err(StreamError::BackendDisconnect);
                    break;
                }
                None => break, // stream complete
            }
        }
    }
}
```

### 3.4 Key Invariants

1. **Slot release before audit:** GPU slot (DispatchToken) ALWAYS released before audit channel send. Audit stalls never hold hardware.
2. **DispatchToken never leaked:** RAII Drop ensures release even on panic, cancellation, or early return.
3. **Biased select for cancellation:** Cancellation token checked before chunk read. No token lost between read and yield.
4. **Connection pool fresh on restart:** `ClientStore` atomically swaps client. New proxy tasks use fresh client. Old in-flight tasks fail-fast on stale connections.
5. **Reconciliation grace period dynamic:** `max(30s, ctx_size / 500)` per server. Accounts for long prefills.
6. **slot_dispatch_map bidirectional consistency:** Forward and reverse maps always updated together under the same lock.
7. **first_seen_processing cleaned every cycle:** No memory leak from short-lived processing states.

---

## 4. Boundary Contracts

### 4.1 What This Component REQUIRES

| Dependency | Contract |
|-----------|----------|
| Smart Routing | `RoutingDecision` with valid server_id pointing to a Running server. |
| Server Manager | Slot states (via /slots polling) fed into reconciliation. `ClientRefreshEvent` on restart. |
| Metrics & Storage | `audit_sender()` available. Channel capacity ≥1000. Timeout behavior (10s) documented. |
| Config Manager | `queue_default_timeout_secs` and per-server drain timeouts available. |
| Request Queue | `notify_slot_available()` callable. Wakes scheduler on slot release. |

### 4.2 What This Component GUARANTEES to Consumers

| Consumer | Guarantee |
|----------|-----------|
| Transport Layer | Response stream yields `Result<Chunk, StreamError>`. Stream is `Send + 'static`. Can be consumed by axum/tonic handlers. |
| Transport Layer | On cancellation: structured error payload written before stream closes. Client always gets a reason. |
| Request Queue | `notify_slot_available()` called within microseconds of slot release. Never delayed by audit. |
| Smart Routing | `tracker.count()` accurately reflects in-flight dispatches (±1 during the microsecond between insert and /completion send). |
| Server Manager | `slot_dispatch_map` entries exist for all dispatched requests until completion or reconciliation cleanup. |

### 4.3 Failure Propagation

| Failure | Propagation |
|---------|-------------|
| Backend connection refused | `DispatchError::ConnectionFailed`. Token released. Request can be requeued or errored to client. |
| Backend HTTP 500 | `DispatchError::BackendError`. Token released. Error details forwarded. |
| Backend disconnect mid-stream | `StreamError::BackendDisconnect` yielded. Token released. Client sees partial response + error. |
| Client disconnect | `cancel_token` fires. Proxy stops reading from backend. Token released. Backend continues (no active cancellation of backend work — backend's continuous batcher handles slot recycling). |
| Audit channel stall | Record dropped after 10s timeout. Counter incremented. GPU slot unaffected (already released). |

---

## 5. Failure Modes

### FM-1: Proxy Task Panic

- **Scenario:** Logic bug in chunk parsing causes panic.
- **State preserved:** DispatchToken's `Drop` fires → tracker updated, slot freed. Client gets broken stream (connection reset).
- **Detection:** JoinSet or task monitoring detects panicked task.
- **Impact:** Single request fails. No cascading failure.

### FM-2: Backend Deadlock After Dispatch

- **Scenario:** Request dispatched, backend hangs, never responds.
- **Detection:** Non-streaming: absolute timeout (queue_timeout + max_generation_timeout). Streaming: client eventually disconnects (or keep-alive fails).
- **Recovery:** Token released on timeout/disconnect. Slot available again after Server Manager's next health check detects the deadlock.

### FM-3: Connection Pool Poisoning (Solved)

- **Scenario:** Backend restarts; old pooled connections are stale.
- **Detection:** First request to stale connection gets connection-reset.
- **Recovery:** `ClientStore` atomically swaps to fresh client on `ClientRefreshEvent`. New proxy tasks use fresh pool. Old tasks fail and their tokens are released.

### FM-4: Reconciliation Erases Active Slot

- **Scenario:** Long prefill (>30s) with dynamic grace not sufficient.
- **Mitigation:** Grace period is `max(30s, ctx_size/500)`. For 128K context: `max(30, 262) = 262s` grace. Additionally, reverse reconciliation skips entirely when `has_unresolved()` is true.

### FM-5: Backpressure Timeout Wastes GPU Slot

- **Scenario:** Client reads tokens very slowly. Channel fills. 2s timeout fires.
- **Impact:** Proxy disconnects from client. But: does NOT stop reading from backend (backend has already allocated the slot). Alternative: stop reading from backend (TCP backpressure → backend stops generating for this slot, freeing compute for other slots).
- **Decision:** Stop reading from backend. This applies TCP backpressure, which tells llama-server's continuous batcher to deprioritize this slot. After 2s with no consumer, disconnect entirely and release the token.

---

## 6. Configuration

| Knob | Location | Default | Valid Range | Hot-Reload |
|------|----------|---------|-------------|------------|
| Backpressure timeout | Hardcoded | 2 seconds | — | No |
| Max generation timeout (non-streaming) | `[global]` | 300 seconds | 10-3600 | Yes |
| Reconciliation grace base | Hardcoded | 30 seconds | — | No |
| Reconciliation grace formula | Hardcoded | `max(30, ctx_size/500)` | — | No |
| reqwest pool_idle_timeout | Hardcoded | 10 seconds | — | No |
| reqwest pool_max_idle_per_host | Computed | `max(parallel_slots, 16)` | — | No |
| SO_KEEPALIVE idle | Hardcoded | 10 seconds | — | No |
| SO_KEEPALIVE interval | Hardcoded | 5 seconds | — | No |
| SO_KEEPALIVE probes | Hardcoded | 3 | — | No |

---

## 7. Observability

### 7.1 Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `llamaforge_dispatches_total` | Counter | `server_id, result={success,error,timeout,cancelled}` | Dispatch outcomes |
| `llamaforge_dispatch_duration_seconds` | Histogram | `server_id` | Time from dispatch to completion |
| `llamaforge_ttft_seconds` | Histogram | `model, task_type_class` | Time to first token |
| `llamaforge_tokens_completion_total` | Counter | `model, server_id` | Generated tokens |
| `llamaforge_backpressure_disconnects_total` | Counter | `server_id` | Slow-client disconnects |
| `llamaforge_reconciliation_erasures_total` | Counter | `server_id, direction={forward,reverse}` | Orphan slot erasures |
| `llamaforge_pending_dispatches` | Gauge | `server_id` | In-flight dispatch count (tracker.count()) |
| `llamaforge_cancellations_dropped_total` | Counter | — | Cancellations lost due to forwarder stall |
| `llamaforge_tool_lookahead_delay_seconds` | Histogram | — | TTFT overhead from tool-call buffering |

### 7.2 Log Events

| Event | Level | Frequency | Content |
|-------|-------|-----------|---------|
| `dispatch.started` | DEBUG | Per-dispatch | Request ID, server, slot |
| `dispatch.completed` | DEBUG | Per-completion | Request ID, tokens, duration |
| `dispatch.failed` | WARN | On error | Request ID, error, server |
| `dispatch.cancelled` | DEBUG | On cancel | Request ID, phase, tokens_so_far |
| `dispatch.backpressure` | WARN | On slow client | Request ID, channel depth |
| `reconciliation.erasure` | INFO | On orphan detect | Server, slot, age, direction |
| `reconciliation.skipped` | DEBUG | When unresolved | Server, unresolved count |

---

## 8. Testing Strategy

### 8.1 Unit Test Boundaries

**Pure-logic tests:**
- DispatchToken RAII: drop releases, explicit release prevents double-release
- DispatchTracker: insert/remove/count correctness, snapshot
- SlotDispatchMap: bidirectional consistency, insert/remove/query
- Reconciliation grace period formula
- Request payload translation to /completion format

### 8.2 Integration Test Requirements

- **Mock HTTP server:** Simulate llama-server responses (streaming SSE chunks, errors, hangs).
- **Tokio runtime:** Multi-threaded for proxy task spawning, cancellation testing.
- **Real channels:** Test backpressure behavior with bounded channels.

### 8.3 Specific Test Cases

```rust
#[tokio::test]
async fn test_dispatch_token_released_on_drop() {
    let tracker = Arc::new(DispatchTracker::new());
    let token = DispatchToken::new(tracker.clone(), rid(), sid());
    assert_eq!(tracker.count(), 1);
    drop(token);
    assert_eq!(tracker.count(), 0);
}

#[tokio::test]
async fn test_dispatch_token_explicit_release_before_audit() {
    let tracker = Arc::new(DispatchTracker::new());
    let mut token = DispatchToken::new(tracker.clone(), rid(), sid());
    token.release();
    assert_eq!(tracker.count(), 0);
    drop(token); // no-op
    assert_eq!(tracker.count(), 0);
}

#[tokio::test]
async fn test_cancellation_checked_before_chunk() {
    // Cancel token before first chunk arrives.
    // Assert: StreamError::Cancelled yielded, no chunk consumed.
}

#[tokio::test]
async fn test_slot_released_before_audit_send() {
    // Mock audit channel that blocks.
    // Dispatch request → assert: slot freed within 1ms of stream end.
    // Audit send hangs → slot still freed.
}

#[tokio::test]
async fn test_client_refresh_on_server_restart() {
    // Dispatch request → backend "restarts" (mock).
    // ClientRefreshEvent fires → next dispatch uses new client.
}

#[tokio::test]
async fn test_reconciliation_grace_period() {
    // Slot appears processing in /slots but not in slot_dispatch_map.
    // Assert: not erased within grace period.
    // Assert: erased after grace period.
}

#[tokio::test]
async fn test_backpressure_timeout_disconnects() {
    // Consumer reads very slowly (1 chunk per 5 seconds).
    // Assert: proxy disconnects after 2s of full channel.
}

#[tokio::test]
async fn test_non_streaming_absolute_timeout() {
    // Backend never responds. Assert: timeout fires, error returned, token released.
}
```

---

## 9. Open Questions

### Deferred to Implementation

1. **Proxy task structure:** Single monolithic async fn, or decomposed into setup/stream/cleanup phases? Decision: decomposed — each phase has clear error handling and the cleanup phase can be shared between success/error/cancel paths.

2. **Zero-byte probe for non-streaming:** Before dispatching a non-streaming request that waited in queue, send a zero-byte TCP write to detect dead clients. Decision: yes, implement SO_KEEPALIVE config + zero-byte probe.

3. **Slot assignment detection:** After `/completion` POST, how do we know which slot was assigned (for slot_dispatch_map)? Decision: observe the next `/slots` poll — the slot with matching request properties (or newly processing) is correlated. For `id_slot != -1` requests (v1.1), we know the exact slot at dispatch time.

4. **Retry on connection-reset:** Should we retry once on `ConnectionFailed` (in case of pool poisoning lag)? Decision: no retry in v1. The ClientStore refresh eliminates the root cause. A single retry adds complexity and masks real failures.

5. **Cancellation forwarding to backend:** Should we `POST /slots/{id}?action=erase` when a client disconnects? Decision: no in v1 — llama-server's continuous batcher naturally reclaims idle slots. The erase is only for orphan reconciliation, not cooperative cancellation.

---

## Cross-Cutting Concerns

### Cancellation Safety

- **Dispatch initiation:** POST to backend. If cancelled between send and first response byte: reqwest aborts the request. Token released via Drop. Backend may or may not have started processing (it handles the TCP close internally).
- **Stream proxying:** `select! { biased; cancel => break, chunk => yield }`. If cancelled at any point: break, release token, yield error. No token consumed-but-not-yielded (biased ensures cancel wins over ready chunk).
- **Audit send:** If cancelled: detached task continues independently. Token already released. No harm.

### Graceful Shutdown

On SIGTERM (per-server CancellationToken):
1. Server's CancellationToken cancelled by Server Manager.
2. All proxy tasks for that server: `cancel_token.cancelled()` fires in biased select.
3. Each proxy task: writes structured `FINISH_REASON_UPSTREAM_FAILURE` error, flushes, drops.
4. DispatchTokens released. Slots freed.
5. Server Manager proceeds to SIGTERM after all proxy tasks complete (JoinSet with 5s timeout).

### Panic Safety

- **Proxy task panic:** DispatchToken::Drop fires (Tokio ensures destructors run on panicking tasks). Tracker updated. Client sees connection reset (no structured error, but slot is freed). Not ideal — consider `catch_unwind` in proxy task for structured error before propagating.
- **DispatchTracker mutex poison:** Catastrophic — routing breaks. Mitigation: critical section is trivial (HashSet insert/remove). Panic nearly impossible.

### Clock Assumptions

- **dispatched_at in SlotDispatchMap:** `Instant` (monotonic). Used for grace period comparison.
- **Reconciliation timers:** `Instant`-based. Not affected by clock skew.
- **Backpressure timeout:** `tokio::time::timeout` (monotonic).

### Memory Bounds

| Resource | Bound | Worst Case |
|----------|-------|------------|
| DispatchTracker per server | ≤ parallel_slots entries | 32 × 64B = 2KB |
| SlotDispatchMap | ≤ total_slots across all servers | 128 entries × 200B = 25KB |
| first_seen_processing | ≤ total_slots | 128 × 100B = 12KB |
| Proxy task stack | One per in-flight request | 32 × 8KB = 256KB |
| reqwest response buffers | One per in-flight stream | 32 × 64KB = 2MB |
| ClientStore | One Client per server | 8 × ~100KB = 800KB |

**Total worst case:** ~3.5 MB. Bounded by total slots across all servers.

### Backpressure

- **Backend → Proxy:** TCP flow control. If proxy stops reading (client slow), TCP window fills, backend stops sending.
- **Proxy → Client:** Bounded channel (or direct response body write). If client slow: 2s timeout → disconnect.
- **Dispatch → Audit:** Decoupled. Audit stall never blocks dispatch. 10s timeout + drop.
- **Reconciliation → Erasure:** POST /slots/{id}?erase is fire-and-forget with 5s timeout. Stall doesn't block next reconciliation cycle.
