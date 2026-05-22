use llamaforge_core::queue::pipeline::RequestPipeline;
use llamaforge_core::queue::types::{DispatchResult, TaskType};
use llamaforge_core::types::traits::{
    AdmissionError, DispatchEvent, IncomingRequest, QueueError, QueueSnapshot, QueueSubmitter,
    QueueTicket,
};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::instrument;

pub struct QueueSubmitterAdapter {
    pipeline: Arc<RequestPipeline>,
}

impl QueueSubmitterAdapter {
    pub fn new(pipeline: Arc<RequestPipeline>) -> Self {
        Self { pipeline }
    }

    fn map_class_to_task(class: &llamaforge_core::types::RequestorClass) -> TaskType {
        match class.as_str() {
            "emergency" => TaskType::Emergency,
            "background" => TaskType::Background,
            "batch_analysis" => TaskType::BatchAnalysis,
            "code_gen" => TaskType::CodeGen,
            "code_review" => TaskType::CodeReview,
            "summarization" => TaskType::Summarization,
            _ => TaskType::RealtimeChat,
        }
    }
}

impl QueueSubmitter for QueueSubmitterAdapter {
    #[instrument(skip(self, req), fields(request_id = %req.request_id))]
    fn submit(&self, req: IncomingRequest) -> Result<QueueTicket, AdmissionError> {
        let (response_tx, response_rx) = oneshot::channel();
        let cancel_token = CancellationToken::new();
        let requestor_id = req.requestor_id.clone();
        let request_id = req.request_id;

        let q_req = llamaforge_core::queue::types::IncomingRequest {
            id: request_id,
            payload: Arc::new(()), // Transport doesn't pass a payload to queue; dispatch takes it directly later
            task_type: Self::map_class_to_task(&req.requestor_class),
            requestor: requestor_id.clone(),
            requestor_class: req.requestor_class.clone(),
            model_target: req.model_target.clone(),
            capabilities: req.capabilities.clone(),
            session_id: req.session_id.clone(),
            explicit_override: None,
            response_tx,
            cancel_token: cancel_token.clone(),
            trace_context: None,
        };

        match self.pipeline.submit(q_req) {
            Ok(pos) => {
                let (events_tx, events_rx) = mpsc::channel(32);
                let _ = events_tx.try_send(DispatchEvent::Queued { position: pos.position });

                let pipeline_clone = self.pipeline.clone();
                let cancel_clone = cancel_token.clone();

                tokio::spawn(async move {
                    tokio::select! {
                        res = response_rx => {
                            match res {
                                Ok(DispatchResult::Dispatched(server, slot)) => {
                                    // By design (PR6), the slot might be None if Auto.
                                    // For now, default to SlotId 0 if Auto, or just wait for InferenceEngineAdapter to refine it.
                                    let slot_id = slot.unwrap_or_else(|| llamaforge_core::types::SlotId::new(0));
                                    let _ = events_tx.send(DispatchEvent::Dispatched { server_id: server, slot_id }).await;
                                    // Wait for ticket to be dropped by the InferenceEngineAdapter
                                    events_tx.closed().await;
                                    pipeline_clone.record_completion(&requestor_id);
                                }
                                Ok(DispatchResult::Cancelled) => {
                                    let _ = events_tx.send(DispatchEvent::Failed { error: "cancelled".to_string() }).await;
                                    // pipeline.cancel() already handled dequeue logic
                                }
                                Ok(DispatchResult::TimedOut { .. }) => {
                                    let _ = events_tx.send(DispatchEvent::Failed { error: "timed out".to_string() }).await;
                                    // Starvation/timeout logic already handled dequeue
                                }
                                Ok(DispatchResult::ShuttingDown) => {
                                    let _ = events_tx.send(DispatchEvent::Failed { error: "shutting down".to_string() }).await;
                                    // Drain queue already handled dequeue
                                }
                                Err(_) => {
                                    // Sender dropped without result, dequeue handled at drop site
                                }
                            }
                        }
                        _ = cancel_clone.cancelled() => {
                            pipeline_clone.cancel(request_id);
                            let _ = events_tx.send(DispatchEvent::Failed { error: "cancelled".to_string() }).await;
                        }
                        _ = events_tx.closed() => {
                            // Ticket was dropped before queue made a decision
                            pipeline_clone.cancel(request_id);
                        }
                    }
                });

                Ok(QueueTicket {
                    request_id,
                    admitted_at: chrono::Utc::now(),
                    cancel_token,
                    events: events_rx,
                })
            }
            Err(e) => match e {
                llamaforge_core::queue::error::AdmissionError::QueueFull { retry_after_secs, .. } => {
                    Err(AdmissionError::RateLimited { retry_after_secs })
                }
                llamaforge_core::queue::error::AdmissionError::RequestorLimit { .. } => {
                    Err(AdmissionError::RateLimited { retry_after_secs: 5 })
                }
                llamaforge_core::queue::error::AdmissionError::NoServerForModel { model } => {
                    Err(AdmissionError::InvalidRequest {
                        reason: format!("No server for model: {}", model),
                    })
                }
                llamaforge_core::queue::error::AdmissionError::UnauthorizedTaskType { task_type } => {
                    Err(AdmissionError::InvalidRequest {
                        reason: format!("Unauthorized task type: {}", task_type),
                    })
                }
                llamaforge_core::queue::error::AdmissionError::PriorityOutOfRange { priority } => {
                    Err(AdmissionError::InvalidRequest {
                        reason: format!("Priority out of range: {}", priority),
                    })
                }
                llamaforge_core::queue::error::AdmissionError::Internal(reason) => {
                    Err(AdmissionError::InvalidRequest { reason })
                }
            },
        }
    }

