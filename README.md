# LlamaForge

Rust orchestrator and gateway for fleets of `llama.cpp` (`llama-server`) processes — supervising lifecycles, multiplexing OpenAI- and Anthropic-compatible HTTP traffic over a typed gRPC core, and routing requests by slot/KV-cache awareness.

> **Status:** Phase 0 skeleton. Workspace compiles; no business logic yet.

## Source of truth

Development is driven by the documents in [`docs/`](docs):

- [`docs/design.md`](docs/design.md) — master design
- [`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md) — phased plan with verification checkpoints
- [`docs/component-designs/`](docs/component-designs) — component-by-component specifications:
  - [`01_CONFIG_MANAGER.md`](docs/component-designs/01_CONFIG_MANAGER.md)
  - [`02_METRICS_AND_STORAGE.md`](docs/component-designs/02_METRICS_AND_STORAGE.md)
  - [`03_SERVER_MANAGER.md`](docs/component-designs/03_SERVER_MANAGER.md)
  - [`04_REQUEST_QUEUE_AND_SCHEDULER.md`](docs/component-designs/04_REQUEST_QUEUE_AND_SCHEDULER.md)
  - [`05_SMART_ROUTING.md`](docs/component-designs/05_SMART_ROUTING.md)
  - [`06_DISPATCH_AND_PROXY.md`](docs/component-designs/06_DISPATCH_AND_PROXY.md)
  - [`07_TRANSPORT_LAYER.md`](docs/component-designs/07_TRANSPORT_LAYER.md)

Read the relevant component design before changing the corresponding module.

## Workspace layout (per Plan Task 0.1)

```
llama-forge/
├── Cargo.toml              # workspace
├── rust-toolchain.toml     # stable, with rustfmt + clippy
├── crates/
│   ├── llamaforge/         # binary crate (the orchestrator)
│   └── llamaforge-proto/   # generated tonic/prost bindings
├── proto/                  # .proto definitions (Task 0.2)
└── config/                 # example TOML configs
```

## Build

```sh
cargo build
cargo test
```

## License

Apache-2.0.
