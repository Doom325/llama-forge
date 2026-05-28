use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use llamaforge_core::config::{ConfigReader, ConfigWriter};
use llamaforge_core::types::{
    traits::{QueueSubmitter, ServerControl},
    RequestId, ServerId,
};
use llamaforge_metrics::{RequestMetricsStore, StoreMaintenance};
use std::sync::Arc;

use super::error::ApiError;

use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct ServerStatusResponse {
    pub id: String,
    pub model: String,
    pub state: String,
    pub slots_total: u32,
    pub slots_busy: u32,
}

pub struct ManagementState<CW, RMS, SM> {
    pub server_control: Arc<dyn ServerControl>,
    pub queue_submitter: Arc<dyn QueueSubmitter>,
    pub config_reader: Arc<dyn ConfigReader>,
    pub config_writer: Arc<CW>,
    pub request_metrics_store: Arc<RMS>,
    pub store_maintenance: Arc<SM>,
}

impl<CW, RMS, SM> Clone for ManagementState<CW, RMS, SM> {
    fn clone(&self) -> Self {
        Self {
            server_control: self.server_control.clone(),
            queue_submitter: self.queue_submitter.clone(),
            config_reader: self.config_reader.clone(),
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

    let dtos: Vec<ServerStatusResponse> = summaries
        .into_iter()
        .map(|s| ServerStatusResponse {
            id: s.id.to_string(),
            model: s.alias.into_inner().to_string(),
            state: format!("{:?}", s.state),
            slots_total: s.slots_total,
            slots_busy: s.slots_busy,
        })
        .collect();

    Ok(Json(dtos).into_response())
}

pub(crate) async fn start_server<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    llamaforge_core::config::validate_server_id_slug(&id)
        .map_err(|e| ApiError::InvalidBody(e.to_string()))?;
    let server_id = ServerId::from_config_id(&id);
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
    llamaforge_core::config::validate_server_id_slug(&id)
        .map_err(|e| ApiError::InvalidBody(e.to_string()))?;
    let server_id = ServerId::from_config_id(&id);
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
    // The gate is in `ConfigHandle::reload` (see grpc.rs for the
    // rationale). Map the dedicated `RestartRequired` error to 412.
    match state.config_writer.reload().await {
        Ok(_) => Ok(axum::http::StatusCode::OK.into_response()),
        Err(llamaforge_core::config::ConfigReloadError::RestartRequired { fields }) => {
            Err(ApiError::PreconditionFailed(format!(
                "config diff touches restart-required server fields ({:?}); \
                 restart the orchestrator to apply",
                fields
            )))
        }
        Err(e) => Err(ApiError::Internal(e.to_string())),
    }
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct MetricsQueryParams {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(flatten)]
    pub filter: llamaforge_metrics::MetricsFilter,
}

pub(crate) async fn query_request_metrics<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Query(params): Query<MetricsQueryParams>,
) -> Result<Response, ApiError>
where
    RMS: RequestMetricsStore + Send + Sync + 'static,
{
    let metrics = state
        .request_metrics_store
        .query_request_metrics(&params.filter, params.limit, params.offset)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(metrics).into_response())
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct AggregatedQueryParams {
    /// Comma-separated list of fields to group by, e.g. `?group_by=server_id,hour`.
    #[serde(default)]
    pub group_by: Option<String>,
    #[serde(flatten)]
    pub filter: llamaforge_metrics::MetricsFilter,
}

fn parse_group_by(s: &str) -> Result<Vec<llamaforge_metrics::GroupByField>, ApiError> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| match p {
            "server_id" => Ok(llamaforge_metrics::GroupByField::ServerId),
            "hour" => Ok(llamaforge_metrics::GroupByField::Hour),
            "model_name" => Ok(llamaforge_metrics::GroupByField::ModelName),
            "requestor" => Ok(llamaforge_metrics::GroupByField::Requestor),
            other => Err(ApiError::Internal(format!("invalid group_by field: {}", other))),
        })
        .collect()
}

pub(crate) async fn query_aggregated<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Query(params): Query<AggregatedQueryParams>,
) -> Result<Response, ApiError>
where
    RMS: RequestMetricsStore + Send + Sync + 'static,
{
    let group_by = match &params.group_by {
        Some(s) => parse_group_by(s)?,
        None => Vec::new(),
    };
    let metrics = state
        .request_metrics_store
        .aggregate_request_metrics(&params.filter, &group_by)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(metrics).into_response())
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct WalCheckpointQuery {
    #[serde(default)]
    pub mode: Option<llamaforge_metrics::CheckpointMode>,
}

fn map_config_checkpoint_mode(
    m: llamaforge_core::config::WalCheckpointMode,
) -> llamaforge_metrics::CheckpointMode {
    match m {
        llamaforge_core::config::WalCheckpointMode::Passive => {
            llamaforge_metrics::CheckpointMode::Passive
        }
        llamaforge_core::config::WalCheckpointMode::Truncate => {
            llamaforge_metrics::CheckpointMode::Truncate
        }
    }
}

pub(crate) async fn wal_checkpoint<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Query(params): Query<WalCheckpointQuery>,
) -> Result<Response, ApiError>
where
    CW: ConfigWriter + Send + Sync + 'static,
    SM: StoreMaintenance + Send + Sync + 'static,
{
    let mode = match params.mode {
        Some(m) => m,
        None => {
            let snap = state.config_reader.snapshot();
            map_config_checkpoint_mode(snap.gateway.wal_checkpoint_mode)
        }
    };
    state
        .store_maintenance
        .wal_checkpoint(mode)
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
    use llamaforge_core::config::{
        source::memory::InMemoryConfigSource, AppConfig, ConfigHandle, GlobalConfig,
    };
    use llamaforge_core::types::traits::*;

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
        fn cluster_state(&self) -> u8 {
            1
        }
        fn is_gateway_initialized(&self) -> bool {
            true
        }
    }

    struct MockQueueSubmitter;
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

    fn build_test_state() -> ManagementState<ConfigHandle, (), ()> {
        let config = AppConfig { global: GlobalConfig::default(), ..Default::default() };
        let source = Arc::new(InMemoryConfigSource::new(config));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());

        ManagementState {
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitter),
            config_reader: handle.clone(),
            config_writer: handle,
            request_metrics_store: Arc::new(()),
            store_maintenance: Arc::new(()),
        }
    }

    #[tokio::test]
    async fn test_route_parameter_parsing() {
        let state = build_test_state();
        // UUIDs and slugs both parse — slug acceptance is the point of #114.
        let uuid_id = uuid::Uuid::now_v7().to_string();
        let res = start_server(axum::extract::State(state.clone()), axum::extract::Path(uuid_id))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let slug_id = "gemma3-27b".to_string();
        let res = start_server(axum::extract::State(state.clone()), axum::extract::Path(slug_id))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Malformed slugs (leading `-`, whitespace, …) must still 400.
        let res_invalid = start_server(
            axum::extract::State(state.clone()),
            axum::extract::Path("-leading-hyphen".to_string()),
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
