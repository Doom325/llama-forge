use llamaforge_config::{RequestorId, ServerId, TaskTypeId};
use llamaforge_metrics::{
    AuditRecord, ManagedProcess, ProcessId, RequestId, RequestMetrics, RequestStatus,
};

#[test]
fn test_type_interop() {
    let server_id = ServerId::from("server-1");
    let task_type = TaskTypeId::from("task-chat");
    let requestor = RequestorId::from("user-42");

    let req_metrics = RequestMetrics {
        request_id: RequestId::from("req-999"),
        model_name: "llama-3".to_string(),
        server_id: Some(server_id.clone()),
        slot_id: Some(1),
        task_type,
        requestor,
        effective_priority: 10,
        received_at: chrono::Utc::now(),
        dispatched_at: None,
        first_token_at: None,
        completed_at: None,
        queue_wait_ms: 0.0,
        ttft_ms: None,
        generation_duration_ms: None,
        total_latency_ms: 0.0,
        prompt_tokens: 0,
        completion_tokens: 0,
        prompt_tps: None,
        generation_tps: None,
        status: RequestStatus::Queued,
        finish_reason: None,
        was_evicted: false,
        eviction_count: 0,
        error_message: None,
    };

    let audit_record = AuditRecord { request_metrics: req_metrics.clone() };
    assert_eq!(audit_record.request_metrics.server_id, Some(server_id.clone()));

    let process = ManagedProcess {
        pid: ProcessId(100),
        server_id: server_id.clone(),
        started_at: chrono::Utc::now(),
        cmdline: "./llama-server".to_string(),
    };
    assert_eq!(process.server_id, server_id);
}
