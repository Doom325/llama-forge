//! Regression for #229. `DELETE /api/queue/{id}` for an id that is not in the
//! queue (unknown, already completed, or — in v1 — an in-flight request past
//! the dispatch boundary) must return **404 `request_not_cancellable`**, not
//! the 500 `internal_error` it returned before. Cancellation is queue-only in
//! v1 (design.md G3: in-flight requests run to completion).
//!
//! This uses a random, never-submitted UUID so the assertion is deterministic
//! (no reliance on catching a request mid-dispatch).

use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_cancel_unknown_request_id_is_404_not_500() {
    let orch = Harness::start_full().await.expect("orchestrator start");
    let client = reqwest::Client::new();

    let unknown_id = uuid::Uuid::now_v7();
    let res = client
        .delete(format!("{}/api/queue/{}", orch.http_url, unknown_id))
        // CSRF guard: mutating /api/* requests must carry this header.
        .header("X-LlamaForge-Request", "1")
        .send()
        .await
        .expect("send DELETE /api/queue/{id}");

    let status = res.status();
    let body = res.text().await.unwrap_or_default();

    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "cancelling an unknown id must be 404, not 500/other. body={body}"
    );
    assert!(
        body.contains("request_not_cancellable"),
        "expected request_not_cancellable error code, got: {body}"
    );

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
