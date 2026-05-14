use crate::error::{AnthropicProtocolError, AnthropicStreamPhase, ApiError};
use llamaforge_types::{ChatCompletionChunk, ChatCompletionRequest, Message, Role};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub(crate) struct AnthropicMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicMessagesRequest {
    pub model: String,
    pub messages: Vec<AnthropicMessage>,
    pub system: Option<String>,
    pub max_tokens: u32,
    #[serde(default)]
    pub stream: bool,
    pub tools: Option<serde_json::Value>,
    pub tool_choice: Option<serde_json::Value>,
}

/// Parses an Anthropic messages request into the internal domain type.
///
/// **Spawn Blocking Contract**: For bodies > 1MB, this function MUST be called
/// via `tokio::task::spawn_blocking` to avoid stalling the async reactor. If
/// `Content-Length` is missing, assume it is large.
pub(crate) fn parse_anthropic_request(
    request: AnthropicMessagesRequest,
) -> Result<ChatCompletionRequest, ApiError> {
    let mut messages = Vec::with_capacity(request.messages.len() + 1);

    if let Some(system_prompt) = request.system {
        messages.push(Message { role: Role::System, content: system_prompt });
    }

    for m in request.messages {
        let role = match m.role.as_str() {
            "assistant" => Role::Assistant,
            _ => Role::User,
        };
        messages.push(Message { role, content: m.content });
    }

    Ok(ChatCompletionRequest {
        model: request.model,
        messages,
        max_tokens: Some(request.max_tokens),
        temperature: None,
        stream: request.stream,
        logit_bias: None,
        tools: request.tools,
        tool_choice: request.tool_choice,
    })
}

/// Anthropic SSE state machine.
/// See design doc §3.2.
#[derive(Debug)]
pub(crate) struct AnthropicState {
    pub phase: AnthropicStreamPhase,
    pub block_index: u32,
}

impl AnthropicState {
    pub fn new() -> Self {
        Self { phase: AnthropicStreamPhase::Init, block_index: 0 }
    }

    pub fn transition(&mut self, to: AnthropicStreamPhase) -> Result<(), AnthropicProtocolError> {
        let valid = matches!(
            (self.phase, to),
            (AnthropicStreamPhase::Init, AnthropicStreamPhase::PreambleSent)
                | (AnthropicStreamPhase::PreambleSent, AnthropicStreamPhase::ContentBlock)
                | (AnthropicStreamPhase::ContentBlock, AnthropicStreamPhase::ContentBlock)
                | (AnthropicStreamPhase::ContentBlock, AnthropicStreamPhase::BlockStopped)
                | (AnthropicStreamPhase::BlockStopped, AnthropicStreamPhase::MessageDone)
                | (AnthropicStreamPhase::MessageDone, AnthropicStreamPhase::Complete)
        );

        if !valid {
            return Err(AnthropicProtocolError::InvalidTransition { from: self.phase, to });
        }

        self.phase = to;
        Ok(())
    }
}

pub(crate) struct SseEvent {
    pub event_type: &'static str,
    pub data: String,
}

impl From<SseEvent> for axum::response::sse::Event {
    fn from(event: SseEvent) -> Self {
        axum::response::sse::Event::default().event(event.event_type).data(event.data)
    }
}

