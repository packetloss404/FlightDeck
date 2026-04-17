//! Test-only `Provider` that replays a scripted sequence of `ProviderEvent`s.
//! Used for deterministic end-to-end tests of the stream-and-tool-loop without
//! touching the network.

use std::sync::Mutex;

use async_trait::async_trait;
use tokio::sync::mpsc;

use super::provider::{Provider, ProviderError, ProviderEvent, ProviderRequest};

pub struct MockProvider {
    script: Mutex<std::collections::VecDeque<Vec<ProviderEvent>>>,
}

impl MockProvider {
    pub fn with_script(script: Vec<Vec<ProviderEvent>>) -> Self {
        Self { script: Mutex::new(script.into()) }
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn stream(
        &self,
        _req: ProviderRequest,
        events: mpsc::UnboundedSender<ProviderEvent>,
    ) -> Result<(), ProviderError> {
        let turn = { self.script.lock().unwrap().pop_front() };
        let Some(events_to_emit) = turn else {
            return Err(ProviderError::Network("script exhausted".into()));
        };
        for ev in events_to_emit {
            let _ = events.send(ev);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use tokio::sync::mpsc as tokio_mpsc;

    use super::*;
    use crate::core::native::conversation::{
        Conversation, Message, MessagePart, TokenUsage, ToolCall, ToolResult,
    };
    use crate::core::native::provider::{ProviderRequest, StopReason};
    use crate::core::native::tool::{read::ReadTool, ToolRegistry};

    const MAX_TURNS: usize = 10;

    struct RunOutcome {
        final_conversation: Conversation,
        turn_count: usize,
        success: bool,
    }

    struct MessageAssembler {
        text: String,
        reasoning: String,
        tool_calls: Vec<ToolCall>,
        pending_tool: Option<(String, String)>,
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
                    self.pending_tool = Some((id, name));
                }
                ProviderEvent::ToolCallInputDelta { .. } => {}
                ProviderEvent::ToolCallEnd { id, input } => {
                    if let Some((pid, pname)) = self.pending_tool.take() {
                        if pid == id {
                            self.tool_calls.push(ToolCall {
                                id,
                                tool_name: pname,
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

    async fn run_mock_loop(
        provider: Arc<dyn Provider>,
        registry: Arc<ToolRegistry>,
        messages: Vec<Message>,
        project_path: PathBuf,
    ) -> RunOutcome {
        let mut conv = Conversation::new(project_path.to_string_lossy().to_string());
        for m in messages {
            conv.push(m);
        }

        for turn in 0..MAX_TURNS {
            let req = ProviderRequest {
                model: "mock".into(),
                system_prompt: conv.system_prompt.clone(),
                messages: conv.messages.clone(),
                tools: registry.schemas_for(&[]),
                max_tokens: 1024,
            };

            let (tx, mut rx) = tokio_mpsc::unbounded_channel::<ProviderEvent>();
            let stream_fut = {
                let p = provider.clone();
                async move { p.stream(req, tx).await }
            };

            let mut assembler = MessageAssembler::new();
            let mut stop_reason = StopReason::EndTurn;
            let mut turn_usage = TokenUsage::default();

            let drain = async {
                while let Some(event) = rx.recv().await {
                    if let ProviderEvent::Stop { reason, usage } = &event {
                        stop_reason = *reason;
                        turn_usage = usage.clone();
                    }
                    assembler.apply(event);
                }
            };

            let (stream_res, ()) = tokio::join!(stream_fut, drain);
            if stream_res.is_err() {
                return RunOutcome {
                    final_conversation: conv,
                    turn_count: turn,
                    success: false,
                };
            }

            let assistant_msg = Message::assistant(assembler.into_parts(), Some(turn_usage));
            let tool_calls: Vec<ToolCall> =
                assistant_msg.tool_calls().iter().map(|c| (*c).clone()).collect();
            conv.push(assistant_msg);

            if matches!(
                stop_reason,
                StopReason::EndTurn | StopReason::StopSequence | StopReason::MaxTokens
            ) && tool_calls.is_empty()
            {
                return RunOutcome {
                    final_conversation: conv,
                    turn_count: turn + 1,
                    success: true,
                };
            }

            if tool_calls.is_empty() {
                return RunOutcome {
                    final_conversation: conv,
                    turn_count: turn + 1,
                    success: true,
                };
            }

            let mut results = Vec::new();
            for call in &tool_calls {
                let Some(tool) = registry.get(&call.tool_name) else {
                    results.push(ToolResult {
                        tool_use_id: call.id.clone(),
                        content: format!("unknown tool: {}", call.tool_name),
                        is_error: true,
                    });
                    continue;
                };
                let output = tool.execute(call.input.clone(), &project_path).await;
                results.push(ToolResult {
                    tool_use_id: call.id.clone(),
                    content: output.content,
                    is_error: output.is_error,
                });
            }
            conv.push(Message::tool_results(results));
        }

        RunOutcome {
            final_conversation: conv,
            turn_count: MAX_TURNS,
            success: false,
        }
    }

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "flightdeck-test-{}-{}-{}",
                std::process::id(),
                tag,
                nanos
            ));
            std::fs::create_dir_all(&path).expect("create tempdir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[tokio::test]
    async fn mock_single_text_turn_ends_cleanly() {
        let guard = TempDirGuard::new("single");
        let provider = Arc::new(MockProvider::with_script(vec![vec![
            ProviderEvent::TextDelta { text: "hello".into() },
            ProviderEvent::Stop {
                reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
            },
        ]]));
        let registry = Arc::new(ToolRegistry::new());
        let user = Message::user_text("hi");
        let outcome = run_mock_loop(
            provider,
            registry,
            vec![user],
            guard.path().to_path_buf(),
        )
        .await;

        assert!(outcome.success);
        assert_eq!(outcome.turn_count, 1);
        assert_eq!(outcome.final_conversation.messages.len(), 2);
        assert_eq!(outcome.final_conversation.messages[1].text_content(), "hello");
    }

    #[tokio::test]
    async fn mock_tool_use_round_trip() {
        let guard = TempDirGuard::new("tool");
        let file_body = "line one\nline two\n";
        std::fs::write(guard.path().join("foo.md"), file_body).expect("write foo.md");

        let provider = Arc::new(MockProvider::with_script(vec![
            vec![
                ProviderEvent::ToolCallStart { id: "t1".into(), name: "read".into() },
                ProviderEvent::ToolCallInputDelta {
                    id: "t1".into(),
                    partial_json: r#"{"path":"foo.md"}"#.into(),
                },
                ProviderEvent::ToolCallEnd {
                    id: "t1".into(),
                    input: serde_json::json!({"path": "foo.md"}),
                },
                ProviderEvent::Stop {
                    reason: StopReason::ToolUse,
                    usage: TokenUsage::default(),
                },
            ],
            vec![
                ProviderEvent::TextDelta { text: "done".into() },
                ProviderEvent::Stop {
                    reason: StopReason::EndTurn,
                    usage: TokenUsage::default(),
                },
            ],
        ]));

        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ReadTool));
        let registry = Arc::new(reg);

        let user = Message::user_text("please read foo.md");
        let outcome = run_mock_loop(
            provider,
            registry,
            vec![user],
            guard.path().to_path_buf(),
        )
        .await;

        assert!(outcome.success);
        assert_eq!(outcome.turn_count, 2);
        assert_eq!(outcome.final_conversation.messages.len(), 4);

        let tool_result_msg = &outcome.final_conversation.messages[2];
        let mut found = false;
        for part in &tool_result_msg.parts {
            if let MessagePart::ToolResult(tr) = part {
                assert!(!tr.is_error, "tool result unexpectedly errored: {}", tr.content);
                assert!(
                    tr.content.contains("line one") && tr.content.contains("line two"),
                    "tool_result content missing file body: {}",
                    tr.content
                );
                found = true;
            }
        }
        assert!(found, "no tool_result part in message 2");

        assert_eq!(outcome.final_conversation.messages[3].text_content(), "done");
    }

    #[tokio::test]
    async fn mock_exhausted_script_returns_error() {
        let guard = TempDirGuard::new("exhausted");
        let provider = Arc::new(MockProvider::with_script(vec![]));
        let registry = Arc::new(ToolRegistry::new());
        let user = Message::user_text("hi");
        let outcome = run_mock_loop(
            provider,
            registry,
            vec![user],
            guard.path().to_path_buf(),
        )
        .await;

        assert!(!outcome.success);
        assert_eq!(outcome.final_conversation.messages.len(), 1);
    }

    #[tokio::test]
    async fn approval_denied_records_denial_in_tool_result() {
        use crate::core::native::chat::{self, ApprovalQuery, ChatEvent, ChatTurnRequest};
        use crate::core::native::tool::bash::BashTool;

        let guard = TempDirGuard::new("approval");
        let provider = Arc::new(MockProvider::with_script(vec![
            vec![
                ProviderEvent::ToolCallStart { id: "t1".into(), name: "bash".into() },
                ProviderEvent::ToolCallInputDelta {
                    id: "t1".into(),
                    partial_json: r#"{"command":"echo hi"}"#.into(),
                },
                ProviderEvent::ToolCallEnd {
                    id: "t1".into(),
                    input: serde_json::json!({"command": "echo hi"}),
                },
                ProviderEvent::Stop {
                    reason: StopReason::ToolUse,
                    usage: TokenUsage::default(),
                },
            ],
            vec![
                ProviderEvent::TextDelta { text: "acknowledged the denial".into() },
                ProviderEvent::Stop {
                    reason: StopReason::EndTurn,
                    usage: TokenUsage::default(),
                },
            ],
        ]));

        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(BashTool));
        let registry = Arc::new(reg);

        let (approval_tx, mut approval_rx) = tokio_mpsc::unbounded_channel::<ApprovalQuery>();
        let (events_tx, _events_rx) = tokio_mpsc::unbounded_channel::<ChatEvent>();

        let req = ChatTurnRequest {
            user_message: "run echo hi".into(),
            provider,
            registry,
            model: "test-model".into(),
            tool_allowlist: vec!["bash".into()],
            project_path: guard.path().to_path_buf(),
            approvals: Some(approval_tx),
        };

        let mut conversation = Conversation::new(guard.path().to_string_lossy().into_owned());

        // Deny whatever approval query comes in.
        let denier = tokio::spawn(async move {
            if let Some(query) = approval_rx.recv().await {
                assert_eq!(query.tool_name, "bash");
                let _ = query.responder.send(false);
            }
        });

        let summary = chat::chat_turn(&mut conversation, req, events_tx).await;
        let _ = denier.await;

        assert!(summary.success);

        // User message + assistant(tool_call) + user(tool_result=denied) + assistant("acknowledged")
        assert_eq!(conversation.messages.len(), 4);
        let tool_msg = &conversation.messages[2];
        let mut denied_found = false;
        for part in &tool_msg.parts {
            if let MessagePart::ToolResult(tr) = part {
                assert!(tr.is_error, "denied tool result should be flagged is_error");
                assert!(
                    tr.content.contains("denied") || tr.content.contains("Denied"),
                    "tool_result content doesn't mention denial: {}",
                    tr.content
                );
                denied_found = true;
            }
        }
        assert!(denied_found, "no tool_result part for the denied call");
    }
}
