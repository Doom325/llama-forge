# Boundaries Review: Config Manager (01_CONFIG_MANAGER.md)

**Reviewer focus:** Architectural boundaries, modularity, dependency rule  
**Rules evaluated:** 11-16  
**Date:** 2026-05-07

---

## Summary

The Config Manager design is pragmatic and well-scoped, with clear boundary contracts and failure-mode analysis. However, it conflates domain entities with infrastructure serialization concerns throughout, lacks a port/adapter separation for filesystem access, and does not use dependency injection for its concrete dependencies. These issues will calcify quickly once downstream components depend on the config types.

**Findings:** 6 violations (2 HIGH, 2 MEDIUM, 2 LOW)  
**Commendations:** 3

---

## Findings

### Finding 1 -- Domain Types Carry Serde Annotations

**Rule:** 14 -- Strict Domain / DTO Segregation  
**Severity:** HIGH  
**Location:** Section 2.3 "Config Types", lines 89-171

**Violation:** `AppConfig`, `GlobalConfig`, `ServerConfig`, and `PriorityRulesConfig` are simultaneously the domain configuration entities used by every consumer (via `snapshot()`) AND the TOML deserialization targets (via `#[derive(Serialize, Deserialize)]` with `#[serde(default)]` attributes). This means every component that reads configuration takes a transitive dependency on `serde` and `toml` through the domain types. If the serialization format changes (e.g., YAML support, JSON API responses, protobuf for gRPC config sync), the domain types must change.

**Suggested fix:** Introduce a separate `RawAppConfig` (or `AppConfigDto`) layer that carries the serde annotations and `#[serde(default)]` attributes. The domain `AppConfig` should be a plain Rust struct with no serialization annotations. A `From<RawAppConfig>` or `TryFrom<RawAppConfig>` conversion in the infrastructure layer handles the mapping and applies defaults/validation. Consumers only ever see the clean domain type.

```
infrastructure/toml_loader.rs:  RawAppConfig  (#[derive(Deserialize)])
                                    |
                                TryFrom / validated conversion
                                    |
domain/config.rs:               AppConfig     (plain struct, no serde)
```

---

### Finding 2 -- ConfigHandle Directly Performs Infrastructure I/O

**Rule:** 13 -- Ports Are Traits, Adapters Are Structs  
**Severity:** HIGH  
**Location:** Section 2.1 "Core Trait", lines 30-66

**Violation:** Despite the section title "Core Trait", `ConfigHandle` is a concrete struct, not a trait. It directly performs filesystem I/O (`load`, `reload`, `writeback`), flock acquisition, TOML parsing, and atomic file operations. There is no port (trait) that abstracts config persistence, which means:

1. The domain logic (diff computation, snapshot management, broadcast) is inseparable from the infrastructure (filesystem, flock, TOML parsing).
2. Unit testing the reload serialization logic or diff broadcasting requires a real filesystem -- the document itself acknowledges this in Section 8.1 by saying "no mocking FS; too fragile", but that fragility is a symptom of missing the port abstraction.
3. Future storage backends (ConfigOverlayStore in v1.1, etcd if requirements change) require modifying the core struct rather than providing a new adapter.

**Suggested fix:** Extract a `ConfigSource` port trait that the domain depends on:

```rust
// domain/ports.rs
#[async_trait]
pub trait ConfigSource: Send + Sync {
    async fn load(&self) -> Result<AppConfig, ConfigLoadError>;
    async fn persist(&self, config: &AppConfig, mutation: &ConfigMutation) -> Result<(), WritebackError>;
}
```

`ConfigHandle` then depends on `Arc<dyn ConfigSource>` and delegates I/O through the port. A `TomlFileConfigSource` adapter in the infrastructure layer implements the flock/read/parse/write/rename logic. The domain `ConfigHandle` retains ownership of ArcSwap, diff computation, broadcast, and mutex serialization.

---

### Finding 3 -- No Typestate for Config Lifecycle

**Rule:** 15 -- Typestate for Business Rule Enforcement  
**Severity:** MEDIUM  
**Location:** Section 2.1, lines 30-66; Section 3.4 "Invariants", lines 325-331

**Violation:** The design describes a clear lifecycle: config is loaded (startup, may fail hard), then enters a running state where reloads are non-fatal and writeback is available. These are distinct states with different error semantics:

- **Unloaded -> Loaded:** `load()` is called once; failure is fatal; `snapshot()` is not yet available.
- **Loaded (Running):** `reload()` and `writeback()` are available; failures are non-fatal; `snapshot()` always succeeds.

The current design uses a single `ConfigHandle` struct for both phases. This means the type system does not prevent calling `reload()` before `load()` completes, or calling `snapshot()` on an uninitialized handle. The invariant "snapshot() always returns valid config" is enforced by construction order, not by type.

**Suggested fix:** Use typestate to model the lifecycle:

```rust
pub struct ConfigHandle<S: ConfigState> { inner: Arc<ConfigManagerInner>, _state: PhantomData<S> }

pub struct Unloaded;
pub struct Running;

impl ConfigHandle<Unloaded> {
    pub fn new(source: Arc<dyn ConfigSource>) -> Self;
    pub async fn load(self) -> Result<ConfigHandle<Running>, ConfigLoadError>;
}

impl ConfigHandle<Running> {
    pub fn snapshot(&self) -> Guard<Arc<AppConfig>>;
    pub async fn reload(&self) -> Result<ConfigDiff, ConfigReloadError>;
    pub async fn writeback(&self, mutation: ConfigMutation) -> Result<ConfigDiff, WritebackError>;
}
```

