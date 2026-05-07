# Component Design: Config Manager

**Component:** Config Manager  
**Phase:** 1 (Foundation)  
**Estimated LOC:** ~1,500  
**Risk:** Low  
**Dependencies:** Filesystem (TOML files, flock), Metrics & Storage (overlay store in v1.1)  
**Consumers:** Every other component (via ArcSwap read)

---

## 1. Purpose & Scope

The Config Manager owns the lifecycle of all configuration state in LlamaForge. It parses TOML files at startup, publishes an immutable config snapshot via `ArcSwap` for lock-free concurrent reads, handles SIGHUP-triggered hot reloads with full serialization guarantees, and provides a crash-safe writeback protocol for runtime mutations (LoRA adapter changes in v1.0).

**What this component explicitly does NOT do:**

- It does not manage server process lifecycles (that's Server Manager — it receives diffs and acts on them).
- It does not store runtime state (that's Metrics & Storage).
- It does not validate model file existence or GPU availability (that's Server Manager at startup).
- It does not serve the Web UI (that's Transport Layer — deferred to Phase 5).
- It does not implement the ConfigOverlayStore (that's Metrics & Storage in v1.1).

---

## 2. Public API Surface

### 2.1 Port Trait: ConfigSource

```rust
/// Port abstraction for config persistence — separates domain logic from I/O.
/// Implementations: `TomlFileConfigSource` (production), `InMemoryConfigSource` (tests).
/// This enables unit testing reload serialization and diff broadcasting without
/// a real filesystem, and allows v1.1 ConfigOverlayStore to be a new adapter
/// rather than a modification of the core struct.
pub trait ConfigSource: Send + Sync {
    /// Load the full config from the backing store.
    fn load(&self) -> Result<AppConfig, ConfigLoadError>;

    /// Persist a mutation to the backing store (crash-safe).
    fn persist(&self, config: &AppConfig, mutation: &ConfigMutation) -> Result<(), WritebackError>;

    /// Export the config as a serialized string (TOML for file-backed).
    fn export(&self, config: &AppConfig) -> Result<String, WritebackError>;
}
```

### 2.2 Consumer Traits: ConfigReader / ConfigWriter

```rust
/// Read-only config access. Implemented by ConfigHandle.
/// Consumers that only read config (Router, Queue, Dispatcher) depend on this
/// trait, not on the full ConfigHandle — interface segregation.
pub trait ConfigReader: Send + Sync {
    fn snapshot(&self) -> arc_swap::Guard<Arc<AppConfig>>;
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Arc<ConfigDiff>>;
}

/// Write access to config. Implemented by ConfigHandle.
/// Only Server Manager and Transport Layer (API endpoints) need this.
///
/// Uses native `async fn in trait` (Rust 1.75+). For `dyn ConfigWriter` usage,
/// methods return `impl Future + Send`.
pub trait ConfigWriter: Send + Sync {
    async fn reload(&self) -> Result<ConfigDiff, ConfigReloadError>;
    async fn writeback(&self, mutation: ConfigMutation) -> Result<ConfigDiff, WritebackError>;
}
```

### 2.3 Core Struct: ConfigHandle

```rust
/// The primary implementation of config management.
/// Delegates persistence I/O through `Arc<dyn ConfigSource>`, keeping domain
/// logic (ArcSwap, diff computation, broadcast, mutex serialization) here.
pub struct ConfigHandle {
    source: Arc<dyn ConfigSource>,
    inner: arc_swap::ArcSwap<AppConfig>,
    reload_mutex: tokio::sync::Mutex<()>,
    change_tx: tokio::sync::broadcast::Sender<Arc<ConfigDiff>>,
}

impl ConfigHandle {
    /// Load config via the provided source. Fails hard on parse error (startup only).
    pub fn load(source: Arc<dyn ConfigSource>) -> Result<Self, ConfigLoadError>;

    /// Get a consistent snapshot of the current config.
    /// Cheap: single atomic load (or cache hit with arc_swap::cache::Cache).
    pub fn snapshot(&self) -> arc_swap::Guard<Arc<AppConfig>>;

    /// Subscribe to config change notifications.
    /// Returns a broadcast receiver that yields diffs on each reload.
    /// Diffs are wrapped in Arc to avoid deep-cloning Vec<ServerConfigField> per subscriber.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Arc<ConfigDiff>>;

    /// Trigger a full reload from the config source. Serialized by internal mutex.
    /// Called by SIGHUP handler and POST /api/config/reload.
    pub async fn reload(&self) -> Result<ConfigDiff, ConfigReloadError>;

    /// Perform a crash-safe writeback via the config source.
    /// Used by LoRA adapter API in v1.0.
    pub async fn writeback(
        &self,
        mutation: ConfigMutation,
    ) -> Result<ConfigDiff, WritebackError>;

    /// Export config via the source's serialization (TOML for file-backed).
    /// Returns Result to avoid hidden unwrap on serialization failure.
    pub fn export_toml(&self) -> Result<String, WritebackError>;
}

impl ConfigReader for ConfigHandle { /* delegates to snapshot/subscribe */ }
impl ConfigWriter for ConfigHandle { /* delegates to reload/writeback */ }
```

> **Typestate lifecycle:** `ConfigHandle` has two distinct phases — startup (`load()`,
> fatal errors) and running (`reload()`/`writeback()`, non-fatal errors). To prevent
> calling `snapshot()` before `load()`, the constructor returns the handle only on
> successful load:
>
> ```rust
> // Startup phase — ConfigHandle is not yet constructed
> let handle = ConfigHandle::load(source)?;  // fails hard → exits binary
>
> // Running phase — snapshot(), reload(), writeback() are now safe
> let config = handle.snapshot();
> ```
>
> A full phantom-type typestate (`ConfigHandle<Unloaded>` → `ConfigHandle<Running>`)
> is overkill here since `load()` is the constructor — there is no `Unloaded` state
> that callers can hold. The invariant is enforced structurally: you can't have a
> `ConfigHandle` without having successfully loaded config.

### 2.4 Hot-Path Reader (for dispatch loop)

```rust
/// Thread-local cached reader for hot paths.
/// Amortizes ArcSwap atomic operations across multiple reads per task.
pub struct CachedConfigReader {
    cache: arc_swap::cache::Cache<Arc<ArcSwap<AppConfig>>, Arc<AppConfig>>,
}

impl CachedConfigReader {
    pub fn new(handle: &ConfigHandle) -> Self;

    /// Returns the current config, updating the thread-local cache
    /// only when a new version has been published.
    pub fn load(&mut self) -> &Arc<AppConfig>;
}
```

### 2.5 Observability Decorator

```rust
/// Instrumentation wrapper that enforces the domain/observability boundary.
/// All tracing spans, Prometheus metrics, and structured log events are emitted
/// HERE — the inner ConfigHandle (and ConfigSource) are telemetry-free.
///
/// This is the type injected into AppState. Consumers see it as `impl ConfigReader`
/// or `impl ConfigWriter` — the decorator is transparent.
pub struct InstrumentedConfigHandle {
    inner: ConfigHandle,
}

impl InstrumentedConfigHandle {
    pub fn new(inner: ConfigHandle) -> Self;
}

impl ConfigReader for InstrumentedConfigHandle {
    // Delegates to inner; no instrumentation (hot path, must stay cheap).
    // DO NOT add tracing spans or metrics to snapshot() or subscribe() —
    // these are called per-request in the dispatch loop. Any overhead here
    // multiplies by every request across every Tokio worker thread.
}

impl ConfigWriter for InstrumentedConfigHandle {
    // reload():
    //   span("config.reload", config.file_path, config.server_count)
    //     → config_reloads_total.inc()
    //     → inner.reload()
    //     → record duration to config_reload_duration_seconds histogram
    //     → emit config.reloaded structured event on success
    //     → emit config.reload_failed on error
    //
    // writeback():
    //   span("config.writeback", config.mutation_type)
    //     → config_writebacks_total.inc()
    //     → inner.writeback()
    //     → record duration to config_writeback_duration_seconds histogram
    //     → emit config.writeback structured event on success
    //     → emit config.writeback_failed on error
}
```

> **Design rationale:** By confining all telemetry to this decorator, the core
> `ConfigHandle` and `ConfigSource` trait remain pure domain/infrastructure with
> no `tracing` or `prometheus` dependencies. This means unit tests of reload
> serialization and diff logic never touch metrics, and the instrumentation can
> be swapped or extended without modifying core logic.

### 2.6 Config Types

> **Note on serde annotations:** `AppConfig` and its children carry `#[derive(Serialize, Deserialize)]`
> and `#[serde(default)]` because Config Manager is fundamentally an infrastructure component — these
> types exist to be parsed from TOML. If `AppConfig` is shared as a dependency to domain-layer
> components (Router, Queue), a separate `RawAppConfig` DTO with `TryFrom` conversion should be
> introduced to avoid leaking serde into the domain. For v1.0, the current approach is acceptable
> since all consumers live within the same binary.

```rust
/// Top-level config struct. Parsed from llamaforge.toml + priority_rules.toml.
/// All fields have #[serde(default)] for backward compatibility.
/// `Default` is derived for all config structs — enables programmatic construction
/// in tests without verbose struct literals. For complex test scenarios, use
/// `typed-builder` (added as a dev-dependency).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    pub global: GlobalConfig,
    pub grpc: GrpcConfig,
    pub prometheus: PrometheusConfig,
    /// Web UI config — parsed even before Phase 5 implementation to prevent
    /// unknown-key warnings when operators include [web_ui] in their TOML.
    #[serde(default)]
    pub web_ui: WebUiConfig,
    pub servers: HashMap<ServerId, ServerConfig>,
    pub priority_rules: PriorityRulesConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    #[serde(default = "default_bind_address")]
    pub bind_address: IpAddr,
    #[serde(default = "default_grpc_port")]
    pub grpc_port: u16,
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_log_level")]
    pub log_level: LogLevel,
    #[serde(default = "default_log_format")]
    pub log_format: LogFormat,
    #[serde(default = "default_metrics_db_path")]
    pub metrics_db_path: PathBuf,
    #[serde(default = "default_retention_days")]
    pub metrics_retention_days: u32,
    #[serde(default = "default_queue_max_depth")]
    pub queue_max_depth: u32,
    /// Integer seconds matching the TOML format (`queue_default_timeout_secs = 300`).
    /// Converted to `Duration` at usage sites via `Duration::from_secs()`.
    /// Using `u64` instead of `Duration` avoids serde roundtrip failures —
    /// `Duration`'s default serde is `{secs: N, nanos: 0}` (a TOML table, not an integer).
    #[serde(default = "default_queue_timeout_secs")]
    pub queue_default_timeout_secs: u64,
    #[serde(default = "default_restart_stagger_secs")]
    pub restart_stagger_secs: u64,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ServerConfig {
    pub model_path: PathBuf,
    #[serde(default)]
    pub alias: Option<String>,
    pub port: u16,
    #[serde(default = "default_backend")]
    pub backend: Backend,
    #[serde(default)]
    pub devices: Vec<DeviceSpec>,
    #[serde(default = "default_gpu_layers")]
    pub gpu_layers: i32,
    #[serde(default = "default_parallel_slots")]
    pub parallel_slots: u32,
    #[serde(default = "default_ctx_size")]
    pub ctx_size: u32,
    #[serde(default)]
    pub cache_type_k: CacheType,
    #[serde(default)]
    pub cache_type_v: CacheType,
    #[serde(default)]
    pub flash_attn: FlashAttnMode,
    #[serde(default)]
    pub lora_adapters: Vec<LoraAdapterConfig>,
    #[serde(default)]
    pub estimated_vram_mb: Option<u64>,
    #[serde(default)]
    pub speculative_method: SpeculativeMethod,
    /// Model architecture — Smart Routing zeroes kv_cache_usage for recurrent models.
    /// Defaults to Transformer. Requires restart on change.
    #[serde(default)]
    pub model_architecture: ModelArchitecture,
}

/// Newtype wrappers for domain identities — prevents accidental transposition
/// of task type IDs, class names, and requestor IDs at compile time.
/// `#[serde(transparent)]` preserves bare-string TOML compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskTypeId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskClassName(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestorId(pub String);

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PriorityRulesConfig {
    pub task_types: HashMap<TaskTypeId, u32>,
    #[serde(default)]
    pub custom_task_types: HashMap<TaskTypeId, u32>,
    #[serde(default)]
    pub task_type_classes: HashMap<TaskTypeId, TaskClassName>,
    pub requestor_bonuses: HashMap<RequestorId, i32>,
    pub starvation: StarvationConfig,
    pub fair_share: FairShareConfig,
    #[serde(default)]
    pub preemption: PreemptionConfig,
}

/// Mutations that can be applied via the writeback API.
/// `#[non_exhaustive]` allows adding variants in future versions without breaking
/// downstream match arms. v1.0 supports only LoRA adapter changes.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ConfigMutation {
    /// Replace the LoRA adapter list for a specific server.
    SetLoraAdapters {
        server_id: ServerId,
        adapters: Vec<LoraAdapterConfig>,
    },
    // Future v1.1+: SetOverlay, RemoveOverlay, SetServerAlias, etc.
}

/// Model architecture classification — determines routing strategy.
/// Smart Routing uses this to zero out kv_cache_usage for recurrent models
/// (Mamba, RWKV) since they don't have a traditional KV cache.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelArchitecture {
    #[default]
    Transformer,
    Recurrent,
}

```

**Shared types reference** — the following types are used in config structs but defined in their respective component designs or as standard enums:

| Type | Defined In | Description |
|------|-----------|-------------|
| `ServerId` | Section 2.6 | Newtype wrapper: `ServerId(String)`, `#[serde(transparent)]` |
| `GrpcConfig` | Transport Layer (Phase 5) | gRPC bind address, TLS settings |
| `PrometheusConfig` | Metrics & Storage | Scrape port, path, push gateway |
| `WebUiConfig` | Transport Layer (Phase 5) | Bind address, static asset path |
| `LogLevel` | Enum: `{trace, debug, info, warn, error}` | Maps to `tracing::Level` |
| `LogFormat` | Enum: `{json, pretty, compact}` | Maps to `tracing_subscriber::fmt::format` |
| `Backend` | Enum: `{llama_server, custom}` | Which inference backend binary to spawn |
| `DeviceSpec` | Struct: `{gpu_id: GpuId, layer_range: Option<Range<u32>>}` | GPU assignment |
| `CacheType` | Enum: `{f16, q8_0, q4_0}` | KV cache quantization type |
| `FlashAttnMode` | Enum: `{auto, enabled, disabled}` | Flash attention configuration |
| `LoraAdapterConfig` | Struct: `{path: PathBuf, scale: f32}` | LoRA adapter file + scale |
| `SpeculativeMethod` | Enum: `{none, draft_model{..}, lookup{..}}` | Speculative decoding config |
| `StarvationConfig` | Struct: `{threshold_secs: u64, boost_per_interval: u32, ..}` | Boost params |
| `FairShareConfig` | Struct: `{weights: HashMap<RequestorId, u32>, ..}` | DRR fair share |
| `PreemptionConfig` | Struct: `{enabled: bool, min_tokens_before_preempt: u32}` | v2 preemption |

### 2.7 Error Types

```rust
/// Error sources use `Box<dyn Error + Send + Sync>` instead of concrete infrastructure
/// types (toml::de::Error, toml::ser::Error) so consumers don't take a transitive
/// dependency on the serialization crate. The Display impl provides the full message.
#[derive(Debug, thiserror::Error)]
pub enum ConfigLoadError {
    #[error("config file not found: {path}")]
    NotFound { path: PathBuf },

    #[error("config file is empty (possible crash during writeback): {path}")]
    EmptyFile { path: PathBuf },

    #[error("parse error in {path}: {source}")]
    ParseError { path: PathBuf, source: Box<dyn std::error::Error + Send + Sync> },

    #[error("validation failed: {message}")]
    ValidationError { message: String },

    #[error("I/O error reading {path}: {source}")]
    IoError { path: PathBuf, source: std::io::Error },
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigReloadError {
    #[error("reload failed: {source}")]
    LoadError(#[from] ConfigLoadError),

    #[error("diff computation failed: {message}")]
    DiffError { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum WritebackError {
    #[error("config file is read-only (Kubernetes ConfigMap?): {path}")]
    ReadOnly { path: PathBuf },

    #[error("flock acquisition timed out after {elapsed:?}")]
    LockTimeout { elapsed: Duration },

    #[error("atomic rename failed (EXDEV), fallback also failed: {source}")]
    WriteFailed { source: std::io::Error },

    #[error("serialization failed: {source}")]
    SerializeError { source: Box<dyn std::error::Error + Send + Sync> },
}
```

### 2.8 Diff Type

```rust
/// Describes what changed between two config snapshots.
/// Consumed by Server Manager to determine lifecycle actions.
///
/// Sent through the broadcast channel wrapped in `Arc<ConfigDiff>` to avoid
/// deep-cloning Vec<ServerConfigField> per subscriber. Reloads are rare (cold path),
/// but Arc makes the cost zero regardless.
#[derive(Debug, Clone)]
pub struct ConfigDiff {
    pub changes: Vec<ConfigChange>,
    /// Monotonic timestamp for ordering diffs. Not used for correctness.
    pub timestamp: std::time::Instant,
    /// Wall-clock timestamp for logging and debugging (displayable, serializable).
    pub wall_clock: std::time::SystemTime,
}

/// Individual config change — extensible as config grows in v1.1+.
/// Replaces the flat struct with mixed leaf/composite fields.
/// `#[non_exhaustive]` allows adding variants (OverlayChanged, WebUiChanged, etc.)
/// without breaking downstream match arms.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ConfigChange {
    GlobalChanged,
    PriorityRulesChanged,
    ServerAdded { id: ServerId },
    ServerRemoved { id: ServerId },
    ServerChanged { id: ServerId, diff: ServerConfigDiff },
}

impl ConfigDiff {
    pub fn servers_added(&self) -> impl Iterator<Item = &ServerId> {
        self.changes.iter().filter_map(|c| match c {
            ConfigChange::ServerAdded { id } => Some(id),
            _ => None,
        })
    }

    pub fn servers_removed(&self) -> impl Iterator<Item = &ServerId> {
        self.changes.iter().filter_map(|c| match c {
            ConfigChange::ServerRemoved { id } => Some(id),
            _ => None,
        })
    }

    pub fn servers_changed(&self) -> impl Iterator<Item = (&ServerId, &ServerConfigDiff)> {
        self.changes.iter().filter_map(|c| match c {
            ConfigChange::ServerChanged { id, diff } => Some((id, diff)),
            _ => None,
        })
    }
}

/// Enum of all server config fields — eliminates stringly-typed field matching.
/// `requires_restart()` and `hot_reloadable()` are derived from the variant,
/// so they can never drift out of sync with the actual field list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ServerConfigField {
    ModelPath,
    Alias,
    Port,
    Backend,
    Devices,
    GpuLayers,
    ParallelSlots,
    CtxSize,
    CacheTypeK,
    CacheTypeV,
    FlashAttn,
    LoraAdapters,
    EstimatedVramMb,
    SpeculativeMethod,
    ModelArchitecture,
}

impl ServerConfigField {
    /// Fields that require a full server restart to take effect.
    pub fn requires_restart(&self) -> bool {
        matches!(self,
            Self::ModelPath | Self::CtxSize | Self::ParallelSlots
            | Self::Devices | Self::Backend | Self::GpuLayers
            | Self::CacheTypeK | Self::CacheTypeV | Self::FlashAttn
            | Self::Port | Self::SpeculativeMethod | Self::ModelArchitecture
        )
    }

    /// Fields that can be applied without restarting the server.
    pub fn hot_reloadable(&self) -> bool {
        matches!(self, Self::LoraAdapters | Self::Alias | Self::EstimatedVramMb)
    }
}

#[derive(Debug, Clone)]
pub struct ServerConfigDiff {
    pub fields_changed: Vec<ServerConfigField>,
}

impl ServerConfigDiff {
    /// True if any changed field requires a server restart.
    pub fn requires_restart(&self) -> bool {
        self.fields_changed.iter().any(|f| f.requires_restart())
    }

    /// True if ALL changed fields are hot-reloadable (no restart needed).
    pub fn hot_reloadable(&self) -> bool {
        self.fields_changed.iter().all(|f| f.hot_reloadable())
    }
}
```

---

## 3. Internal Architecture

### 3.1 Data Flow

```
                    SIGHUP / POST /api/config/reload
                                │
                                ▼
                    ┌───────────────────────┐
                    │    reload_mutex.lock() │  ← serializes all reloads
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │   spawn_blocking {    │
                    │     flock(EXCLUSIVE)  │  ← blocks in threadpool, not reactor
                    │     read TOML files   │
                    │     parse + validate  │
                    │     release flock     │
                    │   }                   │
                    └───────────┬───────────┘
                                │ Result<AppConfig>
                    ┌───────────▼───────────┐
                    │   diff(old, new)      │
                    │   ArcSwap::store(new) │  ← atomic pointer swap
                    │   broadcast(diff)     │  ← notify subscribers
                    └───────────────────────┘
                                │
                    Subscribers (Server Manager, Queue, Router)
                    receive ConfigDiff and act on it
```

### 3.2 Writeback Data Flow

```
    POST /api/servers/{id}/lora-adapters
                    │
                    ▼
    ┌──────────────────────────────────┐
    │ 0. reload_mutex.lock().await     │  ← serializes with reloads
    └──────────────┬───────────────────┘
                   │
    ┌──────────────▼───────────────────┐
    │ 1. Apply to llama-server (HTTP)  │  ← runtime change first
    └──────────────┬───────────────────┘
                   │ success
    ┌──────────────▼───────────────────┐
    │ 2. spawn_blocking {              │
    │      flock(EXCLUSIVE, LOCK_NB    │  ← retry loop, 30s timeout
    │        + retry loop)             │
    │      re-read TOML from disk      │  ← pick up concurrent edits
    │      apply_mutation(parsed)      │
    │      Phase 1: write tmp + rename │  ← crash-safe
    │      Phase 2: fallback truncate  │  ← if EXDEV/EACCES
    │      release flock               │
    │    }                             │
    └──────────────┬───────────────────┘
                   │ Result<ConfigDiff>
    ┌──────────────▼───────────────────┐
    │ 3. Update in-memory via ArcSwap  │
    │ 4. Broadcast Arc<ConfigDiff>     │
    │ 5. Drop mutex guard              │
    └──────────────────────────────────┘
```

### 3.3 Key Data Structures

```rust
/// Paths to all config files. Resolved once at startup.
pub struct ConfigPaths {
    pub main_config: PathBuf,         // llamaforge.toml
    pub priority_rules: PathBuf,      // priority_rules.toml
    pub state_dir: PathBuf,           // /var/run/llamaforge/ or $LLAMAFORGE_STATE_DIR
    pub lock_file: PathBuf,           // state_dir/config.lock
}

// Note: There is no separate `ConfigManagerInner` struct — `ConfigHandle` (Section 2.3)
// holds all state directly. For sharing across tasks, use `Arc<ConfigHandle>`.
// The fields are: source, inner (ArcSwap), reload_mutex, change_tx.
```

### 3.4 Invariants

1. **Single-writer:** Only one reload or writeback executes at a time (enforced by `reload_mutex`).
2. **Atomic visibility:** Readers see either the complete old config or the complete new config — never a mix (enforced by `ArcSwap::store`).
3. **TOML authoritative:** On reload, TOML always wins over in-memory state. Writeback conflicts are logged but TOML is king.
4. **Backward compatible:** Any valid v1.0 TOML parses successfully in all future versions (`#[serde(default)]` on all fields).
5. **Crash safe:** A power-loss at any point during writeback leaves either the old valid TOML or the new valid TOML on disk — never a corrupt/empty file (atomic rename path). The truncation fallback has a microsecond crash window with a rollback snapshot.
6. **No blocking on reactor:** All filesystem I/O (`flock`, `read`, `write`, `fsync`, `rename`) runs inside `spawn_blocking`.

---

## 4. Boundary Contracts

### 4.1 What This Component REQUIRES

| Dependency | Contract |
|-----------|----------|
| Filesystem | TOML files readable at startup. State dir writable (for lock file). Local filesystem for flock correctness (NOT NFS/EFS). |
| Tokio runtime | `spawn_blocking` available. Blocking thread pool has capacity (auto-grows). |
| Metrics & Storage (v1.1) | `ConfigOverlayStore` trait impl for overlay reads. Not required in v1.0. |

### 4.2 What This Component GUARANTEES to Consumers

| Consumer | Guarantee |
|----------|-----------|
| All components | `snapshot()` always returns a valid, fully-populated `AppConfig`. Never panics, never blocks. |
| All components | Config changes are visible to all readers within microseconds of `ArcSwap::store` (no propagation delay). |
| Server Manager | `ConfigDiff` accurately identifies which servers need restart vs. hot-reload. `requires_restart` is conservative (true if uncertain). |
| Server Manager | Reload diffs arrive in order — never two concurrent diffs for the same reload (serialized by mutex). |
| Transport Layer | `reload()` returns the `ConfigDiff`. The Transport Layer handler is responsible for awaiting Server Manager lifecycle actions (triggered by the diff via broadcast) before returning the HTTP response to the client. |
| Audit | Every reload/writeback emits a `config.reloaded` or `config.writeback` event on the broadcast channel. |

### 4.3 Failure Propagation

| Failure | Propagation |
|---------|-------------|
| TOML parse error at startup | `ConfigLoadError` → binary exits with non-zero status. Operator must fix config. |
| Empty/corrupt file at startup | `ConfigLoadError::EmptyFile` → binary exits. Operator restores from VCS. |
| TOML parse error during reload | `ConfigReloadError` → logged as error, in-memory config unchanged, broadcast not sent. API returns 500. |
| flock timeout during writeback | `WritebackError::LockTimeout` → runtime change already applied to llama-server but NOT persisted. Warning logged. State diverges until next restart. |
| Read-only filesystem | `WritebackError::ReadOnly` → graceful degradation. Runtime change applied, not persisted. Warning + UI banner (when UI exists). |
| Broadcast channel full | Lagging subscriber loses intermediate diffs. Not critical — subscribers should re-read full snapshot on reconnect. |

---

## 5. Failure Modes

### FM-1: Crash During Writeback (Atomic Rename Path)

- **Scenario:** OOM kill or power loss between `write(tmp)` and `rename(tmp, config)`.
- **State preserved:** Original TOML file is untouched (rename is atomic). Temp file orphaned in state dir.
- **Recovery:** On next startup, detect orphaned `config.tmp` in state dir → log warning and delete.

### FM-2: Crash During Writeback (Truncation Fallback Path)

- **Scenario:** Kill between `O_TRUNC` open and `write_all` completion.
- **State lost:** Config file may be empty (0 bytes) or partially written.
- **Recovery:** Startup safety check detects empty/unparseable file → exits with descriptive error directing operator to restore from VCS/backup. The rollback snapshot is lost (it was in-memory only).

### FM-3: Concurrent SIGHUP + API Writeback

- **Scenario:** Operator sends SIGHUP while LoRA API writeback holds flock.
- **Behavior:** Reload mutex serializes them. SIGHUP reload waits for writeback to complete. Reload then reads the post-writeback TOML (which includes the LoRA change). No conflict.

### FM-4: TOML Edited Externally During Writeback

- **Scenario:** Operator edits TOML in vi while API writeback is in progress.
- **Behavior:** Writeback re-reads TOML inside flock (serialized). If operator saved before flock acquired, writeback sees the operator's changes and merges its mutation on top. If operator saves after flock acquired, operator's changes are lost until next reload.
- **Mitigation:** This is documented behavior. Lock file prevents simultaneous writes from LlamaForge instances; operator concurrent edits are accepted as operator error.

### FM-5: Broadcast Channel Overflow

- **Scenario:** Subscriber (e.g., Server Manager) is slow to process diffs; channel buffer fills.
- **Behavior:** `broadcast::error::RecvError::Lagged(n)` — subscriber misses n diffs.
- **Recovery:** Subscriber must re-read full config snapshot via `handle.snapshot()` and compute its own diff against its cached state. Logged as warning.

### FM-6: State Directory Not Writable

- **Scenario:** Startup on a system where `/var/run/llamaforge/` doesn't exist and can't be created.
- **Behavior:** Lock file `open(O_CREAT)` fails → `WritebackError`. Reload still works (reload mutex provides process-local serialization; flock only needed for inter-process coordination).
- **Mitigation:** Startup validates state dir writability. If not writable, warn that writeback is disabled but continue running.

---

## 6. Configuration

All config knobs for the Config Manager itself:

| Knob | Location | Default | Valid Range | Hot-Reload |
|------|----------|---------|-------------|------------|
| `state_dir` | `llamaforge.toml` [global] | `/var/run/llamaforge/` | Any writable local path | No (startup only) |
| Config file paths | CLI args or env | `/etc/llamaforge/llamaforge.toml` | Any readable path | No |
| Broadcast channel capacity | Hardcoded | 16 | - | No |
| flock timeout | Hardcoded | 30 seconds | - | No |
| Writeback enabled | Auto-detected | true if state_dir writable | - | No |

**What changes without restart:**

- All values inside `llamaforge.toml` and `priority_rules.toml` are reloadable via SIGHUP (the reload re-reads everything). However, `state_dir`, `bind_address`, `grpc_port`, and `http_port` require a full process restart (listeners are bound at startup).

**Path resolution precedence:** Config paths are resolved at startup: CLI arg > environment variable > default. Supported env vars:

| Env Var | Purpose | Default |
|---------|---------|---------|
| `LLAMAFORGE_CONFIG` | Main config file path | `/etc/llamaforge/llamaforge.toml` |
| `LLAMAFORGE_PRIORITY_RULES` | Priority rules file path | Same directory as main config |
| `LLAMAFORGE_STATE_DIR` | State directory (lock file, temp files) | `/var/run/llamaforge/` |

`ConfigPaths` is constructed in `main()` before `ConfigHandle::load()`.

### 6.1 Validation Rules

Validation runs both at startup (fatal) and during reload (non-fatal — logs error, keeps old config). The `ConfigLoadError::ValidationError` variant is used for all validation failures.

| Rule | Behavior | Severity |
|------|----------|----------|
| `fair_share` weights sum to 100 | Normalize proportionally + WARN log | Warning (auto-corrected) |
| Server ports unique across all servers | ERROR — reject config | Error |
| `model_path` non-empty for every server | ERROR — reject config | Error |
| `ctx_size > 0` for every server | ERROR — reject config | Error |
| `parallel_slots > 0` for every server | ERROR — reject config | Error |
| `device` specs reference valid GPU IDs | ERROR — reject config | Error |
| `queue_default_timeout_secs > 0` | ERROR — reject config | Error |
| No duplicate server IDs | ERROR — reject config | Error |

> **Normalization vs. error:** The `fair_share` rule is the only soft validation — it auto-corrects
> and warns. All other rules are hard errors. At startup, any error is fatal. During reload,
> errors are logged and the in-memory config is unchanged (the old valid config continues to serve).

---

## 7. Observability

> **Boundary enforcement:** All metrics, spans, and structured log events defined below
> are emitted exclusively by the `InstrumentedConfigHandle` decorator (Section 2.5).
> The inner `ConfigHandle` and `ConfigSource` implementations contain NO tracing or
> Prometheus dependencies. This keeps domain/infrastructure logic testable in isolation
> and allows telemetry to be extended or replaced without modifying core logic.

### 7.1 Tracing Spans

All multi-stage operations are wrapped in parent spans with child spans per stage:

| Parent Span | Child Spans | Structured Fields |
|------------|-------------|-------------------|
| `config.reload` | `config.reload.flock`, `config.reload.parse`, `config.reload.diff`, `config.reload.store` | `config.file_path: String`, `config.server_count: u32`, `config.parse_duration_ms: u64`, `config.diff.servers_added: u32`, `config.diff.servers_removed: u32`, `config.diff.servers_changed: u32` |
| `config.writeback` | `config.writeback.apply`, `config.writeback.flock`, `config.writeback.persist`, `config.writeback.store` | `config.mutation_type: String`, `config.server_id: String`, `config.write_strategy: {atomic_rename, truncate_fallback}`, `config.writeback_duration_ms: u64` |

> **Trace context propagation:** Transport Layer middleware extracts `traceparent` from
> inbound HTTP/gRPC headers and creates a parent span before calling Config Manager.
> API-triggered reloads and writebacks inherit this span context automatically.
> SIGHUP-triggered reloads create root spans (no inbound request context).
> Outbound HTTP calls to llama-server during writeback inject `traceparent` for
> end-to-end correlation.

### 7.2 Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `llamaforge_config_reloads_total` | Counter | `result={success,error}` | Total reload attempts |
| `llamaforge_config_reload_duration_seconds` | Histogram | — | Time from SIGHUP to ArcSwap store |
| `llamaforge_config_writebacks_total` | Counter | `result={success,error,readonly}` | Writeback attempts |
| `llamaforge_config_writeback_duration_seconds` | Histogram | — | Time in spawn_blocking for writeback |
| `llamaforge_config_flock_wait_seconds` | Histogram | `operation={reload,writeback}` | Time spent waiting for flock |
| `llamaforge_config_broadcast_lagged_total` | Counter | `subscriber={server_manager,queue,router,transport}` | Subscribers that fell behind |

**Label cardinality:** All labels are bounded enums. The `subscriber` label on
`broadcast_lagged_total` is constrained to the fixed set `{server_manager, queue, router, transport}` —
dynamic subscriber identifiers are not permitted.

### 7.3 Log Events

| Event | Level | Frequency | Structured Fields |
|-------|-------|-----------|-------------------|
| `config.loaded` | INFO | Once at startup | `config.file_path: String`, `config.server_count: u32`, `config.parse_duration_ms: u64` |
| `config.reloaded` | INFO | On each SIGHUP | `config.diff.servers_added: u32`, `config.diff.servers_removed: u32`, `config.diff.servers_changed: u32`, `config.reload_duration_ms: u64` |
| `config.reload_failed` | ERROR | On parse failure | `config.file_path: String`, `config.error: String`, `config.line_number: Option<u32>` |
| `config.reload_conflict` | WARN | On writeback field revert | `config.server_id: String`, `config.fields_reverted: Vec<String>` |
| `config.writeback` | INFO | On LoRA API mutation | `config.server_id: String`, `config.field: String`, `config.write_strategy: String`, `config.writeback_duration_ms: u64` |
| `config.writeback_failed` | WARN | On read-only FS | `config.file_path: String`, `config.error: String`, `config.divergence: bool` |
| `config.flock_slow` | WARN | When flock > 1s | `config.flock_duration_ms: u64`, `config.operation: String` |
| `config.empty_file_detected` | ERROR | At startup | `config.file_path: String` |

### 7.4 Tracing Subscriber Stack

Config Manager does not configure the subscriber — that is the binary entrypoint's
responsibility. The expected composition (documented here for cross-reference):

```rust
// main.rs — tracing subscriber setup
tracing_subscriber::registry()
    .with(
        tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_writer(std::io::stderr)    // Layer 1: fmt to stderr
    )
    .with(
        tracing_opentelemetry::layer()
            .with_exporter(/* OTLP exporter */)  // Layer 2: OTLP to collector
    )
    .with(
        EnvFilter::from_default_env()         // RUST_LOG filtering
    )
    .init();
```

This is specified in the system-level telemetry design. Config Manager's `GlobalConfig`
provides `log_level` and `log_format` which the entrypoint uses to configure the fmt layer.

### 7.5 Health Signals

- **Ready:** Config loaded successfully at startup (publishes to readiness probe).
- **Live:** Always alive once loaded (config manager cannot crash independently).
- **Degraded:** Writeback disabled (state_dir not writable) — reported in `/health` response.

### 7.6 Alerting Guidance

| Condition | Threshold | Severity | Action |
|-----------|-----------|----------|--------|
| Reload latency | p99 > 5s | Warning | Investigate filesystem or flock contention |
| Reload errors | `rate(config_reloads_total{result="error"}) > 0` | Critical | TOML parse error — operator must fix config |
| Writeback errors | `rate(config_writebacks_total{result="error"}) > 0` | Warning | Filesystem issue — check disk/permissions |
| flock contention | p99 `flock_wait_seconds` > 10s | Warning | External process holding flock (concurrent editor?) |
| Broadcast lag | `rate(broadcast_lagged_total) > 0` | Warning | Subscriber processing too slow — may miss diffs |

---

## 8. Testing Strategy

### 8.1 Unit Test Boundaries

**Mockable:**
- Config persistence → `InMemoryConfigSource` impl of `ConfigSource` trait (no filesystem needed for unit tests of reload serialization, diff computation, or broadcast behavior)
- Filesystem I/O → `TomlFileConfigSource` integration tests use `tempdir` + real files
- Clock → not used (no time-dependent logic in config manager itself)

**Pure-logic tests (no I/O):**
- TOML parsing: valid configs, missing fields (default behavior), unknown fields (ignored), invalid values
- Diff computation: server added, removed, changed (restart vs. hot-reload classification)
- Merge semantics: writeback field merge-not-replace logic
- Validation: fair_share percentages sum to 100, port conflicts, device spec validation

### 8.2 Integration Test Requirements

- **Real filesystem:** Atomic rename, truncation fallback (simulated EXDEV), flock contention between threads
- **Tokio runtime:** Multi-threaded runtime for testing reload_mutex serialization under concurrent SIGHUPs
- **No external processes:** Config Manager is fully testable without llama-server

### 8.3 Property-Based Tests (proptest)

- **Roundtrip:** `∀ config: parse(serialize(config)) == config`
- **Backward compat:** `∀ config, ∀ subset of fields removed: parse(serialize(subset)) succeeds with defaults` — Implementation approach: generate full TOML via `toml::to_string`, then randomly delete keys from the TOML string before re-parsing. This tests `#[serde(default)]` coverage without needing to enumerate struct fields at the type level.
- **Diff correctness:** `∀ (old, new): apply(old, diff(old, new)) == new`
- **Serialization:** Concurrent reload + snapshot never panics (loom or shuttle for concurrency testing)

### 8.4 Specific Test Cases

```rust
#[tokio::test]
async fn test_concurrent_sighups_serialized() {
    // Two rapid reloads with a 10ms config change between them.
    // Assert: second reload sees the changed config, not stale.
}

#[tokio::test]
async fn test_writeback_atomic_rename() {
    // Write, verify content, simulate crash (don't rename), verify original intact.
}

#[tokio::test]
async fn test_writeback_fallback_on_exdev() {
    // State dir on different mount → rename returns EXDEV → truncation path taken.
    // (Simulated via mount namespace or by patching rename to return EXDEV.)
}

#[tokio::test]
async fn test_empty_config_at_startup_exits() {
    // Create empty file → ConfigHandle::load returns EmptyFile error.
}

#[tokio::test]
async fn test_serde_default_backward_compat() {
    // Parse a minimal TOML (only required fields) → all defaults populated.
}

#[tokio::test]
async fn test_broadcast_lagged_subscriber() {
    // Create subscriber, send 20 diffs without consuming → assert RecvError::Lagged.
}

#[tokio::test]
async fn test_readonly_fs_writeback_graceful() {
    // Config path is read-only → writeback returns ReadOnly error, no panic.
}
```

---

## 9. Open Questions

### Deferred to Implementation

1. **toml_edit vs toml:** Should writeback use `toml_edit` (preserves comments and formatting) or `toml` (simpler, loses comments)? `toml_edit` is preferred for operator experience but adds a dependency and complexity. Decision: start with `toml_edit` — operators who version-control their TOML will notice formatting churn with plain `toml`.

2. **Overlay merge performance (v1.1):** When ConfigOverlayStore is integrated, should the merge happen on every `snapshot()` call or be pre-computed on overlay change? Pre-computed is better (overlay changes are rare; reads are hot-path).

3. **Config validation strictness:** Should unknown TOML keys be silently ignored or produce a warning? Silently ignored is the `serde` default, but a warning helps catch typos. Decision: log a warning for unknown keys at INFO level during parse.

4. **SIGHUP coalescing:** If 5 SIGHUPs arrive while one reload is in progress, should we reload once after the mutex releases (coalescing) or 5 times? Current design: 5 sequential reloads. Optimization: track a `pending_reload` flag; if set when mutex releases, do one more reload then clear. Implement if profiling shows reload cost is problematic.

### Alternatives Considered

- **etcd/consul for config:** Rejected. Single-binary deployment is a core constraint. Network-distributed config adds operational complexity disproportionate to the 2-4 GPU target.
- **inotify for file watching:** Considered as alternative to SIGHUP. Rejected because: (a) unreliable on network mounts, (b) doesn't cover Kubernetes ConfigMap updates (symlink swap), (c) SIGHUP is explicit and predictable. Could be added as optional in v1.1.
- **Config as code (Rust DSL):** Rejected. TOML is universally understood, Git-friendly, and requires no compilation.

---

## Cross-Cutting Concerns

### flock Timeout Implementation

`flock(2)` does not support a timeout parameter. The implementation uses `LOCK_NB`
(non-blocking) in a retry loop with `thread::sleep(100ms)` between attempts, counted
to 300 iterations (300 × 100ms = 30s). This runs inside `spawn_blocking`, so the
sleeping thread does not block the Tokio reactor.

```rust
// Inside spawn_blocking closure
fn acquire_flock_with_timeout(fd: &File, timeout: Duration) -> Result<(), WritebackError> {
    let deadline = Instant::now() + timeout;
    loop {
        match flock(fd.as_raw_fd(), FlockArg::LockExclusiveNonblock) {
            Ok(()) => return Ok(()),
            Err(Errno::EWOULDBLOCK) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(Errno::EWOULDBLOCK) => {
                return Err(WritebackError::LockTimeout {
                    elapsed: timeout,
                });
            }
            Err(e) => return Err(WritebackError::WriteFailed {
                source: std::io::Error::from(e),
            }),
        }
    }
}
```

This approach is cancellation-safe (the blocking task runs to completion but
the result is dropped), does not leak threads (unlike wrapping a blocking `flock`
in `tokio::time::timeout`), and provides predictable timeout behavior.

### Cancellation Safety

The Config Manager has two `.await` points in the reload path:
1. `reload_mutex.lock().await` — safe. If cancelled here, the mutex is never acquired; no state change.
2. `spawn_blocking(...).await` — safe. If cancelled, the blocking task runs to completion (it holds flock, writes file, releases flock) but the result is dropped. The in-memory config remains unchanged. On next reload, the file reflects the blocking task's write. This is correct — the file is the source of truth.

**Cancellation invariant for `reload_mutex`:** The mutex guard is held while awaiting
`spawn_blocking`, which may block on `flock` for up to 30 seconds. If the calling task
is cancelled after acquiring the mutex but before `spawn_blocking` completes, the guard
drops (releasing the mutex) while the blocking task continues unserialized. The
implementation must ensure the mutex guard is bound to a variable that outlives the
`spawn_blocking` await — i.e., the guard must NOT be held in a temporary:

```rust
// CORRECT: guard lives until end of scope
let _guard = self.reload_mutex.lock().await;
let result = tokio::task::spawn_blocking(move || { /* flock + read + parse */ }).await;
// _guard dropped here, after spawn_blocking completes

// WRONG: guard dropped before spawn_blocking if task is cancelled
```

This must be verified in code review. The existing cancellation analysis (`.await` point 2)
is correct: the blocking task runs to completion regardless, and the file is the
source of truth, so a cancelled reload is safe — it just means the in-memory config
won't be updated until the next reload.

**No cancel-unsafe `.await` points exist in this component.**

### Graceful Shutdown

On SIGTERM:
1. Drop the `broadcast::Sender` — all subscribers receive `RecvError::Closed`.
2. If a reload is in progress, let it complete (hold shutdown until mutex is released). The reload's spawn_blocking will finish; the diff is broadcast to subscribers who use it for their own graceful drain.
3. No file cleanup needed — lock file is advisory; OS releases flock on process exit.

### Panic Safety

If `Config Manager` panics:
- `ArcSwap` is not poisoned (unlike `std::sync::Mutex`). Readers continue with the last-stored config.
- `tokio::sync::Mutex` is not poisoned. Next reload attempt will acquire it normally.
- `flock` is released by the OS on process exit (panic unwinds → Drop → fd closed).
- **Invariant at risk:** None permanently. A panic in the diff computation leaves old config in place (safe). A panic in ArcSwap::store is not possible (it's a pointer swap).

### Clock Assumptions

- **No clock usage.** Config Manager doesn't use wall-clock or monotonic time internally.
- `ConfigDiff::timestamp` uses `Instant` (monotonic) for ordering diffs. Not used for correctness, only for logging.

### Memory Bounds

- **Config snapshot:** Bounded by TOML file size. A pathological config with 100 servers, each with 50 LoRA adapters, is ~100KB. Well under concern.
- **Broadcast channel:** Fixed capacity (16 diffs). Each diff is ~1KB. Max buffered: 16KB.
- **ArcSwap:** Holds 2 snapshots concurrently (old readers + new). Max: 200KB.
- **Worst case total:** < 1MB. Fully bounded.

### Backpressure

- **Reload requests:** Serialized by mutex. Second SIGHUP waits. No queue, no unbounded growth.
- **Writeback requests:** Serialized by same mutex. Concurrent writebacks queue on the mutex. Bounded by the number of concurrent API requests (which is bounded by Transport Layer's connection limit).
- **Broadcast consumers:** If a subscriber is slow, it gets `Lagged` and must re-snapshot. No head-of-line blocking on other subscribers.
