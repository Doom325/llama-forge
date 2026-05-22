//! Dynamic SQL query builders for event and metrics filters.

use rusqlite::ToSql;

use crate::filter::{EventFilter, GroupByField, MetricsFilter};

/// Builds the WHERE clause and parameters for an `EventFilter`.
pub fn build_event_where(filter: &EventFilter) -> (String, Vec<Box<dyn ToSql + Send + Sync>>) {
    let mut clauses = Vec::new();
    let mut params: Vec<Box<dyn ToSql + Send + Sync>> = Vec::new();

    if let Some(server_id) = &filter.server_id {
        clauses.push("server_id = ?");
        params.push(Box::new(server_id.to_string()));
    }

    if let Some(event_type) = &filter.event_type {
        clauses.push("event_type = ?");
        let event_type_str = match event_type {
            crate::types::EventType::Audit => "audit",
            crate::types::EventType::Telemetry => "telemetry",
        };
        params.push(Box::new(event_type_str.to_string()));
    }

    if let Some(since) = filter.since {
        clauses.push("created_at >= ?");
        params.push(Box::new(since.to_rfc3339()));
    }

    if let Some(until) = filter.until {
        clauses.push("created_at <= ?");
        params.push(Box::new(until.to_rfc3339()));
    }

    if let Some(request_id) = &filter.request_id {
        clauses.push("request_id = ?");
        params.push(Box::new(request_id.0.clone()));
    }

    let where_clause =
        if clauses.is_empty() { String::new() } else { format!("WHERE {}", clauses.join(" AND ")) };

    (where_clause, params)
}

/// Builds the WHERE clause and parameters for a `MetricsFilter`.
pub fn build_metrics_where(filter: &MetricsFilter) -> (String, Vec<Box<dyn ToSql + Send + Sync>>) {
    let mut clauses = Vec::new();
    let mut params: Vec<Box<dyn ToSql + Send + Sync>> = Vec::new();

    if let Some(server_id) = &filter.server_id {
        clauses.push("server_id = ?");
        params.push(Box::new(server_id.to_string()));
    }

    if let Some(model_name) = &filter.model_name {
        clauses.push("model_name = ?");
        params.push(Box::new(model_name.clone()));
    }

    if let Some(requestor) = &filter.requestor {
        clauses.push("requestor = ?");
        params.push(Box::new(requestor.0.clone()));
    }

    if let Some(status) = &filter.status {
        clauses.push("status = ?");
        let status_str = match status {
            crate::types::RequestStatus::Queued => "queued",
            crate::types::RequestStatus::Processing => "processing",
            crate::types::RequestStatus::Completed => "completed",
            crate::types::RequestStatus::Failed => "failed",
            crate::types::RequestStatus::Evicted => "evicted",
            crate::types::RequestStatus::Cancelled => "cancelled",
            crate::types::RequestStatus::Timeout => "timeout",
        };
        params.push(Box::new(status_str.to_string()));
    }

    if let Some(since) = filter.since {
        clauses.push("received_at >= ?");
        params.push(Box::new(since.to_rfc3339()));
    }

    if let Some(until) = filter.until {
        clauses.push("received_at <= ?");
        params.push(Box::new(until.to_rfc3339()));
    }

    let where_clause =
        if clauses.is_empty() { String::new() } else { format!("WHERE {}", clauses.join(" AND ")) };

    (where_clause, params)
}

/// Builds the GROUP BY expression for an aggregate query.
pub fn build_group_by(fields: &[GroupByField]) -> String {
    if fields.is_empty() {
        return String::new();
    }

    let field_strs: Vec<String> = fields
        .iter()
        .map(|f| match f {
            GroupByField::ServerId => "server_id".to_string(),
            GroupByField::ModelName => "model_name".to_string(),
            GroupByField::Requestor => "requestor".to_string(),
            GroupByField::Hour => "strftime('%Y-%m-%d %H:00:00', received_at)".to_string(),
        })
        .collect();

    format!("GROUP BY {}", field_strs.join(", "))
}