    fn cancel(&self, id: &llamaforge_core::types::RequestId) -> Result<(), QueueError> {
        if self.pipeline.cancel(*id).is_some() {
            Ok(())
        } else {
            // It might already be dispatched or not found
            Err(QueueError::NotFound { id: *id })
        }
    }

    fn status(&self) -> Result<QueueSnapshot, QueueError> {
        let depth = self.pipeline.depth();
        Ok(QueueSnapshot { depth: depth.total, estimated_throughput_req_sec: 0.0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llamaforge_core::config::{ConfigHandle, ConfigPaths, TomlFileConfigSource};
    use llamaforge_core::queue::admission::AdmissionController;
    use llamaforge_core::queue::pipeline::HealthTracker;
    use llamaforge_core::queue::queue::DispatchQueue;
    use llamaforge_core::routing::CapacityChecker;
    use llamaforge_core::types::{ModelAlias, ModelTarget, RequestId, RequestorClass, RequestorId};
    use std::sync::atomic::AtomicU32;

    struct MockCapacityChecker;
    impl CapacityChecker for MockCapacityChecker {
        fn serves_model(
            &self,
            t: &ModelTarget,
            c: &llamaforge_core::routing::RequestCapabilities,
        ) -> bool {
            self.has_idle_slot(t, c)
        }

        fn has_idle_slot(
            &self,

            _: &ModelTarget,

            _: &llamaforge_core::routing::RequestCapabilities,
        ) -> bool {
            true
        }
    }

    fn test_pipeline() -> Arc<RequestPipeline> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let toml = r#"
[global]
queue_max_depth = 1000
request_max_tokens = 4096
        "#;
        std::fs::write(tmp.path(), toml).unwrap();
        let paths = ConfigPaths::new(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
        );
        let source = Arc::new(TomlFileConfigSource::new(paths));
        let config = Arc::new(ConfigHandle::load(source).unwrap());

        let depth = Arc::new(AtomicU32::new(0));
        let admission = Arc::new(AdmissionController::new(
            config.clone(),
            depth.clone(),
            Arc::new(MockCapacityChecker),
        ));
        let queue = Arc::new(DispatchQueue::new(config.clone(), depth));
        let health = Arc::new(HealthTracker::default());
        *health.last_heartbeat.write().unwrap() = Some(tokio::time::Instant::now()); // simulate scheduler running

        Arc::new(RequestPipeline::new(admission, queue, health))
    }

    fn test_incoming() -> IncomingRequest {
        IncomingRequest {
            request_id: RequestId::new(uuid::Uuid::now_v7()),
            requestor_id: RequestorId::new(Arc::from("test-user")),
            requestor_class: RequestorClass::new(Arc::from("default")),
            model_target: ModelTarget::Exact(ModelAlias::new(Arc::from("test-model"))),
            capabilities: Default::default(),
            messages: vec![],
            params: Default::default(),
            stream: false,
            session_id: None,
            submitted_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_submit_and_cancel() {
        let pipeline = test_pipeline();
        let adapter = QueueSubmitterAdapter::new(pipeline.clone());

        let req = test_incoming();
        let request_id = req.request_id;

        let mut ticket = adapter.submit(req).expect("submit failed");
        assert_eq!(ticket.request_id, request_id);

        let status = adapter.status().expect("status failed");
        assert_eq!(status.depth, 1);

        adapter.cancel(&request_id).expect("cancel failed");

        let status2 = adapter.status().expect("status failed");
        assert_eq!(status2.depth, 0);

        // We should receive Failed event due to cancellation
        let ev = ticket.events.recv().await.expect("event stream closed");
        match ev {
            DispatchEvent::Queued { position } => assert_eq!(position, 1),
            _ => panic!("Expected Queued event first"),
        }

        let ev2 = ticket.events.recv().await.expect("event stream closed");
        match ev2 {
            DispatchEvent::Failed { error } => assert!(error.contains("cancelled")),
            _ => panic!("Expected Failed event"),
        }
    }

    #[tokio::test]
    async fn test_dropped_ticket_records_completion() {
        let pipeline = test_pipeline();
        let adapter = QueueSubmitterAdapter::new(pipeline.clone());

        let req = test_incoming();
        let requestor = req.requestor_id.clone();

        let ticket = adapter.submit(req).expect("submit failed");

        let usage = pipeline.requestor_usage(&requestor);
        assert_eq!(usage.0, 1); // 1 queued

        drop(ticket); // This should trigger cancellation / completion recording

        // Wait a bit for the background task to detect closed channel
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let usage2 = pipeline.requestor_usage(&requestor);
        assert_eq!(usage2.0, 0); // 0 queued
    }

    #[tokio::test]
    async fn test_queue_full_rejection() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let toml = r#"
[global]
queue_max_depth = 1
request_max_tokens = 4096
        "#;
        std::fs::write(tmp.path(), toml).unwrap();
        let paths = ConfigPaths::new(
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
        );
        let source = Arc::new(TomlFileConfigSource::new(paths));
        let config = Arc::new(ConfigHandle::load(source).unwrap());

        let depth = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let admission = Arc::new(AdmissionController::new(
            config.clone(),
            depth.clone(),
            Arc::new(MockCapacityChecker),
        ));
        let queue = Arc::new(DispatchQueue::new(config.clone(), depth));
        let health = Arc::new(HealthTracker::default());
        *health.last_heartbeat.write().unwrap() = Some(tokio::time::Instant::now());

        let pipeline2 = Arc::new(RequestPipeline::new(admission, queue, health));
        let adapter2 = QueueSubmitterAdapter::new(pipeline2);

        let req1 = test_incoming();
        let _ticket1 = adapter2.submit(req1).expect("first submit should succeed");

        let mut req2 = test_incoming();
        req2.request_id = RequestId::new(uuid::Uuid::now_v7());
        let res = adapter2.submit(req2);

        assert!(matches!(res, Err(AdmissionError::RateLimited { .. }))); // QueueFull maps to RateLimited
    }
}