This makes it a compile-time error to call `snapshot()` before loading succeeds.

---

### Finding 4 -- Concrete Infrastructure Wired Inside ConfigHandle

**Rule:** 16 -- Dependency Injection at the Top  
**Severity:** MEDIUM  
**Location:** Section 2.1, line 41; Section 3.3, lines 306-320

**Violation:** `ConfigHandle::load(paths: &ConfigPaths)` is a constructor that internally creates the filesystem reader, flock mechanism, and TOML parser. The concrete infrastructure is wired inside the component, not injected from a composition root. The `ConfigPaths` struct (line 307-312) is passed into domain methods (`reload`, `writeback`) rather than being encapsulated in an injected adapter. This means `main.rs` cannot swap in a test double or alternative config source without modifying `ConfigHandle`.

**Suggested fix:** `ConfigHandle` should accept its dependencies via constructor injection:

```rust
impl ConfigHandle {
    pub fn new(source: Arc<dyn ConfigSource>, broadcast_capacity: usize) -> Self;
}
```

In `main.rs` (the composition root):

```rust
let source = Arc::new(TomlFileConfigSource::new(config_paths));
let handle = ConfigHandle::new(source, 16);
let handle = handle.load().await?;  // if using typestate from Finding 3
```

---

### Finding 5 -- Error Types Carry Infrastructure Details

**Rule:** 12 -- The Dependency Rule is Inviolable  
**Severity:** LOW  
**Location:** Section 2.4 "Error Types", lines 176-217

**Violation (partial):** `ConfigLoadError::ParseError` wraps `toml::de::Error` (line 186), and `WritebackError::SerializeError` wraps `toml::ser::Error` (line 214). If these error types are defined in the domain layer (alongside `AppConfig` and `ConfigHandle`), they create an inward dependency violation: the domain knows about the `toml` crate, which is an infrastructure serialization detail.

This is a low-severity instance because error types are often pragmatically placed at the boundary, and the `toml` crate dependency is lightweight. However, it does mean any consumer matching on these error variants takes a transitive dependency on `toml`.

**Suggested fix:** Domain error types should use opaque source errors:

```rust
#[error("parse error in {path}: {message}")]
ParseError { path: PathBuf, message: String, source: Box<dyn std::error::Error + Send + Sync> },
```

The infrastructure adapter converts `toml::de::Error` into this opaque form. Consumers see the message but don't depend on the `toml` crate.

---

### Finding 6 -- Inconsistent Boundary Pattern Naming

**Rule:** 11 -- Choose the Right Boundary Pattern Consistently  
**Severity:** LOW  
**Location:** Throughout the document

**Violation:** The design does not explicitly declare which architectural pattern it follows (hexagonal, onion, clean). The terminology is mixed:

- Section 2.1 is titled "Core Trait" but defines a concrete struct.
- Section 4 "Boundary Contracts" uses hexagonal-style language (requires/guarantees) but there are no explicit port/adapter declarations.
- Section 4.2 mentions `ConfigOverlayStore trait impl` (v1.1) suggesting awareness of the port pattern, but v1.0 does not apply it to its own dependencies.
- The document references consumers receiving `Arc<ConfigHandle>` directly rather than through a trait, mixing service-locator style with what should be DI.

**Suggested fix:** Add an "Architecture Pattern" subsection to Section 3 that explicitly declares hexagonal architecture with named ports and adapters:

- **Inbound port:** `ConfigHandle` API (already well-defined)
- **Outbound port:** `ConfigSource` trait (to be added per Finding 2)
- **Adapter:** `TomlFileConfigSource` (implements `ConfigSource`)
- **Domain:** `AppConfig`, `ConfigDiff`, diff computation, reload serialization, broadcast

---

## Commendations

### C1 -- Excellent Boundary Contracts (Section 4)

The "What This Component REQUIRES" and "What This Component GUARANTEES" tables are exemplary. They make the component's expectations and promises explicit, which is precisely the discipline needed for a multi-component system. The guarantees are specific and testable (e.g., "never panics, never blocks", "conservative -- true if uncertain").

### C2 -- Failure Mode Completeness (Section 5)

Six explicit failure modes with state-preserved/state-lost analysis and recovery procedures. FM-2 honestly documents the crash window in the truncation fallback rather than hiding it. This level of failure analysis at design time prevents entire classes of production incidents.

### C3 -- Immutability-First Concurrency Model

The ArcSwap + immutable snapshot + broadcast diff design is a strong architectural choice. It eliminates lock contention on the read path (which is the hot path for every component), provides consistent-snapshot semantics without complex locking, and makes the system naturally resistant to reader starvation. The CachedConfigReader (Section 2.2) further optimizes the hot path. This is well-suited to the domain.

---

## Recommendation

**Do not approve as-is.** The two HIGH findings (Rule 14 serde-on-domain and Rule 13 missing port abstraction) should be addressed before implementation begins, as they establish patterns that every downstream component will depend on. The MEDIUM findings (Rule 15 typestate and Rule 16 DI) are strongly recommended but could be deferred to a fast-follow if timeline pressure exists. The LOW findings are suggestions for polish.

Priority order for fixes:
1. Separate `RawAppConfig` DTO from domain `AppConfig` (Finding 1)
2. Extract `ConfigSource` port trait (Finding 2)
3. Move construction to composition root (Finding 4)
4. Add typestate for load lifecycle (Finding 3)
5. Make error types infrastructure-agnostic (Finding 5)
6. Declare architecture pattern explicitly (Finding 6)
