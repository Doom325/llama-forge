# Config Manager — Implementation Plan

**Component:** Config Manager (`docs/component-designs/01_CONFIG_MANAGER.md`)  
**Estimated LOC:** ~1,500  
**Estimated Duration:** 5–7 working days (single engineer) / 3–4 days (two engineers, PR3+PR4 parallelizable)  
**Design Document:** `docs/component-designs/01_CONFIG_MANAGER.md` (R3 PASS)  
**Branch Prefix:** `config-mgr/`  
**Target:** `main` (via squash-merge PRs)

---

## Ground Rules for CoderAgents

1. **Read the design doc first.** Every PR description must reference the specific design doc sections it implements.
2. **No `unwrap()` or `expect()` in production code paths.** Use `?` propagation or explicit error handling.
3. **All filesystem I/O inside `spawn_blocking`.** Never call `std::fs::*` or `flock` directly on the Tokio reactor.
4. **Every public type gets a doc comment.** Reference the design doc section number.
5. **`#[serde(default)]` on every config struct field.** Backward compatibility is a design invariant.
6. **Tests run with `cargo test --workspace`.** Every PR must pass before merge.
7. **`cargo clippy -- -D warnings` must pass.** Zero warnings policy.
8. **Commit messages:** `config-mgr: <imperative summary>` (e.g., `config-mgr: add ConfigSource port trait and InMemoryConfigSource`).

---

## Dependency Graph (PR merge order)

```
PR1 ─── PR2 ─── PR3 ─── PR4 ─── PR5 ─── PR6 ─── PR7
                  │                              │
                  └──────── can overlap ─────────┘
                    (PR3 types vs PR4 logic)
```

PR1 → PR2 → PR3 → PR4 → PR5 → PR6 → PR7 (strict ordering, except PR3/PR4 can be developed concurrently after PR2 merges)

---

## PR1: Project Scaffold & Cargo Workspace

**Branch:** `config-mgr/01-scaffold`  
**Merge Target:** `main`  
**Size:** Small (~150 lines)  
**Estimated Time:** 2–3 hours

### Purpose

Bootstrap the Cargo workspace so all subsequent PRs have a compilable project to build on. No business logic — just structure, dependencies, and CI.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 1.1 | Create Cargo workspace root | `Cargo.toml` with `[workspace]` members: `llamaforge` (binary crate), `llamaforge-config` (library crate). Set `resolver = "2"`, `edition = "2021"`, `rust-version = "1.75"`. | [x] `Cargo.toml` |
| 1.2 | Create `llamaforge-config` lib crate | `cargo init --lib crates/llamaforge-config`. This crate owns all Config Manager code. Add placeholder `lib.rs` with `//! Config Manager — see component-designs/01_CONFIG_MANAGER.md`. | [x] `crates/llamaforge-config/` |
| 1.3 | Create `llamaforge` binary crate | `cargo init crates/llamaforge`. Minimal `main.rs` that prints "llamaforge starting" and exits. Depends on `llamaforge-config`. | [x] `crates/llamaforge/` |
| 1.4 | Add workspace-level dependencies | In workspace `Cargo.toml` `[workspace.dependencies]`: `serde = { version = "1", features = ["derive"] }`, `toml = "0.8"`, `toml_edit = "0.22"`, `thiserror = "2"`, `arc-swap = "1"`, `tokio = { version = "1", features = ["full"] }`, `tracing = "0.1"`, `tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }`, `nix = { version = "0.29", features = ["fs"] }`, `tempfile = "3"` (dev), `proptest = "1"` (dev), `typed-builder = "0.20"` (dev). | [x] `Cargo.toml` |
| 1.5 | Wire crate dependencies | `llamaforge-config/Cargo.toml` inherits workspace deps: `serde`, `toml`, `toml_edit`, `thiserror`, `arc-swap`, `tokio`, `tracing`, `nix`. Dev-deps: `tempfile`, `proptest`, `typed-builder`, `tokio` (with `test-util`). | [x] `crates/llamaforge-config/Cargo.toml` |
| 1.6 | Add `.cargo/config.toml` | Set `[build] rustflags = ["-D", "warnings"]`. Optional: configure `mold` linker if available. | [x] `.cargo/config.toml` |
| 1.7 | Add `rustfmt.toml` | `edition = "2021"`, `max_width = 100`, `use_small_heuristics = "Max"`. | [x] `rustfmt.toml` |
| 1.8 | Add `clippy.toml` | Empty initially (use defaults). | [x] `clippy.toml` |
| 1.9 | Add `.gitignore` | Standard Rust `.gitignore`: `/target`, `Cargo.lock` for libs (keep for binary). | [x] `.gitignore` |
| 1.10 | Add example config files | Create `examples/llamaforge.toml` and `examples/priority_rules.toml` with all fields documented, matching the design doc Section 2.6. These serve as both documentation and test fixtures. | [x] `examples/` |
| 1.11 | Verify build | `cargo build --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace` (no tests yet, should pass trivially). | [x] — |

### PR1 Acceptance Criteria

- [x] `cargo build --workspace` succeeds with zero warnings
- [x] `cargo clippy --workspace -- -D warnings` passes clean
- [x] `cargo run -p llamaforge` prints startup message and exits
- [x] `crates/llamaforge-config/src/lib.rs` compiles as a library
- [x] `examples/llamaforge.toml` contains a complete, valid example config with at least 2 server entries
- [x] `examples/priority_rules.toml` contains valid priority rules with task types, requestor bonuses, starvation, and fair share config

### PR1 Creation Steps

