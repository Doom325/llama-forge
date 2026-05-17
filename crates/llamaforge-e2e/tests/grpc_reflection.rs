//! gRPC reflection smoke test (B-10).
//!
//! With the `dev-reflection` feature on (default), the orchestrator
//! exposes the gRPC reflection v1 service so `grpcurl`-style probing
//! works without shipping the `.proto` files. This test calls
//! `ServerReflection.ListServices` and asserts the two LlamaForge
//! services + health are visible.

use llamaforge_e2e::Harness;
use tonic_reflection::pb::v1::server_reflection_client::ServerReflectionClient;
use tonic_reflection::pb::v1::server_reflection_request::MessageRequest;
use tonic_reflection::pb::v1::server_reflection_response::MessageResponse;
use tonic_reflection::pb::v1::ServerReflectionRequest;

#[tokio::test]
async fn test_e2e_grpc_reflection_lists_services() {
    let orch = Harness::start_full().await.expect("orchestrator start");

    let channel = tonic::transport::Endpoint::new(orch.grpc_url.clone())
        .expect("endpoint")
        .connect()
        .await
        .expect("connect to grpc");
    let mut client = ServerReflectionClient::new(channel);

    let request_stream = futures_util::stream::iter(vec![ServerReflectionRequest {
        host: String::new(),
        message_request: Some(MessageRequest::ListServices(String::new())),
    }]);
    let response = client.server_reflection_info(request_stream).await.expect("reflection rpc");

    let mut response_stream = response.into_inner();
    let msg = tonic::codegen::tokio_stream::StreamExt::next(&mut response_stream)
        .await
        .expect("at least one reflection response")
        .expect("reflection response ok");

    let services: Vec<String> = match msg.message_response {
        Some(MessageResponse::ListServicesResponse(r)) => {
            r.service.into_iter().map(|s| s.name).collect()
        }
        other => panic!("expected ListServicesResponse, got {:?}", other),
    };

    assert!(
        services.iter().any(|s| s == "llamaforge.inference.InferenceService"),
        "expected InferenceService in reflection list, got {:?}",
        services
    );
    assert!(
        services.iter().any(|s| s == "llamaforge.management.ManagementService"),
        "expected ManagementService in reflection list, got {:?}",
        services
    );

    orch.shutdown_and_wait().await.expect("clean shutdown");
}
