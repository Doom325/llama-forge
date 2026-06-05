//! Build orchestrator domain tests (08_SETUP_WIZARD.md Section 11.4).
//!
//! Drives `run_build` with mock `git`/`cmake` shell scripts (no real toolchain)
//! so the phase pipeline, event ordering, cancellation, and per-phase timeout
//! are exercised deterministically.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use llamaforge_server::{
    run_build, BuildBackend, BuildCommands, BuildEvent, BuildOutcome, BuildPhase, BuildRequest,
    PhaseTimeouts,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn write_script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

const MOCK_GIT: &str = "#!/bin/sh\n\
echo \"Cloning into 'llama.cpp'...\"\n\
mkdir -p \"$4\"\n\
echo done\n";

const MOCK_CMAKE: &str = "#!/bin/sh\n\
if [ \"$1\" = \"-B\" ]; then\n\
  mkdir -p \"$2\"\n\
  echo '-- Configuring done'\n\
elif [ \"$1\" = \"--build\" ]; then\n\
  echo '[ 50%] Building CXX object'\n\
  mkdir -p \"$2/bin\"\n\
  printf '#!/bin/sh\\n' > \"$2/bin/llama-server\"\n\
  chmod +x \"$2/bin/llama-server\"\n\
  echo '[100%] Built target llama-server'\n\
else\n\
  echo \"unexpected: $*\" >&2; exit 1\n\
fi\n";

const MOCK_SLEEP: &str = "#!/bin/sh\nsleep 30\n";

fn commands(dir: &Path, git_body: &str, cmake_body: &str) -> BuildCommands {
    let git = dir.join("mock-git");
    let cmake = dir.join("mock-cmake");
    write_script(&git, git_body);
    write_script(&cmake, cmake_body);
    BuildCommands {
        git,
        cmake,
        nice_level: None, // skip the `nice` wrapper for deterministic tests
    }
}

fn request(install_dir: &Path) -> BuildRequest {
    BuildRequest {
        backend: BuildBackend::Cpu,
        install_dir: install_dir.display().to_string(),
        jobs: 2,
    }
}

/// Run a build and collect the streamed events alongside the outcome.
async fn run(
    work: &Path,
    install: &Path,
    cmds: &BuildCommands,
    timeouts: PhaseTimeouts,
    cancel: CancellationToken,
    cancel_after: Option<Duration>,
) -> (BuildOutcome, Vec<BuildEvent>) {
    let (tx, mut rx) = mpsc::channel(256);
    let collector = tokio::spawn(async move {
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        events
    });

    if let Some(delay) = cancel_after {
        let token = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            token.cancel();
        });
    }

    let outcome = run_build(&request(install), work, cmds, &timeouts, tx, &cancel).await;
    let events = collector.await.unwrap();
    (outcome, events)
}

#[tokio::test]
async fn build_succeeds_and_streams_events_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    let install = dir.path().join("install");
    fs::create_dir_all(&work).unwrap();
    let cmds = commands(dir.path(), MOCK_GIT, MOCK_CMAKE);

    let (outcome, events) =
        run(&work, &install, &cmds, PhaseTimeouts::default(), CancellationToken::new(), None).await;

    // The installed binary exists.
    assert!(install.join("llama-server").exists());
    let complete = match outcome {
        BuildOutcome::Succeeded(c) => c,
        other => panic!("expected success, got {other:?}"),
    };
    assert!(complete.binary_path.ends_with("llama-server"));
    assert!(!complete.replaced_existing);

    // First event is a phase; phases appear in order; a complete event is sent.
    let phases: Vec<BuildPhase> = events
        .iter()
        .filter_map(|e| match e {
            BuildEvent::Phase { phase, .. } => Some(*phase),
            _ => None,
        })
        .collect();
    assert_eq!(phases, vec![BuildPhase::Cloning, BuildPhase::Configuring, BuildPhase::Building]);
    assert!(matches!(events.first(), Some(BuildEvent::Phase { .. })));
    assert!(matches!(events.last(), Some(BuildEvent::Complete(_))));
    assert!(events
        .iter()
        .any(|e| matches!(e, BuildEvent::Progress { percent, .. } if *percent >= 50)));
}

#[tokio::test]
async fn build_backs_up_existing_binary() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    let install = dir.path().join("install");
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&install).unwrap();
    fs::write(install.join("llama-server"), b"old binary").unwrap();
    let cmds = commands(dir.path(), MOCK_GIT, MOCK_CMAKE);

    let (outcome, _) =
        run(&work, &install, &cmds, PhaseTimeouts::default(), CancellationToken::new(), None).await;

    let complete = match outcome {
        BuildOutcome::Succeeded(c) => c,
        other => panic!("expected success, got {other:?}"),
    };
    assert!(complete.replaced_existing);
    let backup = complete.backup_path.expect("backup path");
    assert!(PathBuf::from(&backup).exists());
    assert!(backup.contains("llama-server.backup."));
}

#[tokio::test]
async fn build_cancellation_kills_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    let install = dir.path().join("install");
    fs::create_dir_all(&work).unwrap();
    let cmds = commands(dir.path(), MOCK_SLEEP, MOCK_CMAKE);

    let (outcome, _) = run(
        &work,
        &install,
        &cmds,
        PhaseTimeouts::default(),
        CancellationToken::new(),
        Some(Duration::from_millis(200)),
    )
    .await;

    assert_eq!(outcome, BuildOutcome::Cancelled);
}

#[tokio::test]
async fn build_phase_timeout_fails_with_message() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    let install = dir.path().join("install");
    fs::create_dir_all(&work).unwrap();
    let cmds = commands(dir.path(), MOCK_SLEEP, MOCK_CMAKE);

    // Tiny clone timeout against a 30s-sleeping git → clone phase times out.
    let timeouts = PhaseTimeouts {
        clone: Duration::from_millis(200),
        configure: Duration::from_secs(60),
        build: Duration::from_secs(60),
    };
    let (outcome, events) =
        run(&work, &install, &cmds, timeouts, CancellationToken::new(), None).await;

    match outcome {
        BuildOutcome::Failed(f) => {
            assert_eq!(f.phase, BuildPhase::Cloning);
            assert!(f.error.contains("timed out"), "{}", f.error);
        }
        other => panic!("expected timeout failure, got {other:?}"),
    }
    assert!(matches!(events.last(), Some(BuildEvent::Failed(_))));
}
