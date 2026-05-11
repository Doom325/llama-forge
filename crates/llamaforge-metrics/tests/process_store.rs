use std::sync::mpsc;

use chrono::Utc;
use tempfile::TempDir;

use llamaforge_config::ServerId;
use llamaforge_metrics::{
    create_read_pool, open_and_migrate, writer_thread_main, ManagedProcess, MetricsConfig,
    ProcessId, ProcessStore, SqliteMetricsStore,
};

async fn setup_store() -> (TempDir, SqliteMetricsStore) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("llamaforge.db");

    let (conn, lock_file) = open_and_migrate(&db_path).unwrap();
    let (cmd_tx, cmd_rx) = mpsc::sync_channel(100);

    let writer_handle = std::thread::spawn(move || {
        writer_thread_main(conn, cmd_rx, 10);
    });

    let config = MetricsConfig { db_path, ..Default::default() };

    let read_pool = create_read_pool(&config).unwrap();
    let store =
        SqliteMetricsStore::new_for_test(cmd_tx, read_pool, Some(writer_handle), 30, lock_file);

    (temp_dir, store)
}

#[tokio::test]
async fn test_process_store() {
    let (_temp, store) = setup_store().await;

    // Upsert 3 processes
    for i in 1..=3 {
        store
            .upsert_managed_process(&ManagedProcess {
                pid: ProcessId(i),
                server_id: ServerId(format!("s{}", i)),
                started_at: Utc::now(),
                cmdline: "cmd".to_string(),
                port: 8080,
                model_alias: "test-model".to_string(),
            })
            .await
            .unwrap();
    }

    // List -> 3
    let procs = store.list_managed_processes().await.unwrap();
    assert_eq!(procs.len(), 3);

    // Remove 1
    store.remove_managed_process(ProcessId(2)).await.unwrap();

    // List -> 2
    let procs = store.list_managed_processes().await.unwrap();
    assert_eq!(procs.len(), 2);
    assert!(procs.iter().any(|p| p.pid.0 == 1));
    assert!(procs.iter().any(|p| p.pid.0 == 3));

    // Upsert same PID (update)
    store
        .upsert_managed_process(&ManagedProcess {
            pid: ProcessId(1),
            server_id: ServerId("s1-updated".to_string()),
            started_at: Utc::now(),
            cmdline: "cmd2".to_string(),
            port: 8080,
            model_alias: "test-model".to_string(),
        })
        .await
        .unwrap();

    let procs = store.list_managed_processes().await.unwrap();
    assert_eq!(procs.len(), 2); // still 2
    let p1 = procs.iter().find(|p| p.pid.0 == 1).unwrap();
    assert_eq!(p1.server_id.0, "s1-updated");
}
