use async_stream::stream;
use futures_core::Stream;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

use llamaforge_dispatch::dispatcher::{CompletionParams, DispatchCandidate, Dispatcher};
use llamaforge_metrics::MetricsIngest;
use llamaforge_routing::Router;
use llamaforge_transport::InferenceEngine;
use llamaforge_types::traits::{
    DispatchError, DispatchEvent, IncomingRequest, QueueSubmitter,
};
use llamaforge_types::{
    ChatCompletionChunk, ChatCompletionRequest, GenerationParams, ModelAlias, ModelTarget,
    RequestId, RequestorClass, RequestorId, StreamError,
};

pub struct InferenceEngineAdapter {
    queue: Arc<dyn QueueSubmitter>,
    #[allow(dead_code)]
    router: Arc<Router>,
    dispatcher: Arc<Dispatcher>,
    #[allow(dead_code)]
    metrics: Arc<MetricsIngest>,
}

impl InferenceEngineAdapter {
    pub fn new(
        queue: Arc<dyn QueueSubmitter>,
        router: Arc<Router>,
        dispatcher: Arc<Dispatcher>,
        metrics: Arc<MetricsIngest>,
    ) -> Self {
        Self {
            queue,
            router,
            dispatcher,
            metrics,
        }
    }
}

fn map_stream_error(e: llamaforge_dispatch::error::StreamError) -> StreamError {
    match e {
        llamaforge_dispatch::error::StreamError::BackendDisconnect => StreamError::BackendDisconnect,
        llamaforge_dispatch::error::StreamError::Timeout => StreamError::Timeout,
        llamaforge_dispatch::error::StreamError::Cancelled => StreamError::Cancelled,
        llamaforge_dispatch::error::StreamError::ChunkParse { reason } => StreamError::ChunkParse { reason },
        llamaforge_dispatch::error::StreamError::ChannelClosed => StreamError::Upstream(Box::new(e)),
        llamaforge_dispatch::error::StreamError::Internal(inner) => StreamError::Upstream(inner),
    }
}

#[async_trait::async_trait]
impl InferenceEngine for InferenceEngineAdapter {
    async fn chat_completion(
        &self,
        req: ChatCompletionRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>,
        DispatchError,
    > {
        let request_id = RequestId::new(uuid::Uuid::now_v7());
        let requestor_id = RequestorId::new(Arc::from("anonymous"));
        let requestor_class = RequestorClass::new(Arc::from("default"));

        let model_target = ModelTarget::Exact(ModelAlias::new(Arc::from(req.model.clone())));

        let params = GenerationParams {
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            top_p: None,
            top_k: None,
            logit_bias: req.logit_bias.clone(),
            stop: vec![],
            seed: None,
        };

        let incoming_req = IncomingRequest {
            request_id,
            requestor_id: requestor_id.clone(),
            requestor_class,
            model_target,
            capabilities: Default::default(),
            messages: req.messages.clone(),
            params: params.clone(),
            stream: req.stream,
            session_id: None,
            submitted_at: chrono::Utc::now(),
        };

        let mut ticket = match self.queue.submit(incoming_req) {
            Ok(t) => t,
            Err(_) => {
                return Err(DispatchError::Failed);
            }
        };

        let prompt = req
            .messages
            .iter()
            .map(|m| format!("{:?}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let completion_params = CompletionParams {
            n_predict: req.max_tokens,
            temperature: req.temperature.unwrap_or(1.0),
            stream: req.stream,
            stop: vec![],
        };

        let dispatcher = Arc::clone(&self.dispatcher);

        let s = stream! {
            let state = 0; // 0 = WaitingForDispatch, 1 = Streaming
            let mut _abort_handle = None;
            
            let start_time = tokio::time::Instant::now();
            let mut first_token = false;

            while state == 0 {
                tokio::select! {
                    _ = ticket.cancel_token.cancelled() => {
                        yield Err(StreamError::Cancelled);
                        return;
                    }
                    ev_opt = ticket.events.recv() => {
                        match ev_opt {
                            Some(DispatchEvent::Queued { position }) => {
                                yield Ok(ChatCompletionChunk::QueueUpdate { position });
                            }
                            Some(DispatchEvent::Dispatched { server_id, slot_id }) => {
                                let candidate = DispatchCandidate {
                                    request_id,
                                    prompt: prompt.clone(),
                                    params: completion_params.clone(),
                                    server_id,
                                    slot_target: Some(slot_id),
                                };

                                let (audit_tx, mut audit_rx) = mpsc::channel(32);
                                
                                tokio::spawn(async move {
                                    while audit_rx.recv().await.is_some() {}
                                });

                                let (abort, mut chunk_rx) = dispatcher.dispatch(candidate, audit_tx).await;
                                _abort_handle = Some(abort);

                                let wait_time = start_time.elapsed();
                                metrics::histogram!("llamaforge_queue_wait_seconds").record(wait_time.as_secs_f64());

                                while let Some(chunk_res) = chunk_rx.recv().await {
                                    if !first_token {
                                        first_token = true;
                                        metrics::histogram!("llamaforge_ttft_seconds").record(start_time.elapsed().as_secs_f64());
                                    }

                                    match chunk_res {
                                        Ok(parsed) => {
                                            if parsed.stop {
                                                metrics::counter!("llamaforge_tokens_total").increment(parsed.tokens_predicted as u64);
                                                yield Ok(ChatCompletionChunk::Stop(parsed.finish_reason.unwrap_or_default()));
                                            } else {
                                                yield Ok(ChatCompletionChunk::Token(parsed.text));
                                            }
                                        }
                                        Err(e) => {
                                            yield Err(map_stream_error(e));
                                            break;
                                        }
                                    }
                                }
                                break;
                            }
                            Some(DispatchEvent::Failed { error }) => {
                                yield Err(StreamError::Upstream(Box::new(std::io::Error::other(error))));
                                return;
                            }
                            Some(_) => {}
                            None => {
                                yield Err(StreamError::Upstream(Box::new(std::io::Error::other("ChannelClosed"))));
                                return;
                            }
                        }
                    }
                }
            }
        };

        Ok(Box::pin(s))
    }
}