```bash
git checkout -b config-mgr/01-scaffold main
# ... implement tasks 1.1–1.11 ...
git add -A && git commit -m "config-mgr: bootstrap cargo workspace with llamaforge-config crate"
git push -u origin config-mgr/01-scaffold
# Open PR: "Config Manager PR1: Project Scaffold & Cargo Workspace"
# Description: "Bootstraps the Cargo workspace for LlamaForge. Creates llamaforge-config
# library crate that will house all Config Manager logic. Adds workspace-level dependencies,
# example config files, and build tooling. Implements design doc Section 2.6 (config types
# are in PR2). No business logic — structure only."
```

---

## PR2: Config Types, Error Types & Diff Types

**Branch:** `config-mgr/02-types`  
**Base:** `config-mgr/01-scaffold` (or `main` after PR1 merges)  
**Size:** Medium (~400 lines)  
**Estimated Time:** 4–6 hours

### Purpose

Define every data type from the design doc Sections 2.6, 2.7, and 2.8. This is the "type foundation" — everything else in the Config Manager is built on these types. Getting them right early avoids cascading refactors.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 2.1 | Create module structure | `lib.rs` declares modules: `pub mod types;`, `pub mod error;`, `pub mod diff;`, `pub mod source;`, `pub mod handle;`, `pub mod instrumented;`, `pub mod cached;`. Stub each module file. | [x] `src/lib.rs`, `src/*.rs` |
| 2.2 | Implement newtype wrappers | `ServerId`, `TaskTypeId`, `TaskClassName`, `RequestorId` — all with `#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]`, `#[serde(transparent)]`. Implement `Display` for each. Add `impl From<&str>` and `impl From<String>` convenience constructors. | [x] `src/types.rs` |
| 2.3 | Implement enums | `LogLevel`, `LogFormat`, `Backend`, `CacheType`, `FlashAttnMode`, `SpeculativeMethod`, `ModelArchitecture` — all with `Default`, `Serialize`, `Deserialize`, `PartialEq`, `Clone`, `Debug`. Match design doc defaults exactly. | [x] `src/types.rs` |
| 2.4 | Implement config structs | `AppConfig`, `GlobalConfig`, `ServerConfig`, `GrpcConfig`, `PrometheusConfig`, `WebUiConfig`, `PriorityRulesConfig`, `StarvationConfig`, `FairShareConfig`, `PreemptionConfig`, `LoraAdapterConfig`, `DeviceSpec`, `GpuId`. Every field gets `#[serde(default)]` or `#[serde(default = "...")]` with a named default function. All structs derive `Default`. | [x] `src/types.rs` |
| 2.5 | Implement `ConfigPaths` | `main_config: PathBuf`, `priority_rules: PathBuf`, `state_dir: PathBuf`, `lock_file: PathBuf`. Constructor that computes `lock_file` from `state_dir`. | [x] `src/types.rs` |
| 2.6 | Implement `ConfigMutation` | `#[non_exhaustive] pub enum ConfigMutation` with `SetLoraAdapters { server_id: ServerId, adapters: Vec<LoraAdapterConfig> }`. | [x] `src/types.rs` |
| 2.7 | Implement error types | `ConfigLoadError`, `ConfigReloadError`, `WritebackError` — exactly as specified in design doc Section 2.7. Use `thiserror`. Source fields use `Box<dyn Error + Send + Sync>` for `ParseError` and `SerializeError`. | [x] `src/error.rs` |
| 2.8 | Implement diff types | `ConfigDiff`, `ConfigChange` (`#[non_exhaustive]`), `ServerConfigDiff`, `ServerConfigField`. Implement `ConfigDiff::servers_added()`, `servers_removed()`, `servers_changed()` iterator methods. Implement `ServerConfigField::requires_restart()` and `hot_reloadable()`. Implement `ServerConfigDiff::requires_restart()` and `hot_reloadable()`. | [x] `src/diff.rs` |
| 2.9 | Implement diff computation | `pub fn compute_diff(old: &AppConfig, new: &AppConfig) -> ConfigDiff`. Compare `global`, `priority_rules`, server maps (added/removed/changed). For changed servers, field-by-field comparison producing `Vec<ServerConfigField>`. | [x] `src/diff.rs` |
| 2.10 | Unit tests: serde roundtrip | Test that `AppConfig` serializes to TOML and parses back identically. Test that every default value is populated when parsing an empty `[global]` section. Test that unknown keys are silently ignored (serde default behavior). | [x] `src/types.rs` (tests module) |
| 2.11 | Unit tests: diff computation | Test cases: (a) identical configs → empty diff, (b) server added → `ServerAdded`, (c) server removed → `ServerRemoved`, (d) server port changed → `ServerChanged` with `requires_restart() == true`, (e) LoRA adapters changed → `ServerChanged` with `hot_reloadable() == true`, (f) multiple changes simultaneously, (g) global config changed → `GlobalChanged`, (h) priority rules changed → `PriorityRulesChanged`. | [x] `src/diff.rs` (tests module) |
| 2.12 | Unit tests: example config parsing | Parse `examples/llamaforge.toml` and `examples/priority_rules.toml` → assert they produce a valid `AppConfig` with expected values. This is a regression test that the example configs stay valid. | [x] `tests/parse_examples.rs` |

### PR2 Acceptance Criteria

- [x] All types compile and derive the required traits
- [x] `cargo test --workspace` passes with all unit tests green
- [x] `examples/llamaforge.toml` parses into a valid `AppConfig`
- [x] Diff computation correctly identifies added, removed, and changed servers
- [x] `ServerConfigField::requires_restart()` returns `true` for all restart-requiring fields per design doc
- [x] `ServerConfigField::hot_reloadable()` returns `true` only for `LoraAdapters`, `Alias`, `EstimatedVramMb`
- [x] Serde roundtrip: `parse(serialize(config)) == config` for a fully populated config
- [x] Unknown TOML keys don't cause parse errors (verify with a TOML string containing `bogus_key = 42`)

