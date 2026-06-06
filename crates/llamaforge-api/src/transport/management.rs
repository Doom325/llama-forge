use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use llamaforge_core::config::{ConfigReader, ConfigWriter};
use llamaforge_core::types::{
    traits::{QueueSubmitter, ServerControl, ServerSummary},
    RequestId, ServerId, ServerLifecycle,
};
use llamaforge_metrics::{RequestMetricsStore, StoreMaintenance};
use std::sync::Arc;

use super::error::ApiError;

use serde::Serialize;

/// `/api/servers` list item. Matches the frontend `ServerState` contract
/// (FRONTEND_ARCHITECTURE.md §5.2): `{id, name, status, uptime}`, with `status`
/// drawn from the UI's lowercase `ServerStatus` vocabulary rather than the `Debug`
/// rendering of the lifecycle enum — the dashboard's `StatusBadge` keys off the
/// former and crashes on the latter. Per-server slot/throughput detail lives on
/// `GET /api/servers/{id}/metrics`, not here.
#[derive(Serialize)]
pub(crate) struct ServerStatusResponse {
    pub id: String,
    pub name: String,
    pub status: &'static str,
    pub uptime: u64,
}

/// Map the internal lifecycle to the frontend `ServerStatus` vocabulary
/// (FRONTEND_ARCHITECTURE.md §5.2). `Starting`/`Draining` have no exact UI
/// counterpart, so they map to the nearest operational signal the badge renders;
/// `ServerLifecycle` is `#[non_exhaustive]`, so a future variant falls back to the
/// neutral `pending` rather than an inaccurate health signal.
fn lifecycle_status(state: ServerLifecycle) -> &'static str {
    match state {
        ServerLifecycle::Stopped => "stopped",
        ServerLifecycle::Starting => "pending",
        ServerLifecycle::Running => "running",
        ServerLifecycle::Draining => "degraded",
        ServerLifecycle::Restarting => "restarting",
        ServerLifecycle::Failed => "failed",
        _ => "pending",
    }
}

/// Project a `ServerSummary` onto the frontend `ServerState` DTO. Shared by
/// `GET /api/servers` and its `GET /v1/servers` alias so the two can't drift.
pub(crate) fn server_summary_to_dto(s: ServerSummary) -> ServerStatusResponse {
    ServerStatusResponse {
        id: s.id.to_string(),
        name: s.alias.into_inner().to_string(),
        status: lifecycle_status(s.state),
        // Seconds since the supervisor first reported the process up; 0 until the
        // spawn task records `started_at` (clamped so clock skew can't go negative).
        uptime: s
            .started_at
            .map(|t| (chrono::Utc::now() - t).num_seconds().max(0) as u64)
            .unwrap_or(0),
    }
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

    let dtos: Vec<ServerStatusResponse> =
        summaries.into_iter().map(server_summary_to_dto).collect();

    Ok(Json(dtos).into_response())
}

/// `GET /api/servers/{id}` — a single server's `ServerState`
/// (FRONTEND_ARCHITECTURE.md §5.2). Reuses the shared `server_summary_to_dto`
/// projection so the detail view can't drift from the list, and returns
/// `404 not_found` when no server carries the requested id.
pub(crate) async fn get_server<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    llamaforge_core::config::validate_server_id_slug(&id)
        .map_err(|e| ApiError::InvalidBody(e.to_string()))?;
    let server_id = ServerId::from_config_id(&id);
    let summaries =
        state.server_control.list_servers().await.map_err(|e| ApiError::Internal(e.to_string()))?;
    let summary = summaries
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| ApiError::NotFound(format!("server not found: {id}")))?;
    Ok(Json(server_summary_to_dto(summary)).into_response())
}

/// `GET /api/servers/{id}/metrics` — per-server `ServerMetricsSummary`
/// (FRONTEND_ARCHITECTURE.md §5.2). Throughput, latency percentiles and the
/// error rate come from the request-metrics store filtered to this server;
/// slot occupancy comes from the live `ServerSummary`. Returns `404 not_found`
/// when the server id is unknown.
pub(crate) async fn server_metrics<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError>
where
    RMS: RequestMetricsStore + Send + Sync + 'static,
{
    llamaforge_core::config::validate_server_id_slug(&id)
        .map_err(|e| ApiError::InvalidBody(e.to_string()))?;
    let server_id = ServerId::from_config_id(&id);

    let summaries =
        state.server_control.list_servers().await.map_err(|e| ApiError::Internal(e.to_string()))?;
    let summary = summaries
        .into_iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| ApiError::NotFound(format!("server not found: {id}")))?;

    let filter =
        llamaforge_metrics::MetricsFilter { server_id: Some(server_id), ..Default::default() };
    let metrics = state
        .request_metrics_store
        .request_metrics_summary(&filter)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(ServerMetricsResponse::from_parts(&summary, metrics)).into_response())
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

