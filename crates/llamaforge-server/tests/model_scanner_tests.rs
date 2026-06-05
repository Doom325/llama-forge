//! Model scanner tests (08_SETUP_WIZARD.md Section 11.3).
//!
//! Drives `scan_models` against real temp directories so the recursive walk,
//! filename heuristics, bounds (`max_depth`/`max_results`/timeout), hidden-dir
//! skipping, and warning collection are all exercised end to end.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use llamaforge_server::{
    scan_models, scan_models_with_deadline, ModelScanError, ModelScanRequest, ModelSource,
    TruncationReason,
};

fn request(dirs: &[&Path]) -> ModelScanRequest {
    ModelScanRequest {
        source: ModelSource::Local,
        directories: dirs.iter().map(|p| p.display().to_string()).collect(),
        max_depth: None,
        max_results: None,
    }
}

fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, b"GGUF\x00\x00\x00").expect("write fixture file");
}

// --- 2.9: filename heuristic parsing --------------------------------------

#[tokio::test]
async fn filename_heuristics_extract_quantization_and_architecture() {
    let dir = tempfile::tempdir().unwrap();
    touch(&dir.path().join("llama-3-8b-Q4_K_M.gguf"));
    touch(&dir.path().join("mistral-7b-instruct-v0.3-Q5_K_S.gguf"));
    touch(&dir.path().join("randomblob.gguf"));

    let result = scan_models(request(&[dir.path()])).await.unwrap();
    let by_name = |name: &str| {
        result
            .models
            .iter()
            .find(|m| m.filename == name)
            .unwrap_or_else(|| panic!("missing {name}"))
            .clone()
    };

    let llama = by_name("llama-3-8b-Q4_K_M.gguf");
    assert_eq!(llama.quantization_hint, "Q4_K_M");
    assert_eq!(llama.architecture_hint, "llama");

    let mistral = by_name("mistral-7b-instruct-v0.3-Q5_K_S.gguf");
    assert_eq!(mistral.quantization_hint, "Q5_K_S");
    assert_eq!(mistral.architecture_hint, "mistral");

    let unknown = by_name("randomblob.gguf");
    assert_eq!(unknown.quantization_hint, "unknown");
    assert_eq!(unknown.architecture_hint, "unknown");
}

// --- 2.10: tempdir scanning at varying depths -----------------------------

#[tokio::test]
async fn discovers_all_gguf_files_recursively_with_metadata() {
    let dir = tempfile::tempdir().unwrap();
    touch(&dir.path().join("a.gguf"));
    touch(&dir.path().join("sub/b.gguf"));
    touch(&dir.path().join("sub/deep/c.gguf"));
    touch(&dir.path().join("sub/deep/d.gguf"));
    touch(&dir.path().join("other/e.gguf"));
    touch(&dir.path().join("notes.txt")); // ignored — wrong extension

    let result = scan_models(request(&[dir.path()])).await.unwrap();

    assert_eq!(result.models.len(), 5);
    assert!(!result.truncated);
    assert!(result.truncation_reason.is_none());
    for model in &result.models {
        assert!(model.size_bytes > 0);
        assert!(!model.id.is_empty());
        assert!(model.path.ends_with(".gguf"));
    }
    // Stable id == SHA-256 prefix of the canonical path: distinct per file.
    let mut ids: Vec<_> = result.models.iter().map(|m| m.id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 5);
}

// --- 2.11: max_depth enforcement ------------------------------------------

#[tokio::test]
async fn max_depth_limits_recursion() {
    let dir = tempfile::tempdir().unwrap();
    // depth 3: root/l1/l2/shallow.gguf
    touch(&dir.path().join("l1/l2/shallow.gguf"));
    // depth 7: root/d1/d2/d3/d4/d5/d6/deep.gguf
    touch(&dir.path().join("d1/d2/d3/d4/d5/d6/deep.gguf"));

    let mut req = request(&[dir.path()]);
    req.max_depth = Some(5);
    let result = scan_models(req).await.unwrap();

    let names: Vec<_> = result.models.iter().map(|m| m.filename.as_str()).collect();
    assert!(names.contains(&"shallow.gguf"), "depth-3 file should be found");
    assert!(!names.contains(&"deep.gguf"), "depth-7 file should be pruned by max_depth=5");
}

// --- 2.12: max_results enforcement ----------------------------------------