### PR2 Creation Steps

```bash
git checkout -b config-mgr/02-types main  # or config-mgr/01-scaffold
# ... implement tasks 2.1–2.12 ...
git add -A && git commit -m "config-mgr: add config types, error types, and diff computation"
git push -u origin config-mgr/02-types
# Open PR: "Config Manager PR2: Types, Errors & Diff Engine"
# Description: "Implements all data types from design doc Sections 2.6, 2.7, 2.8.
# Includes newtype wrappers (ServerId, TaskTypeId, etc.), all config structs with
# serde defaults, error types with thiserror, and the diff computation engine.
# 12 unit tests covering serde roundtrip, diff correctness, and example config parsing."
```

---

## VC-1: Types Verification Checkpoint

**Gate:** PR2 merged. All types compile, all tests pass.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-1.1 | Serde backward compat | Parse a minimal TOML with only `[global]` and one `[[servers]]` → all defaults populated | All fields have non-zero defaults where applicable |
| VC-1.2 | Diff accuracy | Construct two configs differing by 1 added server, 1 removed server, 1 changed server (port + LoRA) → verify diff | Diff contains exactly 3 `ConfigChange` entries with correct classification |
| VC-1.3 | Example configs valid | `examples/llamaforge.toml` parses without error | Test passes |
| VC-1.4 | Type safety | Attempt to pass `TaskTypeId` where `ServerId` is expected → compiler rejects | Does not compile (verified by doc-test or compile-fail test) |

---

## PR3: ConfigSource Trait & TomlFileConfigSource

**Branch:** `config-mgr/03-config-source`  
**Base:** `config-mgr/02-types`  
**Size:** Medium (~250 lines)  
**Estimated Time:** 4–6 hours

### Purpose

Implement the persistence layer: the `ConfigSource` port trait (design doc Section 2.1) and its two implementations — `TomlFileConfigSource` (production, filesystem-backed) and `InMemoryConfigSource` (tests). This is the infrastructure adapter layer of the hexagonal architecture.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 3.1 | Define `ConfigSource` trait | Exactly as design doc Section 2.1: `load()`, `persist()`, `export()`. All return `Result`. `Send + Sync` bounds. | [x] `src/source.rs` |
| 3.2 | Implement `InMemoryConfigSource` | Holds `Mutex<AppConfig>`. `load()` clones the inner config. `persist()` applies the mutation and stores. `export()` serializes to TOML string. Used exclusively in tests. | [x] `src/source.rs` |
| 3.3 | Implement `TomlFileConfigSource` | Constructor takes `ConfigPaths`. `load()`: reads main TOML + priority rules TOML, merges into `AppConfig`. `persist()`: crash-safe writeback (atomic rename with truncation fallback). `export()`: serializes current config to TOML via `toml_edit` (preserves formatting). | [x] `src/source/toml_file.rs` |
| 3.4 | Implement validation in `load()` | After parsing, run validation rules from design doc Section 6.1: unique server ports, non-empty `model_path`, `ctx_size > 0`, `parallel_slots > 0`, `queue_default_timeout_secs > 0`. `fair_share` weights normalization (auto-correct + warn). Return `ConfigLoadError::ValidationError` on hard errors. | [x] `src/source/toml_file.rs` |
| 3.5 | Implement flock with timeout | `acquire_flock_with_timeout()`: `LOCK_NB` retry loop per design doc cross-cutting concerns. 100ms sleep between attempts, 30s deadline. Runs inside `spawn_blocking`. | [x] `src/source/toml_file.rs` |
| 3.6 | Implement atomic writeback | Phase 1: write to temp file (`state_dir/config.tmp`) → `fsync` → `rename` over original. Phase 2 (fallback on EXDEV/EACCES): truncate original → write → `fsync`. Use `toml_edit` to preserve comments/formatting: parse existing file, apply mutation to the document tree, serialize back. | [x] `src/source/toml_file.rs` |
| 3.7 | Implement mutation application | `fn apply_mutation(config: &mut AppConfig, mutation: &ConfigMutation)` — matches on `ConfigMutation` variants. For `SetLoraAdapters`: find server by ID, replace `lora_adapters` vec. Return error if server ID not found. | [x] `src/source/toml_file.rs` |
| 3.8 | Unit tests: InMemoryConfigSource | Load → modify → persist → load again → verify mutation applied. Test export produces valid TOML. | [x] `src/source.rs` (tests) |
| 3.9 | Integration tests: TomlFileConfigSource | Using `tempfile::TempDir`: (a) create valid TOML → `load()` → verify parsed config. (b) `persist()` with `SetLoraAdapters` → read file back → verify LoRA section updated. (c) `persist()` preserves TOML comments (write a TOML with `# this is a comment`, persist a mutation, verify comment survives). (d) Empty file → `load()` returns `EmptyFile` error. (e) Invalid TOML → `load()` returns `ParseError`. (f) Missing file → `load()` returns `NotFound`. | [x] `tests/toml_file_source.rs` |
| 3.10 | Integration tests: validation | (a) Duplicate server ports → `ValidationError`. (b) Empty `model_path` → `ValidationError`. (c) `ctx_size = 0` → `ValidationError`. (d) `fair_share` weights summing to 50 → auto-normalized to 100 + warning logged. | [x] `tests/validation.rs` |
| 3.11 | Integration tests: flock contention | Spawn two tasks that both try to `persist()` concurrently with flock. Verify both succeed (serialized) and final file contains both mutations. | [x] `tests/flock.rs` |

### PR3 Acceptance Criteria

