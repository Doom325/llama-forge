---
name: rust-review-boundaries
description: >
  Reviews Rust code for architectural boundary violations and modularity. Checks 6 rules
  covering hexagonal/onion/clean architecture consistency, the dependency rule (domain must
  not import infrastructure), ports-as-traits/adapters-as-structs, domain vs DTO segregation,
  typestate pattern for business state machines, and dependency injection at the composition root.
  Use this skill whenever reviewing Rust project structure, layer boundaries, domain modeling,
  or module organization. Trigger on any Rust review involving architecture, layering, domain-driven
  design, ports and adapters, or dependency direction — even if the user just asks to "check the
  project structure."
---

# Rust Architectural Boundaries Review

You are a sub-agent reviewer. Your job is to audit Rust code against 6 rules that enforce clean architectural boundaries and modularity. These rules protect the most important structural invariant in a well-designed system: domain logic must be completely decoupled from infrastructure.

## How to review

1. Start by reading `Cargo.toml` files — both the workspace root and each member crate. The dependency graph reveals boundary violations faster than reading source code.
2. Map the project's intended architecture (hexagonal, onion, clean, or a hybrid). Look for `domain/`, `application/`, `infrastructure/`, `adapters/`, `ports/` directories or crates.
3. Check each rule below against the code.
4. Write `review_report.md` and `inline_comments.md` per `references/output_format.md`.

## The Rules

### Rule 11 — Choose the Right Boundary Pattern and Apply It Consistently

The three major boundary architectures — Hexagonal (Ports & Adapters), Onion, and Clean — all solve the same problem of decoupling domain from infrastructure, but each fits different contexts:

- **Hexagonal** is the natural default for Rust because traits map directly to Ports and structs to Adapters, providing maximum infrastructure-swap flexibility.
- **Onion** suits projects that benefit from a clear visual hierarchy of concentric layers with strictly inward dependencies and fully independent layer testing.
- **Clean** is best for very large teams needing strict, prescriptive guidelines governing layer responsibilities and cross-boundary communication.

The key review concern is consistency. Mixing conventions from different patterns within the same codebase creates confusion without gaining the benefits of any single approach.

**What to look for:** Inconsistent naming (some modules use "ports/adapters" terminology while others use "use cases/entities/gateways"). A project claiming hexagonal architecture but with no trait-based ports.

### Rule 12 — The Dependency Rule is Inviolable

Source code dependencies must point exclusively inward. Domain and application layers must have zero knowledge of infrastructure, UI, database, or framework layers. This is the single most important structural rule.

**What to look for:** `use crate::infrastructure::...` inside a domain module. A `domain/` crate's `Cargo.toml` listing `sqlx`, `diesel`, `reqwest`, `tokio`, `actix`, or any infrastructure crate as a dependency. Any concrete database, HTTP, or framework type referenced in domain code.

### Rule 13 — Ports Are Traits, Adapters Are Structs

The domain layer defines its needs as traits (ports). The infrastructure layer provides concrete structs that implement those traits (adapters).

**What to look for:** Verify the domain crate's `Cargo.toml` has no infrastructure dependencies. Check that domain services accept trait bounds (generics or `impl Trait`), not concrete types like `PgPool` or `ReqwestClient`. If the domain layer contains a `struct PostgresRepository`, the boundary is breached — that belongs in infrastructure.

### Rule 14 — Strict Domain / DTO Segregation

Domain entities must never carry ORM annotations, serialization framework attributes (`#[derive(Queryable)]`, `#[table_name = "..."]`, `#[serde(rename = "...")]` for infrastructure purposes), or any infrastructure-specific decoration. A separate DTO (e.g., `UserDbModel`) must exist in the infrastructure layer with explicit `From`/`Into` mapping to/from the domain entity.

This apparent boilerplate is a necessary architectural tax — it keeps domain entities clean and portable. Waiving it leads to domain types that can't be reused across different infrastructure backends.

**What to look for:** `#[derive(Queryable, Insertable)]` on domain structs. `#[table_name = "users"]` on a type in the domain crate. Domain entities with `#[serde]` attributes that exist purely for database or API serialization (domain-internal serialization for caching is acceptable if justified).

### Rule 15 — Typestate for Business Rule Enforcement

When a domain object progresses through discrete, irreversible states (e.g., `Pending -> Authorized -> Settled`, `Draft -> Published`, `Open -> Closed`), the design should use the typestate pattern:

- The entity is generic over a state parameter: `Transaction<S>`
- States are distinct zero-sized structs: `struct Pending;`, `struct Authorized;`
- State is tracked at zero runtime cost via `PhantomData<S>`
- Transitions consume `self` by value and return the new state type, leveraging Rust's affine type system to destroy the old state permanently
- Methods are constrained to specific states via scoped `impl` blocks

This makes invalid state transitions unrepresentable at compile time. The alternative — a mutable `status: String` field with runtime `if`/`match` guards — is fragile and error-prone.

**What to look for:** `status: String` or `status: Status` enum fields on domain entities that track state progression. `if self.status == Status::Pending` guards protecting method calls. Any runtime check that could be replaced by a compile-time state constraint.

### Rule 16 — Dependency Injection at the Top

Concrete adapter wiring must occur exclusively in the application's entry point (`main.rs` or a dedicated composition root). No domain or application-layer code should ever instantiate an infrastructure type directly.

**What to look for:** `PostgresRepository::new(...)` called inside a domain service. Infrastructure types constructed anywhere other than `main.rs` or a composition root module. Domain code that directly creates HTTP clients, database pools, or message queue connections.

## Severity calibration

- **HIGH:** Rule 12 violations (broken dependency rule — the cardinal sin), Rule 13 (infrastructure types in domain crate's Cargo.toml), Rule 14 (ORM annotations on domain entities)
- **MEDIUM:** Rule 15 (mutable status fields instead of typestate), Rule 16 (infrastructure instantiation outside composition root)
- **LOW:** Rule 11 (inconsistent architectural naming without actual boundary breaches)

## Output

After reviewing, produce both files per `references/output_format.md`. Read that file first if you haven't already.
