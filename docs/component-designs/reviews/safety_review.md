# Safety Review: Config Manager (01_CONFIG_MANAGER.md)

**Reviewer focus:** Runtime safety and performance anti-patterns  
**Rules evaluated:** Rule 21 (No unwrap/expect), Rule 22 (No blocking in async), Rule 23 (Borrowing over cloning)  
**Document reviewed:** `component-designs/01_CONFIG_MANAGER.md`  
**Date:** 2026-05-07

---

## Summary

The Config Manager design is strong on async safety. The reload and writeback paths correctly use `spawn_blocking` for all filesystem I/O and `flock`, and the concurrency primitive choices (`tokio::sync::Mutex`, `ArcSwap`, broadcast channel) are appropriate. The design document explicitly states invariant 6 ("No blocking on reactor") and backs it up with correct architecture. However, there are several findings ranging from medium to low severity, primarily around cloning discipline and a subtle `reload_mutex` holding pattern that deserves attention during implementation.

**Findings:** 5  
**Commendations:** 6

---

## Findings

### F-1: `reload_mutex` held across `spawn_blocking().await`

| Field | Value |
|-------|-------|
| **Rule** | 22 — No Blocking in Async Contexts |
| **Severity** | MEDIUM |
| **Location** | Section 3.1 Data Flow diagram (lines 250-274) and Section 3.2 Writeback Data Flow (lines 278-301) |

**Violation:** The data flow diagram shows `reload_mutex.lock().await` acquired *before* entering `spawn_blocking`, and the mutex is held across the `spawn_blocking(...).await` call. While `reload_mutex` is correctly a `tokio::sync::Mutex` (not `std::sync::Mutex`), holding it across a potentially long `spawn_blocking` call that includes `flock` acquisition (with a 30-second timeout per Section 6) means other reload/writeback callers block on the tokio mutex for up to 30+ seconds.

This is not a Rule 22 *violation* per se (tokio::sync::Mutex across .await is fine), but it is a design concern: if the flock blocks for 30 seconds, the reload_mutex is held for 30 seconds, and all other reload/writeback requests queue behind it. The cancellation safety analysis in the Cross-Cutting section (lines 555-558) correctly notes that a cancelled spawn_blocking runs to completion, but the mutex guard would be dropped on cancellation — meaning flock proceeds without serialization protection.

**Suggested fix:** Consider moving the `reload_mutex` acquisition *inside* the `spawn_blocking` closure (using a `std::sync::Mutex` instead for the blocking context), or accept the current design with a documented note that cancellation of a reload while waiting on the mutex is safe but cancellation after mutex acquisition and before spawn_blocking completion will cause the blocking task to run unserialized. The current design is acceptable if the implementation ensures the mutex guard is not dropped until spawn_blocking completes — verify this in code review.

---

### F-2: `AppConfig` derives `Clone` with `HashMap` fields — hot-path cloning risk

| Field | Value |
|-------|-------|
| **Rule** | 23 — Borrowing Over Cloning |
| **Severity** | MEDIUM |
| **Location** | Section 2.3, lines 89-99 (`AppConfig` struct definition) |

**Violation:** `AppConfig` derives `Clone` and contains `HashMap<ServerId, ServerConfig>`, `HashMap<String, u32>`, and other collection types. If any consumer calls `.clone()` on `AppConfig` in a hot path (e.g., per-request in the dispatch loop), this allocates and copies all nested HashMaps, Vecs, and Strings.

The `CachedConfigReader` (Section 2.2) correctly returns `&Arc<AppConfig>`, which avoids cloning. However, the `snapshot()` method (line 45) returns `arc_swap::Guard<Arc<AppConfig>>`, and consumers may be tempted to call `.clone()` on the inner `Arc<AppConfig>` or dereference and clone the `AppConfig` itself.

**Suggested fix:** Add a design note warning implementors that `AppConfig` must never be cloned in hot paths. Consumers should hold `Arc<AppConfig>` (cheap ref-count bump) or borrow via the Guard. Consider whether `Clone` on `AppConfig` can be removed entirely and replaced with `Arc::clone` at all usage sites. If `Clone` is needed for diff computation, document that diff is a cold path.

---

### F-3: `export_toml(&self) -> String` — potential `unwrap` on serialization

| Field | Value |
|-------|-------|
| **Rule** | 21 — No unwrap()/expect() in Production Code |
| **Severity** | MEDIUM |
| **Location** | Section 2.1, line 65 (`export_toml` method signature) |

