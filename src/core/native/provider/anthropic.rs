//! Anthropic Messages API client with SSE streaming.
//!
//! Normalizes Anthropic's content-block event stream into the shared
//! `ProviderEvent` sequence consumed by the native runner.

use async_trait::async_trait;
use futures::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use super::super::conversation::{Message, MessagePart, Role, TokenUsage};
use super::{Provider, ProviderError, ProviderEvent, ProviderRequest, StopReason, ToolSchema};

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    api_key: Zeroizing<String>,
    base_url: String,
    http: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(api_key: Zeroizing<String>, base_url: Option<String>) -> Self {
        Self {
            api_key,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("reqwest client build"),
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn stream(
        &self,
        req: ProviderRequest,
        events: mpsc::UnboundedSender<ProviderEvent>,
    ) -> Result<(), ProviderError> {
        let body = build_request_body(&req);

        let resp = self
            .http
            .post(self.endpoint())
            .header("x-api-key", self.api_key.as_str())
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(ProviderError::Auth(text));
            }
            return Err(ProviderError::Api {
                status: status.as_u16(),
                body: text,
            });
        }

        let mut stream = resp.bytes_stream();
        let mut buf = Vec::<u8>::new();
        let mut state = StreamState::new();

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| ProviderError::Network(e.to_string()))?;
            buf.extend_from_slice(&bytes);

            while let Some((frame, rest)) = split_sse_frame(&buf) {
                let frame_owned = frame.to_vec();
                buf = rest.to_vec();
                if let Some((event_name, data)) = parse_sse_frame(&frame_owned) {
                    handle_event(&mut state, &event_name, &data, &events)?;
                }
            }
        }

        Ok(())
    }
}

// ---------- Request body ----------

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    stream: bool,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: &'static str,
    content: Vec<Value>,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: Value,
}

fn build_request_body(req: &ProviderRequest) -> AnthropicRequest {
    AnthropicRequest {
        model: req.model.clone(),
        max_tokens: req.max_tokens,
        stream: true,
        system: req.system_prompt.clone(),
        messages: req.messages.iter().map(message_to_anthropic).collect(),
        tools: req.tools.iter().map(tool_to_anthropic).collect(),
    }
}

fn message_to_anthropic(msg: &Message) -> AnthropicMessage {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        // System messages are hoisted into the top-level `system` field; we
        // shouldn't see them here, but if we do, collapse to user.
        Role::System => "user",
    };
    let content: Vec<Value> = msg
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text } => Some(json!({"type": "text", "text": text})),
            MessagePart::Reasoning { .. } => None, // reasoning is output-only
            MessagePart::ToolCall(call) => Some(json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.tool_name,
                "input": call.input,
            })),
            MessagePart::ToolResult(result) => Some(json!({
                "type": "tool_result",
                "tool_use_id": result.tool_use_id,
                "content": result.content,
                "is_error": result.is_error,
            })),
        })
        .collect();
    AnthropicMessage { role, content }
}

fn tool_to_anthropic(tool: &ToolSchema) -> AnthropicTool {
    AnthropicTool {
        name: tool.name.clone(),
        description: tool.description.clone(),
        input_schema: tool.input_schema.clone(),
    }
}

// ---------- SSE parsing ----------

/// Split the buffer on the first `\n\n` or `\r\n\r\n` delimiter. Returns
/// `(frame, remainder)` where `frame` excludes the delimiter and `remainder`
/// is everything after it. Returns `None` if no complete frame is present yet.
fn split_sse_frame(buf: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut i = 0;
    while i + 1 < buf.len() {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some((&buf[..i], &buf[i + 2..]));
        }
        if i + 3 < buf.len()
            && buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some((&buf[..i], &buf[i + 4..]));
        }
        i += 1;
    }
    None
}

/// Parse an SSE frame into `(event_name, data_payload)`. Returns `None` for
/// ping/comment-only frames.
fn parse_sse_frame(frame: &[u8]) -> Option<(String, String)> {
    let text = std::str::from_utf8(frame).ok()?;
    let mut event = None;
    let mut data_lines = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }
    if data_lines.is_empty() {
        return None;
    }
    Some((event.unwrap_or_default(), data_lines.join("\n")))
}

// ---------- Stream state ----------

#[derive(Debug, Default)]
struct StreamState {
    /// Index-keyed partial tool-call inputs (JSON strings assembled from deltas).
    tool_inputs: std::collections::HashMap<u64, ToolInputBuf>,
    /// Tool-call id per index, set on content_block_start for a tool_use block.
    tool_ids: std::collections::HashMap<u64, String>,
    /// Tool names per index.
    tool_names: std::collections::HashMap<u64, String>,
    /// Final usage.
    usage: TokenUsage,
    /// Stop reason resolved by message_delta.
    stop_reason: Option<StopReason>,
}

#[derive(Debug, Default)]
struct ToolInputBuf {
    partial: String,
}

impl StreamState {
    fn new() -> Self {
        Self::default()
    }
}