- [x] `ConfigSource` trait compiles and is object-safe (`Arc<dyn ConfigSource>` works)
- [x] `InMemoryConfigSource` passes all unit tests
- [x] `TomlFileConfigSource` reads real TOML files, validates, and returns correct `AppConfig`
- [x] Writeback preserves TOML comments and formatting
- [x] Empty/missing/invalid files produce the correct `ConfigLoadError` variant
- [x] All 8 validation rules from design doc Section 6.1 are enforced
- [x] flock contention test passes without deadlock
- [x] No `unwrap()` or `expect()` in production code

### PR3 Creation Steps

```bash
git checkout -b config-mgr/03-config-source main
# ... implement tasks 3.1–3.11 ...
git add -A && git commit -m "config-mgr: add ConfigSource trait with TomlFile and InMemory impls"
git push -u origin config-mgr/03-config-source
# Open PR: "Config Manager PR3: ConfigSource Port Trait & Filesystem Adapter"
# Description: "Implements the persistence layer (design doc Section 2.1). Introduces
# ConfigSource trait, TomlFileConfigSource (atomic writeback, flock, toml_edit),
# and InMemoryConfigSource for testing. Includes validation rules (Section 6.1),
# flock timeout implementation (cross-cutting concerns), and crash-safe writeback
# (Section 3.2). 11 integration tests."
```

---

## PR4: ConfigHandle Core — Load, Snapshot, Subscribe, Reload

**Branch:** `config-mgr/04-config-handle`  
**Base:** `config-mgr/03-config-source`  
**Size:** Medium–Large (~300 lines)  
**Estimated Time:** 5–7 hours

### Purpose

Implement the central `ConfigHandle` struct (design doc Section 2.3) with its core lifecycle: construct via `load()`, serve snapshots via `snapshot()`, broadcast changes via `subscribe()`, and hot-reload via `reload()`. This is the domain logic layer — it delegates I/O to `ConfigSource` and owns the `ArcSwap` + `broadcast` + `reload_mutex` coordination.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 4.1 | Implement `ConfigHandle` struct | Fields: `source: Arc<dyn ConfigSource>`, `inner: ArcSwap<AppConfig>`, `reload_mutex: tokio::sync::Mutex<()>`, `change_tx: broadcast::Sender<Arc<ConfigDiff>>`. Broadcast channel capacity: 16. | [x] `src/handle.rs` |
| 4.2 | Implement `ConfigHandle::load()` | Constructor. Calls `source.load()`, wraps result in `ArcSwap::new(Arc::new(config))`. Creates broadcast channel. Returns `Result<Self, ConfigLoadError>`. This is the typestate enforcement — you can't have a `ConfigHandle` without a valid config. | [x] `src/handle.rs` |
| 4.3 | Implement `ConfigHandle::snapshot()` | Returns `arc_swap::Guard<Arc<AppConfig>>`. Single atomic load. | [x] `src/handle.rs` |
| 4.4 | Implement `ConfigHandle::subscribe()` | Returns `broadcast::Receiver<Arc<ConfigDiff>>`. Calls `self.change_tx.subscribe()`. | [x] `src/handle.rs` |
| 4.5 | Implement `ConfigHandle::reload()` | Acquire `reload_mutex`. Call `source.load()` inside `spawn_blocking` (delegate to source). Compute `diff(old, new)`. If diff is non-empty: `ArcSwap::store(new)`, broadcast `Arc<ConfigDiff>`. Return `ConfigDiff`. If source returns error: return `ConfigReloadError`, do NOT update ArcSwap, do NOT broadcast. | [x] `src/handle.rs` |
| 4.6 | Implement `ConfigHandle::export_toml()` | Delegates to `source.export(&current_snapshot)`. Returns `Result<String, WritebackError>`. | [x] `src/handle.rs` |
| 4.7 | Implement `ConfigReader` trait for `ConfigHandle` | Delegate `snapshot()` and `subscribe()`. | [x] `src/handle.rs` |
| 4.8 | Define `ConfigReader` and `ConfigWriter` traits | In a `traits.rs` module. `ConfigReader: Send + Sync` with `snapshot()` and `subscribe()`. `ConfigWriter: Send + Sync` with `async fn reload()` and `async fn writeback()`. | [x] `src/traits.rs` |
| 4.9 | Unit tests: load + snapshot | Using `InMemoryConfigSource`: load a config, `snapshot()` returns the loaded config. Verify snapshot is cheap (call 1000 times, should be < 1ms total). | [x] `src/handle.rs` (tests) |
| 4.10 | Unit tests: reload with diff broadcast | Load config A. Modify source to return config B (add a server). Call `reload()`. Verify: (a) `snapshot()` returns config B, (b) subscriber received `ConfigDiff` with `ServerAdded`, (c) `reload()` return value matches the broadcast diff. | [x] `src/handle.rs` (tests) |
| 4.11 | Unit tests: reload with no changes | Load config. Call `reload()` without changing source. Verify diff has empty `changes` vec. Subscriber still receives the diff (even if empty — design choice for simplicity). | [x] `src/handle.rs` (tests) |
| 4.12 | Unit tests: reload with parse error | Load config. Make source return `ParseError` on next `load()`. Call `reload()`. Verify: (a) returns `ConfigReloadError`, (b) `snapshot()` still returns the OLD valid config, (c) no diff broadcast. | [x] `src/handle.rs` (tests) |
| 4.13 | Unit tests: concurrent reloads serialized | Spawn 10 concurrent `reload()` tasks. Source has a 50ms delay. Verify all complete without panic. Verify final snapshot is consistent (last-writer-wins, but no mixed state). | [x] `src/handle.rs` (tests) |
| 4.14 | Unit tests: broadcast lagged subscriber | Create subscriber. Send 20 rapid reloads without consuming from receiver. Verify subscriber gets `RecvError::Lagged`. | [x] `src/handle.rs` (tests) |

### PR4 Acceptance Criteria

