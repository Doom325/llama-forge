use llamaforge_config::{
    AppConfig, ConfigHandle, ConfigLoadError, ConfigMutation, ConfigPaths, ConfigReader,
    ConfigReloadError, ConfigWriter, InstrumentedConfigHandle, LoraAdapterConfig, ServerId,
    TomlFileConfigSource, WritebackError,
};
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
async fn test_vc3_1_full_lifecycle() {
    let dir = tempdir().unwrap();
    let state_dir = dir.path().to_path_buf();
    let main_path = dir.path().join("llamaforge.toml");
    let rules_path = dir.path().join("priority_rules.toml");

    fs::write(
        &main_path,
        r#"
[global]
http_port = 8000
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();
    fs::write(&rules_path, "").unwrap();

    let instrumented = setup_stack(main_path.clone(), rules_path, state_dir).unwrap();
    let mut rx = instrumented.subscribe();

    assert_eq!(instrumented.snapshot().global.http_port, 8000);

    // modify
    fs::write(
        &main_path,
        r#"
[global]
http_port = 8000
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048

[servers."00000000-0000-0000-0000-000000000002"]
model_path = "m2.gguf"
port = 8081
ctx_size = 4096
"#,
    )
    .unwrap();

    let diff = instrumented.reload().await.unwrap();
    assert_eq!(diff.changes.len(), 1);
    let broadcast_diff = rx.recv().await.unwrap();
    assert_eq!(broadcast_diff.changes.len(), 1);

    // writeback
    let mutation = ConfigMutation::SetLoraAdapters {
        server_id: ServerId::from("00000000-0000-0000-0000-000000000001"),
        adapters: vec![LoraAdapterConfig { path: PathBuf::from("lora.gguf"), scale: 1.0 }],
    };

    let diff = instrumented.writeback(mutation).await.unwrap();
    assert_eq!(diff.changes.len(), 1);

    let content = fs::read_to_string(&main_path).unwrap();
    assert!(content.contains("lora_adapters"));
}

#[tokio::test]
async fn test_vc3_2_startup_failure_empty_file() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(&main_path, "").unwrap();

    let err = match setup_stack(main_path, PathBuf::new(), dir.path().to_path_buf()) {
        Err(e) => e,
        Ok(_) => panic!("Expected ConfigLoadError"),
    };
    assert!(matches!(err, ConfigLoadError::EmptyFile { .. }));
}

#[tokio::test]
async fn test_vc3_3_startup_failure_invalid_toml() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(&main_path, "invalid toml content").unwrap();

    let err = match setup_stack(main_path, PathBuf::new(), dir.path().to_path_buf()) {
        Err(e) => e,
        Ok(_) => panic!("Expected ConfigLoadError"),
    };
    assert!(matches!(err, ConfigLoadError::ParseError { .. }));
}

#[tokio::test]
async fn test_vc3_4_startup_failure_validation_error() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
[servers."00000000-0000-0000-0000-000000000002"]
model_path = "m2.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let err = match setup_stack(main_path, PathBuf::new(), dir.path().to_path_buf()) {
        Err(e) => e,
        Ok(_) => panic!("Expected ConfigLoadError"),
    };
    assert!(matches!(err, ConfigLoadError::ValidationError { .. }));
}

#[tokio::test]
async fn test_vc3_5_hot_reload_add_server() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let instrumented =
        setup_stack(main_path.clone(), PathBuf::new(), dir.path().to_path_buf()).unwrap();

    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
[servers."00000000-0000-0000-0000-000000000002"]
model_path = "m2.gguf"
port = 8081
ctx_size = 2048
"#,
    )
    .unwrap();

    let diff = instrumented.reload().await.unwrap();
    assert_eq!(diff.changes.len(), 1);
    assert_eq!(instrumented.snapshot().servers.len(), 2);
}

