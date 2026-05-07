---
name: rust-review-safety
description: >
  Reviews Rust code for runtime safety and performance anti-patterns. Checks 3 critical rules:
  no unwrap()/expect() in production code paths, no blocking operations inside async contexts,
  and disciplined borrowing over gratuitous cloning. Use this skill whenever reviewing Rust code
  for reliability, panic-freedom, async correctness, or memory efficiency. Trigger on any Rust
  review involving error handling, async code, performance, .clone() usage, or production readiness —
  even if the user just says "is this code production-safe" or "check for panics."
---

# Rust Runtime Safety & Performance Review

You are a sub-agent reviewer. Your job is to audit Rust code against 3 rules that protect runtime reliability and performance. These rules target the most common production defects in Rust services: panics from unwrap, blocked async executors, and unnecessary allocations.

## How to review

1. Use `grep` / search for `unwrap()`, `expect(`, `.clone()`, `spawn_blocking`, `block_on` to find hotspots fast.
2. Identify all `async fn` definitions and check what runs inside them.
3. For clone analysis, focus on hot paths (request handlers, tight loops, data pipelines) — occasional clones in setup code are acceptable.
4. Write `review_report.md` and `inline_comments.md` per `references/output_format.md`.

## The Rules

### Rule 21 — No `unwrap()` or `expect()` in Production Code

Every `unwrap()` and `expect()` in non-test, non-prototype code is a potential thread panic. Rust's `Result`-based error handling exists precisely to transform failures into values that propagate gracefully via `?`. An `unwrap()` on a production code path circumvents this, turning a recoverable situation into a crash.

**Acceptable exceptions:**
- Test code (`#[cfg(test)]` modules, `#[test]` functions)
- One-time initialization in `main()` where failure means the program can't start at all (and even here, a descriptive `.expect("reason")` with a clear message is marginal)
- Cases where the programmer can *prove* the value is always `Some`/`Ok` — but these should carry a `// SAFETY:` comment explaining the invariant, same as `unsafe`

**What to look for:**
- `.unwrap()` and `.expect(` in any file outside `tests/` or `#[cfg(test)]` blocks.
- `.unwrap()` inside request handlers, service methods, data processing pipelines.
- Chained unwraps like `map.get(key).unwrap().parse::<u64>().unwrap()` — double hazard.
- `Option::unwrap()` on user-supplied data or external input.

**The fix pattern:** Replace with `?` propagation, `unwrap_or_default()`, `unwrap_or_else(|| ...)`, or explicit `match`/`if let` handling.

### Rule 22 — No Blocking in Async Contexts

Any blocking I/O operation (file reads, synchronous database calls, CPU-heavy computation) inside an `async fn` blocks the executor's reactor thread. In a Tokio-style runtime, this degrades throughput for the entire service — one blocked task starves all other tasks on that thread.

Blocking work must be offloaded to a dedicated thread pool via `tokio::task::spawn_blocking` or equivalent.

**What to look for:**
- `std::fs::read`, `std::fs::write`, or any `std::fs` call inside an `async fn` (use `tokio::fs` instead).
- `std::thread::sleep` inside async code (use `tokio::time::sleep`).
- Synchronous HTTP calls (e.g., `reqwest::blocking::get`) inside async functions.
- CPU-intensive computation (cryptographic hashing, JSON parsing of large payloads, image processing) directly in an async handler without `spawn_blocking`.
- `std::io::Read` / `std::io::Write` trait usage in async contexts.
- Mutex from `std::sync::Mutex` held across `.await` points (use `tokio::sync::Mutex` or restructure).

**The fix pattern:** Wrap blocking work in `tokio::task::spawn_blocking(move || { ... }).await` or use the async equivalent from `tokio::fs`, `tokio::time`, etc.

### Rule 23 — Borrowing Over Cloning

Gratuitous `.clone()` on heap-allocated types (`String`, `Vec`, `HashMap`, `Arc` contents) wastes allocator bandwidth and cache lines. Rust's ownership model provides deterministic, GC-free memory management — the discipline is to work *with* it through borrowing rather than fighting it with clones.

**Acceptable clones:**
- `Arc::clone()` for shared ownership across threads (this is a reference count bump, not a deep copy)
- Cloning small `Copy`-like types or types where Clone is cheap by design
- Clones in cold paths (startup, configuration loading) where readability matters more than allocation count
- Clones justified by a comment explaining why borrowing isn't feasible

**What to look for:**
- `.clone()` on `String`, `Vec<T>`, `HashMap<K, V>` inside loops or hot paths.
- Functions that take `String` parameters instead of `&str` (forcing callers to clone).
- `.to_string()` or `.to_owned()` where a reference would suffice.
- Cloning data out of a structure just to pass it to a function that only needs to read it.
- `let x = self.field.clone(); do_something(x);` where `do_something(&self.field)` would work.

**The fix pattern:** Use references (`&T`), slices (`&[T]`, `&str`), `Cow<'_, T>` for conditionally owned data, or `Arc<T>` for shared ownership across threads.

## Severity calibration

- **HIGH:** Rule 21 with `unwrap()` on external/user input in production handlers, Rule 22 with `std::fs` or `std::thread::sleep` inside async request handlers
- **MEDIUM:** Rule 21 with `expect()` in non-critical paths, Rule 23 with clones in hot loops, Rule 22 with CPU-bound work in async without spawn_blocking
- **LOW:** Rule 23 with clones in cold paths, Rule 21 in initialization code with descriptive `.expect()`

## Output

After reviewing, produce both files per `references/output_format.md`. Read that file first if you haven't already.
