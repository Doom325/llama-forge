//! CLI surface smoke tests.
//!
//! Exercises the binary's user-visible knobs end-to-end without
//! mounting the full orchestrator: `--help` / subcommand resolution,
//! `--state-dir` threading on `init`, the platform-aware default
//! state dir, and the `validate` no-fallback rule.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_llamaforge")
}

/// `llamaforge serve --help` must resolve to the bare-invocation
/// path: a `serve` subcommand alias was added in this PR so the
/// README documentation and operator muscle-memory work.
#[test]
fn serve_subcommand_exists() {
    let out = Command::new(bin()).arg("serve").arg("--help").output().expect("spawn");
    assert!(
        out.status.success(),
        "expected `serve --help` to succeed, status={:?}, stderr={}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The README and `Serve` doc comment promise that
/// `llamaforge serve --config <path>` works. Clap must accept the
/// flag on the subcommand, not only at the top level.
#[test]
fn serve_accepts_config_flag() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bogus = tmp.path().join("definitely-missing.toml");

    let out = Command::new(bin()).arg("serve").arg("--config").arg(&bogus).output().expect("spawn");

    // Subcommand must parse the flag — we don't care whether the
    // orchestrator then errors out on the missing config; only that
    // clap didn't reject the flag itself.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "clap rejected `serve --config <path>`: stderr={}",
        stderr
    );
}

/// Top-level `--state-dir` flag must propagate into `init` so the
/// per-subcommand fallback honours it. Before PR-5 this was silently
/// dropped and `init` always wrote to `/var/run/llamaforge/`.
#[test]
fn init_honors_top_level_state_dir() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let state_dir = tmp.path().join("state-dir");
    let out_toml = tmp.path().join("llamaforge.toml");
    let rules_toml = tmp.path().join("priority_rules.toml");

    let res = Command::new(bin())
        .arg("--state-dir")
        .arg(&state_dir)
        .arg("init")
        .arg("--out")
        .arg(&out_toml)
        .arg("--rules")
        .arg(&rules_toml)
        .output()
        .expect("spawn");
    assert!(
        res.status.success(),
        "init failed: status={:?} stderr={}",
        res.status,
        String::from_utf8_lossy(&res.stderr)
    );
    assert!(state_dir.exists(), "state dir should have been created at {}", state_dir.display());
    assert!(out_toml.exists(), "config file should exist at {}", out_toml.display());
}

/// `validate --config <missing>` must surface the file-not-found
/// error instead of silently routing to the default search path.
/// Before PR-5 `find_config()` swallowed a missing user path and
/// `validate` reported "config OK" for a working default.
#[test]
fn validate_missing_config_does_not_silently_fall_through() {
    let nonexistent: PathBuf = std::env::temp_dir().join("no-such-llamaforge.toml.{pid}");
    // Make sure it really doesn't exist for this run.
    let _ = std::fs::remove_file(&nonexistent);

    let res = Command::new(bin())
        .arg("--config")
        .arg(&nonexistent)
        .arg("validate")
        .output()
        .expect("spawn");
    assert!(
        !res.status.success(),
        "validate against a missing path must fail; stdout={} stderr={}",
        String::from_utf8_lossy(&res.stdout),
        String::from_utf8_lossy(&res.stderr)
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("config file"),
        "stderr should mention the missing file, got: {}",
        stderr
    );
}

/// Default state-dir resolution must NOT pick `/var/run/llamaforge/`
/// on a per-user macOS run. `find_state_dir()` now prefers
/// `$LLAMAFORGE_STATE_DIR` → platform-native per-user dir →
/// `/var/run/llamaforge/` (the legacy default).
#[cfg(target_os = "macos")]
#[test]
fn default_state_dir_on_macos_is_per_user() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let fake_home = tmp.path();
    let out_toml = tmp.path().join("llamaforge.toml");

    // Clear LLAMAFORGE_STATE_DIR so `find_state_dir` falls through to
    // the platform default. HOME points at the temp dir so we can
    // assert on the resolved path without touching the real
    // `~/Library/Application Support/llamaforge`.
    let res = Command::new(bin())
        .env_remove("LLAMAFORGE_STATE_DIR")
        .env("HOME", fake_home)
        .arg("init")
        .arg("--out")
        .arg(&out_toml)
        .output()
        .expect("spawn");
    assert!(
        res.status.success(),
        "init failed: status={:?} stderr={}",
        res.status,
        String::from_utf8_lossy(&res.stderr)
    );
    let expected = fake_home.join("Library").join("Application Support").join("llamaforge");
    assert!(
        expected.exists(),
        "macOS default state dir should be at {} (HOME-based), got: ls of fake_home: {:?}",
        expected.display(),
        std::fs::read_dir(fake_home).map(|d| d.collect::<Vec<_>>())
    );
}