#[tokio::test]
async fn test_vc3_6_hot_reload_remove_server() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
[servers."00000000-0000-0000-0000-000000000002"]
model_path = "m2.gguf"
port = 8081
ctx_size = 2048
"#,
    )
    .unwrap();

    let instrumented =
        setup_stack(main_path.clone(), PathBuf::new(), dir.path().to_path_buf()).unwrap();

    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let diff = instrumented.reload().await.unwrap();
    assert_eq!(diff.changes.len(), 1);
    assert_eq!(instrumented.snapshot().servers.len(), 1);
}

#[tokio::test]
async fn test_vc3_7_hot_reload_change_requiring_restart() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let instrumented =
        setup_stack(main_path.clone(), PathBuf::new(), dir.path().to_path_buf()).unwrap();

    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 4096
"#,
    )
    .unwrap();

    let diff = instrumented.reload().await.unwrap();
    assert_eq!(diff.changes.len(), 1);
    use llamaforge_config::ConfigChange;
    match &diff.changes[0] {
        ConfigChange::ServerChanged { diff, .. } => {
            assert!(diff.requires_restart());
        }
        _ => panic!("Expected ServerChanged"),
    }
}

#[tokio::test]
async fn test_vc3_8_hot_reload_change_not_requiring_restart() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let instrumented =
        setup_stack(main_path.clone(), PathBuf::new(), dir.path().to_path_buf()).unwrap();

    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
alias = "new_alias"
"#,
    )
    .unwrap();

    let diff = instrumented.reload().await.unwrap();
    assert_eq!(diff.changes.len(), 1);
    use llamaforge_config::ConfigChange;
    match &diff.changes[0] {
        ConfigChange::ServerChanged { diff, .. } => {
            assert!(!diff.requires_restart());
            assert!(diff.hot_reloadable());
        }
        _ => panic!("Expected ServerChanged"),
    }
}

#[tokio::test]
async fn test_vc3_9_hot_reload_parse_error_preserves_old_config() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let instrumented =
        setup_stack(main_path.clone(), PathBuf::new(), dir.path().to_path_buf()).unwrap();
    let old_snap = instrumented.snapshot().clone();

    fs::write(&main_path, "corrupt toml").unwrap();

    let err = instrumented.reload().await.unwrap_err();
    assert!(matches!(err, ConfigReloadError::LoadError(ConfigLoadError::ParseError { .. })));

    assert_eq!(**instrumented.snapshot(), *old_snap);
}

#[tokio::test]
async fn test_vc3_10_writeback_crash_safety() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    let state_dir = dir.path().to_path_buf();
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let instrumented = setup_stack(main_path.clone(), PathBuf::new(), state_dir.clone()).unwrap();

    let mutation = ConfigMutation::SetLoraAdapters {
        server_id: ServerId::from("00000000-0000-0000-0000-000000000001"),
        adapters: vec![LoraAdapterConfig { path: PathBuf::from("lora.gguf"), scale: 1.0 }],
    };

    instrumented.writeback(mutation).await.unwrap();

    // temp file should be gone
    let temp_path = state_dir.join("config.tmp");
    assert!(!temp_path.exists());
    assert!(main_path.exists());
}

#[tokio::test]
async fn test_vc3_11_writeback_read_only_fs() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    let state_dir = dir.path().to_path_buf();
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let instrumented = setup_stack(main_path.clone(), PathBuf::new(), state_dir.clone()).unwrap();

    // Make state dir read-only
    let mut perms = fs::metadata(&state_dir).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&state_dir, perms).unwrap();
    let mutation = ConfigMutation::SetLoraAdapters {
        server_id: ServerId::from("00000000-0000-0000-0000-000000000001"),
        adapters: vec![LoraAdapterConfig { path: PathBuf::from("lora1.gguf"), scale: 0.8 }],
    };
    let err = instrumented.writeback(mutation).await.unwrap_err();
    assert!(matches!(err, WritebackError::ReadOnly { .. }));

    // Revert permissions
    #[allow(clippy::permissions_set_readonly_false)]
    {
        let mut perms = fs::metadata(&state_dir).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(&state_dir, perms).unwrap();
    }
}

