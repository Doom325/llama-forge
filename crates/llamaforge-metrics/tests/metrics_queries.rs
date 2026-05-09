use std::sync::mpsc;

use chrono::{TimeZone, Utc};
use tempfile::TempDir;

use llamaforge_config::{RequestorId, ServerId, TaskTypeId};
use llamaforge_metrics::{
    create_read_pool, open_and_migrate, writer_thread_main, GroupByField, MetricsConfig,
    MetricsFilter, RequestId, RequestMetrics, RequestMetricsStore, RequestStatus,
    SqliteMetricsStore,
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
async fn test_metrics_queries() {
    let (_temp, store) = setup_store().await;

    // Insert 10 metrics
    let mut metrics = Vec::new();
    let base_time = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();

    for i in 0..10 {
        metrics.push(RequestMetrics {
            request_id: RequestId(format!("req-{}", i)),
            model_name: if i < 5 { "modelA".to_string() } else { "modelB".to_string() },
            server_id: Some(ServerId(if i % 2 == 0 { "s1".to_string() } else { "s2".to_string() })),
            slot_id: None,
            task_type: TaskTypeId("test".to_string()),
            requestor: RequestorId(if i < 3 { "u1".to_string() } else { "u2".to_string() }),
            effective_priority: 1,
            received_at: base_time + chrono::Duration::hours(i),
            dispatched_at: None,
            first_token_at: None,
            completed_at: None,
            queue_wait_ms: 0.0,
            ttft_ms: None,
            generation_duration_ms: None,
            total_latency_ms: 100.0 * (i as f64 + 1.0), // 100, 200, 300...
            prompt_tokens: 10,
            completion_tokens: 20,
            prompt_tps: None,
            generation_tps: None,
            status: if i == 9 { RequestStatus::Failed } else { RequestStatus::Completed },
            finish_reason: None,
            was_evicted: false,
            eviction_count: 0,
            error_message: None,
        });
    }

    store.insert_request_metrics(metrics).await.unwrap();

    // Query empty filter
    let res = store.query_request_metrics(&MetricsFilter::default(), None, None).await.unwrap();
    assert_eq!(res.len(), 10);

    // Query by requestor
    let res = store
        .query_request_metrics(
            &MetricsFilter { requestor: Some(RequestorId("u1".to_string())), ..Default::default() },
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(res.len(), 3);

    // Aggregate by ServerId
    let agg = store
        .aggregate_request_metrics(&MetricsFilter::default(), &[GroupByField::ServerId])
        .await
        .unwrap();
    assert_eq!(agg.len(), 2); // s1, s2

    let s1_agg = agg.iter().find(|a| a.group_key == "server_id=s1").unwrap();
    assert_eq!(s1_agg.request_count, 5); // 0, 2, 4, 6, 8 -> 100, 300, 500, 700, 900
    assert_eq!(s1_agg.avg_total_latency_ms, Some(500.0));
    assert_eq!(s1_agg.sum_prompt_tokens, 50);
    // Aggregate by Hour
    let agg = store
        .aggregate_request_metrics(&MetricsFilter::default(), &[GroupByField::Hour])
        .await
        .unwrap();
    assert_eq!(agg.len(), 10); // 10 distinct hours
    assert!(!agg[0].group_key.is_empty());
}
