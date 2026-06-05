<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 966 RULE: 6 SEVERITY: LOW -->
**Rule 6 — Every Piece of Complexity Must Earn Its Place**
The `check-path` route is still registered in the `setup_router()` function, but the endpoint's contract was removed from the document per the first-pass recommendation (Finding 3). No Section 11.x defines its request/response format, no tests reference it, and no frontend code calls it.

This is a stale artifact. A developer implementing this router will find 10 routes but only 9 specifications, leading to a "where is the spec?" investigation.

Suggested: Remove this line:
```rust
// DELETE this line:
.route("/api/setup/check-path", post(check_path_handler))
```

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 975-978 RULE: 6 SEVERITY: LOW -->
**Rule 6 — Every Piece of Complexity Must Earn Its Place**
Section 12.2 describes a minimal axum server (`ui_router().merge(setup_router())` with no gRPC, no inference engine, no config handle). Section 13.4 describes a full transport layer boot (`full_app_router().merge(setup_router())` with HTTP + gRPC on default ports). These are two different server assemblies that must be independently maintained and tested.

The shared `setup_router()` (Section 12.1) mitigates endpoint divergence, which is the highest risk. The remaining difference — which other services are co-hosted — is a legitimate design choice (standalone mode should not bind gRPC ports the user does not need). But the trade-off should be documented explicitly in Section 12 so future maintainers understand it is deliberate, not accidental.

Suggested: Add a note after line 971:
```markdown
> **Implementation note:** The standalone mode intentionally uses a minimal server
> assembly (no gRPC, no inference endpoints) to avoid binding ports that are not
> needed during setup. This means two boot paths exist. The shared `setup_router()`
> ensures endpoint behavior is identical; the difference is only in which other
> services are co-hosted. If maintaining two boot paths becomes burdensome, the
> standalone mode can be collapsed into `llamaforge serve --exit-after-setup
> --force-setup` with acceptable trade-offs (gRPC port briefly bound during setup).
```

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 82 RULE: 7 SEVERITY: LOW -->
**Rule 7 — Optimize for the Reader, the Debugger, and the Operator**
The Key Screens table says "Streaming build log with progress estimation" in the Build row's Loading column. Section 4.4 (line 240) explicitly states: "No estimated time remaining — build time estimation from log heuristics is unreliable and misleading. The elapsed time timer updates every second." These contradict each other.

Suggested: Change the Loading column for the Build row from:
```
Streaming build log with progress estimation
```
to:
```
Streaming build log with target count and elapsed time
```

This aligns with Section 4.4's specification of factual progress display.
