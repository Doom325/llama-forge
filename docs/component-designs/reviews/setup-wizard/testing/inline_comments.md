<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 477-515 RULE: 22 SEVERITY: HIGH -->
**Rule 22 — Validate at Boundaries, Trust Nothing**
Section 10 (Testing Strategy) contains zero backend API tests. The wizard introduces seven new endpoints that accept filesystem paths, numeric parameters, and structured JSON — but the testing plan only covers frontend component tests and frontend integration tests with mocked API responses. The backend boundary — where untrusted user input enters the system — is completely untested.

Add a `10.6 Backend API Tests` subsection covering: input validation boundary tests for every endpoint (empty arrays, path traversal, null bytes, non-existent paths, out-of-range numerics), happy-path response contract tests, and error response shape tests.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 598-636 RULE: 22 SEVERITY: HIGH -->
**Rule 22 — Validate at Boundaries, Trust Nothing**
The `POST /api/setup/build` endpoint is the most dangerous endpoint in the wizard — it runs `git clone` and `cmake` on the host. The design does not specify validation for the `backend` parameter (should be enum-restricted to cuda/metal/vulkan/cpu), `install_dir` (should be canonicalized and restricted to safe directories), or `jobs` (should be bounded to prevent fork-bomb). No tests cover these boundaries.

Suggested: Specify that `backend` is validated against a closed enum. `install_dir` is canonicalized via `std::fs::canonicalize` and checked against an allowlist. `jobs` is clamped to 1..=logical_cores. Add tests for each boundary.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 493-499 RULE: 24 SEVERITY: HIGH -->
**Rule 24 — Test Interactions, Not Just Units**
The integration tests (10.2) mock all API responses and test the frontend in isolation. There are no backend integration tests and no end-to-end tests that exercise the real backend. This means the frontend tests verify behavior against mocks that may not match reality. A field name mismatch between the mock and the real backend (e.g., `path` vs `binary_path`) would go undetected.

Add backend integration tests that exercise each `/api/setup/*` endpoint with realistic inputs (mocked shell commands, temp directories with fixture files) and verify the response matches the contract in Section 11. Consider also adding at least one full-stack smoke test that drives the wizard frontend against the real backend (with mocked system commands).

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 495-498 RULE: 25 SEVERITY: MEDIUM -->
**Rule 25 — Test Recovery, Not Just Failure**
The integration tests test failure detection (build error UI appears, hardware error banner appears) but do not test recovery after failure. After a build fails and the user clicks "Retry," does the SSE connection re-establish? Does the log reset? Does a subsequent success correctly transition the UI? After hardware detection fails and the user enters manual values, do config recommendations use the manual values?

Add recovery-path tests for each failure scenario: failure -> retry -> success, failure -> skip -> manual entry -> subsequent steps use manual data, SSE disconnect -> reconnect -> log resumes without duplication.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 493-499 RULE: 24 SEVERITY: MEDIUM -->
**Rule 24 — Test Interactions, Not Just Units**
The SSE streaming behavior for the build endpoint is only tested as "mock build SSE to emit lines then an error event." This covers one path through a complex interaction. Missing: EventSource connection establishment, parsing of each SSE event type (phase, progress, log, complete, failed), reconnection on unexpected disconnect, malformed event handling, memory management for long build logs (10,000+ lines in the `logLines` array), and cleanup when navigating away mid-build.

Add dedicated SSE interaction tests covering each event type, disconnect/reconnect, malformed events, long logs, and navigation-away cleanup.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 326-333 RULE: 22 SEVERITY: MEDIUM -->
**Rule 22 — Validate at Boundaries, Trust Nothing**
Form validation specifies `gpu_layers: integer, -1 or >= 0` and `ctx_size: integer >= 0` but does not specify upper bounds. Values like `gpu_layers: 999999` or `ctx_size: 2147483647` would pass the described validation but produce invalid configs. The wizard's success criterion is "generated config passes `llamaforge validate` on first try, 100% of the time" — unbounded values risk violating this.

Specify concrete upper bounds: `gpu_layers` -1 to 512, `ctx_size` 0 to 1048576, `parallel_slots` 1 to 64, `port` 1024 to 65535. Add component tests for boundary values (just below min, at min, at max, just above max).

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 700-713 RULE: 25 SEVERITY: MEDIUM -->
**Rule 25 — Test Recovery, Not Just Failure**
The standalone wizard mode (Section 12) has no tests. This mode's lifecycle — ephemeral port, browser open, wait for save, exit — has several recovery-relevant edge cases: save fails (does the server stay alive for retry?), user closes browser (does the server hang forever?), Ctrl+C (clean shutdown?), concurrent tabs (race condition on save?). Since this is the primary first-run experience, these paths are high-traffic.

Add backend tests for standalone mode: save failure -> server stays alive -> retry succeeds -> server exits; SIGINT -> clean shutdown; concurrent save requests -> no race condition.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 522-549 RULE: 23 SEVERITY: MEDIUM -->
**Rule 23 — Fail Fast and Loud**
The hardware detection endpoint parses shell command output (`nvidia-smi`, `system_profiler`, `rocm-smi`), but the design does not specify behavior when parsing encounters unexpected output. If `nvidia-smi` changes its format and the parser silently returns `vram_mb: 0`, the recommendation engine will produce a CPU-only config for a GPU-equipped system. The testing strategy includes no parser tests.

Add parser unit tests for each shell command: known-good format, format with extra fields, empty output, gibberish output. The parser must fail explicitly (return error or mark GPU as "detection_error") rather than silently return zero values. Frontend tests should verify that a `vram_mb: 0` response triggers the "Not detected" UI path, not the "Detected: 0 MB" path.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 796-895 RULE: 24 SEVERITY: LOW -->
**Rule 24 — Test Interactions, Not Just Units**
The TypeScript types (Appendix) and the backend JSON response shapes (Section 11) are defined in the same document but there is no mechanism to keep them in sync over time. Without contract tests or generated types, a backend change that adds/renames a field will not be caught until runtime.

Consider either: (1) JSON schema snapshot tests that compare backend output against TypeScript type definitions, or (2) generated TypeScript types from Rust types via `ts-rs` crate so they cannot drift.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 574-595 RULE: 22 SEVERITY: LOW -->
**Rule 22 — Validate at Boundaries, Trust Nothing**
The model scanner accepts a `directories` array with no specified limits on array length, recursion depth, or scan duration. A request with 10,000 directories or a scan of `/` could tie up the server. The success criteria mention "< 5 seconds for < 10,000 files" but the API does not describe behavior above this limit.

Add limits: max 10 directories per request, symlink cycle detection, 30-second scan timeout with partial results. Add tests: empty directories array, single directory with no .gguf files, directory with symlink cycle, scan timeout behavior.

---
