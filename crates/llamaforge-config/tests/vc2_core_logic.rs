use llamaforge_config::{
    ConfigHandle, ConfigLoadError, ConfigPaths, ConfigReader, ConfigWriter, ServerId,
    TomlFileConfigSource,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_vc2_1_end_to_end_reload() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");
    let rules_path = dir.path().join("priority_rules.toml");

    // 1. Initial valid TOML
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
"#,
    )
    .unwrap();
    fs::write(&rules_path, "").unwrap();

    let paths = ConfigPaths::new(main_path.clone(), rules_path, state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));

    // 2. Load into ConfigHandle
    let handle = ConfigHandle::load(source).unwrap();
    let mut rx = handle.subscribe();

    assert_eq!(handle.snapshot().servers.len(), 1);

    // 3. Modify TOML on disk
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

[[servers."00000000-0000-0000-0000-000000000001".lora_adapters]]
path = "lora1.gguf"
scale = 1.0

[servers."00000000-0000-0000-0000-000000000002"]
binary_path = "llama-server"
model_path = "m2.gguf"
port = 8081
ctx_size = 4096
"#,
    )
    .unwrap();

    // 4. Reload
    let diff = handle.reload().await.unwrap();

    // 5. Verify snapshot reflects changes and subscriber got diff
    let snap = handle.snapshot();
    assert_eq!(snap.servers.len(), 2);
    assert_eq!(
        snap.servers
            .get(&ServerId::from("00000000-0000-0000-0000-000000000001"))
            .unwrap()
            .lora_adapters
            .len(),
        1
    );

    let broadcast_diff = rx.recv().await.unwrap();
    assert_eq!(diff.changes.len(), 2);
    assert_eq!(broadcast_diff.changes.len(), 2);
}

#[tokio::test]
async fn test_vc2_3_concurrent_reload_stress() {
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
"#,
    )
    .unwrap();

    let paths = ConfigPaths::new(main_path, PathBuf::new(), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    let handle = Arc::new(ConfigHandle::load(source).unwrap());

    let mut tasks = vec![];
    for _ in 0..50 {
        let h = handle.clone();
        tasks.push(tokio::spawn(async move {
            // Source reading blocks briefly (I/O).
            // We just ensure no panics and serialization works.
            let _ = h.reload().await;
        }));
    }

    for t in tasks {
        t.await.unwrap();
    }

    // Verify final state is valid
    let snap = handle.snapshot();
    assert_eq!(snap.servers.len(), 1);
}

#[tokio::test]
async fn test_vc2_4_validation_rejects_bad_config() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");

    let check_val = |content: &str, expected: &str| {
        fs::write(&main_path, content).unwrap();
        let paths = ConfigPaths::new(main_path.clone(), PathBuf::new(), state_dir.clone());
        let source = Arc::new(TomlFileConfigSource::new(paths));
        let err = match ConfigHandle::load(source) {
            Err(e) => e,
            Ok(_) => panic!("expected validation error"),
        };
        if let ConfigLoadError::ValidationError { message } = err {
            assert!(message.contains(expected));
        } else {
            panic!("expected ValidationError");
        }
    };

    check_val("[servers.\"00000000-0000-0000-0000-000000000001\"]\nbinary_path='llama-server'\nmodel_path='m1'\nport=8080\nctx_size=1\n[servers.\"00000000-0000-0000-0000-000000000002\"]\nbinary_path='llama-server'\nmodel_path='m2'\nport=8080\nctx_size=1", "duplicate server port");
    check_val(
        "[servers.\"00000000-0000-0000-0000-000000000001\"]\nbinary_path='llama-server'\nmodel_path=''\nport=8080\nctx_size=1",
        "empty model_path",
    );
    check_val("[servers.\"00000000-0000-0000-0000-000000000001\"]\nbinary_path='llama-server'\nmodel_path='m1'\nport=8080\nctx_size=0", "ctx_size must be > 0");
}

#[tokio::test]
async fn test_vc2_5_backward_compatibility() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");

    // V0.9 TOML missing fields (e.g. parallel_slots, gpu_layers)
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
"#,
    )
    .unwrap();

    let paths = ConfigPaths::new(main_path, PathBuf::new(), state_dir);
    let source = Arc::new(TomlFileConfigSource::new(paths));
    let handle = ConfigHandle::load(source).unwrap();

    let snap = handle.snapshot();
    let server = snap.servers.get(&ServerId::from("00000000-0000-0000-0000-000000000001")).unwrap();

    // Check defaults were populated
    assert_eq!(server.gpu_layers, -1);
    assert_eq!(server.parallel_slots, 1);
    assert_eq!(snap.global.grpc_port, 50051);
}
