# Traits & Composability Review: Config Manager

**Reviewer:** Trait Design Sub-Agent  
**Document:** `component-designs/01_CONFIG_MANAGER.md`  
**Date:** 2026-05-07  
**Rules Applied:** 10 (Composition, Newtypes, Orphan Rule, Extension Traits, Sealed Traits, Interface Segregation, Static Dispatch, Builder Pattern, Composite Pattern, GATs)

---

## Summary

The Config Manager design is structurally sound for a configuration subsystem -- it avoids deep trait hierarchies and keeps its API surface concrete. However, it has several domain-identity gaps (bare `String` keys in HashMaps), missing builder patterns for large structs, and a sealed-trait opportunity for the `ConfigMutation` type. 6 findings total: 2 HIGH, 3 MEDIUM, 1 LOW.

---

## Findings

### F-1: Bare `String` Keys in `PriorityRulesConfig` HashMaps

**Rule:** 2 -- Newtype Wrapper for Domain Identity  
**Severity:** HIGH  
**Location:** Section 2.3, lines 160-166  

**Violation:** Four HashMaps in `PriorityRulesConfig` use bare `String` as keys:

```rust
pub task_types: HashMap<String, u32>,
pub custom_task_types: HashMap<String, u32>,
pub task_type_classes: HashMap<String, String>,
pub requestor_bonuses: HashMap<String, i32>,
```

`task_types` and `custom_task_types` map task-type names to priority weights. `task_type_classes` maps task-type names to class names. `requestor_bonuses` maps requestor identifiers to bonus values. All four use raw `String` for what are clearly domain identities -- a `TaskTypeId` could be accidentally used where a `RequestorId` is expected, and the compiler would not catch it.

**Suggested fix:** Introduce newtypes:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskTypeId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskClassName(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestorId(pub String);
```

Then rewrite the fields as:

```rust
pub task_types: HashMap<TaskTypeId, u32>,
pub custom_task_types: HashMap<TaskTypeId, u32>,
pub task_type_classes: HashMap<TaskTypeId, TaskClassName>,
pub requestor_bonuses: HashMap<RequestorId, i32>,
```

This also improves the value side of `task_type_classes` -- the second `String` becomes `TaskClassName`, preventing a class name from being confused with a task-type name.

---

### F-2: `ServerConfigDiff.fields_changed` Uses `Vec<String>` Instead of an Enum

**Rule:** 2 -- Newtype Wrapper for Domain Identity (secondary: Rule 7 -- Static Dispatch)  
**Severity:** HIGH  
**Location:** Section 2.5, line 239  

**Violation:**

```rust
pub fields_changed: Vec<String>,
```

Field names are represented as bare strings. Consumers (Server Manager) must match on magic string literals like `"model_path"` or `"ctx_size"` to determine behavior. This is fragile -- a typo compiles silently, and refactoring a field name won't produce a compiler error at the consumer site.

**Suggested fix:** Define a closed enum:

```rust
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
}
```

Then: `pub fields_changed: Vec<ServerConfigField>`. The `requires_restart` and `hot_reloadable` booleans can then be derived from the set of changed fields via a method on `ServerConfigField`, eliminating the possibility of those booleans drifting out of sync with the actual field list.

---

### F-3: `GlobalConfig` Has 12 Fields -- Missing Builder

**Rule:** 8 -- Builder Pattern  
**Severity:** MEDIUM  
**Location:** Section 2.3, lines 101-127  

**Violation:** `GlobalConfig` has 12 fields, many of which have defaults. While `#[serde(default)]` handles deserialization, programmatic construction in tests and integration code requires specifying all 12 fields or using `..Default::default()` (which requires `Default` to be derived -- not shown). There is no builder.

**Suggested fix:** Either derive `Default` explicitly (sufficient given all fields have serde defaults) or provide a builder via `#[derive(Builder)]` from the `typed-builder` or `derive_builder` crate. At minimum, add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GlobalConfig { ... }
```

This also applies to `ServerConfig` (14 fields, line 130-157) and `PriorityRulesConfig` (7 fields, lines 160-171). All three structs exceed the 4-field threshold.

---

### F-4: `ConfigHandle` Is a Concrete Struct -- No Trait for Testing

**Rule:** 6 -- Interface Segregation (secondary: Rule 1 -- Composition Over Inheritance)  
**Severity:** MEDIUM  
**Location:** Section 2.1, lines 33-66  

**Violation:** `ConfigHandle` is defined as a concrete struct with 5 public methods covering both read operations (`snapshot`, `export_toml`) and write operations (`reload`, `writeback`). Consumers that only need to read config (Router, Queue, Dispatcher) are coupled to the full write API. More importantly, there is no trait abstraction, so unit-testing consumers requires constructing a real `ConfigHandle` with real `ArcSwap` internals even if the test only needs `snapshot()`.

**Suggested fix:** Extract focused traits:

```rust
pub trait ConfigReader {
    fn snapshot(&self) -> arc_swap::Guard<Arc<AppConfig>>;
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ConfigDiff>;
}