- [x] `ConfigHandle::load()` constructs successfully from `InMemoryConfigSource`
- [x] `snapshot()` returns the current config without blocking
- [x] `reload()` atomically swaps the config and broadcasts the diff
- [x] Failed reloads leave the old config intact and don't broadcast
- [x] Concurrent reloads are serialized (no mixed state, no panics)
- [x] Lagged subscribers get `RecvError::Lagged` (not a panic or deadlock)
- [x] `ConfigReader` and `ConfigWriter` traits compile and are object-safe
- [x] All 6 unit test cases pass

### PR4 Creation Steps

```bash
git checkout -b config-mgr/04-config-handle main
# ... implement tasks 4.1–4.14 ...
git add -A && git commit -m "config-mgr: implement ConfigHandle with load, snapshot, reload, and broadcast"
git push -u origin config-mgr/04-config-handle
# Open PR: "Config Manager PR4: ConfigHandle Core — Load, Snapshot, Subscribe, Reload"
# Description: "Implements the central ConfigHandle (design doc Section 2.3) with ArcSwap-based
# lock-free reads, mutex-serialized reloads, and broadcast-based change notification.
# Defines ConfigReader/ConfigWriter port traits (Section 2.2). 6 unit test suites
# covering happy path, error recovery, concurrency, and broadcast overflow."
```

---

## VC-2: Core Logic Verification Checkpoint

**Gate:** PR3 + PR4 merged. The full load → snapshot → reload → diff → broadcast pipeline works.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-2.1 | End-to-end reload | Write a TOML file → `TomlFileConfigSource::load()` → `ConfigHandle::load()` → modify TOML on disk → `handle.reload()` → verify `snapshot()` reflects changes and subscriber got diff | All assertions pass |
| VC-2.2 | Crash-safe writeback (file level) | Write TOML → persist mutation → kill process mid-write (simulate) → verify original file intact OR new file complete (never corrupt) | File is valid TOML after simulated crash |
| VC-2.3 | Concurrent reload stress | 50 concurrent `reload()` calls with 10ms source delay each → no panics, no deadlocks, final state consistent | Completes within 10s, `snapshot()` returns a valid config |
| VC-2.4 | Validation rejects bad config | TOML with duplicate ports, zero ctx_size, empty model_path → all rejected with appropriate `ConfigLoadError::ValidationError` | 3 separate rejection tests pass |
| VC-2.5 | Backward compatibility | Parse a "v0.9" TOML missing half the fields → all defaults populated, no errors | Parsed config has expected default values |

**Integration test file:** `tests/vc2_core_logic.rs`

Implement this as a dedicated integration test file that exercises the full stack (real files + ConfigHandle) in a single test binary. This is NOT a unit test — it tests the composition of PR3 and PR4 together.

---

## PR5: Writeback Protocol

**Branch:** `config-mgr/05-writeback`  
**Base:** `config-mgr/04-config-handle`  
**Size:** Medium (~200 lines)  
**Estimated Time:** 3–5 hours

### Purpose

Implement `ConfigHandle::writeback()` — the crash-safe mutation path used by the LoRA adapter API. This adds the write side to the ConfigHandle (design doc Section 2.3, data flow Section 3.2).

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 5.1 | Implement `ConfigHandle::writeback()` | Acquire `reload_mutex`. Delegate to `source.persist(current_config, mutation)`. If persist succeeds: apply mutation to in-memory config, `ArcSwap::store(new)`, compute diff, broadcast `Arc<ConfigDiff>`. If persist fails: return `WritebackError`, in-memory config unchanged. | [x] `src/handle.rs` |
| 5.2 | Implement `ConfigWriter` trait for `ConfigHandle` | Delegate `reload()` and `writeback()`. | [x] `src/handle.rs` |
| 5.3 | Handle read-only filesystem gracefully | In `TomlFileConfigSource::persist()`: detect read-only FS (check `fs::metadata().permissions().readonly()` before attempting write). Return `WritebackError::ReadOnly` with the path. | [x] `src/source/toml_file.rs` |
| 5.4 | Handle EXDEV fallback in persist | When `rename()` fails with `EXDEV` (cross-device) or `EACCES`: fall back to truncation path. Log warning about reduced crash safety. | [x] `src/source/toml_file.rs` |
| 5.5 | Unit tests: writeback happy path | `InMemoryConfigSource`: writeback `SetLoraAdapters` → verify snapshot updated, diff broadcast with `ServerChanged { fields: [LoraAdapters] }`, `hot_reloadable() == true`. | [x] `src/handle.rs` (tests) |
| 5.6 | Unit tests: writeback invalid server ID | Writeback with a non-existent `ServerId` → verify error returned, snapshot unchanged, no broadcast. | [x] `src/handle.rs` (tests) |
| 5.7 | Unit tests: writeback + reload serialized | Start a long-running reload (source has 100ms delay). Immediately call `writeback()`. Verify writeback waits for reload to complete (both succeed, no interleaving). | [x] `src/handle.rs` (tests) |
| 5.8 | Integration tests: filesystem writeback | Using `tempfile::TempDir`: (a) write TOML → writeback LoRA change → read file → verify LoRA section updated AND other config sections untouched. (b) Verify TOML comments preserved after writeback. (c) Read-only file → `WritebackError::ReadOnly`. | [x] `tests/writeback_integration.rs` |
| 5.9 | Integration tests: atomic rename verification | Write TOML, start writeback, verify temp file created in state_dir during write, verify temp file cleaned up after successful rename. | [x] `tests/writeback_integration.rs` |

### PR5 Acceptance Criteria

- [x] `writeback()` updates both disk and in-memory config atomically
- [x] Failed writeback leaves in-memory config unchanged
- [x] Writeback and reload are serialized by the shared mutex
- [x] Read-only filesystem returns `WritebackError::ReadOnly` (not a panic)
- [x] TOML comments survive writeback
- [x] All 5 unit + 3 integration tests pass

