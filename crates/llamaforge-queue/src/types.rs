use chrono::{DateTime, Utc};
use llamaforge_types::{ModelAlias, ModelTarget, RequestId, RequestorClass, RequestorId};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub const PRIORITY_CEILING: u32 = 1500;
pub const SLIDING_WINDOW_MAX_ENTRIES: usize = 2000;

pub type InferencePayload = dyn std::any::Any + Send + Sync;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    Emergency,
    RealtimeChat,
    CodeReview,
    CodeGen,
    BatchAnalysis,
    Summarization,
    Background,
    Backfill,
}

impl fmt::Display for TaskType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Emergency => "emergency",
            Self::RealtimeChat => "realtime_chat",
            Self::CodeReview => "code_review",
            Self::CodeGen => "code_gen",
            Self::BatchAnalysis => "batch_analysis",
            Self::Summarization => "summarization",
            Self::Background => "background",
            Self::Backfill => "backfill",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TierLabel {
    Background,
    Standard,
    Realtime,
    Emergency,
}

impl TierLabel {
    pub fn from_priority(priority: u32) -> Self {
        match priority {
            0..=199 => Self::Background,
            200..=399 => Self::Standard,
            400..=599 => Self::Realtime,
            _ => Self::Emergency,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Background => "Background",
            Self::Standard => "Standard",
            Self::Realtime => "Realtime",
            Self::Emergency => "Emergency",
        }
    }
}

impl PartialOrd for TierLabel {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TierLabel {
    fn cmp(&self, other: &Self) -> Ordering {
        let val_self = match self {
            Self::Background => 0,
            Self::Standard => 1,
            Self::Realtime => 2,
            Self::Emergency => 3,
        };
        let val_other = match other {
            Self::Background => 0,
            Self::Standard => 1,
            Self::Realtime => 2,
            Self::Emergency => 3,
        };
        val_self.cmp(&val_other)
    }
}

impl fmt::Display for TierLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
pub struct IncomingRequest {
    pub id: RequestId,
    pub payload: Arc<InferencePayload>,
    pub task_type: TaskType,
    pub requestor: RequestorId,
    pub requestor_class: RequestorClass,
    pub model_target: ModelTarget,
    pub capabilities: llamaforge_routing::RequestCapabilities,
    pub session_id: Option<llamaforge_types::SessionId>,
    pub explicit_override: Option<u32>,
    pub response_tx: oneshot::Sender<DispatchResult>,
    pub cancel_token: CancellationToken,
    pub trace_context: Option<opentelemetry::Context>,
}

impl fmt::Debug for IncomingRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IncomingRequest")
            .field("id", &self.id)
            .field("task_type", &self.task_type)
            .field("requestor", &self.requestor)
            .field("requestor_class", &self.requestor_class)
            .field("model_target", &self.model_target)
            .field("capabilities", &self.capabilities)
            .field("session_id", &self.session_id)
            .field("explicit_override", &self.explicit_override)
            .finish()
    }
}

pub struct AdmittedRequest {
    pub id: RequestId,
    pub payload: Arc<InferencePayload>,
    pub task_type: TaskType,
    pub requestor: RequestorId,
    pub requestor_class: RequestorClass,
    pub model_target: ModelTarget,
    pub capabilities: llamaforge_routing::RequestCapabilities,
    pub session_id: Option<llamaforge_types::SessionId>,
    pub effective_priority: u32,
    pub base_priority: u32,
    pub requestor_bonus: i32,
    pub explicit_override: Option<u32>,
    pub enqueued_at: tokio::time::Instant,
    pub enqueued_at_utc: DateTime<Utc>,
    pub queue_timeout: Duration,
    pub response_tx: oneshot::Sender<DispatchResult>,
    pub cancel_token: CancellationToken,
    pub trace_context: Option<opentelemetry::Context>,
    pub request_span: Option<tracing::Span>,
}

impl AdmittedRequest {
    pub fn from_incoming(
        req: IncomingRequest,
        effective_priority: u32,
        base_priority: u32,
        requestor_bonus: i32,
        queue_timeout: Duration,
        request_span: Option<tracing::Span>,
    ) -> Self {
        Self {
            id: req.id,
            payload: req.payload,
            task_type: req.task_type,
            requestor: req.requestor,
            requestor_class: req.requestor_class,
            model_target: req.model_target,
            capabilities: req.capabilities,
            session_id: req.session_id,
            effective_priority,
            base_priority,
            requestor_bonus,
            explicit_override: req.explicit_override,
            enqueued_at: tokio::time::Instant::now(),
            enqueued_at_utc: Utc::now(),
            queue_timeout,
            response_tx: req.response_tx,
            cancel_token: req.cancel_token,
            trace_context: req.trace_context,
            request_span,
        }
    }
}

pub struct QueuedRequest {
    pub id: RequestId,
    pub payload: Arc<InferencePayload>,
    pub task_type: TaskType,
    pub requestor: RequestorId,
    pub requestor_class: RequestorClass,
    pub model_target: ModelTarget,
    pub capabilities: llamaforge_routing::RequestCapabilities,
    pub session_id: Option<llamaforge_types::SessionId>,
    pub current_effective_priority: u32,
    pub base_priority: u32,
    pub requestor_bonus: i32,
    pub explicit_override: Option<u32>,
    pub enqueued_at: tokio::time::Instant,
    pub enqueued_at_utc: DateTime<Utc>,
    pub queue_timeout: Duration,
    pub response_tx: oneshot::Sender<DispatchResult>,
    pub cancel_token: CancellationToken,
    pub trace_context: Option<opentelemetry::Context>,
    pub request_span: Option<tracing::Span>,
}

