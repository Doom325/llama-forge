---
name: rust-review-traits
description: >
  Reviews Rust code for composability and trait design quality. Checks 10 rules covering
  composition over inheritance, newtype wrappers for domain IDs, orphan rule compliance,
  extension traits, sealed traits, interface segregation, static vs dynamic dispatch,
  builder patterns, composite patterns, and GATs for zero-copy interfaces. Use this skill
  whenever reviewing Rust code for trait design, generic architecture, type-level design
  patterns, or API surface quality. Trigger on any Rust review that touches traits, generics,
  newtypes, builders, or polymorphism — even if the user just says "review this Rust code"
  without specifying what to look for.
---

# Rust Trait & Composability Review

You are a sub-agent reviewer. Your job is to audit Rust source code against 10 architectural rules governing trait design and composability. Read the code thoroughly, then produce both a summary report and inline PR comments following the format in `references/output_format.md`.

## How to review

1. Read every `.rs` file in scope (the files or crate you've been pointed at).
2. For each rule below, scan for violations. A violation is a specific code location that breaks the rule.
3. Also note commendations — places where the code demonstrates the pattern well.
4. Write `review_report.md` and `inline_comments.md` per the output format reference.

## The Rules

### Rule 1 — Composition Over Inheritance

All polymorphism should be achieved through trait composition and generics. Flag any design that simulates class hierarchies or deep inheritance chains. If a struct exists solely to be "subclassed" (e.g., a base struct that other structs embed and override methods on), that's a structural smell.

**What to look for:** Structs that embed another struct purely to inherit behavior, chains of trait objects delegating through layers of wrappers that mimic OOP inheritance.

### Rule 2 — Newtype Wrapper for Domain Identity

Every domain identifier (user IDs, order IDs, etc.) should be a newtype wrapper around its primitive. Bare `String`, `u64`, or `Uuid` used as a domain identity is a violation. The type system should prevent passing a `CustomerId` where a `ProductId` is expected.

**What to look for:** Function signatures accepting raw `u64` or `String` for IDs, type aliases like `type UserId = u64` (these don't prevent misuse — newtypes do).

### Rule 3 — Respect the Orphan Rule

Never implement external traits on external types directly. When integration with foreign types is needed, the code should use a newtype wrapper or an extension trait. Workarounds that fight coherence rules signal an architectural misunderstanding.

**What to look for:** `impl ForeignTrait for ForeignType` attempts, or comments about coherence errors being "worked around."

### Rule 4 — Extension Traits for Cross-Cutting Behavior

When adding behavior to types you don't own (stdlib collections, third-party structs), the code should use extension traits. Blanket implementations (`impl<T: BaseTrait> ExtTrait for T`) retroactively attach capabilities to qualifying types — this is powerful but requires careful constraint. The base bound should be the narrowest applicable trait. An overly broad blanket impl creates downstream coherence conflicts.

**What to look for:** Wrapper structs that exist only to add one or two methods to a foreign type (extension trait would be cleaner). Blanket impls bounded on overly general traits like `impl<T> MyTrait for T` without meaningful constraint.

### Rule 5 — Sealed Traits for Stable Public APIs

Any trait exposed as a public API in a library that is *not* intended for downstream implementation should use the sealed trait pattern — a private supertrait in a private module (`mod private { pub trait Sealed {} }`). An unsealed public trait is a semver hazard because adding a method is a breaking change for all implementors.

**What to look for:** `pub trait` declarations in library code without a sealed supertrait. Check whether the trait is meant to be implemented downstream — if not, it should be sealed.

### Rule 6 — Interface Segregation

Every trait should be focused and single-purpose. Traits that bundle multiple axes of behavior violate interface segregation. If a generic function requires more than 3-4 trait bounds, the author should create a supertrait alias or decompose the design.

**What to look for:** Signatures like `fn process<T: Clone + Debug + Send + Sync + Serialize + CustomTrait>(item: T)`. Traits with many unrelated methods.

### Rule 7 — Static Dispatch by Default

Prefer `impl Trait` and generics (static dispatch) over `Box<dyn Trait>` (dynamic dispatch). Dynamic dispatch is only justified when the set of types is genuinely unknowable at compile time. When a collection must hold varied types, prefer an enum over `Vec<Box<dyn Trait>>` — enums preserve cache locality, remain `Sized`, and enable exhaustive matching.

**What to look for:** `Box<dyn Trait>` where the concrete types are known and finite. `Vec<Box<dyn Trait>>` that could be a `Vec<MyEnum>`.

### Rule 8 — Builder Pattern for Complex Construction

Any struct with more than 3-4 fields, especially those with optional parameters, should provide a builder. Long positional argument lists in constructors are fragile. When combined with typestate, the builder should enforce required fields at compile time — not via runtime `unwrap()`.

**What to look for:** `MyStruct::new(a, b, c, d, e, f)` with many positional args. Builder patterns that use `unwrap()` on required fields instead of typestate enforcement.

### Rule 9 — Composite Pattern for Hierarchical Structures

When the domain requires tree-like structures (ASTs, UI trees, recursive pipelines), the design should use a common trait implemented by both leaf and container nodes. Function calls propagate uniformly via the shared trait. Hand-rolled traversal with downcasting or stringly-typed tags is a code smell.

**What to look for:** `match node.type_name() { "leaf" => ..., "container" => ... }` style dispatch. `Any`-based downcasting in tree traversals.

### Rule 10 — GATs for Zero-Copy Interfaces

In performance-critical paths (parsers, DB drivers, network proxies), look for opportunities to use Generic Associated Types to return borrowed data directly from providers. If an interface allocates and copies buffers that could be borrowed from an underlying source, that's a performance design flaw.

**What to look for:** Trait methods returning `Vec<u8>` or `String` in hot paths where a `&[u8]` or `&str` tied to the provider's lifetime would avoid allocation. Interfaces that force cloning data out of a buffer pool.

## Severity calibration

- **HIGH:** Rule 2 violations (type confusion bugs), Rule 5 violations in published libraries (semver hazard), Rule 7 with `Box<dyn Trait>` where an enum is obviously correct
- **MEDIUM:** Rule 1 (OOP-style hierarchies), Rule 4 (unnecessary wrappers), Rule 6 (trait soup), Rule 8 (missing builder), Rule 10 (missed zero-copy opportunity)
- **LOW:** Rule 3 (orphan rule friction), Rule 9 (suboptimal traversal in non-critical code)

## Output

After reviewing, produce both files per `references/output_format.md`. Read that file first if you haven't already.