### PR5 Creation Steps

```bash
git checkout -b config-mgr/05-writeback main
# ... implement tasks 5.1–5.9 ...
git add -A && git commit -m "config-mgr: implement crash-safe writeback protocol"
git push -u origin config-mgr/05-writeback
# Open PR: "Config Manager PR5: Crash-Safe Writeback Protocol"
# Description: "Implements ConfigHandle::writeback() with crash-safe atomic rename
# (design doc Section 3.2). Handles read-only FS gracefully, EXDEV fallback to
# truncation, and serialization with reload via shared mutex. Preserves TOML
# formatting via toml_edit. 8 tests covering happy path, error cases, and concurrency."
```

---

## PR6: InstrumentedConfigHandle & CachedConfigReader

**Branch:** `config-mgr/06-instrumented`  
**Base:** `config-mgr/05-writeback`  
**Size:** Medium (~200 lines)  
**Estimated Time:** 3–4 hours

### Purpose

Add the observability decorator (design doc Section 2.5) and the hot-path cached reader (design doc Section 2.4). The decorator enforces the domain/observability boundary — all tracing and metrics live here, not in ConfigHandle.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 6.1 | Implement `InstrumentedConfigHandle` | Wraps `ConfigHandle`. Implements `ConfigReader` (delegates directly — NO instrumentation on hot path). Implements `ConfigWriter` with tracing spans and structured log events. | [x] `src/instrumented.rs` |
| 6.2 | Add reload instrumentation | `reload()`: parent span `config.reload` with fields `config.server_count`. Child spans for flock, parse, diff, store (emitted by structured events, not actual child spans — ConfigHandle doesn't expose internal stages). Record duration. Emit `config.reloaded` on success, `config.reload_failed` on error. Increment `config_reloads_total{result}` counter. Record `config_reload_duration_seconds` histogram. | [x] `src/instrumented.rs` |
| 6.3 | Add writeback instrumentation | `writeback()`: parent span `config.writeback` with fields `config.mutation_type`, `config.server_id`. Record duration. Emit `config.writeback` on success, `config.writeback_failed` on error. Increment `config_writebacks_total{result}` counter. Record `config_writeback_duration_seconds` histogram. | [x] `src/instrumented.rs` |
| 6.4 | Define Prometheus metrics | Use `prometheus` or `metrics` crate. Define: `config_reloads_total`, `config_reload_duration_seconds`, `config_writebacks_total`, `config_writeback_duration_seconds`, `config_flock_wait_seconds`, `config_broadcast_lagged_total`. All with bounded label sets per design doc Section 7.2. | [x] `src/instrumented.rs` |
| 6.5 | Implement `CachedConfigReader` | Wraps `arc_swap::cache::Cache<Arc<ArcSwap<AppConfig>>, Arc<AppConfig>>`. `new()` takes `&ConfigHandle` (accesses inner ArcSwap via a public accessor). `load()` returns `&Arc<AppConfig>` with thread-local caching. | [x] `src/cached.rs` |
| 6.6 | Add public accessor on ConfigHandle | `pub fn arc_swap(&self) -> &Arc<ArcSwap<AppConfig>>` — needed by `CachedConfigReader` to construct the cache. Alternatively, `impl ConfigHandle { pub fn cached_reader(&self) -> CachedConfigReader }`. | [x] `src/handle.rs` |
| 6.7 | Unit tests: InstrumentedConfigHandle | Use `tracing_subscriber::fmt::TestWriter` or `tracing::subscriber::with_default` to capture spans. Verify: (a) `reload()` emits `config.reload` span, (b) failed reload emits `config.reload_failed` event, (c) `snapshot()` does NOT emit any span (hot path). | [x] `src/instrumented.rs` (tests) |
| 6.8 | Unit tests: CachedConfigReader | Load config, create `CachedConfigReader`. Call `load()` 1000 times → verify all return same pointer (no ArcSwap atomic op per call when config unchanged). Reload config → call `load()` → verify returns new config. | [x] `src/cached.rs` (tests) |
| 6.9 | Re-export public API from lib.rs | `pub use handle::ConfigHandle;`, `pub use traits::{ConfigReader, ConfigWriter};`, `pub use instrumented::InstrumentedConfigHandle;`, `pub use cached::CachedConfigReader;`, `pub use types::*;`, `pub use error::*;`, `pub use diff::*;`, `pub use source::ConfigSource;`. | [x] `src/lib.rs` |

### PR6 Acceptance Criteria

- [x] `InstrumentedConfigHandle` implements both `ConfigReader` and `ConfigWriter`
- [x] `snapshot()` on `InstrumentedConfigHandle` has zero instrumentation overhead
- [x] `reload()` emits tracing spans and structured events
- [x] `CachedConfigReader::load()` avoids ArcSwap atomic ops when config hasn't changed
- [x] All Prometheus metrics defined with bounded label cardinality
- [x] Public API is cleanly re-exported from `lib.rs`
- [x] All unit tests pass

### PR6 Creation Steps

```bash
git checkout -b config-mgr/06-instrumented main
# ... implement tasks 6.1–6.9 ...
git add -A && git commit -m "config-mgr: add InstrumentedConfigHandle decorator and CachedConfigReader"
git push -u origin config-mgr/06-instrumented
# Open PR: "Config Manager PR6: Observability Decorator & Cached Reader"
# Description: "Implements InstrumentedConfigHandle (design doc Section 2.5) and
# CachedConfigReader (Section 2.4). All tracing/metrics confined to the decorator —
# core ConfigHandle remains telemetry-free. Defines 6 Prometheus metrics (Section 7.2).
# Clean public API re-exports from lib.rs."
```

---

## VC-3: Full Component Verification Checkpoint

**Gate:** PR1–PR6 merged. The Config Manager is feature-complete.

This is the most important checkpoint. It validates the entire Config Manager against real-world scenarios before declaring it ready for integration with other components.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-3.1 | Full lifecycle test | Create TOML on disk → `TomlFileConfigSource` → `ConfigHandle::load()` → `InstrumentedConfigHandle::new()` → `snapshot()` returns valid config → modify TOML → `reload()` → subscriber gets diff → `writeback(SetLoraAdapters)` → read TOML back → LoRA change persisted | All assertions pass in a single integration test |
| VC-3.2 | Startup failure: empty file | Create 0-byte TOML → `load()` fails with `EmptyFile` | Error variant matches |
| VC-3.3 | Startup failure: invalid TOML | Create TOML with syntax error → `load()` fails with `ParseError` | Error variant matches, error message contains line number |
| VC-3.4 | Startup failure: validation error | Create TOML with duplicate server ports → `load()` fails with `ValidationError` | Error variant matches, message mentions "duplicate" |
| VC-3.5 | Hot reload: add server | Start with 1 server → add server to TOML → `reload()` → diff has `ServerAdded` → subscriber receives it | Diff correct, snapshot updated |
| VC-3.6 | Hot reload: remove server | Start with 2 servers → remove one from TOML → `reload()` → diff has `ServerRemoved` | Diff correct, snapshot updated |
| VC-3.7 | Hot reload: change requiring restart | Change `ctx_size` → `reload()` → diff has `ServerChanged` with `requires_restart() == true` | Classification correct |
| VC-3.8 | Hot reload: change NOT requiring restart | Change `lora_adapters` → `reload()` → diff has `ServerChanged` with `hot_reloadable() == true` | Classification correct |
| VC-3.9 | Hot reload: parse error preserves old config | Corrupt TOML → `reload()` → returns error → `snapshot()` still returns old valid config | Old config preserved |
| VC-3.10 | Writeback: crash safety | Write TOML → start writeback → verify temp file exists → after completion verify only final file exists, temp cleaned up | No orphaned temp files |
| VC-3.11 | Writeback: read-only FS | Make TOML read-only (`chmod 444`) → `writeback()` → returns `ReadOnly` error | Graceful error, no panic |
| VC-3.12 | Concurrent stress test | 20 concurrent reloads + 10 concurrent writebacks → no panics, no deadlocks, final state consistent | Completes within 30s |
| VC-3.13 | Observability verification | Run a reload through `InstrumentedConfigHandle` → capture tracing output → verify `config.reload` span present with correct structured fields | Span fields match design doc Section 7.1 |
| VC-3.14 | CachedConfigReader consistency | Reload 100 times while CachedConfigReader is reading → no stale reads, no panics | All reads return a valid snapshot |
| VC-3.15 | Memory bounds | Load config with 100 servers, each with 50 LoRA adapters → measure memory → verify < 1MB | Assertion on `std::mem::size_of_val` or similar |

**Integration test file:** `tests/vc3_full_component.rs`

This file contains 15 test functions, one per check. They all use `tempfile::TempDir` for filesystem isolation. They construct the full stack: `TomlFileConfigSource` → `ConfigHandle` → `InstrumentedConfigHandle`.

---

## PR7: Property-Based Tests & VC-3 Integration Tests

**Branch:** `config-mgr/07-verification`  
**Base:** `config-mgr/06-instrumented`  
**Size:** Medium (~300 lines of tests)  
**Estimated Time:** 4–6 hours

### Purpose

The final PR adds the property-based tests (design doc Section 8.3), implements all 15 VC-3 integration tests, and adds the specific test cases from design doc Section 8.4. This PR is pure test code — no production logic changes.

### Tasks

| # | Task | Detail | File(s) |
|---|------|--------|---------|
| 7.1 | Property test: serde roundtrip | `proptest! { fn roundtrip(config in arb_app_config()) { assert_eq!(parse(serialize(config)), config) } }`. Requires implementing `Arbitrary` for `AppConfig` (derive or manual strategy). | `tests/proptest_config.rs` |
| 7.2 | Property test: backward compat | Generate full TOML via `toml::to_string`, then randomly delete keys before re-parsing. Verify parse succeeds with defaults for all deleted fields. | `tests/proptest_config.rs` |
| 7.3 | Property test: diff correctness | `proptest! { fn diff_correct(old in arb_app_config(), new in arb_app_config()) { let diff = compute_diff(&old, &new); assert!(apply_diff_produces_new(&old, &diff, &new)) } }`. Note: `apply_diff` is a test-only helper, not a production function. The assertion is that the diff correctly describes the delta. | `tests/proptest_config.rs` |
| 7.4 | Implement `arb_app_config()` strategy | proptest strategy that generates random `AppConfig` instances with 0–5 servers, random ports (1024–65535), random field values. Keep it bounded to avoid slow tests. | `tests/proptest_config.rs` |
| 7.5 | Implement VC-3 integration tests | All 15 checks from VC-3 above. Each as a separate `#[tokio::test]` function. All use `tempfile::TempDir`. | `tests/vc3_full_component.rs` |
| 7.6 | Implement design doc Section 8.4 tests | `test_concurrent_sighups_serialized`, `test_writeback_atomic_rename`, `test_empty_config_at_startup_exits`, `test_serde_default_backward_compat`, `test_broadcast_lagged_subscriber`, `test_readonly_fs_writeback_graceful`. These overlap with VC-3 but use the exact test signatures from the design doc. | `tests/design_doc_tests.rs` |
| 7.7 | Add `#[ignore]` slow tests | Tag property-based tests and the 100-server memory test with `#[ignore]` so they don't run in normal `cargo test`. Run with `cargo test -- --ignored`. | — |
| 7.8 | Final CI check | `cargo test --workspace`, `cargo test --workspace -- --ignored`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --check`. | — |

### PR7 Acceptance Criteria

- [ ] All 3 property-based tests pass (with reasonable iteration counts: 256 cases each)
- [ ] All 15 VC-3 integration tests pass
- [ ] All 6 design doc Section 8.4 test cases pass
- [ ] `cargo test --workspace` passes (excluding `#[ignore]` tests)
- [ ] `cargo test --workspace -- --ignored` passes (including slow tests)
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] No production code changes in this PR — test code only

