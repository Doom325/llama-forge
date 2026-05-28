use futures_core::Stream;
use std::pin::Pin;

use llamaforge_api::transport::InferenceEngine;
use llamaforge_core::types::{
    traits::{
        AdmissionError, GatewayLifecycle, IncomingRequest, MetricsExporter, QueueError,
        QueueSnapshot, QueueSubmitter, QueueTicket, RouteValidator, ServerControl, ServerError,
        ServerSummary,
    },
    ChatCompletionChunk, RequestId, ServerId, StreamError,
};

// `#[path = "mocks.rs"]`-included from multiple integration-test
// binaries (`readiness_tests`, `vc3_transport`, …). Each binary uses
// only a subset of these mocks; the `dead_code` lints would otherwise
// fire per-target. The marker is local, not architectural.
#[allow(dead_code)]
pub struct MockEngine {
    pub chunks: std::sync::Mutex<Vec<Result<ChatCompletionChunk, StreamError>>>,
    pub sleep_ms: u64,
}

#[async_trait::async_trait]
impl InferenceEngine for MockEngine {
    async fn chat_completion(
        &self,
        _req: llamaforge_core::types::ChatCompletionRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>,
        llamaforge_core::types::traits::DispatchError,
    > {
        let chunks = std::mem::take(&mut *self.chunks.lock().unwrap());
        let sleep_time = self.sleep_ms;

        let stream = futures_util::stream::unfold(
            (chunks.into_iter(), sleep_time),
            |(mut iter, delay)| async move {
                if delay > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
                iter.next().map(|item| (item, (iter, delay)))
            },
        );

        Ok(Box::pin(stream))
    }
}

#[allow(dead_code)]
pub struct MockRouteValidator;
impl RouteValidator for MockRouteValidator {
    fn validate_routable(
        &self,
        _model: &llamaforge_core::types::ModelTarget,
        _capabilities: &llamaforge_core::types::RequestCapabilities,
    ) -> Result<(), AdmissionError> {
        Ok(())
    }
}

#[allow(dead_code)]
pub struct MockMetricsExporter;
impl MetricsExporter for MockMetricsExporter {
    fn gather_text(&self) -> String {
        "".into()
    }
}

#[allow(dead_code)]
pub struct MockServerControl;
#[async_trait::async_trait]
impl ServerControl for MockServerControl {
    async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError> {
        Ok(vec![])
    }
    async fn start_server(&self, _id: &ServerId) -> Result<(), ServerError> {
        Ok(())
    }
    async fn stop_server(&self, _id: &ServerId) -> Result<(), ServerError> {
        Ok(())
    }
    fn cluster_status(&self) -> llamaforge_core::types::ClusterStatus {
        llamaforge_core::types::ClusterStatus {
            state: llamaforge_core::types::ClusterState::SomeRunning,
            failed_servers: 0,
        }
    }
}

#[allow(dead_code)]
pub struct MockGatewayLifecycle;
impl GatewayLifecycle for MockGatewayLifecycle {
    fn is_gateway_initialized(&self) -> bool {
        true
    }
}

#[allow(dead_code)]
pub struct MockQueueSubmitter;
impl QueueSubmitter for MockQueueSubmitter {
    fn submit(&self, _req: IncomingRequest) -> Result<QueueTicket, AdmissionError> {
        Ok(QueueTicket {
            request_id: llamaforge_core::types::RequestId::new(uuid::Uuid::now_v7()),
            admitted_at: chrono::Utc::now(),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            events: tokio::sync::mpsc::channel(1).1,
        })
    }
    fn cancel(&self, _id: &RequestId) -> Result<(), QueueError> {
        Ok(())
    }
    fn status(&self) -> Result<QueueSnapshot, QueueError> {
        Ok(QueueSnapshot { depth: 0, estimated_throughput_req_sec: 1.0 })
    }
}
