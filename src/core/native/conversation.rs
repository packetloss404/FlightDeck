//! Conversation model for the native agent.
//!
//! Analog of OpenCode's `MessageV2` — a sequence of messages whose content is
//! a list of typed parts (text, reasoning, tool calls, tool results). This
//! shape maps cleanly onto Anthropic's Messages API content blocks and is
//! generic enough for OpenAI/Google to slot in later.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
    Text { text: String },
    Reasoning { text: String },
    ToolCall(ToolCall),
    ToolResult(ToolResult),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-assigned id (e.g., Anthropic's `toolu_XYZ`).
    pub id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    pub fn accumulate(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub parts: Vec<MessagePart>,
    pub created_at: u64,
    #[serde(default)]
    pub usage: Option<TokenUsage>,
}

impl Message {
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            id: new_id("msg"),
            role: Role::User,
            parts: vec![MessagePart::Text { text: text.into() }],
            created_at: now_millis(),
            usage: None,
        }
    }

    pub fn assistant(parts: Vec<MessagePart>, usage: Option<TokenUsage>) -> Self {
        Self {
            id: new_id("msg"),
            role: Role::Assistant,
            parts,
            created_at: now_millis(),
            usage,
        }
    }

    pub fn tool_results(results: Vec<ToolResult>) -> Self {
        Self {
            id: new_id("msg"),
            role: Role::User,
            parts: results.into_iter().map(MessagePart::ToolResult).collect(),
            created_at: now_millis(),
            usage: None,
        }
    }

    /// True iff this message requests one or more tool calls.
    pub fn tool_calls(&self) -> Vec<&ToolCall> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::ToolCall(c) => Some(c),
                _ => None,
            })
            .collect()
    }

    /// Concatenated plain text of all text parts, separated by newlines.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        for part in &self.parts {
            if let MessagePart::Text { text } = part {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub system_prompt: Option<String>,
    pub messages: Vec<Message>,
    pub project_path: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub total_usage: TokenUsage,
}

impl Conversation {
    pub fn new(project_path: impl Into<String>) -> Self {
        let now = now_millis();
        Self {
            id: new_id("conv"),
            system_prompt: None,
            messages: Vec::new(),
            project_path: project_path.into(),
            created_at: now,
            updated_at: now,
            total_usage: TokenUsage::default(),
        }
    }

    pub fn push(&mut self, message: Message) {
        if let Some(usage) = &message.usage {
            self.total_usage.accumulate(usage);
        }
        self.messages.push(message);
        self.updated_at = now_millis();
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn new_id(prefix: &str) -> String {
    format!("{}_{}", prefix, uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_conversation_through_json() {
        let mut conv = Conversation::new("/tmp/x");
        conv.system_prompt = Some("be brief".into());
        conv.push(Message::user_text("hi"));
        conv.push(Message::assistant(
            vec![
                MessagePart::Text { text: "hello".into() },
                MessagePart::ToolCall(ToolCall {
                    id: "toolu_1".into(),
                    tool_name: "read".into(),
                    input: serde_json::json!({"path": "foo.md"}),
                }),
            ],
            Some(TokenUsage { input_tokens: 10, output_tokens: 20, ..Default::default() }),
        ));
        conv.push(Message::tool_results(vec![ToolResult {
            tool_use_id: "toolu_1".into(),
            content: "file body".into(),
            is_error: false,
        }]));

        let json = serde_json::to_string(&conv).unwrap();
        let back: Conversation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.messages.len(), 3);
        assert_eq!(back.total_usage.total(), 30);
        assert_eq!(back.messages[1].tool_calls().len(), 1);
    }

    #[test]
    fn text_content_joins_text_parts_only() {
        let m = Message::assistant(
            vec![
                MessagePart::Text { text: "a".into() },
                MessagePart::ToolCall(ToolCall {
                    id: "t".into(),
                    tool_name: "read".into(),
                    input: serde_json::Value::Null,
                }),
                MessagePart::Text { text: "b".into() },
            ],
            None,
        );
        assert_eq!(m.text_content(), "a\nb");
    }
}
