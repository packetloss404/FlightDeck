//! Interactive-chat driver for the native agent.
//!
//! Analog of `runner::run`, but scoped to a single user message within a
//! long-lived `Conversation`. The view layer owns the conversation, hands it
//! in as `&mut` (or behind a `Mutex` via `spawn_chat_turn`), and consumes a
//! stream of `ChatEvent`s as the turn progresses.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::compaction::{self, CompactionSettings};
use super::conversation::{Conversation, Message, MessagePart, TokenUsage, ToolCall, ToolResult};
use super::provider::{Provider, ProviderEvent, ProviderRequest, StopReason};
use super::safety::DoomLoopDetector;
use super::tool::ToolRegistry;

/// Maximum tool-use iterations per user message. Mirrors `runner::MAX_TURNS`.
const MAX_TURNS: usize = 25;

/// Max output tokens per provider call.
const MAX_TOKENS: u32 = 4096;

#[derive(Debug, Clone)]
pub enum ChatEvent {
    TextDelta { text: String },
    ReasoningDelta { text: String },
    ToolCallStarted { id: String, name: String },
    ToolCallFinished {
        id: String,
        name: String,
        input: serde_json::Value,
        is_error: bool,
    },
    TurnComplete { usage: TokenUsage, turn_index: usize },
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ChatTurnSummary {
    pub turn_count: usize,
    pub usage: TokenUsage,
    pub success: bool,
}

/// A request from the chat driver asking the host whether a sensitive tool
/// call should be allowed to proceed. The driver awaits the `responder` for up
/// to `APPROVAL_TIMEOUT`; if the timeout fires or the channel drops, the call
/// is treated as denied.
#[derive(Debug)]
pub struct ApprovalQuery {
    pub id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub responder: tokio::sync::oneshot::Sender<bool>,
}

pub struct ChatTurnRequest {
    pub user_message: String,
    pub provider: Arc<dyn Provider>,
    pub registry: Arc<ToolRegistry>,
    pub model: String,
    pub tool_allowlist: Vec<String>,
    pub project_path: PathBuf,
    /// Channel the driver uses to ask the host whether a dangerous tool may
    /// run. `None` means auto-approve (used by tests and headless contexts).
    pub approvals: Option<mpsc::UnboundedSender<ApprovalQuery>>,
}

const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Drive one full chat turn to completion: append the user message, call the
/// provider, execute any requested tool calls, loop on tool_use, exit when the
/// model stops cleanly (or after MAX_TURNS tool-use iterations).
pub async fn chat_turn(
    conversation: &mut Conversation,
    req: ChatTurnRequest,
    events: mpsc::UnboundedSender<ChatEvent>,
) -> ChatTurnSummary {
    conversation.push(Message::user_text(req.user_message.clone()));

    let tools = req.registry.schemas_for(&req.tool_allowlist);
    let mut total_usage = TokenUsage::default();
    let mut inner_iterations = 0usize;
    let mut doom_loop = DoomLoopDetector::new();

    for turn in 0..MAX_TURNS {
        inner_iterations = turn + 1;

        let _ = compaction::compact(conversation, CompactionSettings::default());

        let provider_req = ProviderRequest {
            model: req.model.clone(),
            system_prompt: conversation.system_prompt.clone(),
            messages: conversation.messages.clone(),
            tools: tools.clone(),
            max_tokens: MAX_TOKENS,
        };

        let (tx, mut rx) = mpsc::unbounded_channel::<ProviderEvent>();
        let provider = req.provider.clone();
        let stream_task = {
            let pr = provider_req.clone();
            let provider = provider.clone();
            async move { provider.stream(pr, tx).await }
        };

        let mut assembler = MessageAssembler::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut turn_usage = TokenUsage::default();

        let drain = async {
            while let Some(event) = rx.recv().await {
                match &event {
                    ProviderEvent::TextDelta { text } => {
                        let _ = events.send(ChatEvent::TextDelta { text: text.clone() });
                    }
                    ProviderEvent::ReasoningDelta { text } => {
                        let _ = events.send(ChatEvent::ReasoningDelta { text: text.clone() });
                    }
                    ProviderEvent::ToolCallStart { id, name } => {
                        let _ = events.send(ChatEvent::ToolCallStarted {
                            id: id.clone(),
                            name: name.clone(),
                        });
                    }
                    ProviderEvent::ToolCallInputDelta { .. } => {}
                    ProviderEvent::ToolCallEnd { .. } => {}
                    ProviderEvent::Stop { reason, usage } => {
                        stop_reason = *reason;
                        turn_usage = usage.clone();
                    }
                }
                assembler.apply(event);
            }
        };

        let (stream_res, ()) = tokio::join!(stream_task, drain);
        if let Err(e) = stream_res {
            let msg = format!("provider error: {}", e);
            let _ = events.send(ChatEvent::Error(msg));
            return ChatTurnSummary {
                turn_count: inner_iterations,
                usage: total_usage,
                success: false,
            };
        }

        total_usage.accumulate(&turn_usage);
        let assistant_msg = Message::assistant(assembler.into_parts(), Some(turn_usage));
        let tool_calls: Vec<ToolCall> = assistant_msg
            .tool_calls()
            .iter()
            .map(|c| (*c).clone())
            .collect();
        conversation.push(assistant_msg);

        let stopped_cleanly = matches!(
            stop_reason,
            StopReason::EndTurn | StopReason::StopSequence | StopReason::MaxTokens
        );

        if tool_calls.is_empty() {
            // Either a clean stop, or the model produced no calls; either way,
            // this user message's turn is finished.
            if !stopped_cleanly {
                tracing::warn!(?stop_reason, "model stopped with no tool calls but not cleanly");
            }
            let _ = events.send(ChatEvent::TurnComplete {
                usage: total_usage.clone(),
                turn_index: inner_iterations,
            });
            return ChatTurnSummary {
                turn_count: inner_iterations,
                usage: total_usage,
                success: true,
            };
        }

        let mut results = Vec::with_capacity(tool_calls.len());
        for call in &tool_calls {
            let verdict = doom_loop.observe(&call.tool_name, &call.input);
            if verdict.detected {
                let msg = verdict.message.clone().unwrap_or_else(|| "doom loop".into());
                let _ = events.send(ChatEvent::ToolCallFinished {
                    id: call.id.clone(),
                    name: format!("[halted] {}", call.tool_name),
                    input: call.input.clone(),
                    is_error: true,
                });
                results.push(ToolResult {
                    tool_use_id: call.id.clone(),
                    content: format!(
                        "Tool execution halted: {}. Stop calling this tool with identical input and change strategy.",
                        msg
                    ),
                    is_error: true,
                });
                continue;
            }

            let Some(tool) = req.registry.get(&call.tool_name) else {
                let _ = events.send(ChatEvent::ToolCallFinished {
                    id: call.id.clone(),
                    name: call.tool_name.clone(),
                    input: call.input.clone(),
                    is_error: true,
                });
                results.push(ToolResult {
                    tool_use_id: call.id.clone(),
                    content: format!("unknown tool: {}", call.tool_name),
                    is_error: true,
                });
                continue;
            };

            if tool.requires_approval(&call.input, &req.project_path) {
                let approved = if let Some(approvals) = &req.approvals {
                    let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
                    let query = ApprovalQuery {
                        id: call.id.clone(),
                        tool_name: call.tool_name.clone(),
                        input: call.input.clone(),
                        responder: tx,
                    };
                    if approvals.send(query).is_err() {
                        false
                    } else {
                        match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
                            Ok(Ok(decision)) => decision,
                            _ => false,
                        }
                    }
                } else {
                    tracing::warn!(
                        tool = %call.tool_name,
                        "auto-approving — no approval channel wired"
                    );
                    true
                };

                if !approved {
                    let _ = events.send(ChatEvent::ToolCallFinished {
                        id: call.id.clone(),
                        name: format!("[denied] {}", call.tool_name),
                        input: call.input.clone(),
                        is_error: true,
                    });
                    results.push(ToolResult {
                        tool_use_id: call.id.clone(),
                        content: format!(
                            "User denied the {} call. Do not retry with the same input; propose an alternative or ask the user for guidance.",
                            call.tool_name
                        ),
                        is_error: true,
                    });
                    continue;
                }
            }

            let output = tool.execute(call.input.clone(), &req.project_path).await;
            let _ = events.send(ChatEvent::ToolCallFinished {
                id: call.id.clone(),
                name: call.tool_name.clone(),
                input: call.input.clone(),
                is_error: output.is_error,
            });
            results.push(ToolResult {
                tool_use_id: call.id.clone(),
                content: output.content,
                is_error: output.is_error,
            });
        }

        conversation.push(Message::tool_results(results));
    }

