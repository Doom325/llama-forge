# Server Manager Testing Guide

The Server Manager component has two primary test suites, designed to be run separately to optimize developer iteration speed.

## 1. Unit & Property Tests (Fast)

These tests execute purely in-memory, without spawning actual subprocesses or hitting real timeouts. They cover configuration parsing, state transitions, mathematical bounds (like the backoff schedule generator), and SQLite query construction.

```bash
# Runs all unit and property tests. Completes in < 5 seconds.
cargo test -p llamaforge-server-manager
```

## 2. End-to-End Integration Suite (Slow)

These tests orchestrate the full `ServerManager` pipeline against the `mock-llama` binary (a lightweight Rust executable that simulates `llama-server` HTTP endpoints and lifecycle behaviors). They verify graceful shutdown sequences, deadlock detection via `kill` signals, orphan process recovery, and the correctness of the Prometheus metrics surface.

Because they rely on real wall-clock timeouts and OS-level process management, they are deliberately marked `#[ignore]`.

```bash
# Runs the full integration suite. Completes in ~30-60 seconds.
cargo test -p llamaforge-server-manager --features test-utils -- --ignored
```

### Notes for Integration Tests

*   **Port Allocation:** The E2E tests dynamically allocate ports starting from `18000+` using an atomic counter to avoid collisions. If a test crashes mid-execution, it may strand a `mock-llama` zombie process on one of these ports.
*   **Zombie Cleanup:** If subsequent test runs fail due to `AddrInUse` (port already bound) errors, you can identify and clean up stranded processes via:
    ```bash
    # Identify leaked mock-llama processes
    lsof -i:18000-19500
    # Kill them
    killall mock-llama
    ```
*   **Wall-Clock Budgets:** Several tests (e.g., `test_degradation_e2e_restart` and `test_multi_server_orchestration`) require 10-30 seconds to run because they verify behavior across multiple polling intervals (like the 10-second metrics tick).
*   **Time Pausing:** The backoff schedule test (`test_backoff_schedule`) uses `tokio::time::pause()` to fast-forward through minutes of backoff delays instantly. Tests that bind real TCP sockets (like those using `mock-llama`) **cannot** use paused time, which is why the E2E suite takes longer.
