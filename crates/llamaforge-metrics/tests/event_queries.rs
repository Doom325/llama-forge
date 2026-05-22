use std::sync::mpsc;

use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use llamaforge_metrics::{
    create_read_pool, open_and_migrate, writer_thread_main, AuditEvent, EventFilter, EventStore,
    EventType, MetricsConfig, RequestId, SqliteMetricsStore,
};
use llamaforge_core::types::ServerId;

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
async fn test_event_queries() {
    let (_temp, store) = setup_store().await;

    // Insert 10 events
    let mut events = Vec::new();
    let base_time = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();

    for i in 0..10 {
        let event_type = if i % 2 == 0 { EventType::Audit } else { EventType::Telemetry };
        let server_id = if i < 5 {
            Some(ServerId::from_config_id("s1"))
        } else {
            Some(ServerId::from_config_id("s2"))
        };

        events.push(AuditEvent {
            event_id: format!("evt-{}", i),
            event_type,
            event_kind: "test".to_string(),
            server_id,
            request_id: Some(RequestId(format!("req-{}", i))),
            created_at: base_time + chrono::Duration::hours(i),
            payload: serde_json::json!({}),
        });
    }

    store.insert_events(events).await.unwrap();

    // Query empty filter -> all 10 (sorted desc by default)
    let res = store.query_events(&EventFilter::default(), None, None).await.unwrap();
    assert_eq!(res.len(), 10);
    assert_eq!(res[0].event_id, "evt-9"); // newest first

    // Query by server_id
    let res = store
        .query_events(
            &EventFilter { server_id: Some(ServerId::from_config_id("s1")), ..Default::default() },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(res.len(), 5);
    assert!(res.iter().all(|e| e.server_id.as_ref() == Some(&ServerId::from_config_id("s1"))));

    // Query by event_type
    let res = store
        .query_events(
            &EventFilter { event_type: Some(EventType::Telemetry), ..Default::default() },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(res.len(), 5);
    assert!(res.iter().all(|e| e.event_type == EventType::Telemetry));

    // Query by time range
    let res = store
        .query_events(
            &EventFilter {
                since: Some(base_time + chrono::Duration::hours(2)),
                until: Some(base_time + chrono::Duration::hours(4)),
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(res.len(), 3); // hours 2, 3, 4

    // Query by request_id
    let res = store
        .query_events(
            &EventFilter { request_id: Some(RequestId("req-3".to_string())), ..Default::default() },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].event_id, "evt-3");

    // Pagination
    let res = store.query_events(&EventFilter::default(), Some(3), Some(2)).await.unwrap();
    assert_eq!(res.len(), 3);
    assert_eq!(res[0].event_id, "evt-7"); // events sorted desc: 9, 8, [7, 6, 5]
    assert_eq!(res[2].event_id, "evt-5");
}
