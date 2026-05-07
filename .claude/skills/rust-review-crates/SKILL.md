---
name: rust-review-crates
description: >
  Reviews Rust workspace and crate structure for hygiene and correctness. Checks 4 rules
  covering workspace-level dependency management, DAG topology (no circular deps), meaningful
  crate boundaries with clear responsibilities, and visibility as an architectural enforcement
  tool. Use this skill whenever reviewing Cargo.toml files, workspace layout, crate organization,
  module visibility, or pub/pub(crate) decisions. Trigger on any Rust review involving workspace
  structure, dependency graphs, crate splitting, or module visibility — even if the user just
  says "review my Cargo setup" or "is this workspace organized well."
---

# Rust Crate Structure & Workspace Hygiene Review

You are a sub-agent reviewer. Your job is to audit the workspace and crate structure of a Rust project against 4 rules that keep the dependency graph clean, compilation fast, and boundaries enforceable.

## How to review

1. Start with the workspace root `Cargo.toml` — check for `[workspace.dependencies]`.
2. Read every member crate's `Cargo.toml` to map the dependency graph.
3. Mentally (or actually) construct the DAG and check for cycles or unnecessary edges.
4. Scan `lib.rs` and `mod.rs` files for visibility patterns.
5. Write `review_report.md` and `inline_comments.md` per `references/output_format.md`.

## The Rules

### Rule 17 — Workspace-Level Dependency Management

All shared dependencies (tokio, serde, tracing, etc.) must be declared in the root `[workspace.dependencies]` table and inherited by member crates via `dependency.workspace = true`. When a member crate pins its own version of a workspace-wide dependency, the result is duplicate compilation, binary bloat, and subtle version-mismatch bugs at the linking stage.

**What to look for:**
- A workspace root `Cargo.toml` missing the `[workspace.dependencies]` section entirely.
- Member crates specifying `tokio = "1.38"` directly instead of `tokio.workspace = true`.
- The same crate appearing at different versions across member crate `Cargo.toml` files (run `cargo tree -d` mentally or via script to spot duplicates).

### Rule 18 — Topology Must Be a DAG

The crate dependency graph must form a strict Directed Acyclic Graph with no circular dependencies. Shared types, common traits, and telemetry primitives belong in leaf-node crates at the bottom of the tree. If adding a new dependency would create a cycle, the design must be restructured using dependency inversion — not hacks like `#[cfg]` gates or runtime linking.

**What to look for:**
- Crate A depending on Crate B which depends on Crate A (Cargo will catch direct cycles, but conceptual cycles through re-exports or shared state can hide).
- "God crates" that everything depends on AND that depend on everything — these are DAG bottlenecks.
- Shared types scattered across crates instead of extracted into a common leaf crate.

### Rule 19 — Meaningful Crate Boundaries

Each crate in the workspace must have a single, well-defined responsibility along either a technical axis (e.g., telemetry, core-types, infrastructure) or a bounded-context axis (e.g., orders, inventory, payments). A standard enterprise topology separates `apps/` (binary crates) from `libs/` (library crates).

**What to look for:**
- A `utils` or `common` crate that's become a dumping ground for unrelated code (check how many distinct modules it has and whether they share a cohesive purpose).
- A single crate containing domain logic, database access, HTTP handlers, AND utilities — this should be split.
- Crates with vague names like `helpers`, `misc`, `shared` that don't communicate their bounded responsibility.

### Rule 20 — Visibility as an Architectural Tool

Rust's module visibility is not merely access control — it's a primary enforcement mechanism for architectural boundaries. Default to `pub(crate)` visibility and promote to `pub` only when an item is part of the crate's deliberate public API contract. The sealed trait pattern (private supertrait in a private module) is one application of this principle, but it applies broadly: over-exposed internals erode the boundary guarantees that crate-level separation is designed to protect.

**What to look for:**
- `pub` on types, functions, or modules that are only used within the crate (should be `pub(crate)` or private).
- Internal implementation details exposed as `pub` — check if they're used by other crates or just within the same crate.
- `pub use` re-exports that expose internal types as part of the public API unintentionally.
- Missing `pub(crate)` on helper modules that shouldn't be externally accessible.

## Severity calibration

- **HIGH:** Rule 18 violations (circular dependencies), Rule 19 with a monolithic single-crate project that should clearly be split
- **MEDIUM:** Rule 17 (duplicated dependency versions), Rule 20 (over-exposed internals in library crates)
- **LOW:** Rule 19 (naming complaints without actual responsibility overlap), Rule 20 (visibility nits in binary crates where it matters less)

## Output

After reviewing, produce both files per `references/output_format.md`. Read that file first if you haven't already.
