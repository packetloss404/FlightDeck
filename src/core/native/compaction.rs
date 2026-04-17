//! Conversation compaction: prune older messages to fit a token budget while
//! preserving a protected tail of recent messages. A future iteration will
//! replace the placeholder summary with an LLM-generated digest.

use super::conversation::{Conversation, Message, MessagePart, Role};

#[derive(Debug, Clone, Copy)]
pub struct CompactionSettings {
    pub budget_tokens: u64,
    pub protected_tail_tokens: u64,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        Self { budget_tokens: 180_000, protected_tail_tokens: 40_000 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompactionReport {
    pub applied: bool,
    pub pruned_message_count: usize,
    pub pruned_tokens: u64,
    pub resulting_tokens: u64,
}

pub fn estimate_tokens(text: &str) -> u64 {
    (text.chars().count() as u64 + 3) / 4
}

pub fn estimate_part_tokens(part: &MessagePart) -> u64 {
    match part {
        MessagePart::Text { text } => estimate_tokens(text),
        MessagePart::Reasoning { text } => estimate_tokens(text),
        MessagePart::ToolCall(call) => {
            let input_s = serde_json::to_string(&call.input).unwrap_or_default();
            estimate_tokens(&input_s) + estimate_tokens(&call.tool_name) + 4
        }
        MessagePart::ToolResult(res) => estimate_tokens(&res.content) + 4,
    }
}

pub fn estimate_message_tokens(message: &Message) -> u64 {
    message.parts.iter().map(estimate_part_tokens).sum::<u64>() + 4
}

pub fn estimate_conversation_tokens(conversation: &Conversation) -> u64 {
    let system = conversation
        .system_prompt
        .as_deref()
        .map(estimate_tokens)
        .unwrap_or(0);
    system + conversation.messages.iter().map(estimate_message_tokens).sum::<u64>()
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

pub fn compact(
    conversation: &mut Conversation,
    settings: CompactionSettings,
) -> CompactionReport {
    let total = estimate_conversation_tokens(conversation);
    if total <= settings.budget_tokens {
        return CompactionReport {
            applied: false,
            pruned_message_count: 0,
            pruned_tokens: 0,
            resulting_tokens: total,
        };
    }

    let n = conversation.messages.len();
    if n == 0 {
        return CompactionReport {
            applied: false,
            pruned_message_count: 0,
            pruned_tokens: 0,
            resulting_tokens: total,
        };
    }

    // Walk from the end, counting tokens until we cross the protected-tail
    // threshold. Include the message that crossed it.
    let mut tail_tokens: u64 = 0;
    let mut tail_count: usize = 0;
    for msg in conversation.messages.iter().rev() {
        tail_count += 1;
        tail_tokens += estimate_message_tokens(msg);
        if tail_tokens >= settings.protected_tail_tokens {
            break;
        }
    }

    // Guard: if the tail already contains everything, nothing to prune.
    if tail_count >= n {
        return CompactionReport {
            applied: false,
            pruned_message_count: 0,
            pruned_tokens: 0,
            resulting_tokens: total,
        };
    }

    let tail_start = n - tail_count;

    // Decide whether to keep the very first message.
    let keep_first = n > 1
        && matches!(conversation.messages[0].role, Role::User)
        && tail_start > 0;

    let first_end = if keep_first { 1 } else { 0 };
    let prune_start = first_end;
    let prune_end = tail_start;

    // If the prunable range is empty, no-op.
    if prune_start >= prune_end {
        return CompactionReport {
            applied: false,
            pruned_message_count: 0,
            pruned_tokens: 0,
            resulting_tokens: total,
        };
    }

    let pruned_slice = &conversation.messages[prune_start..prune_end];
    let pruned_message_count = pruned_slice.len();
    let pruned_tokens: u64 = pruned_slice.iter().map(estimate_message_tokens).sum();

    let summary_text = format!(
        "[compacted {} messages, ~{} tokens \u{2014} a real LLM summary lands in a later iteration]",
        pruned_message_count, pruned_tokens
    );
    let summary = Message {
        id: new_id("msg"),
        role: Role::User,
        parts: vec![MessagePart::Text { text: summary_text }],
        created_at: now_millis(),
        usage: None,
    };

    // Rebuild the vector: [optional first] [summary] [protected tail].
    let old_messages = std::mem::take(&mut conversation.messages);
    let mut new_messages: Vec<Message> = Vec::with_capacity(n - pruned_message_count + 1);
    let mut summary_inserted = false;
    for (i, msg) in old_messages.into_iter().enumerate() {
        if keep_first && i == 0 {
            new_messages.push(msg);
            continue;
        }
        if i >= prune_start && i < prune_end {
            continue;
        }
        if i >= prune_end && !summary_inserted {
            new_messages.push(summary.clone());
            summary_inserted = true;
        }
        new_messages.push(msg);
    }

    conversation.messages = new_messages;
    conversation.updated_at = now_millis();

    let resulting_tokens = estimate_conversation_tokens(conversation);
    CompactionReport {
        applied: true,
        pruned_message_count,
        pruned_tokens,
        resulting_tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::super::conversation::{Message, MessagePart, Role, ToolCall, ToolResult};
    use super::*;

    #[test]
    fn estimate_tokens_ballpark() {
        let s = "a".repeat(100);
        let t = estimate_tokens(&s);
        assert!(t >= 20 && t <= 30, "expected ~25, got {}", t);
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
    }

    #[test]
    fn estimate_part_tokens_covers_all_variants() {
        let text = MessagePart::Text { text: "hello world".into() };
        let reasoning = MessagePart::Reasoning { text: "thinking".into() };
        let call = MessagePart::ToolCall(ToolCall {
            id: "t".into(),
            tool_name: "read".into(),
            input: serde_json::json!({"path": "a.md"}),
        });
        let result = MessagePart::ToolResult(ToolResult {
            tool_use_id: "t".into(),
            content: "file".into(),
            is_error: false,
        });
        assert!(estimate_part_tokens(&text) > 0);
        assert!(estimate_part_tokens(&reasoning) > 0);
        assert!(estimate_part_tokens(&call) >= 4);
        assert!(estimate_part_tokens(&result) >= 4);
    }

    #[test]
    fn compact_noop_when_under_budget() {
        let mut conv = Conversation::new("/tmp/x");
        conv.push(Message::user_text("hi"));
        conv.push(Message::user_text("there"));
        let settings = CompactionSettings { budget_tokens: 10_000, protected_tail_tokens: 100 };
        let report = compact(&mut conv, settings);
        assert!(!report.applied);
        assert_eq!(report.pruned_message_count, 0);
        assert_eq!(conv.messages.len(), 2);
    }

    #[test]
    fn compact_prunes_many_messages_preserving_tail() {
        let mut conv = Conversation::new("/tmp/x");
        // 200 small messages. Each "msg N body" ~ 10 chars ~ 3 tokens + 4 overhead = 7 tokens.
        for i in 0..200 {
            conv.push(Message::user_text(format!("msg {} body here", i)));
        }
        let before_total = estimate_conversation_tokens(&conv);
        assert!(before_total > 1000);

        let settings = CompactionSettings { budget_tokens: 1000, protected_tail_tokens: 200 };
        let report = compact(&mut conv, settings);
        assert!(report.applied);
        assert!(report.pruned_message_count > 0);

        // Tail should still be intact: last messages match the original tail content.
        // Find the summary index (one of the early messages, after possible first).
        let summary_idx = conv
            .messages
            .iter()
            .position(|m| {
                matches!(m.role, Role::User)
                    && m.parts.len() == 1
                    && matches!(&m.parts[0], MessagePart::Text { text } if text.starts_with("[compacted "))
            })
            .expect("summary message present");

        // Everything after the summary is the protected tail.
        let tail = &conv.messages[summary_idx + 1..];
        assert!(!tail.is_empty(), "protected tail must be non-empty");

        // The very last message should be "msg 199 body here".
        let last_text = conv.messages.last().unwrap().text_content();
        assert_eq!(last_text, "msg 199 body here");

        // resulting_tokens is below the original.
        assert!(report.resulting_tokens < before_total);
    }

    #[test]
    fn compact_preserves_first_message_when_multiple() {
        let mut conv = Conversation::new("/tmp/x");
        conv.push(Message::user_text("INITIAL PROMPT SENTINEL"));
        for i in 0..100 {
            conv.push(Message::user_text("x".repeat(200) + &format!(" {}", i)));
        }
        let settings = CompactionSettings { budget_tokens: 500, protected_tail_tokens: 200 };
        let report = compact(&mut conv, settings);
        assert!(report.applied);
        // First message preserved.
        assert_eq!(conv.messages[0].text_content(), "INITIAL PROMPT SENTINEL");
        // Second message is the summary.
        assert!(matches!(&conv.messages[1].parts[0], MessagePart::Text { text } if text.starts_with("[compacted ")));
    }

    #[test]
    fn summary_message_has_expected_shape() {
        let mut conv = Conversation::new("/tmp/x");
        for i in 0..50 {
            conv.push(Message::user_text("x".repeat(500) + &format!(" {}", i)));
        }
        let settings = CompactionSettings { budget_tokens: 200, protected_tail_tokens: 100 };
        let report = compact(&mut conv, settings);
        assert!(report.applied);
        let summary = conv
            .messages
            .iter()
            .find(|m| {
                matches!(&m.parts.get(0), Some(MessagePart::Text { text }) if text.starts_with("[compacted "))
            })
            .expect("summary present");
        assert!(matches!(summary.role, Role::User));
        assert_eq!(summary.parts.len(), 1);
        let body = summary.text_content();
        assert!(body.contains(&format!("{}", report.pruned_message_count)));
    }
}