    let _ = events.send(ChatEvent::Error(format!(
        "halted at {} tool-use iterations",
        MAX_TURNS
    )));
    ChatTurnSummary {
        turn_count: MAX_TURNS,
        usage: total_usage,
        success: false,
    }
}

/// Fire-and-forget: spawn a dedicated OS thread with a current-thread tokio
/// runtime that executes `chat_turn` against the supplied conversation. The
/// conversation is wrapped in a Mutex so the view layer can render it while
/// the turn is in flight.
pub fn spawn_chat_turn(
    conversation: Arc<std::sync::Mutex<Conversation>>,
    req: ChatTurnRequest,
    events: mpsc::UnboundedSender<ChatEvent>,
) {
    let thread_name = format!("flightdeck-chat-{}", short_id());
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = events.send(ChatEvent::Error(format!(
                        "runtime build failed: {}",
                        e
                    )));
                    return;
                }
            };

            // Snapshot-then-merge: take a clone of the conversation, run the
            // turn against it, then swap the updated copy back in under the
            // lock. This keeps the lock free for the view layer while the
            // provider streams.
            let mut snapshot = match conversation.lock() {
                Ok(guard) => guard.clone(),
                Err(e) => {
                    let _ = events.send(ChatEvent::Error(format!(
                        "conversation lock poisoned: {}",
                        e
                    )));
                    return;
                }
            };

            let summary = runtime.block_on(chat_turn(&mut snapshot, req, events.clone()));

            match conversation.lock() {
                Ok(mut guard) => {
                    *guard = snapshot;
                }
                Err(e) => {
                    let _ = events.send(ChatEvent::Error(format!(
                        "conversation lock poisoned on write-back: {}",
                        e
                    )));
                }
            }

            tracing::info!(
                turns = summary.turn_count,
                success = summary.success,
                input_tokens = summary.usage.input_tokens,
                output_tokens = summary.usage.output_tokens,
                "chat turn complete"
            );
        })
        .expect("spawn chat-turn thread");
}

