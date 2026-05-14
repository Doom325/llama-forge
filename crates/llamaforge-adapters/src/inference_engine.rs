use futures_core::Stream;
use llamaforge_transport::InferenceEngine;
use llamaforge_types::traits::DispatchError;
use llamaforge_types::{ChatCompletionChunk, ChatCompletionRequest, StreamError};
use std::pin::Pin;

#[derive(Default)]
pub struct InferenceEngineAdapter {}

impl InferenceEngineAdapter {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl InferenceEngine for InferenceEngineAdapter {
    async fn chat_completion(
        &self,
        _req: ChatCompletionRequest,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, StreamError>> + Send + 'static>>,
        DispatchError,
    > {
        unimplemented!("Implemented in PR6")
    }
}