/// Formats an internal chunk into Anthropic SSE events.
///
/// Ensures strict state machine transitions per design doc G-11.
pub(crate) fn format_anthropic_event(
    id: &str,
    model: &str,
    chunk: &ChatCompletionChunk,
    state: &mut AnthropicState,
) -> Result<Vec<SseEvent>, AnthropicProtocolError> {
    let mut events = Vec::new();

    match chunk {
        ChatCompletionChunk::QueueUpdate { position: _ } => {
            events.push(SseEvent {
                event_type: "ping",
                data: "{}".to_string(), // Anthropic uses ping for keepalives
            });
            return Ok(events);
        }
        ChatCompletionChunk::Token(token) => {
            if state.phase == AnthropicStreamPhase::Init {
                state.transition(AnthropicStreamPhase::PreambleSent)?;
                events.push(SseEvent {
                    event_type: "message_start",
                    data: serde_json::json!({
                        "type": "message_start",
                        "message": {
                            "id": id,
                            "type": "message",
                            "role": "assistant",
                            "content": [],
                            "model": model,
                            "stop_reason": null,
                            "stop_sequence": null,
                            "usage": {"input_tokens": 0, "output_tokens": 0}
                        }
                    })
                    .to_string(),
                });

                state.transition(AnthropicStreamPhase::ContentBlock)?;
                events.push(SseEvent {
                    event_type: "content_block_start",
                    data: serde_json::json!({
                        "type": "content_block_start",
                        "index": state.block_index,
                        "content_block": {
                            "type": "text",
                            "text": ""
                        }
                    })
                    .to_string(),
                });
            } else if state.phase != AnthropicStreamPhase::ContentBlock {
                state.transition(AnthropicStreamPhase::ContentBlock)?;
                events.push(SseEvent {
                    event_type: "content_block_start",
                    data: serde_json::json!({
                        "type": "content_block_start",
                        "index": state.block_index,
                        "content_block": {
                            "type": "text",
                            "text": ""
                        }
                    })
                    .to_string(),
                });
            }

            events.push(SseEvent {
                event_type: "content_block_delta",
                data: serde_json::json!({
                    "type": "content_block_delta",
                    "index": state.block_index,
                    "delta": {
                        "type": "text_delta",
                        "text": token
                    }
                })
                .to_string(),
            });
        }
        ChatCompletionChunk::Stop(finish_reason) => {
            if state.phase == AnthropicStreamPhase::Init {
                // Handle case where stream ended before any tokens were sent
                state.transition(AnthropicStreamPhase::PreambleSent)?;
                events.push(SseEvent {
                    event_type: "message_start",
                    data: serde_json::json!({
                        "type": "message_start",
                        "message": {
                            "id": id,
                            "type": "message",
                            "role": "assistant",
                            "content": [],
                            "model": model,
                            "stop_reason": null,
                            "stop_sequence": null,
                            "usage": {"input_tokens": 0, "output_tokens": 0}
                        }
                    })
                    .to_string(),
                });

                state.transition(AnthropicStreamPhase::ContentBlock)?;
                events.push(SseEvent {
                    event_type: "content_block_start",
                    data: serde_json::json!({
                        "type": "content_block_start",
                        "index": state.block_index,
                        "content_block": {
                            "type": "text",
                            "text": ""
                        }
                    })
                    .to_string(),
                });
            }

            state.transition(AnthropicStreamPhase::BlockStopped)?;
            events.push(SseEvent {
                event_type: "content_block_stop",
                data: serde_json::json!({
                    "type": "content_block_stop",
                    "index": state.block_index
                })
                .to_string(),
            });

            state.transition(AnthropicStreamPhase::MessageDone)?;
            events.push(SseEvent {
                event_type: "message_delta",
                data: serde_json::json!({
                    "type": "message_delta",
                    "delta": {
                        "stop_reason": finish_reason,
                        "stop_sequence": null
                    },
                    "usage": {"output_tokens": 0} // Placeholder for v1 as true usage requires Engine support
                })
                .to_string(),
            });

            events.push(SseEvent {
                event_type: "message_stop",
                data: serde_json::json!({
                    "type": "message_stop"
                })
                .to_string(),
            });

            state.transition(AnthropicStreamPhase::Complete)?;
        }
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_system_prompt_extraction() {
        let req = AnthropicMessagesRequest {
            model: "claude".to_string(),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            system: Some("You are a helpful assistant.".to_string()),
            max_tokens: 100,
            stream: false,
            tools: None,
            tool_choice: None,
        };

        let result = parse_anthropic_request(req).unwrap();
        assert_eq!(result.messages.len(), 2);
        assert_eq!(result.messages[0].role, Role::System);
        assert_eq!(result.messages[0].content, "You are a helpful assistant.");
        assert_eq!(result.messages[1].role, Role::User);
        assert_eq!(result.messages[1].content, "hello");
    }

    #[test]
    fn test_anthropic_state_machine_valid_paths() {
        let mut state = AnthropicState::new();
        assert!(state.transition(AnthropicStreamPhase::PreambleSent).is_ok());
        assert!(state.transition(AnthropicStreamPhase::ContentBlock).is_ok());
        assert!(state.transition(AnthropicStreamPhase::ContentBlock).is_ok()); // multiple tokens
        assert!(state.transition(AnthropicStreamPhase::BlockStopped).is_ok());
        assert!(state.transition(AnthropicStreamPhase::MessageDone).is_ok());
        assert!(state.transition(AnthropicStreamPhase::Complete).is_ok());
    }

    #[test]
    fn test_anthropic_state_machine_invalid_transition() {
        let mut state = AnthropicState::new();
        // Trying to skip PreambleSent
        let result = state.transition(AnthropicStreamPhase::ContentBlock);
        assert!(matches!(result, Err(AnthropicProtocolError::InvalidTransition { .. })));
    }

    #[test]
    fn test_anthropic_event_ordering() {
        let mut state = AnthropicState::new();
        let token_chunk = ChatCompletionChunk::Token("Hi".to_string());

        let events = format_anthropic_event("req_123", "claude", &token_chunk, &mut state).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, "message_start");
        assert_eq!(events[1].event_type, "content_block_start");
        assert_eq!(events[2].event_type, "content_block_delta");

        let events2 = format_anthropic_event("req_123", "claude", &token_chunk, &mut state).unwrap();
        assert_eq!(events2.len(), 1);
        assert_eq!(events2[0].event_type, "content_block_delta");

        let stop_chunk = ChatCompletionChunk::Stop("end_turn".to_string());
        let events_stop = format_anthropic_event("req_123", "claude", &stop_chunk, &mut state).unwrap();
        assert_eq!(events_stop.len(), 3);
        assert_eq!(events_stop[0].event_type, "content_block_stop");
        assert_eq!(events_stop[1].event_type, "message_delta");
        assert_eq!(events_stop[2].event_type, "message_stop");
    }

    #[test]
    fn test_anthropic_no_token_before_preamble() {
        let mut state = AnthropicState::new();
        state.phase = AnthropicStreamPhase::PreambleSent;
        let token_chunk = ChatCompletionChunk::Token("x".to_string());
        let res = format_anthropic_event("req_123", "claude", &token_chunk, &mut state).unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].event_type, "content_block_start");
        assert_eq!(res[1].event_type, "content_block_delta");

        let mut state = AnthropicState::new();
        state.phase = AnthropicStreamPhase::BlockStopped;
        let res = format_anthropic_event("req_123", "claude", &token_chunk, &mut state);
        assert!(matches!(res, Err(AnthropicProtocolError::InvalidTransition { .. })));
    }}