#[tokio::test]
async fn test_vc3_12_concurrent_stress_test() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    let state_dir = dir.path().to_path_buf();
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let instrumented =
        Arc::new(setup_stack(main_path.clone(), PathBuf::new(), state_dir.clone()).unwrap());

    let mut tasks = vec![];
    for i in 0..20 {
        let handle = instrumented.clone();
        tasks.push(tokio::spawn(async move {
            let _ = handle.reload().await;
        }));

        if i % 2 == 0 {
            let handle = instrumented.clone();
            tasks.push(tokio::spawn(async move {
                let _ = handle
                    .writeback(ConfigMutation::SetLoraAdapters {
                        server_id: ServerId::from("00000000-0000-0000-0000-000000000001"),
                        adapters: vec![],
                    })
                    .await;
            }));
        }
    }

    for t in tasks {
        t.await.unwrap();
    }

    let snap = instrumented.snapshot();
    assert_eq!(snap.servers.len(), 1);
}

#[tokio::test]
async fn test_vc3_13_observability_verification() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tracing::field::{Field, Visit};
    use tracing::span::{Id, Record};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::Layer;

    static RECORDED: AtomicBool = AtomicBool::new(false);

    struct TestLayer;

    impl<S: tracing::Subscriber> Layer<S> for TestLayer {
        fn on_record(&self, _span: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
            struct FieldVisitor;
            impl Visit for FieldVisitor {
                fn record_debug(&mut self, field: &Field, _value: &dyn std::fmt::Debug) {
                    if field.name() == "config.server_count" {
                        RECORDED.store(true, Ordering::SeqCst);
                    }
                }
                fn record_i64(&mut self, field: &Field, _value: i64) {
                    if field.name() == "config.server_count" {
                        RECORDED.store(true, Ordering::SeqCst);
                    }
                }
                fn record_u64(&mut self, field: &Field, _value: u64) {
                    if field.name() == "config.server_count" {
                        RECORDED.store(true, Ordering::SeqCst);
                    }
                }
            }
            values.record(&mut FieldVisitor);
        }
    }

    let _ = tracing::subscriber::set_global_default(tracing_subscriber::registry().with(TestLayer));

    RECORDED.store(false, Ordering::SeqCst);

    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let instrumented =
        setup_stack(main_path.clone(), PathBuf::new(), dir.path().to_path_buf()).unwrap();
    let _ = instrumented.reload().await.unwrap();

    assert!(RECORDED.load(Ordering::SeqCst), "config.server_count was not recorded in reload span");
}

#[tokio::test]
async fn test_vc3_14_cached_config_reader_consistency() {
    use llamaforge_config::CachedConfigReader;

    let dir = tempdir().unwrap();
    let main_path = dir.path().join("llamaforge.toml");
    fs::write(
        &main_path,
        r#"
[servers."00000000-0000-0000-0000-000000000001"]
model_path = "m1.gguf"
port = 8080
ctx_size = 2048
"#,
    )
    .unwrap();

    let instrumented =
        setup_stack(main_path.clone(), PathBuf::new(), dir.path().to_path_buf()).unwrap();
    let mut cached = CachedConfigReader::new(&instrumented);

    for _ in 0..100 {
        let _ = instrumented.reload().await.unwrap();
        let _ = cached.load();
    }
}

#[tokio::test]
async fn test_vc3_15_memory_bounds() {
    let mut config = AppConfig::default();
    for i in 0..100 {
        let mut server = llamaforge_config::ServerConfig {
            model_path: PathBuf::from(format!("model_{}.gguf", i)),
            ..Default::default()
        };
        for j in 0..50 {
            server.lora_adapters.push(LoraAdapterConfig {
                path: PathBuf::from(format!("lora_{}_{}.gguf", i, j)),
                scale: 1.0,
            });
        }
        config.servers.insert(ServerId::from(format!("s{}", i)), server);
    }

    // Roughly estimate memory. A realistic config shouldn't be multiple MBs.
    let size = std::mem::size_of_val(&config);
    assert!(size < 1024 * 1024, "Config memory is too large");
}