impl QueuedRequest {
    pub fn from_admitted(req: AdmittedRequest) -> Self {
        Self {
            id: req.id,
            payload: req.payload,
            task_type: req.task_type,
            requestor: req.requestor,
            requestor_class: req.requestor_class,
            model_target: req.model_target,
            capabilities: req.capabilities,
            session_id: req.session_id,
            current_effective_priority: req.effective_priority,
            base_priority: req.base_priority,
            requestor_bonus: req.requestor_bonus,
            explicit_override: req.explicit_override,
            enqueued_at: req.enqueued_at,
            enqueued_at_utc: req.enqueued_at_utc,
            queue_timeout: req.queue_timeout,
            response_tx: req.response_tx,
            cancel_token: req.cancel_token,
            trace_context: req.trace_context,
            request_span: req.request_span,
        }
    }
}

impl llamaforge_routing::RequestView for QueuedRequest {
    fn model_target(&self) -> &ModelTarget {
        &self.model_target
    }

    fn capabilities(&self) -> &llamaforge_routing::RequestCapabilities {
        let Self { capabilities, .. } = self;
        capabilities
    }

    fn session_id(&self) -> Option<&llamaforge_types::SessionId> {
        let Self { session_id, .. } = self;
        session_id.as_ref()
    }
}

impl fmt::Debug for QueuedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueuedRequest")
            .field("id", &self.id)
            .field("task_type", &self.task_type)
            .field("requestor", &self.requestor)
            .field("current_effective_priority", &self.current_effective_priority)
            .finish()
    }
}

#[derive(Debug)]
pub struct DispatchCandidate {
    pub request: QueuedRequest,
    pub dispatch_priority: u32,
    pub queue_wait: Duration,
}

pub enum DispatchResult {
    Dispatched(llamaforge_types::ServerId, Option<llamaforge_types::SlotId>),
    TimedOut { waited: Duration },
    Cancelled,
    ShuttingDown,
}

impl fmt::Debug for DispatchResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dispatched(server, slot) => {
                f.debug_tuple("Dispatched").field(server).field(slot).finish()
            }
            Self::TimedOut { waited } => {
                f.debug_struct("TimedOut").field("waited", waited).finish()
            }
            Self::Cancelled => write!(f, "Cancelled"),
            Self::ShuttingDown => write!(f, "ShuttingDown"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct QueuePosition {
    pub position: u32,
    pub estimated_wait_secs: Option<f64>,
    pub tier_label: TierLabel,
}

#[derive(Debug, Clone)]
pub struct QueueDepthInfo {
    pub total: u32,
    pub by_tier: HashMap<TierLabel, u32>,
    pub by_model: HashMap<ModelAlias, u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_tier_label_from_priority() {
        assert_eq!(TierLabel::from_priority(0), TierLabel::Background);
        assert_eq!(TierLabel::from_priority(199), TierLabel::Background);
        assert_eq!(TierLabel::from_priority(200), TierLabel::Standard);
        assert_eq!(TierLabel::from_priority(399), TierLabel::Standard);
        assert_eq!(TierLabel::from_priority(400), TierLabel::Realtime);
        assert_eq!(TierLabel::from_priority(599), TierLabel::Realtime);
        assert_eq!(TierLabel::from_priority(600), TierLabel::Emergency);
        assert_eq!(TierLabel::from_priority(1500), TierLabel::Emergency);
        assert_eq!(TierLabel::from_priority(2000), TierLabel::Emergency); // Above ceiling
    }

    #[test]
    fn test_tier_label_ordering() {
        assert!(TierLabel::Background < TierLabel::Standard);
        assert!(TierLabel::Standard < TierLabel::Realtime);
        assert!(TierLabel::Realtime < TierLabel::Emergency);
        assert!(TierLabel::Background < TierLabel::Emergency);
    }

    #[test]
    fn test_tier_label_display() {
        assert_eq!(format!("{}", TierLabel::Background), "Background");
        assert_eq!(format!("{}", TierLabel::Emergency), "Emergency");
    }

    #[test]
    fn test_admitted_request_from_incoming() {
        let (tx, _rx) = oneshot::channel();
        let cancel_token = CancellationToken::new();
        let payload: Arc<InferencePayload> = Arc::new(());
        let requestor = RequestorId::new(Arc::from("user1"));
        let requestor_class = RequestorClass::new(Arc::from("default"));
        let model_target = ModelTarget::Exact(ModelAlias::new(Arc::from("model1")));

        let incoming = IncomingRequest {
            id: RequestId::new(uuid::Uuid::now_v7()),
            payload: payload.clone(),
            task_type: TaskType::RealtimeChat,
            requestor: requestor.clone(),
            requestor_class: requestor_class.clone(),
            model_target: model_target.clone(),
            capabilities: Default::default(),
            session_id: None,
            explicit_override: None,
            response_tx: tx,
            cancel_token: cancel_token.clone(),
            trace_context: None,
        };

        let admitted =
            AdmittedRequest::from_incoming(incoming, 450, 400, 50, Duration::from_secs(60), None);

        assert_eq!(admitted.effective_priority, 450);
        assert_eq!(admitted.queue_timeout, Duration::from_secs(60));
        assert!(admitted.enqueued_at <= tokio::time::Instant::now());
    }

    #[test]
    fn test_dispatch_result_debug() {
        // Can't easily construct a Dispatched without mocking streams, but we can verify Cancelled.
        let result = DispatchResult::Cancelled;
        let debug_str = format!("{:?}", result);
        assert_eq!(debug_str, "Cancelled");

        let result = DispatchResult::TimedOut { waited: Duration::from_secs(5) };
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("TimedOut"));
        assert!(debug_str.contains("5s"));
    }
}
