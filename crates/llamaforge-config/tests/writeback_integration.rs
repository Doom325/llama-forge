use llamaforge_config::{
    ConfigChange, ConfigHandle, ConfigMutation, ConfigPaths, ConfigReader, ConfigWriter,
    LoraAdapterConfig, ServerConfigField, ServerId, TomlFileConfigSource, WritebackError,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_filesystem_writeback_happy_path_and_comments() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");
    let rules_path = dir.path().join("priority_rules.toml");

    let initial_toml = r#"
# Global configuration comment
[global]
http_port = 8000

# Server 1 configuration comment
[servers.s1]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#;

    fs::write(&main_path, initial_toml).unwrap();
    fs::write(&rules_path, "").unwrap();

    let paths = ConfigPaths::new(main_path.clone(), rules_path, state_dir.clone());
    let source = Arc::new(TomlFileConfigSource::new(paths));
    let handle = ConfigHandle::load(source).unwrap();

    let mutation = ConfigMutation::SetLoraAdapters {
        server_id: ServerId::from("s1"),
        adapters: vec![LoraAdapterConfig { path: PathBuf::from("lora1.gguf"), scale: 0.8 }],
    };

    let diff = handle.writeback(mutation).await.unwrap();

    // Verify diff
    assert_eq!(diff.0.changes.len(), 1);
    match &diff.0.changes[0] {
        ConfigChange::ServerChanged { id, diff } => {
            assert_eq!(id.0, "s1");
            assert!(diff.hot_reloadable());
            assert!(diff.fields_changed.contains(&ServerConfigField::LoraAdapters));
        }
        _ => panic!("expected ServerChanged"),
    }

    // Verify snapshot updated
    let snap = handle.snapshot();
    let server = snap.servers.get(&ServerId::from("s1")).unwrap();
    assert_eq!(server.lora_adapters.len(), 1);
    assert_eq!(server.lora_adapters[0].path, PathBuf::from("lora1.gguf"));

    // Verify file contents and comments preserved
    let updated_toml = fs::read_to_string(&main_path).unwrap();
    assert!(updated_toml.contains("# Global configuration comment"));
    assert!(updated_toml.contains("# Server 1 configuration comment"));
    assert!(updated_toml.contains("lora_adapters"));
    assert!(updated_toml.contains("lora1.gguf"));
    assert!(updated_toml.contains("0.8") || updated_toml.contains("0.80"));

    // Verify temp file cleaned up
    let temp_path = state_dir.join("config.tmp");
    assert!(!temp_path.exists(), "temp file should be cleaned up after atomic rename");
}

#[tokio::test]
async fn test_filesystem_writeback_readonly() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");

    fs::write(
        &main_path,
        r#"
[global]
http_port = 8000
[servers.s1]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    // Make state_dir read-only so lock_file and temp_file creation fail
    let mut perms = fs::metadata(&state_dir).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&state_dir, perms).unwrap();

    let paths = ConfigPaths::new(main_path.clone(), PathBuf::new(), state_dir.clone());
    let source = Arc::new(TomlFileConfigSource::new(paths));
    let handle = ConfigHandle::load(source).unwrap();

    let mutation = ConfigMutation::SetLoraAdapters {
        server_id: ServerId::from("s1"),
        adapters: vec![LoraAdapterConfig { path: PathBuf::from("lora1.gguf"), scale: 0.8 }],
    };

    let err = handle.writeback(mutation).await.unwrap_err();
    assert!(matches!(err, WritebackError::ReadOnly { .. }));

    // Cleanup
    #[allow(clippy::permissions_set_readonly_false)]
    {
        let mut perms = fs::metadata(&state_dir).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(&state_dir, perms).unwrap();
    }
}