### PR7 Creation Steps

```bash
git checkout -b config-mgr/07-verification main
# ... implement tasks 7.1–7.8 ...
git add -A && git commit -m "config-mgr: add property tests, VC-3 integration tests, and design doc test cases"
git push -u origin config-mgr/07-verification
# Open PR: "Config Manager PR7: Property Tests & Full Component Verification"
# Description: "Pure test PR. Adds 3 proptest property-based tests (design doc Section 8.3),
# 15 VC-3 integration tests covering the full component lifecycle, and 6 specific
# test cases from design doc Section 8.4. Total: 24 new test functions.
# No production code changes."
```

---

## VC-4: Final Sign-Off Checkpoint

**Gate:** All 7 PRs merged. Config Manager is complete.

| # | Check | Method | Pass Criteria |
|---|-------|--------|---------------|
| VC-4.1 | All tests pass | `cargo test --workspace && cargo test --workspace -- --ignored` | Zero failures |
| VC-4.2 | Zero clippy warnings | `cargo clippy --workspace -- -D warnings` | Exit code 0 |
| VC-4.3 | Code coverage | Run `cargo llvm-cov --workspace` (or `tarpaulin`). Config Manager should have > 85% line coverage. | Coverage report generated |
| VC-4.4 | Public API audit | Review `lib.rs` re-exports. Verify only intended types are public. No internal implementation details leak. | Manual review |
| VC-4.5 | Design doc compliance | Cross-check every Section in `01_CONFIG_MANAGER.md` against the implementation. Create a checklist of all specified behaviors and verify each is implemented. | Manual review with checklist |
| VC-4.6 | Documentation | Every public type, trait, method, and enum variant has a doc comment. `cargo doc --no-deps -p llamaforge-config` builds without warnings. | `cargo doc` succeeds |
| VC-4.7 | Dependency audit | `cargo deny check` passes. No duplicate dependency versions. No AGPL/GPL dependencies (if licensing requires MIT/Apache). | `cargo deny` green |
| VC-4.8 | Binary size check | `cargo build --release -p llamaforge`. Binary exists and is reasonable size (< 20MB). | Build succeeds |

