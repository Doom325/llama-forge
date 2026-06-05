//! Setup-wizard lifecycle integration tests (08_SETUP_WIZARD.md Sections 12-13).
//!
//! Spawns the real `llamaforge` binary. The first-run-mode HTTP behavior
//! (503 inference guard, `/api/status`, reconfigure guard) is covered
//! deterministically by the `build_router` unit tests in
//! `llamaforge-api::transport::http`; here we exercise the standalone wizard's
//! process lifecycle, which can only be observed end-to-end.

use std::process::Command;
use std::time::{Duration, Instant};

/// `llamaforge init --interactive --timeout=2s` binds an ephemeral wizard port,
/// prints the URL, and exits 0 once the idle timeout elapses (Section 12.5).
#[test]
fn standalone_wizard_exits_cleanly_on_idle_timeout() {
    let bin = env!("CARGO_BIN_EXE_llamaforge");

    let start = Instant::now();
    let output = Command::new(bin)
        .args(["init", "--interactive", "--timeout=2s"])
        .env("LLAMAFORGE_NO_BROWSER", "1")
        .output()
        .expect("spawn llamaforge init --interactive");
    let elapsed = start.elapsed();

    assert!(
        output.status.success(),
        "expected exit 0, got {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    // The idle monitor polls within the (short) timeout, so it should exit
    // well under the test's patience window rather than after a fixed minute.
    assert!(elapsed < Duration::from_secs(20), "wizard took too long to time out: {elapsed:?}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("setup wizard is running at"),
        "missing startup banner; stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("/ui/setup"),
        "banner should include the wizard URL; stdout:\n{stdout}"
    );
    assert!(stdout.contains("idle"), "should report the idle exit; stdout:\n{stdout}");
}
