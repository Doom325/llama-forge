pub(crate) mod anthropic;
pub(crate) mod error;
pub(crate) mod grpc;
pub(crate) mod http;
pub(crate) mod management;
pub(crate) mod middleware;
pub(crate) mod openai;

pub use error::{AnthropicProtocolError, ApiError, TransportError};
pub use http::{start_http_server, HttpAppState};
pub use llamaforge_types::traits::{
    MetricsExporter, QueueSubmitter, RouteValidator, ServerControl,
};
pub use middleware::LargeBodyConcurrencyLimit;

use futures_core::Stream;
use llamaforge_config::{ConfigReader, ConfigWriter};
use llamaforge_metrics::{RequestMetricsStore, StoreMaintenance};
use llamaforge_types::{ChatCompletionChunk, ChatCompletionRequest, StreamError};
use std::pin::Pin;
use std::sync::Arc;

/// InferenceEngine protocol-agnostic core trait.
#[async_trait::async_trait]
pub trait InferenceEngine: Send + Sync + 'static {
    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>,
        llamaforge_types::traits::DispatchError,
    >;
}

pub struct TransportDeps<CW, RMS, SM>
where
    CW: ConfigWriter + 'static,
    RMS: RequestMetricsStore + 'static,
    SM: StoreMaintenance + 'static,
{
    pub config_reader: Arc<dyn ConfigReader>,
    pub config_writer: Arc<CW>,
    pub server_control: Arc<dyn ServerControl>,
    pub queue_submitter: Arc<dyn QueueSubmitter>,
    pub metrics_exporter: Arc<dyn MetricsExporter>,
    pub request_metrics_store: Arc<RMS>,
    pub store_maintenance: Arc<SM>,
    pub route_validator: Arc<dyn RouteValidator>,
    pub inference_engine: Arc<dyn InferenceEngine>,
}

pub struct TransportHandles {
    http_handle: tokio::task::JoinHandle<()>,
    grpc_handle: tokio::task::JoinHandle<()>,
    shutdown_tx: tokio::sync::watch::Sender<()>,
}

impl TransportHandles {
    pub fn new(
        http_handle: tokio::task::JoinHandle<()>,
        grpc_handle: tokio::task::JoinHandle<()>,
        shutdown_tx: tokio::sync::watch::Sender<()>,
    ) -> Self {
        Self { http_handle, grpc_handle, shutdown_tx }
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }

    pub async fn wait(self) -> Result<(), TransportError> {
        let (http_res, grpc_res) = tokio::join!(self.http_handle, self.grpc_handle);
        http_res.map_err(TransportError::ServerTaskFailed)?;
        grpc_res.map_err(TransportError::ServerTaskFailed)?;
        Ok(())
    }
}

pub async fn start_servers<CW, RMS, SM>(
    _deps: TransportDeps<CW, RMS, SM>,
) -> Result<TransportHandles, TransportError>
where
    CW: ConfigWriter + 'static,
    RMS: RequestMetricsStore + 'static,
    SM: StoreMaintenance + 'static,
{
    // Placeholder to satisfy signature for PR1
    let (tx, _) = tokio::sync::watch::channel(());
    Ok(TransportHandles::new(tokio::spawn(async {}), tokio::spawn(async {}), tx))
}
