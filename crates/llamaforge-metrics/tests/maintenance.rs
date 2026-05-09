use std::sync::mpsc;

use chrono::{Duration, Utc};
use tempfile::TempDir;

use llamaforge_metrics::{
    create_read_pool, open_and_migrate, writer_thread_main, AuditEvent, ConfigSnapshot,
    ConfigSnapshotTrigger, EventStore, EventType, MetricsConfig, SqliteMetricsStore,
    StoreMaintenance,
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
async fn test_retention_prune() {
    let (_temp, store) = setup_store().await;

    let now = Utc::now();
    let old_time = now - Duration::days(5);

    // Insert an old event and a new event
    store
        .insert_events(vec![
            AuditEvent {
                event_id: "evt-old".to_string(),
                event_type: EventType::Telemetry,
                event_kind: "test".to_string(),
                server_id: None,
                request_id: None,
                created_at: old_time,
                payload: serde_json::json!({}),
            },
            AuditEvent {
                event_id: "evt-new".to_string(),
                event_type: EventType::Telemetry,
                event_kind: "test".to_string(),
                server_id: None,
                request_id: None,
                created_at: now,
                payload: serde_json::json!({}),
            },
        ])
        .await
        .unwrap();

    // insert config snapshot
    store
        .insert_config_snapshot(&ConfigSnapshot {
            id: "snap-1".to_string(),
            trigger: ConfigSnapshotTrigger::Startup,
            created_at: now,
            config_json: "{}".to_string(),
        })
        .await
        .unwrap();

    // Check size
    let size = store.estimated_size_bytes().await.unwrap();
    assert!(size > 0);

    // Prune
    let cutoff = now - Duration::days(1);
    let deleted = store.prune_events_older_than(cutoff).await.unwrap();
    assert_eq!(deleted, 1); // Only the old event should be deleted

    // Check remaining events
    let events = store.query_events(&Default::default(), None, None).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "evt-new");
}