---

## Summary: PR → File Mapping

| PR | Key Files Created/Modified | LOC (est.) |
|----|---------------------------|------------|
| PR1 | `Cargo.toml`, `crates/*/Cargo.toml`, `examples/*.toml`, `.cargo/config.toml` | ~150 |
| PR2 | `src/types.rs`, `src/error.rs`, `src/diff.rs`, `tests/parse_examples.rs` | ~400 |
| PR3 | `src/source.rs`, `src/source/toml_file.rs`, `tests/toml_file_source.rs`, `tests/validation.rs`, `tests/flock.rs` | ~250 |
| PR4 | `src/handle.rs`, `src/traits.rs` | ~300 |
| PR5 | `src/handle.rs` (additions), `tests/writeback_integration.rs` | ~200 |
| PR6 | `src/instrumented.rs`, `src/cached.rs`, `src/lib.rs` (re-exports) | ~200 |
| PR7 | `tests/proptest_config.rs`, `tests/vc3_full_component.rs`, `tests/design_doc_tests.rs` | ~300 |
| **Total** | | **~1,800** |

---

## Timeline (Single Engineer)

| Day | Work | PRs |
|-----|------|-----|
| Day 1 | Project scaffold + all types | PR1, PR2 |
| Day 2 | ConfigSource + TomlFileConfigSource | PR3 |
| Day 3 | ConfigHandle core + reload | PR4 |
| Day 4 | Writeback protocol + VC-2 | PR5 |
| Day 5 | Instrumented + cached + VC-3 setup | PR6 |
| Day 6 | Property tests + all VC-3 integration tests | PR7 |
| Day 7 | Final verification (VC-4), code review fixes, docs | — |

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `toml_edit` comment preservation is lossy for edge cases | Medium | Low | Accept minor formatting drift; document in operator guide. Test with realistic configs. |
| `flock` semantics differ on macOS vs Linux | Low | Medium | CI runs on Linux. macOS dev testing with integration tests. `nix` crate abstracts platform differences. |
| Property tests are slow | Medium | Low | Tag with `#[ignore]`. Run in CI only, not local dev loop. |
| `ArcSwap` version incompatibility with future deps | Low | Low | Pin `arc-swap = "1"` in workspace. |
| Cross-device rename (EXDEV) hard to test without mount namespaces | Medium | Medium | Mock `rename` to return `EXDEV` in test, or use `cfg(test)` feature flag to inject error. |

---

## Integration Points (for next component)

After Config Manager is complete, the following integration seams are ready for consumers:

1. **Server Manager** will depend on `ConfigReader` trait for `snapshot()` and `subscribe()`.
2. **Transport Layer** will depend on `ConfigWriter` trait for `reload()` (SIGHUP handler) and `writeback()` (LoRA API).
3. **Request Queue & Router** will use `CachedConfigReader` for hot-path config reads.
4. **`main.rs`** will construct: `ConfigPaths` → `TomlFileConfigSource` → `ConfigHandle::load()` → `InstrumentedConfigHandle::new()` → inject as `Arc<dyn ConfigReader>` / `Arc<dyn ConfigWriter>`.