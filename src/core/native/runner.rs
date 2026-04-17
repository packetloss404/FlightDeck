//! Native agent runner: the loop that drives a single task end-to-end.
//!
//! Structure:
//!   1. Build a `Conversation` from the request (system prompt + user prompt).
//!   2. Call the provider; stream events; assemble an assistant `Message`.
//!   3. If the message contains tool calls, execute each, collect results,
//!      push a tool-results message, and loop.
//!   4. If the message ends cleanly (no tool calls), break.
//!
//! Output is formatted to look like Claude Code's terminal output so the
//! existing `SessionBuffer` machinery and visual cues Just Work until step 6
//! wires up proper structured status events.

use std::path::Path;
use std::sync::{mpsc as std_mpsc, Arc};

use tokio::sync::mpsc as tokio_mpsc;
use zeroize::Zeroizing;

use super::compaction::{self, CompactionSettings};
use super::conversation::{Conversation, Message, MessagePart, Role, TokenUsage, ToolCall, ToolResult};
use super::provider::anthropic::AnthropicProvider;
use super::provider::{Provider, ProviderEvent, ProviderRequest, StopReason};
use super::safety::DoomLoopDetector;
use super::tool::ToolRegistry;
use super::NativeRunRequest;
use crate::core::pty::PtyEvent;

/// Maximum tool-use turns we'll run before bailing out. A safety cap on top of
/// the provider's own `max_tokens` — step 6 adds a smarter doom-loop detector.
const MAX_TURNS: usize = 25;

/// Max output tokens per provider call. Tuned for Claude Sonnet-class models
/// with room to draft a meaningful response plus a handful of tool calls.
const MAX_TOKENS: u32 = 4096;

const DEFAULT_SYSTEM_PROMPT: &str =
    "You are FlightDeck's in-process coding agent. Work against the project directory you're placed in. Prefer targeted reads and edits; run commands only when needed, and keep your responses concise.";

pub struct RunnerConfig {
    pub api_key: Zeroizing<String>,
    pub base_url: Option<String>,
    pub registry: Arc<ToolRegistry>,
}

/// Emit-friendly summary of a single run. Returned to the caller so it can
/// update Task state (cost, tokens, completion status).
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub usage: TokenUsage,
    pub turn_count: usize,
    pub success: bool,
    pub message: String,
}

