use chrono::{Duration, TimeZone, Utc};
use proptest::prelude::*;
use tempfile::TempDir;

use llamaforge_core::types::ServerId;
use llamaforge_metrics::{
    create_metrics_pipeline, AuditEvent, EventFilter, EventStore, EventType, MetricsConfig,
};

prop_compose! {
    fn arb_audit_event()(
        event_id in "[a-zA-Z0-9_-]{10,20}",
        event_type in prop_oneof![Just(EventType::Audit), Just(EventType::Telemetry)],
        event_kind in "[a-z_]{5,15}",
        server_id in prop::option::of("[a-z0-9-]{5,10}").prop_map(|s| s.map(|x| ServerId::from_config_id(&x))),
        timestamp in any::<u32>().prop_map(|t| Utc.timestamp_opt(t as i64, 0).unwrap()),
    ) -> AuditEvent {
        AuditEvent {
            event_id,
            event_type,
            event_kind,
            server_id,
            request_id: None,
            created_at: timestamp,
            payload: serde_json::json!({}),
        }
    }
}

async fn setup_store() -> (TempDir, llamaforge_metrics::MetricsPipeline) {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("llamaforge.toml");
    let priority_rules = temp_dir.path().join("priority_rules.toml");
    let state_dir = temp_dir.path().join("state");
    std::fs::create_dir(&state_dir).unwrap();

    let db_path = temp_dir.path().join("llamaforge.db");

    let mut file = std::fs::File::create(&config_path).unwrap();
    use std::io::Write;
    writeln!(file, "[global]").unwrap();
    writeln!(file, "metrics_db_path = \"{}\"", db_path.to_str().unwrap()).unwrap();
    writeln!(file, "metrics_retention_days = 999999").unwrap();
    std::fs::write(&priority_rules, "").unwrap();

    let paths = llamaforge_core::config::ConfigPaths::new(config_path, priority_rules, state_dir);
    let source = llamaforge_core::config::TomlFileConfigSource::new(paths);
    let handle = llamaforge_core::config::ConfigHandle::load(std::sync::Arc::new(source)).unwrap();
    let reader: std::sync::Arc<dyn llamaforge_core::config::ConfigReader> =
        std::sync::Arc::new(handle);

    let config = MetricsConfig { db_path, retention_days: 999999, ..Default::default() };
    let pipeline = create_metrics_pipeline(&config, reader).unwrap();
    (temp_dir, pipeline)
}

proptest! {
    // We use a small number of cases to keep tests fast, but these can be run heavily if needed.
    #![proptest_config(ProptestConfig::with_cases(10))]

    #[test]
    #[ignore]
    fn test_roundtrip_insert_query(event in arb_audit_event()) {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (_temp, pipeline) = setup_store().await;

            pipeline.store.insert_events(vec![event.clone()]).await.unwrap();

            let filter = EventFilter {
                event_type: Some(event.event_type),
                ..Default::default()
            };

            let res = pipeline.store.query_events(&filter, None, None).await.unwrap();

            // Proptest might generate duplicate event IDs if we're not careful, but the strategy is quite large.
            let found = res.iter().find(|e| e.event_id == event.event_id).expect("Event not found");
            assert_eq!(found.event_type, event.event_type);
            assert_eq!(found.event_kind, event.event_kind);
            assert_eq!(found.server_id, event.server_id);
            assert_eq!(found.created_at, event.created_at);

            pipeline.shutdown().await;
        });
    }

    #[test]
    #[ignore]
    fn test_retention_correctness(
        events in prop::collection::vec(arb_audit_event(), 1..50),
        cutoff_offset in 0..1000u32
    ) {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (_temp, pipeline) = setup_store().await;

            // Deduplicate events by ID to avoid SQLite PK constraint errors in tests
            let mut unique_events = std::collections::HashMap::new();
            for e in events {
                unique_events.insert(e.event_id.clone(), e);
            }
            let events: Vec<AuditEvent> = unique_events.into_values().collect();

            // Find min timestamp
            let min_ts = events.iter().map(|e| e.created_at).min().unwrap_or(Utc::now());
            let cutoff = min_ts + Duration::seconds(cutoff_offset as i64);

            pipeline.store.insert_events(events.clone()).await.unwrap();

            use llamaforge_metrics::StoreMaintenance;
            let deleted = pipeline.store.prune_events_older_than(cutoff).await.unwrap();

            let remaining = pipeline.store.query_events(&EventFilter::default(), None, None).await.unwrap();

            assert_eq!(remaining.len() as u64 + deleted, events.len() as u64);
            for e in remaining {
                assert!(e.created_at >= cutoff);
            }

            pipeline.shutdown().await;
        });
    }

    #[test]
    #[ignore]
    fn test_ordering_created_at(
        events in prop::collection::vec(arb_audit_event(), 1..50)
    ) {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (_temp, pipeline) = setup_store().await;

            let mut unique_events = std::collections::HashMap::new();
            for e in events {
                unique_events.insert(e.event_id.clone(), e);
            }
            let events: Vec<AuditEvent> = unique_events.into_values().collect();

            pipeline.store.insert_events(events).await.unwrap();

            let res = pipeline.store.query_events(&EventFilter::default(), None, None).await.unwrap();

            let mut last_ts = None;
            for e in res {
                if let Some(ts) = last_ts {
                    assert!(e.created_at <= ts);
                }
                last_ts = Some(e.created_at);
            }

            pipeline.shutdown().await;
        });
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn test_concurrent_read_during_write() {
    let (_temp, pipeline) = setup_store().await;
    let pipeline = std::sync::Arc::new(tokio::sync::Mutex::new(Some(pipeline)));

    // Spawn writer task: 100 inserts/sec for 2 seconds
    let pipeline_clone = pipeline.clone();
    let writer_task = tokio::spawn(async move {
        for i in 0..200 {
            let event = AuditEvent {
                event_id: format!("evt-c-{}", i),
                event_type: EventType::Audit,
                event_kind: "concurrent_test".to_string(),
                server_id: None,
                request_id: None,
                created_at: Utc::now(),
                payload: serde_json::json!({}),
            };
            if let Some(p) = pipeline_clone.lock().await.as_ref() {
                p.store.insert_events(vec![event]).await.unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });

    // Spawn reader tasks
    let mut reader_tasks = Vec::new();
    for _ in 0..10 {
        let pipeline_clone = pipeline.clone();
        reader_tasks.push(tokio::spawn(async move {
            for _ in 0..50 {
                if let Some(p) = pipeline_clone.lock().await.as_ref() {
                    let _ = p
                        .store
                        .query_events(&EventFilter::default(), Some(10), None)
                        .await
                        .unwrap();
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }));
    }

    // Wait for all to finish, expecting no timeouts
    let res = tokio::time::timeout(std::time::Duration::from_secs(30), async move {
        writer_task.await.unwrap();
        for task in reader_tasks {
            task.await.unwrap();
        }
    })
    .await;

    assert!(res.is_ok(), "Concurrent read/write test timed out!");

    // Clean up
    let p_opt = pipeline.lock().await.take();
    if let Some(p) = p_opt {
        p.shutdown().await;
    }
}
