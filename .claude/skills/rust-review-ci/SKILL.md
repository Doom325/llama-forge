---
name: rust-review-ci
description: >
  Reviews Rust build configuration and CI pipeline for completeness and optimization. Checks
  3 rules covering optimized build toolchain (mold linker, cranelift, sccache, cargo-nextest),
  comprehensive CI verification beyond cargo test (clippy, fmt, audit, miri, cross-compilation),
  and supply chain auditing of third-party dependencies. Use this skill whenever reviewing
  Cargo.toml build profiles, CI configuration files (.github/workflows, .gitlab-ci.yml, etc.),
  build.rs scripts, or dependency decisions. Trigger on any Rust review involving CI, build
  speed, dependency auditing, or pipeline configuration — even if the user just says "check
  our CI" or "is the build setup good."
---

# Rust Build & CI Pipeline Review

You are a sub-agent reviewer. Your job is to audit the build configuration and CI pipeline of a Rust project against 3 rules that ensure fast builds, comprehensive verification, and supply chain safety.

## How to review

1. Read the workspace `Cargo.toml` — check `[profile.dev]` and `[profile.release]` settings.
2. Find and read CI configuration files (`.github/workflows/*.yml`, `.gitlab-ci.yml`, `Makefile`, `justfile`, etc.).
3. Check for `.cargo/config.toml` — this is where linker and build settings often live.
4. Scan for `build.rs` scripts in any crate and review their contents.
5. Examine the dependency tree for supply chain concerns.
6. Write `review_report.md` and `inline_comments.md` per `references/output_format.md`.

## The Rules

### Rule 30 — Optimized Build Toolchain

Enterprise Rust projects should employ build optimization. Development builds and CI both suffer when using default, unoptimized Cargo settings. The specific optimizations to check for:

- **Alternative linker** — `mold` (Linux) or `lld` for local development. The default `ld` linker is significantly slower for large projects. Check `.cargo/config.toml` for `[target.x86_64-unknown-linux-gnu] linker = "clang" rustflags = ["-C", "link-arg=-fuse-ld=mold"]` or similar.
- **Cranelift backend** — For debug builds, the Cranelift codegen backend compiles faster than LLVM at the cost of runtime performance. Check for `[profile.dev] codegen-backend = "cranelift"` or `CARGO_PROFILE_DEV_CODEGEN_BACKEND=cranelift`.
- **Distributed caching** — `sccache` for CI pipelines to avoid recompiling unchanged crates across pipeline runs. Check CI config for `RUSTC_WRAPPER=sccache` or equivalent.
- **Parallel test execution** — `cargo-nextest` runs tests significantly faster than `cargo test` through parallel process execution and better failure reporting.

**What to look for:**
- Default `[profile.dev]` with no optimizations.
- CI pipelines rebuilding everything from scratch on every run (no caching strategy).
- `cargo test` instead of `cargo nextest run` in CI.
- Missing `.cargo/config.toml` entirely.

### Rule 31 — CI Verification Beyond `cargo test`

A CI pipeline that only runs `cargo test` is incomplete. Comprehensive verification requires multiple layers:

| Check | Command | Purpose |
|-------|---------|---------|
| Linting | `cargo clippy -- -D warnings` | Catch idiom violations, treat warnings as errors |
| Formatting | `cargo fmt --check` | Enforce consistent style |
| Vulnerability scan | `cargo audit` | Detect known vulnerabilities in dependencies |
| Undefined behavior | `cargo +nightly miri test` | Detect UB in unsafe code |
| Memory profiling | Valgrind under realistic workloads | Catch leaks and invalid access |

For multi-target projects (embedded, WASM, ARM64 serverless), CI must also:
- Manage cross-compilation target triples systematically.
- Verify builds against each target, not just the host.
- Review `build.rs` scripts for correctness and supply-chain risk (they run arbitrary code at compile time).

**What to look for:**
- CI config with only `cargo test` and nothing else.
- Missing `cargo clippy` or clippy without `-D warnings` (warnings that aren't errors get ignored).
- No `cargo fmt --check` — formatting drift accumulates fast without enforcement.
- No `cargo audit` — vulnerable dependencies ship silently.
- `unsafe` code present but no Miri in CI.
- Multi-target projects with builds only for the host platform.
- `build.rs` scripts that download or execute external resources without review.

### Rule 32 — Supply Chain Auditing

All third-party dependencies must be periodically audited. The Rust ecosystem's ease of adding dependencies (`cargo add foo`) makes it easy to accumulate risk.

**What to look for:**
- Dependencies that are unmaintained (check last commit date, open issues, archived status).
- Excessively broad feature flags — e.g., pulling in `tokio` with `features = ["full"]` when only `["rt-multi-thread", "macros"]` is needed. Unnecessary features increase compile time and attack surface.
- Duplicate functionality — two crates doing the same thing (e.g., both `reqwest` and `hyper` as HTTP clients, or both `serde_json` and `simd-json` for JSON parsing without clear justification).
- `build.rs` scripts in dependencies that run arbitrary code at compile time — these are the highest-risk supply chain vector in Rust.
- Missing `cargo-deny` or equivalent for automated license and advisory checking.
- No `Cargo.lock` committed (for binary crates — library crates intentionally omit it).

## Severity calibration

- **HIGH:** Rule 31 with no clippy or no audit in CI, Rule 32 with known vulnerable dependencies, `build.rs` scripts that download external code
- **MEDIUM:** Rule 30 (no build optimization in CI), Rule 31 (missing fmt check, no Miri for unsafe code), Rule 32 (overly broad feature flags)
- **LOW:** Rule 30 (missing local dev optimizations — impacts DX not correctness), Rule 32 (minor duplicate functionality)

## Output

After reviewing, produce both files per `references/output_format.md`. Read that file first if you haven't already.
