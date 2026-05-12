use llamaforge_types::{ModelAlias, RequestId, RequestorId};

#[derive(Debug, thiserror::Error)]
pub enum AdmissionError {
    #[error("queue full ({depth}/{max}), retry after {retry_after_secs}s")]
    QueueFull {
        depth: u32,
        max: u32,
        retry_after_secs: u32,
    },

    #[error("requestor '{requestor}' at concurrent limit ({current}/{max})")]
    RequestorLimit {
        requestor: RequestorId,
        current: u32,
        max: u32,
    },

    #[error("no server configured for model '{model}'")]
    NoServerForModel { model: ModelAlias },

    #[error("unauthorized task type '{task_type}' for requestor")]
    UnauthorizedTaskType { task_type: crate::types::TaskType },

    #[error("priority '{priority}' out of range for requestor")]
    PriorityOutOfRange { priority: u32 },

    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("request {id} not found in queue")]
    NotFound { id: RequestId },
}
