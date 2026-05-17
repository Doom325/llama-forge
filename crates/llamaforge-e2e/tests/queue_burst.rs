//! Regression test for the queue-admission split.
//!
//! Before the split, the router's capacity check folded "any server
//! serves this model" and "any matching server has an idle slot" into
//! a single boolean. Once all slots were busy the function returned
//! false, admission rejected the request with `NoServerForModel`, and
//! the queue never held the burst. With the split, admission only
//! asks "is the model knowable" — busy slots are the queue's problem
//! and the dispatch iterator picks up the request when capacity
//! reopens.
//!
//! This test fires 12 concurrent non-streaming chat completions and
//! asserts every one of them eventually returns `200`. Without the
//! fix, 8+ of 12 would have returned `5xx` or `404` immediately.

use futures_util::future::join_all;
use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_queue_admits_burst_traffic() {
    let orch = Harness::start_full().await.expect("orchestrator start");
    let url = format!("{}/v1/chat/completions", orch.http_url);
    let body = serde_json::json!({
        "model": "mock",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": false,
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("reqwest client");

    let mut tasks = Vec::new();
    for _ in 0..12 {
        let c = client.clone();
        let u = url.clone();
        let b = body.clone();
        tasks.push(tokio::spawn(
            async move { c.post(&u).json(&b).send().await.map(|r| r.status()) },
        ));
    }

    let outcomes = join_all(tasks).await;
    let statuses: Vec<reqwest::StatusCode> =
        outcomes.into_iter().map(|t| t.expect("task panic").expect("send")).collect();

    let ok = statuses.iter().filter(|s| s.as_u16() == 200).count();
    let other: Vec<_> = statuses.iter().filter(|s| s.as_u16() != 200).copied().collect();
    assert_eq!(
        ok, 12,
        "expected all 12 burst requests to return 200, got {} successes (others: {:?})",
        ok, other
    );

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