pub trait ConfigWriter {
    async fn reload(&self, paths: &ConfigPaths) -> Result<ConfigDiff, ConfigReloadError>;
    async fn writeback(&self, paths: &ConfigPaths, mutation: ConfigMutation) -> Result<ConfigDiff, WritebackError>;
}
```

Read-only consumers depend on `impl ConfigReader`; only the SIGHUP handler and API layer need `impl ConfigWriter`. Test doubles become trivial.

---

### F-5: `ConfigMutation` Is Undefined -- Should Be a Sealed Enum

**Rule:** 5 -- Sealed Traits (applied to enum extensibility)  
**Severity:** MEDIUM  
**Location:** Section 2.1, line 61  

**Violation:** `ConfigMutation` is referenced as a parameter to `writeback()` but never defined in the document. When it is defined, it should be a closed enum (effectively sealed) -- downstream consumers should not be able to invent arbitrary mutations that bypass the Config Manager's validation and merge logic.

**Suggested fix:** Define as:

```rust
#[non_exhaustive]
pub enum ConfigMutation {
    SetLoraAdapters {
        server_id: ServerId,
        adapters: Vec<LoraAdapterConfig>,
    },
    // Future: SetAlias, SetGpuLayers, etc.
}
```

Using `#[non_exhaustive]` on the enum prevents downstream code from exhaustively matching, allowing new variants to be added without breaking changes -- the enum-level equivalent of sealing.

---

### F-6: `ConfigDiff` Tree Structure Lacks a Shared Trait

**Rule:** 9 -- Composite Pattern  
**Severity:** LOW  
**Location:** Section 2.5, lines 225-240  

**Violation:** `ConfigDiff` contains a mix of leaf changes (`global_changed: bool`, `priority_rules_changed: bool`) and composite changes (`servers_changed: Vec<(ServerId, ServerConfigDiff)>`). There is no shared abstraction for "a change to a config subtree" -- each consumer must know the exact structure of every diff variant. As the config grows (overlay store in v1.1, transport config in Phase 5), this flat struct will accumulate fields.

**Suggested fix:** Consider a trait or enum for diff nodes:

```rust
pub enum ConfigChange {
    Global,
    PriorityRules,
    Server { id: ServerId, diff: ServerConfigDiff },
}

pub struct ConfigDiff {
    pub changes: Vec<ConfigChange>,
    pub timestamp: Instant,
}
```

This is low severity because the current structure is adequate for v1.0's scope, but worth flagging for the v1.1 design pass.

---

## Commendations

### K-1: `ServerId` Is Already a Newtype

The design uses `ServerId` as the key type in `HashMap<ServerId, ServerConfig>` and throughout `ConfigDiff`. Cross-referencing with the main design document confirms this is defined as `pub struct ServerId(Arc<str>)` -- a proper newtype with cheap cloning. This is a textbook application of Rule 2.

### K-2: No Dynamic Dispatch Anywhere

The entire API surface uses concrete types. There are no `Box<dyn Trait>` or trait objects. `ArcSwap<AppConfig>` uses a concrete `AppConfig` type, and `broadcast::Receiver<ConfigDiff>` is monomorphized. This is exactly right for a config subsystem where the types are known at compile time (Rule 7).

### K-3: No Trait Hierarchies

The design avoids any form of OOP-style inheritance. `ConfigHandle` is a plain struct, not a base class. `AppConfig` is composed of sub-structs (`GlobalConfig`, `ServerConfig`, etc.) via aggregation, not inheritance. This fully satisfies Rule 1.

### K-4: Clean Error Type Segregation

The three error types (`ConfigLoadError`, `ConfigReloadError`, `WritebackError`) are cleanly separated by operation, each with focused variants. This follows interface segregation principles (Rule 6) at the error-type level -- consumers of `snapshot()` never encounter `WritebackError`.

### K-5: `CachedConfigReader` Is a Smart Optimization Wrapper

Rather than embedding caching behavior into `ConfigHandle` itself, the design provides a separate `CachedConfigReader` that wraps the same `ArcSwap`. This is composition over inheritance in practice -- the hot-path optimization is an opt-in decorator, not a base-class feature.

---

## Verdict

| Severity | Count |
|----------|-------|
| HIGH     | 2     |
| MEDIUM   | 3     |
| LOW      | 1     |

The two HIGH findings (F-1, F-2) should be addressed before implementation begins -- bare `String` keys in priority rules and string-based field-change tracking will propagate type confusion across every consumer. The MEDIUM findings (F-3, F-4, F-5) are important for testability and maintainability but can be addressed during implementation. F-6 is a forward-looking suggestion for v1.1.

Overall, the design demonstrates good Rust idioms: concrete types, composition, proper use of `ArcSwap` for lock-free reads, and clean error segregation. The gaps are primarily in domain-identity typing and trait extraction for testability.
