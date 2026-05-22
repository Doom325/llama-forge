use std::sync::Arc;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn test_metrics_db_path_honored() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let config_dir = tmp.path().join("config");
    std::fs::create_dir_all(&config_dir)?;

    let explicit_db_path = config_dir.join("explicit.db");

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

    let paths = llamaforge_core::config::ConfigPaths::new(
        config_path.clone(),
        config_dir.join("priority_rules.toml"),
        config_dir.join("state"),
    );
    let source = Arc::new(llamaforge_core::config::TomlFileConfigSource::new(paths));
    let handle = Arc::new(llamaforge_core::config::ConfigHandle::load(source)?);

    let shutdown = CancellationToken::new();
    let (deps, bg) = llamaforge::build(handle, shutdown.clone()).await?;

    // building should have created the metrics DB
    assert!(explicit_db_path.exists(), "metrics DB was not created at explicit path");

    // clean up
    shutdown.cancel();

    // Drop the dependencies to let the sqlite connection close
    drop(deps);
    drop(bg);

    Ok(())
}