#[tokio::test]
async fn max_results_truncates_with_reason() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..20 {
        touch(&dir.path().join(format!("model-{i:02}.gguf")));
    }

    let mut req = request(&[dir.path()]);
    req.max_results = Some(10);
    let result = scan_models(req).await.unwrap();

    assert_eq!(result.models.len(), 10);
    assert!(result.truncated);
    assert_eq!(result.truncation_reason, Some(TruncationReason::MaxResults));
}

// --- 2.13: permission errors become warnings ------------------------------

#[cfg(unix)]
#[tokio::test]
async fn permission_errors_collected_as_warnings() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let restricted = dir.path().join("restricted");
    fs::create_dir(&restricted).unwrap();
    touch(&restricted.join("hidden.gguf"));
    fs::set_permissions(&restricted, fs::Permissions::from_mode(0o000)).unwrap();

    let result = scan_models(request(&[&restricted])).await;

    // Restore permissions so the tempdir can be cleaned up.
    let _ = fs::set_permissions(&restricted, fs::Permissions::from_mode(0o755));

    let result = result.unwrap();
    // As non-root the directory is unreadable → a warning, never a hard error.
    // As root (e.g. some CI) the read may succeed; in that case there is simply
    // no warning, which is also acceptable.
    if !result.warnings.is_empty() {
        assert!(
            result.warnings.iter().any(|w| w.contains("Permission denied")
                || w.contains("Cannot access")
                || w.contains("Cannot read")),
            "unexpected warnings: {:?}",
            result.warnings
        );
    }
}

// --- 2.14: empty directory returns empty list -----------------------------

#[tokio::test]
async fn empty_directory_returns_no_models() {
    let dir = tempfile::tempdir().unwrap();
    let result = scan_models(request(&[dir.path()])).await.unwrap();

    assert!(result.models.is_empty());
    assert!(!result.truncated);
    assert!(result.truncation_reason.is_none());
    assert!(result.warnings.is_empty());
}

// --- 2.15: hidden directories are skipped ---------------------------------

#[tokio::test]
async fn hidden_directories_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    touch(&dir.path().join(".hidden/secret.gguf"));
    touch(&dir.path().join("visible.gguf"));

    let result = scan_models(request(&[dir.path()])).await.unwrap();

    let names: Vec<_> = result.models.iter().map(|m| m.filename.as_str()).collect();
    assert_eq!(names, vec!["visible.gguf"]);
}

// --- Timeout truncation (deadline seam) -----------------------------------

#[tokio::test]
async fn past_deadline_truncates_with_timeout_reason() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..5 {
        touch(&dir.path().join(format!("model-{i}.gguf")));
    }

    // An already-elapsed deadline trips the in-walk timeout check on the first
    // entry, exercising the TruncationReason::Timeout arm without a 30 s wait.
    let result =
        scan_models_with_deadline(request(&[dir.path()]), Instant::now() - Duration::from_secs(1))
            .await
            .unwrap();

    assert!(result.truncated);
    assert_eq!(result.truncation_reason, Some(TruncationReason::Timeout));
}

// --- Symlink cycle detection ----------------------------------------------

#[cfg(unix)]
#[tokio::test]
async fn symlink_cycle_reported_as_warning() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("model.gguf"));
    fs::create_dir(root.join("sub")).unwrap();
    // sub/loop → root: following it revisits an ancestor, a cycle walkdir flags.
    symlink(root, root.join("sub/loop")).unwrap();

    let result = scan_models(request(&[root])).await.unwrap();

    assert!(
        result.warnings.iter().any(|w| w.contains("Symlink cycle detected")),
        "expected a symlink-cycle warning, got: {:?}",
        result.warnings
    );
}

// --- Input validation -----------------------------------------------------

#[tokio::test]
async fn empty_directories_is_a_validation_error() {
    let req = ModelScanRequest {
        source: ModelSource::Local,
        directories: vec![],
        max_depth: None,
        max_results: None,
    };
    assert_eq!(scan_models(req).await, Err(ModelScanError::NoDirectories));
}

#[tokio::test]
async fn too_many_directories_is_a_validation_error() {
    let req = ModelScanRequest {
        source: ModelSource::Local,
        directories: (0..11).map(|i| format!("/tmp/dir{i}")).collect(),
        max_depth: None,
        max_results: None,
    };
    assert!(matches!(
        scan_models(req).await,
        Err(ModelScanError::TooManyDirectories { count: 11, max: 10 })
    ));
}
