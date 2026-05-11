//! SQLite row definitions and conversions for domain types.
//!
//! Maps database flat rows to typed domain objects.

use chrono::{DateTime, Utc};
use llamaforge_config::{RequestorId, ServerId, TaskTypeId};

use crate::traits::ConfigOverlay;
use crate::types::{
    AuditEvent, ConfigSnapshot, ConfigSnapshotTrigger, EventType, ManagedProcess, ProcessId,
    RequestId, RequestMetrics, RequestStatus,
};

// --- AuditEventRow ---

pub struct AuditEventRow {
    pub event_id: String,
    pub event_type: String,
    pub event_kind: String,
    pub server_id: Option<String>,
    pub request_id: Option<String>,
    pub created_at: String,
    pub payload: String,
}

impl TryFrom<AuditEventRow> for AuditEvent {
    type Error = crate::error::RowConversionError;

    fn try_from(row: AuditEventRow) -> Result<Self, Self::Error> {
        let event_type: EventType =
            serde_json::from_value(serde_json::Value::String(row.event_type))?;
        let server_id = row.server_id.map(ServerId);
        let request_id = row.request_id.map(RequestId);
        let created_at = DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc);
        let payload = serde_json::from_str(&row.payload)?;

        Ok(Self {
            event_id: row.event_id,
            event_type,
            event_kind: row.event_kind,
            server_id,
            request_id,
            created_at,
            payload,
        })
    }
}

impl From<AuditEvent> for AuditEventRow {
    fn from(event: AuditEvent) -> Self {
        Self {
            event_id: event.event_id,
            event_type: match event.event_type {
                EventType::Audit => "audit".to_string(),
                EventType::Telemetry => "telemetry".to_string(),
            },
            event_kind: event.event_kind,
            server_id: event.server_id.map(|s| s.0),
            request_id: event.request_id.map(|r| r.0),
            created_at: event.created_at.to_rfc3339(),
            payload: serde_json::to_string(&event.payload).unwrap_or_else(|_| "{}".to_string()),
        }
    }
}

// --- RequestMetricsRow ---

pub struct RequestMetricsRow {
    pub request_id: String,
    pub model_name: String,
    pub server_id: Option<String>,
    pub slot_id: Option<i64>,
    pub task_type: String,
    pub requestor: String,
    pub effective_priority: i64,

    pub received_at: String,
    pub dispatched_at: Option<String>,
    pub first_token_at: Option<String>,
    pub completed_at: Option<String>,

    pub queue_wait_ms: f64,
    pub ttft_ms: Option<f64>,
    pub generation_duration_ms: Option<f64>,
    pub total_latency_ms: f64,

    pub prompt_tokens: i64,
    pub completion_tokens: i64,

    pub prompt_tps: Option<f64>,
    pub generation_tps: Option<f64>,

    pub status: String,
    pub finish_reason: Option<String>,
    pub was_evicted: i64,
    pub eviction_count: i64,
    pub error_message: Option<String>,
}

impl TryFrom<RequestMetricsRow> for RequestMetrics {
    type Error = crate::error::RowConversionError;

    fn try_from(row: RequestMetricsRow) -> Result<Self, Self::Error> {
        let status: RequestStatus = serde_json::from_value(serde_json::Value::String(row.status))?;

        let parse_time =
            |s: Option<String>| -> Result<Option<DateTime<Utc>>, crate::error::RowConversionError> {
                if let Some(st) = s {
                    Ok(Some(DateTime::parse_from_rfc3339(&st)?.with_timezone(&Utc)))
                } else {
                    Ok(None)
                }
            };

        Ok(Self {
            request_id: RequestId(row.request_id),
            model_name: row.model_name,
            server_id: row.server_id.map(ServerId),
            slot_id: row.slot_id.map(|v| v as u32),
            task_type: TaskTypeId(row.task_type),
            requestor: RequestorId(row.requestor),
            effective_priority: row.effective_priority as u32,

            received_at: DateTime::parse_from_rfc3339(&row.received_at)?.with_timezone(&Utc),
            dispatched_at: parse_time(row.dispatched_at)?,
            first_token_at: parse_time(row.first_token_at)?,
            completed_at: parse_time(row.completed_at)?,

            queue_wait_ms: row.queue_wait_ms,
            ttft_ms: row.ttft_ms,
            generation_duration_ms: row.generation_duration_ms,
            total_latency_ms: row.total_latency_ms,

            prompt_tokens: row.prompt_tokens as u32,
            completion_tokens: row.completion_tokens as u32,

            prompt_tps: row.prompt_tps,
            generation_tps: row.generation_tps,

            status,
            finish_reason: row.finish_reason,
            was_evicted: row.was_evicted != 0,
            eviction_count: row.eviction_count as u32,
            error_message: row.error_message,
        })
    }
}

