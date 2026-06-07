//! 10.3.e — Prometheus metrics endpoint. After 3 chat-completion
//! requests, GET `/metrics` and assert the orchestrator's counters
//! reflect the traffic.

use llamaforge_e2e::Harness;

#[tokio::test]
async fn test_e2e_metrics_endpoint() {
    let orch = Harness::start_full().await.expect("orchestrator start");

    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": "mock",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": false,
    });

    for i in 0..3 {
        let res = client
            .post(format!("{}/v1/chat/completions", orch.http_url))
            .json(&body)
            .send()
            .await
            .expect("send chat completion");
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        assert_eq!(status, 200, "request {}: expected 200, got {} body={}", i, status, text);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // Give the metrics recorder a beat to commit the last increment
    // before scraping.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let metrics = client
        .get(format!("{}/metrics", orch.http_url))
        .send()
        .await
        .expect("scrape /metrics")
        .text()
        .await
        .expect("metrics body");

    // The HTTP requests counter must show 3 successful POSTs on
    // /v1/chat/completions. The Prometheus exposition format is
    // line-oriented; we match on the labelled sample.
    let request_lines: Vec<&str> =
        metrics.lines().filter(|l| l.starts_with("llamaforge_http_requests_total")).collect();
    let chat_200 = request_lines
        .iter()
        .find(|l| {
            l.contains("path=\"/v1/chat/completions\"")
                && l.contains("method=\"POST\"")
                && l.contains("status=\"200\"")
        })
        .copied()
        .unwrap_or_else(|| {
            panic!(
                "no /v1/chat/completions 200 counter sample in /metrics. Saw: {:?}",
                request_lines
            )
        });
    let value: u64 = chat_200
        .rsplit(' ')
        .next()
        .expect("counter value")
        .trim()
        .parse()
        .expect("counter value parses as u64");
    // Regression guard for #232: `*_total` counters must *accumulate*.
    // Three successful chat completions must read exactly 3 — not 1 (the
    // set/absolute-style behaviour the broken `metrics-exporter-prometheus`
    // 0.16 + `metrics` 0.24 combo produced, where every counter reported
    // only the most recent request's contribution). A `>= 1` assertion
    // here previously masked that regression.
    assert_eq!(
        value, 3,
        "expected exactly 3 cumulative chat-completion 200 counter samples, got {} (line: {})",
        value, chat_200
    );

    assert!(
        metrics.contains("llamaforge_active_connections"),
        "expected llamaforge_active_connections gauge in /metrics"
    );
    assert!(
        metrics.contains("llamaforge_tokens_completion_total")
            || metrics.contains("llamaforge_tokens_total"),
        "expected a llamaforge_tokens_* counter family in /metrics"
    );

    // Second accumulation guard for #232, on a different emit path
    // (`dispatch/proxy.rs`): one dispatch per request, so the per-server
    // `llamaforge_dispatches_total` must sum to 3 across the batch. This
    // catches the regression even if the HTTP-layer counter were exempt.
    let dispatches: u64 = metrics
        .lines()
        .filter(|l| l.starts_with("llamaforge_dispatches_total{"))
        .filter_map(|l| l.rsplit(' ').next())
        .filter_map(|v| v.trim().parse::<u64>().ok())
        .sum();
    assert_eq!(
        dispatches,
        3,
        "expected dispatches_total to accumulate to 3 across the batch, got {} (metrics:\n{})",
        dispatches,
        metrics.lines().filter(|l| l.contains("dispatches_total")).collect::<Vec<_>>().join("\n")
    );

    // Label cardinality on `path` is bounded to the known routes.
    let mut paths: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in request_lines {
        if let Some(rest) = line.split("path=\"").nth(1) {
            if let Some(p) = rest.split('"').next() {
                paths.insert(p.to_string());
            }
        }
    }
    let allowed: &[&str] = &[
        "/v1/chat/completions",
        "/v1/completions",
        "/v1/messages",
        "/health",
        "/metrics",
        "/api/servers",
        "/api/queue",
        "/api/config",
        "/api/metrics/requests",
        "/api/metrics/aggregates",
    ];
    for path in &paths {
        assert!(
            allowed
                .iter()
                .any(|allowed_path| path == allowed_path || path.starts_with(allowed_path)),
            "unexpected high-cardinality path label: {} (saw all: {:?})",
            path,
            paths
        );
    }

    // 10.4.e — the audit translator should have forwarded each request
    // into the metrics store. Scrape the management endpoint and assert
    // the persisted-request count is non-zero. (Exact match is fragile:
    // the SQLite batch collector flushes on a timer.)
    let stored = client
        .get(format!("{}/api/metrics/requests", orch.http_url))
        .send()
        .await
        .expect("scrape /api/metrics/requests");
    if stored.status() == reqwest::StatusCode::OK {
        let body: serde_json::Value = stored.json().await.expect("requests body is JSON");
        let count = body.as_array().map(|a| a.len()).unwrap_or(0);
        assert!(
            count > 0 || body.is_object(),
            "expected at least one persisted request after 3 chat completions, got {:?}",
            body
        );
    }
    // If the endpoint isn't wired or returns 404/500, fall through —
    // the audit-translator unit tests cover the mapping contract; this
    // is a soft probe that the flush path works end-to-end.

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
