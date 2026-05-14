#![allow(dead_code)]
use crate::error::ApiError;
use llamaforge_types::{ChatCompletionChunk, ChatCompletionRequest, Message, Role};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub(crate) struct OpenAiMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
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
    pub tools: Option<serde_json::Value>,
    pub tool_choice: Option<serde_json::Value>,
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

    let mut messages = Vec::with_capacity(request.messages.len());
    for m in request.messages {
        let role = match m.role.as_str() {
            "system" => Role::System,
            "assistant" => Role::Assistant,
            _ => Role::User,
        };
        messages.push(Message { role, content: m.content });
    }

    Ok(ChatCompletionRequest { messages })
}

/// Formats an internal chunk into an OpenAI SSE format string.
///
/// Note: v2 could use `BytesMut` for zero-copy formatting.
pub(crate) fn format_openai_chunk(chunk: &ChatCompletionChunk) -> String {
    match chunk {
        ChatCompletionChunk::QueueUpdate { position } => {
            let update = serde_json::json!({
                "id": "chatcmpl-q",
                "choices": [{
                    "delta": {},
                    "index": 0
                }],
                "llamaforge": {
                    "status": "queued",
                    "position": position
                }
            });
            format!("data: {}\n\n", update)
        }
        ChatCompletionChunk::Token(token) => {
            let token_json = serde_json::json!({
                "id": "chatcmpl-t",
                "choices": [{
                    "delta": { "content": token },
                    "index": 0
                }]
            });
            format!("data: {}\n\n", token_json)
        }
        ChatCompletionChunk::Stop(finish_reason) => {
            let stop_json = serde_json::json!({
                "id": "chatcmpl-s",
                "choices": [{
                    "delta": {},
                    "index": 0,
                    "finish_reason": finish_reason
                }],
                "usage": null
            });
            format!("data: {}\n\ndata: [DONE]\n\n", stop_json)
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
    fn test_openai_chunk_formatting() {
        let queued = ChatCompletionChunk::QueueUpdate { position: 3 };
        let formatted_queued = format_openai_chunk(&queued);
        assert!(formatted_queued.contains("\"llamaforge\":{\"position\":3,\"status\":\"queued\"}"));

        let token = ChatCompletionChunk::Token("Hello".to_string());
        let formatted_token = format_openai_chunk(&token);
        assert!(formatted_token.contains("\"content\":\"Hello\""));

        let stop = ChatCompletionChunk::Stop("stop".to_string());
        let formatted_stop = format_openai_chunk(&stop);
        assert!(formatted_stop.contains("\"finish_reason\":\"stop\""));
        assert!(formatted_stop.ends_with("data: [DONE]\n\n"));
    }
}
