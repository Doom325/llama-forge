use llamaforge_config::{
    ConfigHandle, ConfigLoadError, ConfigMutation, ConfigPaths, ConfigReader, ConfigWriter,
    InstrumentedConfigHandle, LoraAdapterConfig, TomlFileConfigSource, WritebackError,
};
use llamaforge_types::ServerId;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

fn setup_stack(
    main_path: PathBuf,
    rules_path: PathBuf,
    state_dir: PathBuf,
) -> Result<InstrumentedConfigHandle, ConfigLoadError> {
    let paths = ConfigPaths::new(main_path, rules_path, state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    let handle = ConfigHandle::load(source)?;
    Ok(InstrumentedConfigHandle::new(handle))
}

#[tokio::test]
async fn test_concurrent_sighups_serialized() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");

    fs::write(
        &main_path,
        r#"
[global]
http_port = 8000
[servers."00000000-0000-0000-0000-000000000001"]
binary_path = "llama-server"
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
parallel_slots = 4
"#,
    )
    .unwrap();

    let instrumented = Arc::new(setup_stack(main_path.clone(), PathBuf::new(), state_dir).unwrap());

    let mut tasks = vec![];
    for _ in 0..10 {
        let handle = instrumented.clone();
        tasks.push(tokio::spawn(async move {
            let _ = handle.reload().await;
        }));
    }

    for t in tasks {
        t.await.unwrap();
    }
}

#[tokio::test]
async fn test_writeback_atomic_rename() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");

    fs::write(
        &main_path,
        r#"
[global]
http_port = 8000
[servers."00000000-0000-0000-0000-000000000001"]
binary_path = "llama-server"
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
parallel_slots = 4
"#,
    )
    .unwrap();

    let instrumented = setup_stack(main_path.clone(), PathBuf::new(), state_dir.clone()).unwrap();

    let mutation = ConfigMutation::SetLoraAdapters {
        server_id: ServerId::from_config_id("00000000-0000-0000-0000-000000000001"),
        adapters: vec![LoraAdapterConfig { path: PathBuf::from("lora.gguf"), scale: 1.0 }],
    };

    instrumented.writeback(mutation).await.unwrap();

    let temp_path = state_dir.join("config.tmp");
    assert!(!temp_path.exists());
    assert!(main_path.exists());
}

#[tokio::test]
async fn test_empty_config_at_startup_exits() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(&main_path, "").unwrap();

    let err = match setup_stack(main_path, PathBuf::new(), dir.path().to_path_buf()) {
        Err(e) => e,
        Ok(_) => panic!("expected ConfigLoadError"),
    };
    assert!(matches!(err, ConfigLoadError::EmptyFile { .. }));
}

#[tokio::test]
async fn test_serde_default_backward_compat() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");

    fs::write(
        &main_path,
        r#"
[global]
http_port = 8000
[servers."00000000-0000-0000-0000-000000000001"]
binary_path = "llama-server"
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
parallel_slots = 4
"#,
    )
    .unwrap();

    let instrumented = setup_stack(main_path, PathBuf::new(), state_dir).unwrap();
    let snap = instrumented.snapshot();

    assert_eq!(snap.global.grpc_port, 50051);
    let server = snap
        .servers
        .get(&ServerId::from_config_id("00000000-0000-0000-0000-000000000001"))
        .unwrap();
    assert_eq!(server.gpu_layers, -1);
}

#[tokio::test]
async fn test_broadcast_lagged_subscriber() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");

    fs::write(
        &main_path,
        r#"
[global]
http_port = 8000
[servers."00000000-0000-0000-0000-000000000001"]
binary_path = "llama-server"
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
parallel_slots = 4
"#,
    )
    .unwrap();

    let instrumented = setup_stack(main_path, PathBuf::new(), state_dir).unwrap();
    let mut rx = instrumented.subscribe();

    for _ in 0..20 {
        instrumented.reload().await.unwrap();
    }

    let err = rx.try_recv().unwrap_err();
    assert!(matches!(err, tokio::sync::broadcast::error::TryRecvError::Lagged(_)));
}

#[tokio::test]
async fn test_readonly_fs_writeback_graceful() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");

    fs::write(
        &main_path,
        r#"
[global]
http_port = 8000
[servers."00000000-0000-0000-0000-000000000001"]
binary_path = "llama-server"
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
parallel_slots = 4
"#,
    )
    .unwrap();

    let instrumented = setup_stack(main_path.clone(), PathBuf::new(), state_dir.clone()).unwrap();

    // Make state dir read-only
    let mut perms = fs::metadata(&state_dir).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&state_dir, perms).unwrap();
    let mutation = ConfigMutation::SetLoraAdapters {
        server_id: ServerId::from_config_id("00000000-0000-0000-0000-000000000001"),
        adapters: vec![LoraAdapterConfig { path: PathBuf::from("lora.gguf"), scale: 1.0 }],
    };
    let err = instrumented.writeback(mutation).await.unwrap_err();
    assert!(matches!(err, WritebackError::ReadOnly { .. }));

    // Cleanup
    #[allow(clippy::permissions_set_readonly_false)]
    {
        let mut perms = fs::metadata(&state_dir).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(&state_dir, perms).unwrap();
    }
}