pub(crate) async fn restart_server<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    llamaforge_core::config::validate_server_id_slug(&id)
        .map_err(|e| ApiError::InvalidBody(e.to_string()))?;
    let server_id = ServerId::from_config_id(&id);
    state
        .server_control
        .restart_server(&server_id)
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

/// Frontend `RequestRecord` shape (FRONTEND_ARCHITECTURE.md §5.2):
/// `{id, timestamp, model, taskType, duration, status}` — the requests
/// explorer's per-row view. Distinct from `GET /api/metrics/requests`, which
/// returns the full raw `RequestMetrics` for analysis; this is the trimmed,
/// UI-facing projection.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestRecordResponse {
    id: String,
    /// RFC3339 receive time.
    timestamp: String,
    model: String,
    task_type: String,
    /// Total request latency in ms.
    duration: f64,
    status: &'static str,
}

/// Lowercase string for a request status (`RequestStatus` is `#[non_exhaustive]`,
/// so an unknown future variant maps to `"unknown"` rather than failing to build).
fn request_status_str(s: llamaforge_metrics::RequestStatus) -> &'static str {
    use llamaforge_metrics::RequestStatus::*;
    match s {
        Queued => "queued",
        Processing => "processing",
        Completed => "completed",
        Failed => "failed",
        Evicted => "evicted",
        Cancelled => "cancelled",
        Timeout => "timeout",
        _ => "unknown",
    }
}

impl From<llamaforge_metrics::RequestMetrics> for RequestRecordResponse {
    fn from(m: llamaforge_metrics::RequestMetrics) -> Self {
        RequestRecordResponse {
            id: m.request_id.to_string(),
            timestamp: m.received_at.to_rfc3339(),
            model: m.model_name,
            task_type: m.task_type.to_string(),
            duration: m.total_latency_ms,
            status: request_status_str(m.status),
        }
    }
}

/// `GET /api/requests` — the requests explorer (FRONTEND_ARCHITECTURE.md §5.2).
/// Queries the request-metrics store with the same filter/pagination surface as
/// `/api/metrics/requests`, then projects each record onto the trimmed
/// `RequestRecord` shape the table renders.
pub(crate) async fn query_requests<CW, RMS, SM>(
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
    let records: Vec<RequestRecordResponse> =
        metrics.into_iter().map(RequestRecordResponse::from).collect();
    Ok(Json(records).into_response())
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

/// `GET /api/metrics/summary` query params — a bare `MetricsFilter`.
#[derive(serde::Deserialize, Default)]
pub(crate) struct SummaryQueryParams {
    #[serde(flatten)]
    pub filter: llamaforge_metrics::MetricsFilter,
}

/// Frontend `AggregatedMetrics` shape (`{throughput_total, latency:{p50,p90,p99}}`,
/// FRONTEND_ARCHITECTURE.md §7.2). `latency` is omitted when there are no samples,
/// matching the optional TS field. This is the dashboard summary — distinct from
/// `/api/metrics/aggregated`, which is the explorer's grouped view.
#[derive(Serialize)]
struct SummaryResponse {
    throughput_total: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency: Option<LatencyPercentiles>,
}

#[derive(Serialize)]
struct LatencyPercentiles {
    p50: f64,
    p90: f64,
    p99: f64,
}

impl From<llamaforge_metrics::RequestMetricsSummary> for SummaryResponse {
    fn from(s: llamaforge_metrics::RequestMetricsSummary) -> Self {
        let latency = match (s.latency_p50_ms, s.latency_p90_ms, s.latency_p99_ms) {
            (Some(p50), Some(p90), Some(p99)) => Some(LatencyPercentiles { p50, p90, p99 }),
            _ => None,
        };
        SummaryResponse { throughput_total: s.throughput_total, latency }
    }
}

/// Frontend `ServerMetricsSummary` shape (FRONTEND_ARCHITECTURE.md §5.2):
/// `{throughput, latency:{p50,p90,p99}, errorRate, slots:{total,idle,processing,loading}}`.
/// Unlike the fleet `SummaryResponse`, `latency` is non-optional here (the
/// frontend `ServerMetricsSummary.latency` is required), so absent percentiles
/// surface as `0`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerMetricsResponse {
    throughput: f64,
    latency: LatencyPercentiles,
    error_rate: f64,
    slots: SlotBreakdown,
}

