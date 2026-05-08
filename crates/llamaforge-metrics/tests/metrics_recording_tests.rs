//! Tests for the MetricsFacadeRecorder and metrics crate integration.

use metrics_util::debugging::DebuggingRecorder;

use llamaforge_metrics::{MetricsFacadeRecorder, RequestMetricsRecorder};

#[test]
fn test_metrics_facade_recorder() {
    // Install the debugging recorder
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    recorder.install().unwrap();

    let facade = MetricsFacadeRecorder::new();

    // Test record_request
    facade.record_request("llama", "chat", "completed");

    let snapshot = snapshotter.snapshot();
    let vec = snapshot.into_vec();
    let names: Vec<_> = vec.iter().map(|(k, _, _, _)| k.key().name().to_string()).collect();
    let requests_total = vec
        .into_iter()
        .find(|(key, _, _, _)| key.key().name() == "llamaforge_requests_total")
        .unwrap_or_else(|| panic!("Counter not found. Available: {:?}", names));

    let labels: std::collections::HashMap<String, String> = requests_total
        .0
        .key()
        .labels()
        .map(|l| (l.key().to_string(), l.value().to_string()))
        .collect();
    assert_eq!(labels.get("model").map(|s| s.as_str()), Some("llama"));
    assert_eq!(labels.get("task_type").map(|s| s.as_str()), Some("chat"));
    assert_eq!(labels.get("status").map(|s| s.as_str()), Some("completed"));

    match requests_total.3 {
        metrics_util::debugging::DebugValue::Counter(c) => assert_eq!(c, 1),
        _ => panic!("Expected counter"),
    }

    // Test observe_ttft
    facade.observe_ttft("llama", 0.5);
    let snapshot2 = snapshotter.snapshot();
    let ttft = snapshot2
        .into_vec()
        .into_iter()
        .find(|(key, _, _, _)| key.key().name() == "llamaforge_ttft_seconds")
        .expect("Histogram not found");

    match ttft.3 {
        metrics_util::debugging::DebugValue::Histogram(h) => {
            assert_eq!(h.len(), 1);
            assert_eq!(h[0].into_inner(), 0.5);
        }
        _ => panic!("Expected histogram"),
    }

    // Test set_queue_depth
    facade.set_queue_depth("high", 42);
    let snapshot3 = snapshotter.snapshot();
    let queue_depth = snapshot3
        .into_vec()
        .into_iter()
        .find(|(key, _, _, _)| key.key().name() == "llamaforge_queue_depth")
        .expect("Gauge not found");

    match queue_depth.3 {
        metrics_util::debugging::DebugValue::Gauge(g) => assert_eq!(g.into_inner(), 42.0),
        _ => panic!("Expected gauge"),
    }
}
