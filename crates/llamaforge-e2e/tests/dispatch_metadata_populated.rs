//! Regression for #230. Every `GET /api/metrics/requests` row had
//! `dispatched_at: null` and `slot_id: null` — the writer never populated
//! the dispatch metadata, so `queue_wait_ms` was structurally always 0 and
//! queue-vs-dispatch latency could not be diagnosed. After a successful
//! completion the persisted row must carry both fields.

use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_metrics_row_has_dispatch_metadata() {
    let orch = Harness::start_full().await.expect("orchestrator start");
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": "mock",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": false,
    });
    let res = client
        .post(format!("{}/v1/chat/completions", orch.http_url))
        .json(&body)
        .send()
        .await
        .expect("send chat completion");
    assert_eq!(res.status(), 200, "chat completion should succeed");
    let _ = res.text().await;

    // The audit row lands via the SQLite batch collector, which flushes on a
    // timer — poll until the row materializes (bounded), then assert on it.
    let mut last_seen = serde_json::Value::Null;
    let mut row = None;
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let resp = client
            .get(format!("{}/api/metrics/requests?limit=1", orch.http_url))
            .send()
            .await
            .expect("scrape /api/metrics/requests");
        if resp.status() != reqwest::StatusCode::OK {
            continue;
        }
        let json: serde_json::Value = resp.json().await.expect("requests body is JSON");
        last_seen = json.clone();
        if let Some(first) = json.as_array().and_then(|a| a.first()) {
            row = Some(first.clone());
            break;
        }
    }

    let row = row.unwrap_or_else(|| {
        panic!("no persisted request row after a completion; last response: {last_seen}")
    });

    assert!(!row["dispatched_at"].is_null(), "dispatched_at must be populated (#230), row: {row}");
    assert!(!row["slot_id"].is_null(), "slot_id must be populated (#230), row: {row}");
    // queue_wait_ms is derived from dispatched_at - received_at; it must be a
    // real, non-negative number now that dispatched_at exists.
    assert!(
        row["queue_wait_ms"].as_f64().is_some_and(|v| v >= 0.0),
        "queue_wait_ms must be a real number (#230), row: {row}"
    );

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
