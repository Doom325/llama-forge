use super::error::ApiError;
use llamaforge_core::types::{ChatCompletionChunk, ChatCompletionRequest, Message, Role};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub(crate) struct OpenAiMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct OpenAiChatRequest {
    pub model: String,
    pub messages: Vec<OpenAiMessage>,
    #[serde(default, deserialize_with = "deserialize_null_as_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_null_as_none")]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, deserialize_with = "deserialize_logit_bias")]
    pub logit_bias: Option<HashMap<i32, f64>>,
    pub tools: Option<llamaforge_core::types::ToolSpec>,
    pub tool_choice: Option<llamaforge_core::types::ToolSpec>,
    pub function_call: Option<serde_json::Value>,
}

/// Custom deserializer that parses explicit `null` as `None` rather than erroring.
/// See design doc §3.4 invariant 3.
pub(crate) fn deserialize_null_as_none<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let opt: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match opt {
        Some(serde_json::Value::Null) => Ok(None),
        Some(v) => T::deserialize(v).map(Some).map_err(serde::de::Error::custom),
        None => Ok(None),
    }
}

/// Custom deserializer for `logit_bias` that ensures string keys are parsed as `i32`
/// so that negative token IDs (like BOS/EOS) are preserved.
/// See design doc §3.4 invariant 4.
pub(crate) fn deserialize_logit_bias<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<i32, f64>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<HashMap<String, f64>> = Option::deserialize(deserializer)?;
    match opt {
        Some(map) => {
            let mut result = HashMap::new();
            for (k, v) in map {
                let parsed_key = k.parse::<i32>().map_err(serde::de::Error::custom)?;
                result.insert(parsed_key, v);
            }
            Ok(Some(result))
        }
        None => Ok(None),
    }
}

/// Parses an OpenAI chat request into the internal domain type.
///
/// **Spawn Blocking Contract**: For bodies > 1MB, this function MUST be called
/// via `tokio::task::spawn_blocking` to avoid stalling the async reactor. If
/// `Content-Length` is missing, assume it is large.
pub(crate) fn parse_openai_request(
    request: OpenAiChatRequest,
) -> Result<ChatCompletionRequest, ApiError> {
    if request.function_call.is_some() {
        return Err(ApiError::InvalidBody(
            "function_call is deprecated, please use tool_calls instead".to_string(),
        ));
    }
    if request.messages.is_empty() {
        return Err(ApiError::InvalidBody("messages must contain at least one entry".to_string()));
    }

    if let Some(temp) = request.temperature {
        if !(0.0..=2.0).contains(&temp) {
            return Err(ApiError::InvalidBody(format!(
                "temperature must be between 0.0 and 2.0, got {}",
                temp
            )));
        }
    }
    if let Some(max) = request.max_tokens {
        if max == 0 || max > 131072 {
            return Err(ApiError::InvalidBody(format!(
                "max_tokens must be between 1 and 131072, got {}",
                max
            )));
        }
    }

    let mut messages = Vec::with_capacity(request.messages.len());
    for m in request.messages {
        let role = match m.role.as_str() {
            "system" => Role::System,
            "assistant" => Role::Assistant,
            "user" => Role::User,
            unknown => {
                return Err(ApiError::InvalidBody(format!(
                    "unsupported role: '{}'; expected system, assistant, or user",
                    unknown
                )))
            }
        };
        messages.push(Message { role, content: m.content });
    }

    Ok(ChatCompletionRequest {
        model: request.model,
        messages,
        max_tokens: request.max_tokens,
        temperature: request.temperature,
        stream: request.stream,
        logit_bias: request.logit_bias,
        tools: request.tools,
        tool_choice: request.tool_choice,
    })
}

