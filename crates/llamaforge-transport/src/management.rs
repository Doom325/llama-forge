use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use llamaforge_config::ConfigWriter;
use llamaforge_metrics::{RequestMetricsStore, StoreMaintenance};
use llamaforge_types::{
    traits::{QueueSubmitter, ServerControl},
    RequestId, ServerId,
};
use std::sync::Arc;

use crate::error::ApiError;

pub struct ManagementState<CW, RMS, SM> {
    pub server_control: Arc<dyn ServerControl>,
    pub queue_submitter: Arc<dyn QueueSubmitter>,
    pub config_writer: Arc<CW>,
    pub request_metrics_store: Arc<RMS>,
    pub store_maintenance: Arc<SM>,
}

impl<CW, RMS, SM> Clone for ManagementState<CW, RMS, SM> {
    fn clone(&self) -> Self {
        Self {
            server_control: self.server_control.clone(),
            queue_submitter: self.queue_submitter.clone(),
            config_writer: self.config_writer.clone(),
            request_metrics_store: self.request_metrics_store.clone(),
            store_maintenance: self.store_maintenance.clone(),
        }
    }
}

pub(crate) async fn list_servers<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
) -> Result<Response, ApiError> {
    let summaries =
        state.server_control.list_servers().await.map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(summaries).into_response())
}

pub(crate) async fn start_server<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let server_id = ServerId::new(
        uuid::Uuid::parse_str(&id).map_err(|_| ApiError::InvalidBody("Invalid ServerId".into()))?,
    );
    state
        .server_control
        .start_server(&server_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::OK.into_response())
}

pub(crate) async fn stop_server<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let server_id = ServerId::new(
        uuid::Uuid::parse_str(&id).map_err(|_| ApiError::InvalidBody("Invalid ServerId".into()))?,
    );
    state
        .server_control
        .stop_server(&server_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::OK.into_response())
}

pub(crate) async fn queue_status<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
) -> Result<Response, ApiError> {
    let status = state.queue_submitter.status().map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(status).into_response())
}

pub(crate) async fn cancel_request<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let request_id = RequestId::new(
        uuid::Uuid::parse_str(&id)
            .map_err(|_| ApiError::InvalidBody("Invalid RequestId".into()))?,
    );
    state.queue_submitter.cancel(&request_id).map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::OK.into_response())
}

pub(crate) async fn reload_config<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
) -> Result<Response, ApiError>
where
    CW: ConfigWriter + Send + Sync + 'static,
{
    state.config_writer.reload().await.map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::OK.into_response())
}

#[derive(serde::Deserialize)]
pub(crate) struct MetricsQueryParams {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub(crate) async fn query_request_metrics<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Query(params): Query<MetricsQueryParams>,
) -> Result<Response, ApiError>
where
    RMS: RequestMetricsStore + Send + Sync + 'static,
{
    // Local invariant: filter query parameters will be supported in v1.1
    let filter = llamaforge_metrics::MetricsFilter::default();
    let metrics = state
        .request_metrics_store
        .query_request_metrics(&filter, params.limit, params.offset)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(metrics).into_response())
}

pub(crate) async fn query_aggregated<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
) -> Result<Response, ApiError>
where
    RMS: RequestMetricsStore + Send + Sync + 'static,
{
    let filter = llamaforge_metrics::MetricsFilter::default();
    // Local invariant: group_by query parameters will be supported in v1.1
    let group_by = vec![];
    let metrics = state
        .request_metrics_store
        .aggregate_request_metrics(&filter, &group_by)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(metrics).into_response())
}

pub(crate) async fn wal_checkpoint<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
) -> Result<Response, ApiError>
where
    SM: StoreMaintenance + Send + Sync + 'static,
{
    // Local invariant: CheckpointMode parameterization will be supported in v1.1.
    // Use passive checkpoint as a safe default for manual checkpoints via REST API.
    state
        .store_maintenance
        .wal_checkpoint(llamaforge_metrics::CheckpointMode::Passive)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(axum::http::StatusCode::OK.into_response())
}

pub(crate) fn management_routes<CW, RMS, SM>(state: ManagementState<CW, RMS, SM>) -> Router
where
    CW: ConfigWriter + Send + Sync + 'static,
    RMS: RequestMetricsStore + Send + Sync + 'static,
    SM: StoreMaintenance + Send + Sync + 'static,
{
    Router::new()
        .route("/api/servers", get(list_servers::<CW, RMS, SM>))
        .route("/api/servers/{id}/start", post(start_server::<CW, RMS, SM>))
        .route("/api/servers/{id}/stop", post(stop_server::<CW, RMS, SM>))
        .route("/api/queue", get(queue_status::<CW, RMS, SM>))
        .route("/api/queue/{id}", delete(cancel_request::<CW, RMS, SM>))
        .route("/api/config/reload", post(reload_config::<CW, RMS, SM>))
        .route("/api/metrics/requests", get(query_request_metrics::<CW, RMS, SM>))
        .route("/api/metrics/aggregated", get(query_aggregated::<CW, RMS, SM>))
        .route("/api/maintenance/wal-checkpoint", post(wal_checkpoint::<CW, RMS, SM>))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use llamaforge_config::{
        source::memory::InMemoryConfigSource, AppConfig, ConfigHandle, GlobalConfig,
    };
    use llamaforge_types::traits::*;

    struct MockServerControl;
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
    }

    struct MockQueueSubmitter;
    impl QueueSubmitter for MockQueueSubmitter {
        fn submit(&self, _req: IncomingRequest) -> Result<QueueTicket, AdmissionError> {
            Ok(QueueTicket {})
        }
        fn cancel(&self, _id: &RequestId) -> Result<(), QueueError> {
            Ok(())
        }
        fn status(&self) -> Result<QueueSnapshot, QueueError> {
            Ok(QueueSnapshot { depth: 0, estimated_throughput_req_sec: 1.0 })
        }
    }

    fn build_test_state() -> ManagementState<ConfigHandle, (), ()> {
        let config = AppConfig { global: GlobalConfig::default(), ..Default::default() };
        let source = Arc::new(InMemoryConfigSource::new(config));
        let cw = Arc::new(ConfigHandle::load(source).unwrap());

        ManagementState {
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitter),
            config_writer: cw,
            request_metrics_store: Arc::new(()),
            store_maintenance: Arc::new(()),
        }
    }

    #[tokio::test]
    async fn test_route_parameter_parsing() {
        let state = build_test_state();
        let valid_id = uuid::Uuid::now_v7().to_string();
        let res = start_server(axum::extract::State(state.clone()), axum::extract::Path(valid_id))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let res_invalid = start_server(
            axum::extract::State(state.clone()),
            axum::extract::Path("invalid-id".to_string()),
        )
        .await;
        assert!(matches!(res_invalid.unwrap_err(), ApiError::InvalidBody(_)));
    }

    #[tokio::test]
    async fn test_config_reload_delegation() {
        let state = build_test_state();
        let res = reload_config(axum::extract::State(state)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
