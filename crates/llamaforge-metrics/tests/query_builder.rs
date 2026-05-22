use chrono::{TimeZone, Utc};

use llamaforge_core::config::RequestorId;
use llamaforge_metrics::{build_event_where, build_group_by, build_metrics_where};
use llamaforge_metrics::{
    EventFilter, EventType, GroupByField, MetricsFilter, RequestId, RequestStatus,
};
use llamaforge_core::types::ServerId;

#[test]
fn test_build_event_where_empty() {
    let filter = EventFilter::default();
    let (sql, params) = build_event_where(&filter);
    assert!(sql.is_empty());
    assert!(params.is_empty());
}

#[test]
fn test_build_event_where_full() {
    let since = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
    let filter = EventFilter {
        server_id: Some(ServerId::from_config_id("s1")),
        event_type: Some(EventType::Audit),
        since: Some(since),
        until: None,
        request_id: Some(RequestId("r1".to_string())),
    };

    let (sql, params) = build_event_where(&filter);
    assert_eq!(
        sql,
        "WHERE server_id = ? AND event_type = ? AND created_at >= ? AND request_id = ?"
    );
    assert_eq!(params.len(), 4);
}

#[test]
fn test_build_metrics_where_empty() {
    let filter = MetricsFilter::default();
    let (sql, params) = build_metrics_where(&filter);
    assert!(sql.is_empty());
    assert!(params.is_empty());
}

#[test]
fn test_build_metrics_where_full() {
    let filter = MetricsFilter {
        server_id: Some(ServerId::from_config_id("s1")),
        model_name: Some("llama".to_string()),
        requestor: Some(RequestorId("u1".to_string())),
        status: Some(RequestStatus::Completed),
        since: None,
        until: None,
    };

    let (sql, params) = build_metrics_where(&filter);
    assert_eq!(sql, "WHERE server_id = ? AND model_name = ? AND requestor = ? AND status = ?");
    assert_eq!(params.len(), 4);
}

#[test]
fn test_build_group_by_empty() {
    let sql = build_group_by(&[]);
    assert!(sql.is_empty());
}

#[test]
fn test_build_group_by_fields() {
    let fields = vec![GroupByField::ServerId, GroupByField::Hour];
    let sql = build_group_by(&fields);
    assert_eq!(sql, "GROUP BY server_id, strftime('%Y-%m-%d %H:00:00', received_at)");
}
