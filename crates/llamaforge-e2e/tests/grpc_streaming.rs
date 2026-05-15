//! 10.3.c — gRPC `StreamInference` through the orchestrator. Builds a
//! tonic client against the bound gRPC port, calls `StreamInference`,
//! and asserts at least one chunk arrives, every chunk parses cleanly,
//! and the final status is OK.

use llamaforge_e2e::Harness;
use llamaforge_proto::inference::inference_service_client::InferenceServiceClient;
use llamaforge_proto::inference::InferenceRequest;

#[tokio::test]
async fn test_e2e_grpc_streaming() {
    let orch = Harness::start_full().await.expect("orchestrator start");

    let mut client =
        InferenceServiceClient::connect(orch.grpc_url.clone()).await.expect("grpc connect");

    let payload = serde_json::json!({
        "model": "dummy",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": true,
    })
    .to_string();

    let req = tonic::Request::new(InferenceRequest { model: "dummy".to_string(), payload });
    let mut stream = client.stream_inference(req).await.expect("stream_inference").into_inner();

    let mut chunks_seen = 0usize;
    while let Some(msg) = stream.message().await.expect("grpc message error") {
        assert!(!msg.payload.is_empty(), "empty payload chunk");
        // Every payload chunk must be valid JSON (serialized ChatCompletionChunk).
        let _: serde_json::Value =
            serde_json::from_str(&msg.payload).expect("chunk payload is JSON");
        chunks_seen += 1;
    }

    assert!(chunks_seen >= 1, "expected at least one streamed chunk, got 0");

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
