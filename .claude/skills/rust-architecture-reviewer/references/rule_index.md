# Rule Index — Quick Reference

Maps every rule to its sub-agent skill for fast lookup during orchestration and deduplication.

## Composability & Trait Design → `rust-review-traits`

| Rule | Name | Key Signal |
|------|------|------------|
| 1 | Composition Over Inheritance | Structs simulating class hierarchies |
| 2 | Newtype Wrapper for Domain Identity | Bare `u64`/`String` as IDs |
| 3 | Respect the Orphan Rule | `impl ForeignTrait for ForeignType` |
| 4 | Extension Traits for Cross-Cutting | Wrapper structs for one method on foreign types |
| 5 | Sealed Traits for Public APIs | Unsealed `pub trait` in libraries |
| 6 | Interface Segregation | 5+ trait bounds on one parameter |
| 7 | Static Dispatch by Default | `Box<dyn Trait>` where enum fits |
| 8 | Builder Pattern for Complex Construction | Constructors with 4+ positional args |
| 9 | Composite Pattern for Hierarchies | Downcasting/stringly-typed tree traversal |
| 10 | GATs for Zero-Copy | Allocating in hot-path interfaces |

## Architectural Boundaries → `rust-review-boundaries`

| Rule | Name | Key Signal |
|------|------|------------|
| 11 | Consistent Boundary Pattern | Mixed hex/onion/clean conventions |
| 12 | Dependency Rule | Domain imports from infrastructure |
| 13 | Ports = Traits, Adapters = Structs | Infrastructure crates in domain Cargo.toml |
| 14 | Domain / DTO Segregation | ORM annotations on domain structs |
| 15 | Typestate for Business Rules | Mutable `status: String` field |
| 16 | DI at the Composition Root | Infrastructure instantiation outside main.rs |

## Crate Structure → `rust-review-crates`

| Rule | Name | Key Signal |
|------|------|------------|
| 17 | Workspace-Level Dependencies | Duplicated dependency versions |
| 18 | DAG Topology | Circular crate dependencies |
| 19 | Meaningful Crate Boundaries | "utils" dumping ground crate |
| 20 | Visibility as Architecture | Over-exposed `pub` internals |

## Runtime Safety → `rust-review-safety`

| Rule | Name | Key Signal |
|------|------|------------|
| 21 | No unwrap/expect in Production | `.unwrap()` outside tests |
| 22 | No Blocking in Async | `std::fs` inside `async fn` |
| 23 | Borrowing Over Cloning | `.clone()` on heap types in hot paths |

## Observability → `rust-review-observability`

| Rule | Name | Key Signal |
|------|------|------------|
| 24 | Structured Tracing | `println!` or `log::info!` |
| 25 | Multi-Layer Subscriber | Single-backend subscriber setup |
| 26 | Observability Out of Domain | `#[instrument]` in domain code |
| 27 | Trace Context Propagation | Missing traceparent on outbound calls |
| 28 | Telemetry Transport Efficiency | Uncompressed JSON OTLP at scale |
| 29 | Symptom-Based Alerting | No latency percentiles/error dimensions |

## Build & CI → `rust-review-ci`

| Rule | Name | Key Signal |
|------|------|------------|
| 30 | Optimized Build Toolchain | Default linker, no sccache |
| 31 | CI Beyond cargo test | Missing clippy/fmt/audit/miri |
| 32 | Supply Chain Auditing | Unmaintained deps, broad features |

## Cross-Domain Overlap

These rule pairs can flag the same code — the orchestrator should deduplicate:

- **Rule 12 + Rule 13:** Both detect infrastructure in domain. Keep Rule 13 (more specific).
- **Rule 12 + Rule 26:** Both detect `tracing` in domain crate. Keep Rule 26 (more specific).
- **Rule 5 + Rule 20:** Both concern visibility as API boundary. Keep Rule 5 for sealed traits, Rule 20 for general visibility.
- **Rule 14 + Rule 12:** ORM annotations on domain structs violate both DTO segregation and the dependency rule. Keep Rule 14 (more specific diagnosis).