**Violation:** `export_toml` returns `String`, not `Result<String, ...>`. TOML serialization can fail (e.g., if a value contains characters that can't be represented in TOML, or if `toml_edit` encounters an edge case). The implementation will likely need to call `toml::to_string(&config)` or equivalent, which returns `Result`. To return a bare `String`, the implementation must `unwrap()` or `expect()` internally.

While serialization failure on a config that was originally parsed from TOML is unlikely, it is not impossible — especially after runtime mutations (writeback) introduce values that weren't originally parsed.

**Suggested fix:** Change signature to `pub fn export_toml(&self) -> Result<String, toml::ser::Error>`. The `WritebackError` enum already has a `SerializeError` variant (line 216), showing this failure mode is recognized elsewhere.

---

### F-4: `ConfigDiff` derives `Clone` with `Vec<String>` in `ServerConfigDiff`

| Field | Value |
|-------|-------|
| **Rule** | 23 — Borrowing Over Cloning |
| **Severity** | LOW |
| **Location** | Section 2.5, lines 226-241 (`ConfigDiff` and `ServerConfigDiff` structs) |

**Violation:** `ConfigDiff` derives `Clone` and is sent through a `broadcast` channel. `broadcast::Sender::send` requires `Clone` on the message type because each receiver gets its own copy. `ServerConfigDiff::fields_changed` is `Vec<String>`, which means each broadcast clone allocates N strings.

This is a cold path (reloads are infrequent), so the severity is LOW. However, with the broadcast channel capacity of 16 and potential for multiple subscribers, unnecessary allocation pressure exists.

**Suggested fix:** Wrap `ConfigDiff` in `Arc` before sending: `broadcast::Sender<Arc<ConfigDiff>>`. `Arc::clone()` is a ref-count bump (Rule 23 explicitly allows this). Subscribers receive `Arc<ConfigDiff>` and can borrow the contents. Alternatively, use `&'static str` or an enum for `fields_changed` instead of `Vec<String>`.

---

### F-5: `load()` returns `Result` but design says "fails hard" — verify no `expect` in callers

| Field | Value |
|-------|-------|
| **Rule** | 21 — No unwrap()/expect() in Production Code |
| **Severity** | LOW |
| **Location** | Section 2.1, line 41 (comment: "Fails hard on parse error (startup only)") |

**Violation:** The comment "Fails hard on parse error (startup only)" combined with `load()` returning `Result<Self, ConfigLoadError>` suggests that `main()` will call `load().expect("...")` or similar. This is acceptable per Rule 21 (descriptive expect in main() init is LOW severity), but the design should make the intent explicit.

**Suggested fix:** Add a design note that `main()` is the only caller of `load()`, and that it should use `.expect("config load failed: check file paths and TOML syntax")` with a descriptive message — or better, print a structured error and `std::process::exit(1)` for a clean user-facing error.

---

## Commendations

### C-1: Correct use of `spawn_blocking` for all filesystem I/O

**Location:** Section 3.1, 3.2, and Invariant 6 (line 331)

The design correctly identifies that `flock`, `read`, `write`, `fsync`, and `rename` are all blocking syscalls and wraps every one in `spawn_blocking`. This is textbook Rule 22 compliance. The invariant is explicitly stated, making it auditable during implementation.

### C-2: `tokio::sync::Mutex` instead of `std::sync::Mutex`

**Location:** Section 2.1, line 35 (`reload_mutex: tokio::sync::Mutex<()>`)

The design correctly uses `tokio::sync::Mutex` for the reload serialization lock, which is safe to hold across `.await` points. Using `std::sync::Mutex` here would be a Rule 22 violation (blocking the reactor while waiting for the lock). This is a common mistake in async Rust; the design avoids it.

### C-3: `ArcSwap` for lock-free concurrent reads

**Location:** Section 2.1, Section 2.2, and Invariant 2 (line 326)

`ArcSwap` provides atomic pointer swaps for config updates and lock-free reads for consumers. This eliminates any `RwLock` contention on the hot path. The `CachedConfigReader` further amortizes even the atomic load cost. This is excellent for a config that is read thousands of times per second but written once per SIGHUP.

### C-4: `CachedConfigReader` returns `&Arc<AppConfig>` — zero-clone hot path

**Location:** Section 2.2, line 83

The `load()` method returns `&Arc<AppConfig>`, which is a borrow of the cached Arc. No allocation, no ref-count bump, no clone. This is ideal Rule 23 compliance for the hottest path in the system (per-request config access in the dispatch loop).

### C-5: Comprehensive error types with no catch-all variants

**Location:** Section 2.4, lines 177-217

`ConfigLoadError`, `ConfigReloadError`, and `WritebackError` have specific, descriptive variants for every failure mode. There is no `Other(String)` or `Unknown` variant that would tempt implementors to use `unwrap()` and convert to a generic error. Each variant carries structured context (path, source error), enabling proper error propagation with `?`.

### C-6: Cancellation safety analysis is thorough and correct

**Location:** Cross-Cutting Concerns, lines 555-559

The analysis of cancellation at each `.await` point is precise. The observation that a cancelled `spawn_blocking` still runs to completion (and the file is the source of truth) is an important subtlety that many designs miss. This prevents a class of bugs where cancelled operations leave inconsistent state.

---

## Verification Checklist for Implementation Review

The following items should be verified during code review of the implementation:

- [ ] No `unwrap()` or `expect()` in `reload()`, `writeback()`, or `export_toml()` implementations
- [ ] `export_toml()` signature changed to return `Result` (per F-3)
- [ ] `spawn_blocking` closures do not capture the `tokio::sync::Mutex` guard (per F-1)
- [ ] No `.clone()` on `AppConfig` in hot paths — only `Arc::clone()` or borrows
- [ ] `ConfigDiff` sent as `Arc<ConfigDiff>` through broadcast channel (per F-4)
- [ ] `load()` caller in `main()` uses descriptive error handling, not bare `.unwrap()`
- [ ] `CachedConfigReader::load()` implementation returns a borrow, not a clone
- [ ] All `flock`, `read`, `write`, `fsync`, `rename` calls are inside `spawn_blocking`
- [ ] No `std::sync::Mutex` or `std::sync::RwLock` held across `.await` points
