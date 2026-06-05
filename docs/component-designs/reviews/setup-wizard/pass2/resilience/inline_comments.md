<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 1179-1183 RULE: 2 SEVERITY: LOW -->
**Rule 2 -- Timeouts Everywhere**
The validation-launch step spawns a `llama-server` process and says "kill the validation server" after the health check completes (success or failure), but does not specify a timeout for the kill itself. If the llama-server process ignores SIGTERM (e.g., stuck in a CUDA driver call), the save-config handler hangs. Apply the same SIGTERM -> 5s wait -> SIGKILL pattern already specified for build subprocess cleanup in Section 11.4.

---

<!-- FILE: docs/component-designs/08_SETUP_WIZARD.md LINES: 1155-1158 RULE: 1 SEVERITY: LOW -->
**Rule 1 -- Graceful Degradation & Recoverability**
The pre-validation port availability check (`TcpListener::bind` + release) has an inherent TOCTOU gap -- another process can grab the port between the check and the actual server startup in the validation-launch step. The validation-launch catches this, so no functional gap exists. However, adding a one-line note ("This check is advisory; the validation-launch is the authoritative verification") would prevent an implementer from treating pre-validation as sufficient and skipping the validation-launch.