/// Per-server slot occupancy. `loading` is always `0`: the `ServerSummary`
/// exposes only `slots_total`/`slots_busy`, with no per-slot model-loading
/// signal yet (tracked in 07_TRANSPORT_LAYER.md). `idle` is the saturating
/// remainder so a transient `busy > total` race can't underflow.
#[derive(Serialize)]
struct SlotBreakdown {
    total: u32,
    idle: u32,
    processing: u32,
    loading: u32,
}

impl ServerMetricsResponse {
    fn from_parts(
        summary: &ServerSummary,
        metrics: llamaforge_metrics::RequestMetricsSummary,
    ) -> Self {
        ServerMetricsResponse {
            throughput: metrics.throughput_total,
            latency: LatencyPercentiles {
                p50: metrics.latency_p50_ms.unwrap_or(0.0),
                p90: metrics.latency_p90_ms.unwrap_or(0.0),
                p99: metrics.latency_p99_ms.unwrap_or(0.0),
            },
            error_rate: metrics.error_rate,
            slots: SlotBreakdown {
                total: summary.slots_total,
                idle: summary.slots_total.saturating_sub(summary.slots_busy),
                processing: summary.slots_busy,
                loading: 0,
            },
        }
    }
}

pub(crate) async fn query_summary<CW, RMS, SM>(
    State(state): State<ManagementState<CW, RMS, SM>>,
    Query(params): Query<SummaryQueryParams>,
) -> Result<Response, ApiError>
where
    RMS: RequestMetricsStore + Send + Sync + 'static,
{
    let summary = state
        .request_metrics_store
        .request_metrics_summary(&params.filter)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(SummaryResponse::from(summary)).into_response())
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
        .route("/api/servers/{id}", get(get_server::<CW, RMS, SM>))
        .route("/api/servers/{id}/metrics", get(server_metrics::<CW, RMS, SM>))
        .route("/api/servers/{id}/start", post(start_server::<CW, RMS, SM>))
        .route("/api/servers/{id}/stop", post(stop_server::<CW, RMS, SM>))
        .route("/api/servers/{id}/restart", post(restart_server::<CW, RMS, SM>))
        .route("/api/queue", get(queue_status::<CW, RMS, SM>))
        .route("/api/queue/{id}", delete(cancel_request::<CW, RMS, SM>))
        .route("/api/config/reload", post(reload_config::<CW, RMS, SM>))
        .route("/api/requests", get(query_requests::<CW, RMS, SM>))
        // Cancel reuses the queue submitter's cancel (same as DELETE /api/queue/{id});
        // the explorer drives it via POST per the frontend client.
        .route("/api/requests/{id}/cancel", post(cancel_request::<CW, RMS, SM>))
        .route("/api/metrics/requests", get(query_request_metrics::<CW, RMS, SM>))
        .route("/api/metrics/aggregated", get(query_aggregated::<CW, RMS, SM>))
        .route("/api/metrics/summary", get(query_summary::<CW, RMS, SM>))
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
        async fn restart_server(&self, _id: &ServerId) -> Result<(), ServerError> {
            Ok(())
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

    /// Like [`build_test_state`] but backed by a real (empty) `InMemoryStore`,
    /// for handlers bounded on `RMS: RequestMetricsStore` (e.g. `server_metrics`).
    fn build_test_state_with_store() -> ManagementState<
        ConfigHandle,
        llamaforge_metrics::InMemoryStore,
        llamaforge_metrics::InMemoryStore,
    > {
        let config = AppConfig { global: GlobalConfig::default(), ..Default::default() };
        let source = Arc::new(InMemoryConfigSource::new(config));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());

        ManagementState {
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitter),
            config_reader: handle.clone(),
            config_writer: handle,
            request_metrics_store: Arc::new(llamaforge_metrics::InMemoryStore::default()),
            store_maintenance: Arc::new(llamaforge_metrics::InMemoryStore::default()),
        }
    }

    struct OneRunningServer;
    #[async_trait::async_trait]
    impl ServerControl for OneRunningServer {
        async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError> {
            Ok(vec![ServerSummary {
                id: ServerId::new(uuid::Uuid::nil()),
                alias: llamaforge_core::types::ModelAlias::new(std::sync::Arc::from("gemma-3-1b")),
                state: ServerLifecycle::Running,
                port: 8081,
                slots_total: 2,
                slots_busy: 0,
                started_at: Some(chrono::Utc::now() - chrono::Duration::seconds(42)),
                last_restart_at: None,
            }])
        }
        async fn start_server(&self, _id: &ServerId) -> Result<(), ServerError> {
            Ok(())
        }
        async fn stop_server(&self, _id: &ServerId) -> Result<(), ServerError> {
            Ok(())
        }
        async fn restart_server(&self, _id: &ServerId) -> Result<(), ServerError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn list_servers_emits_frontend_serverstate_shape() {
        // Regression for #292: the dashboard's StatusBadge keys off a lowercase
        // `status` and ServerCard reads `name`/`uptime`. The old `{model,
        // state:Debug, slots_*}` shape left all three undefined → StatusBadge threw
        // and the cards were stuck on skeletons.
        let mut state = build_test_state();
        state.server_control = Arc::new(OneRunningServer);

        let res = list_servers(axum::extract::State(state)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let server = &body[0];

        assert_eq!(server["name"], "gemma-3-1b");
        assert_eq!(server["status"], "running"); // lowercase ServerStatus, not Debug "Running"
        assert!(server["uptime"].as_u64().unwrap() >= 42);
        // The pre-#292 keys must be gone so the shape can't silently regress back.
        assert!(server.get("state").is_none());
        assert!(server.get("model").is_none());
        assert!(server.get("slots_total").is_none());
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

        // #293: the restart route is wired and parses the id the same way.
        let res = restart_server(
            axum::extract::State(state.clone()),
            axum::extract::Path("gemma3-27b".to_string()),
        )
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

    /// A control that reports one Running server whose id is the UUIDv5 of the
    /// slug `"gemma3-27b"`, so `from_config_id("gemma3-27b")` (the path-derived
    /// id) matches it. Carries 4 slots with 1 busy for the slot-breakdown test.
    struct ServerWithSlots;
    #[async_trait::async_trait]
    impl ServerControl for ServerWithSlots {
        async fn list_servers(&self) -> Result<Vec<ServerSummary>, ServerError> {
            Ok(vec![ServerSummary {
                id: ServerId::from_config_id("gemma3-27b"),
                alias: llamaforge_core::types::ModelAlias::new(std::sync::Arc::from("gemma-3-27b")),
                state: ServerLifecycle::Running,
                port: 8081,
                slots_total: 4,
                slots_busy: 1,
                started_at: Some(chrono::Utc::now() - chrono::Duration::seconds(42)),
                last_restart_at: None,
            }])
        }
        async fn start_server(&self, _id: &ServerId) -> Result<(), ServerError> {
            Ok(())
        }
        async fn stop_server(&self, _id: &ServerId) -> Result<(), ServerError> {
            Ok(())
        }
        async fn restart_server(&self, _id: &ServerId) -> Result<(), ServerError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn get_server_returns_single_serverstate() {
        // #293: GET /api/servers/{id} returns the same ServerState shape as the
        // list, for the one matching server.
        let mut state = build_test_state();
        state.server_control = Arc::new(ServerWithSlots);

        let res =
            get_server(axum::extract::State(state), axum::extract::Path("gemma3-27b".to_string()))
                .await
                .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["name"], "gemma-3-27b");
        assert_eq!(body["status"], "running");
        assert!(body["uptime"].as_u64().unwrap() >= 42);
        // Not the list — a single object, not an array.
        assert!(body.is_object());
    }

    #[tokio::test]
    async fn get_server_unknown_is_404() {
        // The default MockServerControl reports no servers, so any id is absent.
        let state = build_test_state();
        let err = get_server(axum::extract::State(state), axum::extract::Path("ghost".to_string()))
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::NotFound(_)));
        assert_eq!(err.into_response().status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn server_metrics_assembles_summary_and_slots() {
        // #293: GET /api/servers/{id}/metrics returns the frontend
        // ServerMetricsSummary. With an empty metrics store the throughput/
        // latency/errorRate are zero, while the slot breakdown comes from the
        // live ServerSummary (4 total, 1 busy → 3 idle, 0 loading).
        let mut state = build_test_state_with_store();
        state.server_control = Arc::new(ServerWithSlots);

        let res = server_metrics(
            axum::extract::State(state),
            axum::extract::Path("gemma3-27b".to_string()),
        )
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(body["throughput"], 0.0);
        assert_eq!(body["latency"]["p50"], 0.0);
        assert_eq!(body["latency"]["p90"], 0.0);
        assert_eq!(body["latency"]["p99"], 0.0);
        // camelCase, matching the TS `ServerMetricsSummary.errorRate`.
        assert_eq!(body["errorRate"], 0.0);
        assert!(body.get("error_rate").is_none());
        assert_eq!(body["slots"]["total"], 4);
        assert_eq!(body["slots"]["idle"], 3);
        assert_eq!(body["slots"]["processing"], 1);
        assert_eq!(body["slots"]["loading"], 0);
    }

    #[tokio::test]
    async fn server_metrics_unknown_is_404() {
        let state = build_test_state_with_store();
        let err =
            server_metrics(axum::extract::State(state), axum::extract::Path("ghost".to_string()))
                .await
                .unwrap_err();
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    fn mk_request_metrics(
        id: &str,
        task: &str,
        status: llamaforge_metrics::RequestStatus,
    ) -> llamaforge_metrics::RequestMetrics {
        use llamaforge_core::config::{RequestorId, TaskTypeId};
        llamaforge_metrics::RequestMetrics {
            request_id: llamaforge_metrics::RequestId(id.to_string()),
            model_name: "gemma-3-27b".to_string(),
            server_id: None,
            slot_id: None,
            task_type: TaskTypeId(task.to_string()),
            requestor: RequestorId("anon".to_string()),
            effective_priority: 0,
            received_at: chrono::DateTime::parse_from_rfc3339("2026-06-06T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            dispatched_at: None,
            first_token_at: None,
            completed_at: None,
            queue_wait_ms: 0.0,
            ttft_ms: None,
            generation_duration_ms: None,
            total_latency_ms: 123.0,
            prompt_tokens: 0,
            completion_tokens: 0,
            prompt_tps: None,
            generation_tps: None,
            status,
            finish_reason: None,
            was_evicted: false,
            eviction_count: 0,
            error_message: None,
        }
    }

    #[tokio::test]
    async fn query_requests_projects_requestrecord_shape() {
        // #293: GET /api/requests returns the trimmed RequestRecord projection,
        // not the raw RequestMetrics. `taskType` is camelCase; `status` is the
        // lowercase string.
        use llamaforge_metrics::RequestMetricsStore;
        let store = Arc::new(llamaforge_metrics::InMemoryStore::default());
        store
            .insert_request_metrics(vec![mk_request_metrics(
                "req-1",
                "chat",
                llamaforge_metrics::RequestStatus::Completed,
            )])
            .await
            .unwrap();

        let config = AppConfig { global: GlobalConfig::default(), ..Default::default() };
        let source = Arc::new(InMemoryConfigSource::new(config));
        let handle = Arc::new(ConfigHandle::load(source).unwrap());
        let state = ManagementState {
            server_control: Arc::new(MockServerControl),
            queue_submitter: Arc::new(MockQueueSubmitter),
            config_reader: handle.clone(),
            config_writer: handle,
            request_metrics_store: store,
            store_maintenance: Arc::new(llamaforge_metrics::InMemoryStore::default()),
        };

        let res = query_requests(
            axum::extract::State(state),
            axum::extract::Query(MetricsQueryParams::default()),
        )
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let row = &body[0];
        assert_eq!(row["id"], "req-1");
        assert_eq!(row["model"], "gemma-3-27b");
        assert_eq!(row["taskType"], "chat"); // camelCase, not task_type
        assert!(row.get("task_type").is_none());
        assert_eq!(row["duration"], 123.0);
        assert_eq!(row["status"], "completed");
        assert_eq!(row["timestamp"], "2026-06-06T00:00:00+00:00");
        // Trimmed projection — raw RequestMetrics fields must not leak.
        assert!(row.get("completion_tokens").is_none());
        assert!(row.get("requestor").is_none());
    }

    #[tokio::test]
    async fn cancel_request_via_post_route_parses_and_delegates() {
        // POST /api/requests/{id}/cancel reuses cancel_request: a valid UUID
        // delegates to the queue submitter (200), a malformed id is a 400.
        let state = build_test_state();
        let id = uuid::Uuid::now_v7().to_string();
        let res = cancel_request(axum::extract::State(state.clone()), axum::extract::Path(id))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let bad = cancel_request(
            axum::extract::State(state),
            axum::extract::Path("not-a-uuid".to_string()),
        )
        .await;
        assert!(matches!(bad.unwrap_err(), ApiError::InvalidBody(_)));
    }
}