/// Run the native agent. Consumes the request and drives to completion.
pub async fn run(
    req: NativeRunRequest,
    session_id: String,
    config: RunnerConfig,
    pty_tx: std_mpsc::Sender<PtyEvent>,
) -> RunSummary {
    let provider = AnthropicProvider::new(config.api_key, config.base_url);
    let project_path = Path::new(&req.project_path).to_path_buf();
    let registry = config.registry;
    let tools = registry.schemas_for(&req.tool_allowlist);

    let mut conv = Conversation::new(req.project_path.clone());
    conv.system_prompt = Some(
        req.system_prompt_override
            .clone()
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string()),
    );
    conv.push(Message::user_text(req.prompt.clone()));

    emit_output(&pty_tx, &session_id, &format!("▸ Starting task {}\n", req.task_id));

    let mut total_usage = TokenUsage::default();
    let mut last_message_text = String::new();
    let mut doom_loop = DoomLoopDetector::new();

    for turn in 0..MAX_TURNS {
        let compaction_report = compaction::compact(&mut conv, CompactionSettings::default());
        if compaction_report.applied {
            emit_output(
                &pty_tx,
                &session_id,
                &format!(
                    "[compacted {} messages (~{} tokens) → now {} tokens]\n",
                    compaction_report.pruned_message_count,
                    compaction_report.pruned_tokens,
                    compaction_report.resulting_tokens
                ),
            );
        }

        let provider_req = ProviderRequest {
            model: req.model.clone(),
            system_prompt: conv.system_prompt.clone(),
            messages: conv.messages.clone(),
            tools: tools.clone(),
            max_tokens: MAX_TOKENS,
        };

        let (tx, mut rx) = tokio_mpsc::unbounded_channel::<ProviderEvent>();
        let stream_task = {
            let provider = &provider;
            let pr = provider_req.clone();
            async move { provider.stream(pr, tx).await }
        };

        let mut assembler = MessageAssembler::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut turn_usage = TokenUsage::default();

        let drain = async {
            while let Some(event) = rx.recv().await {
                match &event {
                    ProviderEvent::TextDelta { text } => {
                        emit_output(&pty_tx, &session_id, text);
                    }
                    ProviderEvent::ToolCallStart { name, .. } => {
                        emit_output(&pty_tx, &session_id, &format!("\n⏺ {}(", capitalize(name)));
                    }
                    ProviderEvent::ToolCallEnd { input, .. } => {
                        let display = tool_call_display(input);
                        emit_output(&pty_tx, &session_id, &format!("{})\n", display));
                    }
                    ProviderEvent::ReasoningDelta { .. } => {
                        // keep reasoning silent in the transcript by default.
                    }
                    ProviderEvent::ToolCallInputDelta { .. } => {}
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
            emit_output(&pty_tx, &session_id, &format!("\n[error] {}\n", msg));
            emit_exit(&pty_tx, &session_id, false);
            return RunSummary {
                usage: total_usage,
                turn_count: turn + 1,
                success: false,
                message: msg,
            };
        }

        total_usage.accumulate(&turn_usage);
        last_message_text = assembler.text_content();
        let assistant_msg = Message::assistant(assembler.into_parts(), Some(turn_usage));
        let tool_calls: Vec<ToolCall> = assistant_msg.tool_calls().iter().map(|c| (*c).clone()).collect();
        conv.push(assistant_msg);

        if matches!(stop_reason, StopReason::EndTurn | StopReason::StopSequence | StopReason::MaxTokens)
            && tool_calls.is_empty()
        {
            emit_output(&pty_tx, &session_id, "\n▸ done\n");
            emit_exit(&pty_tx, &session_id, true);
            return RunSummary {
                usage: total_usage,
                turn_count: turn + 1,
                success: true,
                message: last_message_text,
            };
        }

        if tool_calls.is_empty() {
            // Model returned no tool calls but didn't cleanly stop — treat as end.
            emit_exit(&pty_tx, &session_id, true);
            return RunSummary {
                usage: total_usage,
                turn_count: turn + 1,
                success: true,
                message: last_message_text,
            };
        }

        // Execute each tool call and collect the results.
        let mut results = Vec::new();
        for call in &tool_calls {
            let verdict = doom_loop.observe(&call.tool_name, &call.input);
            if verdict.detected {
                emit_output(
                    &pty_tx,
                    &session_id,
                    &format!("[doom-loop] {}\n", verdict.message.clone().unwrap_or_default()),
                );
                let msg = verdict.message.unwrap_or_else(|| "doom loop detected".into());
                results.push(ToolResult {
                    tool_use_id: call.id.clone(),
                    content: format!(
                        "Tool execution halted: {}. The loop is broken — stop calling this tool with identical input and either change strategy or report back.",
                        msg
                    ),
                    is_error: true,
                });
                continue;
            }

            let Some(tool) = registry.get(&call.tool_name) else {
                results.push(ToolResult {
                    tool_use_id: call.id.clone(),
                    content: format!("unknown tool: {}", call.tool_name),
                    is_error: true,
                });
                emit_output(&pty_tx, &session_id, &format!("[tool:{}] unknown\n", call.tool_name));
                continue;
            };

            if tool.requires_approval(&call.input, &project_path) {
                // Step 7 of the plan wires proper approval gating. For now log
                // and proceed — the tool itself still runs. Production use is
                // gated on step 7 before shipping.
                emit_output(
                    &pty_tx,
                    &session_id,
                    &format!("[approval needed for {} — auto-approving until step 7]\n", call.tool_name),
                );
            }

            let output = tool.execute(call.input.clone(), &project_path).await;
            emit_output(
                &pty_tx,
                &session_id,
                &format!(
                    "[{}] {}\n",
                    call.tool_name,
                    if output.is_error { "error" } else { "ok" }
                ),
            );
            results.push(ToolResult {
                tool_use_id: call.id.clone(),
                content: output.content,
                is_error: output.is_error,
            });
        }

        conv.push(Message::tool_results(results));
    }

    emit_output(&pty_tx, &session_id, "\n[halt] max turns reached\n");
    emit_exit(&pty_tx, &session_id, false);
    RunSummary {
        usage: total_usage,
        turn_count: MAX_TURNS,
        success: false,
        message: format!("halted at {} turns", MAX_TURNS),
    }
}

// ---------- Message assembly ----------

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

    fn text_content(&self) -> String {
        self.text.clone()
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

// ---------- Output helpers ----------

fn emit_output(tx: &std_mpsc::Sender<PtyEvent>, session_id: &str, data: &str) {
    let _ = tx.send(PtyEvent::Output {
        session_id: session_id.to_string(),
        data: data.to_string(),
    });
}

fn emit_exit(tx: &std_mpsc::Sender<PtyEvent>, session_id: &str, success: bool) {
    let _ = tx.send(PtyEvent::Exit {
        session_id: session_id.to_string(),
        exit_code: Some(if success { 0 } else { 1 }),
        success,
        killed: false,
    });
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Turn a tool-call input JSON blob into a short human-readable bracket body
/// (e.g. `src/foo.rs` for a read, or the `command` string for a bash call).
fn tool_call_display(input: &serde_json::Value) -> String {
    if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
        return path.to_string();
    }
    if let Some(pattern) = input.get("pattern").and_then(|v| v.as_str()) {
        return pattern.to_string();
    }
    if let Some(command) = input.get("command").and_then(|v| v.as_str()) {
        return command.chars().take(60).collect::<String>();
    }
    String::new()
}

/// Build a runner config from a provider id by looking up the stored
/// `ProviderConfig` and pulling the API key from the keyring.
pub fn load_runner_config(
    provider_id: &str,
    providers: &[crate::core::provider_config::ProviderConfig],
) -> Result<RunnerConfig, String> {
    let cfg = providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("provider {} not found", provider_id))?;
    let api_key = crate::core::provider_config::get_api_key(provider_id)?;
    Ok(RunnerConfig {
        api_key,
        base_url: cfg.base_url.clone(),
        registry: Arc::new(ToolRegistry::defaults()),
    })
}

/// Spawn a dedicated OS thread that owns a current-thread tokio runtime and
/// runs `run()` to completion. Returns immediately. This is the entry point
/// from `app.rs::orchestrator_tick` on a `TaskDispatch::Native`.
pub fn spawn(
    req: NativeRunRequest,
    session_id: String,
    config: RunnerConfig,
    pty_tx: std_mpsc::Sender<PtyEvent>,
) {
    std::thread::Builder::new()
        .name(format!("flightdeck-native-{}", session_id))
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = pty_tx.send(PtyEvent::Output {
                        session_id: session_id.clone(),
                        data: format!("[fatal] runtime build failed: {}\n", e),
                    });
                    let _ = pty_tx.send(PtyEvent::Exit {
                        session_id: session_id.clone(),
                        exit_code: Some(1),
                        success: false,
                        killed: false,
                    });
                    return;
                }
            };
            let summary = runtime.block_on(run(req, session_id.clone(), config, pty_tx.clone()));
            tracing::info!(
                session_id = %session_id,
                turns = summary.turn_count,
                success = summary.success,
                input_tokens = summary.usage.input_tokens,
                output_tokens = summary.usage.output_tokens,
                "native agent run complete"
            );
        })
        .expect("spawn native runner thread");
}

