use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use tokio::process::Command;

fn get_bin(name: &str) -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("target"))
            .unwrap_or_else(|| PathBuf::from("target"))
    });
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    target.join(profile).join(name)
}

fn mock_llama_bin() -> PathBuf {
    get_bin("mock-llama")
}

fn llamaforge_bin() -> PathBuf {
    get_bin("llamaforge")
}

#[tokio::test]
async fn test_sigint_orphans() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();

    let config_path = tmp.path().join("llamaforge.toml");
    let mock_bin = mock_llama_bin();
    let port = 18105; // Arbitrary port

    let toml = format!(
        r#"
[global]
bind_address = "127.0.0.1"
http_port = 0
grpc_port = 0

[gateway]

[servers.mock]
binary_path = "{binary}"
model_path = "model.gguf"
port = {port}
ctx_size = 1024
parallel_slots = 1
alias = "mock"
estimated_vram_mb = 100
"#,
        binary = mock_bin.display(),
        port = port,
    );
    std::fs::write(&config_path, toml).unwrap();

    let mut cmd = Command::new(llamaforge_bin());
    cmd.arg("serve");
    cmd.arg("--config").arg(&config_path);
    cmd.arg("--state-dir").arg(&state_dir);

    // Run as an independent process group so we can SIGINT it
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }

    let mut child = cmd.spawn().unwrap();

    // Wait for the server to spawn by polling the mock-llama port or waiting a bit.
    // Give it 3 seconds to boot and spawn mock-llama.
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify mock-llama is running
    #[cfg(unix)]
    {
        let check_cmd = std::process::Command::new("pgrep")
            .arg("-f")
            .arg(mock_bin.to_string_lossy().as_ref())
            .output()
            .unwrap();
        assert!(check_cmd.status.success(), "mock-llama process should be running");
    }

    // Send SIGINT
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .arg("-2") // SIGINT
            .arg(child.id().unwrap().to_string())
            .status()
            .unwrap();
    }

    // Wait for orchestrator exit
    let status = child.wait().await.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert!(
            status.success() || status.code() == Some(0) || status.signal() == Some(2),
            "Expected clean exit or SIGINT termination"
        );
    }

    // Verify mock-llama is NOT running
    #[cfg(unix)]
    {
        let check_cmd = std::process::Command::new("pgrep")
            .arg("-f")
            .arg(mock_bin.to_string_lossy().as_ref())
            .output()
            .unwrap();
        assert!(
            !check_cmd.status.success(),
            "mock-llama process should NOT be running after orchestrator exit"
        );
    }

    // Check that managed_processes table is empty
    let db_path = state_dir.join("metrics.db");
    if db_path.exists() {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM managed_processes", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0, "managed_processes table should be empty");
    }
}
