# LlamaForge

Rust orchestrator and gateway for fleets of `llama.cpp` (`llama-server`) processes. Supervises process lifecycles, multiplexes OpenAI- and Anthropic-compatible HTTP traffic over a typed gRPC core, and routes requests by slot availability and KV-cache (prompt) affinity. SQLite-backed metrics and a `tracing`-based observability stack are wired throughout.

## Build & run

```sh
cargo build --release
./target/release/llamaforge init                       # writes config/llamaforge.toml.example
./target/release/llamaforge serve --config <path>      # starts the orchestrator
```

## Documentation

- Master design: [`docs/design.md`](docs/design.md)
- Component designs: [`docs/component-designs/`](docs/component-designs)
- Phased plan + acceptance criteria: [`docs/IMPLEMENTATION_PLAN.md`](docs/IMPLEMENTATION_PLAN.md) and [`PLAN.md`](PLAN.md)
- Deferred to v1.1: [`docs/v1.1-deferred.md`](docs/v1.1-deferred.md)

## End-to-end tests

```sh
cargo build --bin mock-llama                           # required by the e2e harness
cargo test -p llamaforge-e2e --tests                   # also needs `protoc`
                                                       # on Linux, also needs `mold` (fast linker; macOS uses the system linker)
```

## License

Apache-2.0.