// ---------- Message assembly (duplicated from runner.rs by design) ----------

struct MessageAssembler {
    text: String,
    reasoning: String,
    tool_calls: Vec<ToolCall>,
    pending_tool: Option<PendingTool>,
}

struct PendingTool {
    id: String,
    name: String,
}

impl MessageAssembler {
    fn new() -> Self {
        Self {
            text: String::new(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
            pending_tool: None,
        }
    }

    fn apply(&mut self, event: ProviderEvent) {
        match event {
            ProviderEvent::TextDelta { text } => self.text.push_str(&text),
            ProviderEvent::ReasoningDelta { text } => self.reasoning.push_str(&text),
            ProviderEvent::ToolCallStart { id, name } => {
                self.pending_tool = Some(PendingTool { id, name });
            }
            ProviderEvent::ToolCallInputDelta { .. } => {}
            ProviderEvent::ToolCallEnd { id, input } => {
                if let Some(pending) = self.pending_tool.take() {
                    if pending.id == id {
                        self.tool_calls.push(ToolCall {
                            id,
                            tool_name: pending.name,
                            input,
                        });
                    }
                }
            }
            ProviderEvent::Stop { .. } => {}
        }
    }

    fn into_parts(self) -> Vec<MessagePart> {
        let mut parts = Vec::new();
        if !self.reasoning.is_empty() {
            parts.push(MessagePart::Reasoning { text: self.reasoning });
        }
        if !self.text.is_empty() {
            parts.push(MessagePart::Text { text: self.text });
        }
        for call in self.tool_calls {
            parts.push(MessagePart::ToolCall(call));
        }
        parts
    }
}

fn short_id() -> String {
    uuid::Uuid::new_v4().simple().to_string().chars().take(8).collect()
}