impl From<RequestMetrics> for RequestMetricsRow {
    fn from(m: RequestMetrics) -> Self {
        Self {
            request_id: m.request_id.0,
            model_name: m.model_name,
            server_id: m.server_id.map(|s| s.0),
            slot_id: m.slot_id.map(|v| v as i64),
            task_type: m.task_type.0,
            requestor: m.requestor.0,
            effective_priority: m.effective_priority as i64,

            received_at: m.received_at.to_rfc3339(),
            dispatched_at: m.dispatched_at.map(|t| t.to_rfc3339()),
            first_token_at: m.first_token_at.map(|t| t.to_rfc3339()),
            completed_at: m.completed_at.map(|t| t.to_rfc3339()),

            queue_wait_ms: m.queue_wait_ms,
            ttft_ms: m.ttft_ms,
            generation_duration_ms: m.generation_duration_ms,
            total_latency_ms: m.total_latency_ms,

            prompt_tokens: m.prompt_tokens as i64,
            completion_tokens: m.completion_tokens as i64,

            prompt_tps: m.prompt_tps,
            generation_tps: m.generation_tps,

            status: match m.status {
                RequestStatus::Queued => "queued".to_string(),
                RequestStatus::Processing => "processing".to_string(),
                RequestStatus::Completed => "completed".to_string(),
                RequestStatus::Failed => "failed".to_string(),
                RequestStatus::Evicted => "evicted".to_string(),
                RequestStatus::Cancelled => "cancelled".to_string(),
                RequestStatus::Timeout => "timeout".to_string(),
            },
            finish_reason: m.finish_reason,
            was_evicted: if m.was_evicted { 1 } else { 0 },
            eviction_count: m.eviction_count as i64,
            error_message: m.error_message,
        }
    }
}

// --- ManagedProcessRow ---

pub struct ManagedProcessRow {
    pub pid: i64,
    pub server_id: String,
    pub started_at: String,
    pub cmdline: String,
    pub port: u16,
    pub model_alias: String,
}

impl TryFrom<ManagedProcessRow> for ManagedProcess {
    type Error = crate::error::RowConversionError;

    fn try_from(row: ManagedProcessRow) -> Result<Self, Self::Error> {
        Ok(Self {
            pid: ProcessId(row.pid as u32),
            server_id: ServerId(row.server_id),
            started_at: DateTime::parse_from_rfc3339(&row.started_at)?.with_timezone(&Utc),
            cmdline: row.cmdline,
            port: row.port,
            model_alias: row.model_alias,
        })
    }
}

impl From<ManagedProcess> for ManagedProcessRow {
    fn from(p: ManagedProcess) -> Self {
        Self {
            pid: p.pid.0 as i64,
            server_id: p.server_id.0,
            started_at: p.started_at.to_rfc3339(),
            cmdline: p.cmdline,
            port: p.port,
            model_alias: p.model_alias,
        }
    }
}

// --- ConfigSnapshotRow ---

pub struct ConfigSnapshotRow {
    pub id: String,
    pub trigger: String,
    pub created_at: String,
    pub config_json: String,
}

impl TryFrom<ConfigSnapshotRow> for ConfigSnapshot {
    type Error = crate::error::RowConversionError;

    fn try_from(row: ConfigSnapshotRow) -> Result<Self, Self::Error> {
        let trigger: ConfigSnapshotTrigger =
            serde_json::from_value(serde_json::Value::String(row.trigger))?;

        Ok(Self {
            id: row.id,
            trigger,
            created_at: DateTime::parse_from_rfc3339(&row.created_at)?.with_timezone(&Utc),
            config_json: row.config_json,
        })
    }
}

impl From<ConfigSnapshot> for ConfigSnapshotRow {
    fn from(c: ConfigSnapshot) -> Self {
        Self {
            id: c.id,
            trigger: match c.trigger {
                ConfigSnapshotTrigger::Startup => "startup".to_string(),
                ConfigSnapshotTrigger::Reload => "reload".to_string(),
                ConfigSnapshotTrigger::Writeback => "writeback".to_string(),
            },
            created_at: c.created_at.to_rfc3339(),
            config_json: c.config_json,
        }
    }
}

// --- ConfigOverlayRow ---

pub struct ConfigOverlayRow {
    pub server_id: String,
    pub overlay_key: String,
    pub value: String,
    pub applied_at: String,
}

impl TryFrom<ConfigOverlayRow> for ConfigOverlay {
    type Error = crate::error::RowConversionError;

    fn try_from(row: ConfigOverlayRow) -> Result<Self, Self::Error> {
        Ok(Self {
            server_id: ServerId(row.server_id),
            overlay_key: row.overlay_key,
            value: row.value,
            applied_at: DateTime::parse_from_rfc3339(&row.applied_at)?.with_timezone(&Utc),
        })
    }
}

impl From<ConfigOverlay> for ConfigOverlayRow {
    fn from(c: ConfigOverlay) -> Self {
        Self {
            server_id: c.server_id.0,
            overlay_key: c.overlay_key,
            value: c.value,
            applied_at: c.applied_at.to_rfc3339(),
        }
    }
}