fn handle_event(
    state: &mut StreamState,
    event_name: &str,
    data: &str,
    events: &mpsc::UnboundedSender<ProviderEvent>,
) -> Result<(), ProviderError> {
    if data == "[DONE]" {
        return Ok(());
    }

    let value: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Ok(()), // ignore malformed pings etc.
    };

    let kind = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or(event_name);

    match kind {
        "message_start" => {
            if let Some(usage) = value.get("message").and_then(|m| m.get("usage")) {
                merge_usage(&mut state.usage, usage);
            }
        }
        "content_block_start" => {
            let idx = value.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            if let Some(block) = value.get("content_block") {
                let ty = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match ty {
                    "tool_use" => {
                        let id = block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        state.tool_ids.insert(idx, id.clone());
                        state.tool_names.insert(idx, name.clone());
                        state.tool_inputs.insert(idx, ToolInputBuf::default());
                        let _ = events.send(ProviderEvent::ToolCallStart { id, name });
                    }
                    _ => {}
                }
            }
        }
        "content_block_delta" => {
            let idx = value.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            if let Some(delta) = value.get("delta") {
                let dty = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match dty {
                    "text_delta" => {
                        if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                            let _ = events.send(ProviderEvent::TextDelta { text: text.to_string() });
                        }
                    }
                    "thinking_delta" => {
                        if let Some(text) = delta.get("thinking").and_then(|v| v.as_str()) {
                            let _ = events.send(ProviderEvent::ReasoningDelta { text: text.to_string() });
                        }
                    }
                    "input_json_delta" => {
                        if let Some(partial) = delta.get("partial_json").and_then(|v| v.as_str()) {
                            if let Some(buf) = state.tool_inputs.get_mut(&idx) {
                                buf.partial.push_str(partial);
                            }
                            if let Some(id) = state.tool_ids.get(&idx) {
                                let _ = events.send(ProviderEvent::ToolCallInputDelta {
                                    id: id.clone(),
                                    partial_json: partial.to_string(),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        "content_block_stop" => {
            let idx = value.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            if let (Some(id), Some(buf)) =
                (state.tool_ids.remove(&idx), state.tool_inputs.remove(&idx))
            {
                let input: Value = if buf.partial.is_empty() {
                    Value::Object(Default::default())
                } else {
                    serde_json::from_str(&buf.partial).unwrap_or(Value::Null)
                };
                state.tool_names.remove(&idx);
                let _ = events.send(ProviderEvent::ToolCallEnd { id, input });
            }
        }
        "message_delta" => {
            if let Some(delta) = value.get("delta") {
                if let Some(sr) = delta.get("stop_reason").and_then(|v| v.as_str()) {
                    state.stop_reason = Some(match sr {
                        "end_turn" => StopReason::EndTurn,
                        "tool_use" => StopReason::ToolUse,
                        "max_tokens" => StopReason::MaxTokens,
                        "stop_sequence" => StopReason::StopSequence,
                        _ => StopReason::Other,
                    });
                }
            }
            if let Some(usage) = value.get("usage") {
                merge_usage(&mut state.usage, usage);
            }
        }
        "message_stop" => {
            let reason = state.stop_reason.unwrap_or(StopReason::EndTurn);
            let _ = events.send(ProviderEvent::Stop {
                reason,
                usage: std::mem::take(&mut state.usage),
            });
        }
        "error" => {
            let message = value
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown provider error");
            return Err(ProviderError::InvalidStream(message.to_string()));
        }
        _ => {}
    }

    Ok(())
}

fn merge_usage(dst: &mut TokenUsage, src: &Value) {
    if let Some(n) = src.get("input_tokens").and_then(|v| v.as_u64()) {
        dst.input_tokens += n;
    }
    if let Some(n) = src.get("output_tokens").and_then(|v| v.as_u64()) {
        dst.output_tokens += n;
    }
    if let Some(n) = src.get("cache_creation_input_tokens").and_then(|v| v.as_u64()) {
        dst.cache_creation_input_tokens += n;
    }
    if let Some(n) = src.get("cache_read_input_tokens").and_then(|v| v.as_u64()) {
        dst.cache_read_input_tokens += n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_sse_frame_splits_on_double_newline() {
        let buf = b"event: test\ndata: {}\n\nevent: two\ndata: {}\n\n";
        let (frame, rest) = split_sse_frame(buf).unwrap();
        assert_eq!(frame, b"event: test\ndata: {}");
        assert!(rest.starts_with(b"event: two"));
    }

    #[test]
    fn parse_sse_frame_extracts_event_and_data() {
        let frame = b"event: message_start\ndata: {\"type\":\"message_start\"}";
        let (event, data) = parse_sse_frame(frame).unwrap();
        assert_eq!(event, "message_start");
        assert!(data.starts_with("{\"type\":\"message_start\""));
    }

    #[test]
    fn parse_sse_frame_handles_multiline_data() {
        let frame = b"event: x\ndata: part1\ndata: part2";
        let (_, data) = parse_sse_frame(frame).unwrap();
        assert_eq!(data, "part1\npart2");
    }

    #[tokio::test]
    async fn handle_event_emits_text_delta() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = StreamState::new();
        let frame = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#;
        handle_event(&mut state, "content_block_delta", frame, &tx).unwrap();
        match rx.recv().await {
            Some(ProviderEvent::TextDelta { text }) => assert_eq!(text, "hi"),
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn handle_event_emits_tool_call_lifecycle() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = StreamState::new();
        handle_event(
            &mut state,
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"read","input":{}}}"#,
            &tx,
        )
        .unwrap();
        handle_event(
            &mut state,
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a\"}"}}"#,
            &tx,
        )
        .unwrap();
        handle_event(
            &mut state,
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
            &tx,
        )
        .unwrap();

        let start = rx.recv().await.unwrap();
        matches!(start, ProviderEvent::ToolCallStart { .. });
        let _ = rx.recv().await.unwrap(); // input delta
        match rx.recv().await.unwrap() {
            ProviderEvent::ToolCallEnd { id, input } => {
                assert_eq!(id, "toolu_1");
                assert_eq!(input.get("path").and_then(|v| v.as_str()), Some("a"));
            }
            other => panic!("unexpected: {:?}", other),
        }
    }
}
