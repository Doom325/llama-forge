//! Binary scanner tests (08_SETUP_WIZARD.md Section 11.2).
//!
//! "Found" and "timeout" cases place a mock `llama-server` script in a temp
//! directory passed via `extra_paths` (searched first, so they are
//! deterministic regardless of any real binary on the host). "Not found" and
//! "extra_paths first" cases control `PATH`/`HOME` under `#[serial]`.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use llamaforge_server::{scan_binary, BinaryScanRequest, BinaryScanResult};
use serial_test::serial;

/// Write an executable `llama-server` script into `dir` and return the dir.
fn install_mock(dir: &Path, body: &str) {
    let script = dir.join("llama-server");
    fs::write(&script, body).expect("write mock script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod +x");
}

fn request(extra_paths: &[&Path]) -> BinaryScanRequest {
    BinaryScanRequest { extra_paths: extra_paths.iter().map(|p| p.display().to_string()).collect() }
}

// --- 3.8: binary found ----------------------------------------------------

#[tokio::test]
#[serial]
async fn finds_binary_and_parses_version_and_backends() {
    let dir = tempfile::tempdir().unwrap();
    install_mock(
        dir.path(),
        "#!/bin/sh\necho 'version: b4567 (2026-05-01)'\necho 'backends: CUDA, CPU'\n",
    );

    let result = scan_binary(request(&[dir.path()])).await.unwrap();

    match result {
        BinaryScanResult::Found { path, version, backends, .. } => {
            assert!(path.ends_with("llama-server"));
            assert_eq!(version, "b4567 (2026-05-01)");
            assert!(backends.contains(&"cuda".to_string()));
            assert!(backends.contains(&"cpu".to_string()));
        }
        BinaryScanResult::NotFound { .. } => panic!("expected the mock binary to be found"),
    }
}

// --- 3.10: version timeout → found-but-unresponsive -----------------------

#[tokio::test]
#[serial]
async fn unresponsive_binary_reported_as_found() {
    let dir = tempfile::tempdir().unwrap();
    // Sleeps past the 5 s --version timeout.
    install_mock(dir.path(), "#!/bin/sh\nsleep 30\n");

    let result = scan_binary(request(&[dir.path()])).await.unwrap();

    match result {
        BinaryScanResult::Found { version, backends, .. } => {
            assert_eq!(version, "found but unresponsive to --version");
            assert!(backends.is_empty());
        }
        BinaryScanResult::NotFound { .. } => panic!("unresponsive binary must still be 'found'"),
    }
}

// --- 3.9: binary not found ------------------------------------------------

#[tokio::test]
#[serial]
async fn reports_not_found_with_searched_paths() {
    let empty = tempfile::tempdir().unwrap();
    let fake_home = tempfile::tempdir().unwrap();
    let prev_path = std::env::var_os("PATH");
    let prev_home = std::env::var_os("HOME");
    std::env::set_var("PATH", "");
    std::env::set_var("HOME", fake_home.path());

    let result = scan_binary(request(&[empty.path()])).await.unwrap();

    restore("PATH", prev_path);
    restore("HOME", prev_home);

    match result {
        BinaryScanResult::NotFound { searched_paths, .. } => {
            assert!(searched_paths.iter().any(|p| p == &empty.path().display().to_string()));
        }
        // Only happens if the host has llama-server in a hardcoded common
        // location (e.g. /usr/local/bin) — acceptable, not a scanner bug.
        BinaryScanResult::Found { .. } => {}
    }
}

// --- 3.11: extra_paths searched first -------------------------------------

#[tokio::test]
#[serial]
async fn extra_paths_are_searched_first() {
    let fake_home = tempfile::tempdir().unwrap();
    let prev_path = std::env::var_os("PATH");
    let prev_home = std::env::var_os("HOME");
    std::env::set_var("PATH", "/some/nonexistent/bin");
    std::env::set_var("HOME", fake_home.path());

    let result = scan_binary(request(&[Path::new("/custom/path")])).await.unwrap();

    restore("PATH", prev_path);
    restore("HOME", prev_home);

    if let BinaryScanResult::NotFound { searched_paths, .. } = result {
        assert_eq!(searched_paths.first().map(String::as_str), Some("/custom/path"));
    }
    // If Found, the host has a real binary; ordering is still extra-first by
    // construction, so there is nothing to assert.
}

// --- Wire shape: the `found` discriminator is a real boolean literal -------

#[tokio::test]
#[serial]
async fn found_result_serializes_with_boolean_discriminator() {
    let dir = tempfile::tempdir().unwrap();
    install_mock(dir.path(), "#!/bin/sh\necho 'b9999'\n");

    let result = scan_binary(request(&[dir.path()])).await.unwrap();
    let json = serde_json::to_value(&result).unwrap();

    assert_eq!(json["found"], serde_json::json!(true));
    assert!(json["path"].is_string());
    // The binary's version is `binary_version`, not `version` — the latter is
    // reserved for the `"version": 1` API envelope (added by the PR5 handler).
    assert!(json["binary_version"].is_string());
    assert!(json.get("version").is_none());
    assert!(json["backends"].is_array());
    assert!(json.get("searched_paths").is_none());
}

fn restore(key: &str, previous: Option<std::ffi::OsString>) {
    match previous {
        Some(value) => std::env::set_var(key, value),
        None => std::env::remove_var(key),
    }
}