/// Render an internal `ChatCompletionChunk` as one or more SSE event
/// payloads. Each returned string is the *data* portion of an SSE event
/// (no `data: ` prefix, no trailing blank line) and is meant to be
/// handed to `axum::response::sse::Event::default().data(...)`, which
/// adds the framing itself. Stop chunks expand to two events: the
/// terminal completion chunk and the OpenAI-canonical `[DONE]`
/// sentinel.
pub(crate) fn format_openai_chunk(
    id: &str,
    model: &str,
    chunk: &ChatCompletionChunk,
) -> Vec<String> {
    let now = chrono::Utc::now().timestamp();
    match chunk {
        ChatCompletionChunk::QueueUpdate { position } => {
            let update = serde_json::json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": now,
                "model": model,
                "choices": [{
                    "delta": {},
                    "index": 0
                }],
                "llamaforge": {
                    "status": "queued",
                    "position": position
                }
            });
            vec![update.to_string()]
        }
        ChatCompletionChunk::Token(token) => {
            let token_json = serde_json::json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": now,
                "model": model,
                "choices": [{
                    "delta": { "content": token },
                    "index": 0
                }]
            });
            vec![token_json.to_string()]
        }
        ChatCompletionChunk::Stop { finish_reason, output_tokens, prompt_tokens } => {
            let stop_json = serde_json::json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": now,
                "model": model,
                "choices": [{
                    "delta": {},
                    "index": 0,
                    "finish_reason": finish_reason
                }],
                "usage": {
                    "prompt_tokens": prompt_tokens,
                    "completion_tokens": output_tokens,
                    "total_tokens": prompt_tokens.saturating_add(*output_tokens),
                }
            });
            vec![stop_json.to_string(), "[DONE]".to_string()]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_openai_null_max_tokens() {
        let json_body = json!({
            "model": "llama",
            "messages": [{"role": "user", "content": "hello"}],
            "max_tokens": null
        });
        let req: OpenAiChatRequest = serde_json::from_value(json_body).unwrap();
        assert_eq!(req.max_tokens, None);
    }

    #[test]
    fn test_openai_logit_bias_i32() {
        let json_body = json!({
            "model": "llama",
            "messages": [{"role": "user", "content": "hello"}],
            "logit_bias": {
                "-1": 5.0,
                "100": -2.0
            }
        });
        let req: OpenAiChatRequest = serde_json::from_value(json_body).unwrap();
        let bias = req.logit_bias.unwrap();
        assert_eq!(bias.get(&-1), Some(&5.0));
        assert_eq!(bias.get(&100), Some(&-2.0));
    }

    #[test]
    fn test_openai_function_call_rejection() {
        let req = OpenAiChatRequest {
            model: "llama".to_string(),
            messages: vec![],
            max_tokens: None,
            temperature: None,
            stream: false,
            logit_bias: None,
            tools: None,
            tool_choice: None,
            function_call: Some(json!({"name": "foo"})),
        };
        let result = parse_openai_request(req);
        assert!(matches!(result, Err(ApiError::InvalidBody(_))));
    }

    #[test]
    fn test_openai_empty_messages_rejected() {
        // Both OpenAI and Anthropic require at least one message;
        // empty arrays must surface as `InvalidBody` (400) rather
        // than being silently admitted.
        let req = OpenAiChatRequest {
            model: "llama".to_string(),
            messages: vec![],
            max_tokens: None,
            temperature: None,
            stream: false,
            logit_bias: None,
            tools: None,
            tool_choice: None,
            function_call: None,
        };
        let result = parse_openai_request(req);
        assert!(
            matches!(result, Err(ApiError::InvalidBody(ref msg)) if msg.contains("at least one")),
            "empty messages must surface as InvalidBody, got {:?}",
            result
        );
    }

    #[test]
    fn test_openai_chunk_formatting() {
        let model = "test-model";
        let queued = ChatCompletionChunk::QueueUpdate { position: 3 };
        let formatted_queued = format_openai_chunk("req_123", model, &queued);
        assert_eq!(formatted_queued.len(), 1);
        assert!(
            formatted_queued[0].contains("\"llamaforge\":{\"position\":3,\"status\":\"queued\"}")
        );
        assert!(formatted_queued[0].contains("\"id\":\"req_123\""));
        assert!(formatted_queued[0].contains("\"object\":\"chat.completion.chunk\""));
        assert!(formatted_queued[0].contains("\"model\":\"test-model\""));
        assert!(formatted_queued[0].contains("\"created\":"));

        let token = ChatCompletionChunk::Token("Hello".to_string());
        let formatted_token = format_openai_chunk("req_123", model, &token);
        assert_eq!(formatted_token.len(), 1);
        assert!(formatted_token[0].contains("\"content\":\"Hello\""));
        assert!(formatted_token[0].contains("\"id\":\"req_123\""));
        assert!(formatted_token[0].contains("\"object\":\"chat.completion.chunk\""));
        assert!(formatted_token[0].contains("\"model\":\"test-model\""));

        let stop = ChatCompletionChunk::Stop {
            finish_reason: "stop".to_string(),
            output_tokens: 10,
            prompt_tokens: 5,
        };
        let formatted_stop = format_openai_chunk("req_123", model, &stop);
        assert_eq!(formatted_stop.len(), 2);
        assert!(formatted_stop[0].contains("\"finish_reason\":\"stop\""));
        assert!(formatted_stop[0].contains("\"id\":\"req_123\""));
        assert!(formatted_stop[0].contains("\"object\":\"chat.completion.chunk\""));
        assert!(formatted_stop[0].contains("\"model\":\"test-model\""));
        assert_eq!(formatted_stop[1], "[DONE]");
    }

    #[test]
    fn test_openai_unknown_role_rejected() {
        let req = OpenAiChatRequest {
            model: "llama".to_string(),
            messages: vec![OpenAiMessage { role: "tool".to_string(), content: "foo".to_string() }],
            max_tokens: None,
            temperature: None,
            stream: false,
            logit_bias: None,
            tools: None,
            tool_choice: None,
            function_call: None,
        };
        let result = parse_openai_request(req);
        assert!(
            matches!(result, Err(ApiError::InvalidBody(ref msg)) if msg.contains("unsupported role")),
            "unknown role must surface as InvalidBody, got {:?}",
            result
        );
    }

    #[test]
    fn test_openai_out_of_range_rejected() {
        let mut req = OpenAiChatRequest {
            model: "llama".to_string(),
            messages: vec![OpenAiMessage { role: "user".to_string(), content: "foo".to_string() }],
            max_tokens: Some(0),
            temperature: None,
            stream: false,
            logit_bias: None,
            tools: None,
            tool_choice: None,
            function_call: None,
        };
        let result = parse_openai_request(req.clone());
        assert!(
            matches!(result, Err(ApiError::InvalidBody(ref msg)) if msg.contains("max_tokens must be between 1 and 131072"))
        );

        req.max_tokens = Some(200000);
        let result = parse_openai_request(req.clone());
        assert!(
            matches!(result, Err(ApiError::InvalidBody(ref msg)) if msg.contains("max_tokens must be between 1 and 131072"))
        );

        req.max_tokens = Some(100);
        req.temperature = Some(-0.1);
        let result = parse_openai_request(req.clone());
        assert!(
            matches!(result, Err(ApiError::InvalidBody(ref msg)) if msg.contains("temperature must be between 0.0 and 2.0"))
        );

        req.temperature = Some(2.1);
        let result = parse_openai_request(req);
        assert!(
            matches!(result, Err(ApiError::InvalidBody(ref msg)) if msg.contains("temperature must be between 0.0 and 2.0"))
        );
    }
}
