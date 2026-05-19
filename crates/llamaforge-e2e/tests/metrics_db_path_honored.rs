use std::sync::Arc;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn test_metrics_db_path_honored() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(&config_dir)?;

    let explicit_db_path = config_dir.join("explicit.db");
    
    // Ensure we are in a known CWD, maybe inside tmp so we don't pollute the real workspace
    let cwd = std::env::current_dir()?;
    let default_db_path = cwd.join("llamaforge.db");

    // Clean up default_db_path just in case it exists from previous runs
    let _ = std::fs::remove_file(&default_db_path);
    let _ = std::fs::remove_file(cwd.join("llamaforge.db-shm"));
    let _ = std::fs::remove_file(cwd.join("llamaforge.db-wal"));

    let toml = format!(
        r#"
[global]
bind_address = "127.0.0.1"
http_port = 0
grpc_port = 0
metrics_db_path = "{explicit}"
"#,
        explicit = explicit_db_path.display()
    );

    let config_path = config_dir.join("llamaforge.toml");
    std::fs::write(&config_path, toml)?;

    let paths = llamaforge_config::ConfigPaths::new(
        config_path.clone(),
        config_dir.join("priority_rules.toml"),
        config_dir.join("state"),
    );
    let source = Arc::new(llamaforge_config::TomlFileConfigSource::new(paths));
    let handle = Arc::new(llamaforge_config::ConfigHandle::load(source)?);

    let shutdown = CancellationToken::new();
    let (deps, bg) = llamaforge::build(handle, shutdown.clone()).await?;

    // building should have created the metrics DB
    assert!(explicit_db_path.exists(), "metrics DB was not created at explicit path");
    assert!(!default_db_path.exists(), "metrics DB was created at default path instead of explicit");

    // clean up
    shutdown.cancel();
    
    // Drop the dependencies to let the sqlite connection close
    drop(deps);
    drop(bg);

    Ok(())
}