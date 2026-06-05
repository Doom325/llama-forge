//! Setup-wizard lifecycle integration tests (08_SETUP_WIZARD.md Sections 12-13).
//!
//! Spawns the real `llamaforge` binary. The first-run-mode HTTP behavior
//! (503 inference guard, `/api/status`, reconfigure guard) is covered
//! deterministically by the `build_router` unit tests in
//! `llamaforge-api::transport::http`; here we exercise the standalone wizard's
//! process lifecycle, which can only be observed end-to-end.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
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

/// The standalone wizard exits 130 on Ctrl-C/SIGINT after a client has been
/// active (Section 12.5, task 7.14). Spawns with a long idle timeout so only
/// the signal can end it, waits for the server to bind, then sends SIGINT.
#[cfg(unix)]
#[test]
fn standalone_wizard_exits_130_on_sigint() {
    let bin = env!("CARGO_BIN_EXE_llamaforge");

    let mut child = Command::new(bin)
        .args(["init", "--interactive", "--timeout=60s"])
        .env("LLAMAFORGE_NO_BROWSER", "1")
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn llamaforge init --interactive");

    // Wait for the server to report its URL (i.e. it has bound the port).
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut bound = false;
    for _ in 0..40 {
        let mut line = String::new();
        if reader.read_line(&mut line).expect("read wizard stdout") == 0 {
            break;
        }
        if line.contains("/ui/setup") {
            bound = true;
            break;
        }
    }
    assert!(bound, "wizard never reported its URL");

    // The banner prints just before the serve `select!` registers tokio's
    // Ctrl-C handler; settle briefly so the signal is caught (→ 130) rather
    // than hitting the default disposition (terminate).
    std::thread::sleep(Duration::from_millis(300));

    // Send SIGINT — the tokio Ctrl-C handler returns exit code 130.
    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGINT).expect("send SIGINT");

    let status = child.wait().expect("wait for wizard");
    assert_eq!(status.code(), Some(130), "expected exit 130 on SIGINT, got {status:?}");
}