// Avoid unused-import warning for `Role` — kept in scope because future
// compaction logic will care about role-filtering the history.
#[allow(dead_code)]
fn _touch_role(r: Role) -> Role { r }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembler_collects_text_and_tool_calls() {
        let mut a = MessageAssembler::new();
        a.apply(ProviderEvent::TextDelta { text: "hel".into() });
        a.apply(ProviderEvent::TextDelta { text: "lo".into() });
        a.apply(ProviderEvent::ToolCallStart {
            id: "t1".into(),
            name: "read".into(),
        });
        a.apply(ProviderEvent::ToolCallEnd {
            id: "t1".into(),
            input: serde_json::json!({"path": "foo"}),
        });
        let parts = a.into_parts();
        assert_eq!(parts.len(), 2);
        if let MessagePart::Text { text } = &parts[0] {
            assert_eq!(text, "hello");
        } else {
            panic!("expected text part first");
        }
        if let MessagePart::ToolCall(call) = &parts[1] {
            assert_eq!(call.tool_name, "read");
        } else {
            panic!("expected tool_call part second");
        }
    }

    #[test]
    fn tool_call_display_prefers_path_then_pattern_then_command() {
        assert_eq!(tool_call_display(&serde_json::json!({"path": "x.rs"})), "x.rs");
        assert_eq!(tool_call_display(&serde_json::json!({"pattern": "p"})), "p");
        assert_eq!(tool_call_display(&serde_json::json!({"command": "ls"})), "ls");
        assert_eq!(tool_call_display(&serde_json::json!({})), "");
    }
}
