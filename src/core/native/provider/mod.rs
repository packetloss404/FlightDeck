//! Provider abstraction: LLM clients that send messages and stream back
//! model output. Each provider implementation normalizes its wire format into
//! the common `ProviderEvent` stream defined here.

pub mod anthropic;

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::conversation::{Message, TokenUsage};

/// Tool description sent to the provider so the model can choose to call it.
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub model: String,
    pub system_prompt: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSchema>,
    pub max_tokens: u32,
}

#[derive(Debug, Clone)]
pub enum ProviderEvent {
    /// Incremental text delta for a visible text block.
    TextDelta { text: String },
    /// Incremental reasoning delta (e.g., extended thinking).
    ReasoningDelta { text: String },
    /// A new tool-call block has started.
    ToolCallStart { id: String, name: String },
    /// JSON-encoded partial argument for an in-flight tool call.
    ToolCallInputDelta { id: String, partial_json: String },
    /// Finished tool-call block, with the fully parsed input.
    ToolCallEnd { id: String, input: serde_json::Value },
    /// Stream ended; the `StopReason` indicates what the caller should do next.
    Stop { reason: StopReason, usage: TokenUsage },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
    Other,
}

#[derive(Debug)]
pub enum ProviderError {
    Network(String),
    Auth(String),
    Api { status: u16, body: String },
    InvalidStream(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Network(e) => write!(f, "network: {}", e),
            ProviderError::Auth(e) => write!(f, "auth: {}", e),
            ProviderError::Api { status, body } => {
                let snippet: String = body.chars().take(200).collect();
                write!(f, "API {}: {}", status, snippet)
            }
            ProviderError::InvalidStream(e) => write!(f, "invalid stream: {}", e),
        }
    }
}

impl std::error::Error for ProviderError {}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn stream(
        &self,
        req: ProviderRequest,
        events: mpsc::UnboundedSender<ProviderEvent>,
    ) -> Result<(), ProviderError>;
}
