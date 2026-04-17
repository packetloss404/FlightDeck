//! Agent view: native coding-agent chat surface.
//!
//! Owns its own local state (`AgentViewState`) held on `App`. The host is
//! responsible for actually driving provider turns and pushing `MessagePart`s
//! into the `Conversation`; this module just renders the transcript and
//! interprets keys into `AgentAction` verbs.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::core::native::conversation::{
    Conversation, Message, MessagePart, Role, ToolCall, ToolResult,
};
use crate::core::provider_config::ProviderConfig;
use crate::theme::Theme;

const REASONING_PREVIEW_LINES: usize = 2;
const TOOL_RESULT_PREVIEW_LINES: usize = 12;
const TOOL_INPUT_SUMMARY_MAX: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentFocus {
    Input,
    Transcript,
}

pub struct AgentViewState {
    pub input: String,
    pub cursor: usize,
    pub scroll_offset: u16,
    pub selected_provider_id: Option<String>,
    pub selected_model: Option<String>,
    pub focus: AgentFocus,
    pub running: bool,
    pub last_error: Option<String>,
    /// Tracks whether the user has manually scrolled up; suppresses auto-scroll
    /// while a turn is in flight so the caller can keep the user's viewport.
    pub user_scrolled: bool,
    /// Optional label for the in-flight tool (e.g. "bash") shown in the header.
    pub current_tool: Option<String>,
}

impl Default for AgentViewState {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            scroll_offset: 0,
            selected_provider_id: None,
            selected_model: None,
            focus: AgentFocus::Input,
            running: false,
            last_error: None,
            user_scrolled: false,
            current_tool: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AgentAction {
    None,
    SendUserMessage(String),
    SelectProvider(String),
    SelectModel(String),
    InterruptTurn,
    ClearConversation,
    ScrollUp,
    ScrollDown,
}

pub fn handle_key(state: &mut AgentViewState, key: KeyEvent) -> AgentAction {
    match state.focus {
        AgentFocus::Input => handle_input_key(state, key),
        AgentFocus::Transcript => handle_transcript_key(state, key),
    }
}

fn handle_input_key(state: &mut AgentViewState, key: KeyEvent) -> AgentAction {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Enter => {
            if state.input.trim().is_empty() {
                AgentAction::None
            } else {
                let msg = std::mem::take(&mut state.input);
                state.cursor = 0;
                state.user_scrolled = false;
                AgentAction::SendUserMessage(msg)
            }
        }
        KeyCode::Char('l') if ctrl => {
            state.input.clear();
            state.cursor = 0;
            AgentAction::ClearConversation
        }
        KeyCode::Char('c') if ctrl => {
            if state.running {
                AgentAction::InterruptTurn
            } else {
                AgentAction::None
            }
        }
        KeyCode::Char(ch) => {
            if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
                insert_char(&mut state.input, &mut state.cursor, ch);
            }
            AgentAction::None
        }
        KeyCode::Backspace => {
            delete_before(&mut state.input, &mut state.cursor);
            AgentAction::None
        }
        KeyCode::Left => {
            if state.cursor > 0 {
                state.cursor = prev_char_boundary(&state.input, state.cursor);
            }
            AgentAction::None
        }
        KeyCode::Right => {
            if state.cursor < state.input.len() {
                state.cursor = next_char_boundary(&state.input, state.cursor);
            }
            AgentAction::None
        }
        KeyCode::Home => {
            state.cursor = 0;
            AgentAction::None
        }
        KeyCode::End => {
            state.cursor = state.input.len();
            AgentAction::None
        }
        KeyCode::Tab => {
            state.focus = AgentFocus::Transcript;
            AgentAction::None
        }
        KeyCode::PageUp => {
            state.user_scrolled = true;
            AgentAction::ScrollUp
        }
        KeyCode::PageDown => AgentAction::ScrollDown,
        _ => AgentAction::None,
    }
}

fn handle_transcript_key(state: &mut AgentViewState, key: KeyEvent) -> AgentAction {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down | KeyCode::PageDown => AgentAction::ScrollDown,
        KeyCode::Char('k') | KeyCode::Up | KeyCode::PageUp => {
            state.user_scrolled = true;
            AgentAction::ScrollUp
        }
        KeyCode::Tab | KeyCode::Char('i') => {
            state.focus = AgentFocus::Input;
            AgentAction::None
        }
        _ => AgentAction::None,
    }
}

fn insert_char(buf: &mut String, cursor: &mut usize, ch: char) {
    buf.insert(*cursor, ch);
    *cursor += ch.len_utf8();
}

fn delete_before(buf: &mut String, cursor: &mut usize) {
    if *cursor == 0 {
        return;
    }
    let new_cursor = prev_char_boundary(buf, *cursor);
    buf.replace_range(new_cursor..*cursor, "");
    *cursor = new_cursor;
}

fn prev_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.saturating_sub(1);
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn next_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx + 1;
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i.min(s.len())
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &AgentViewState,
    conversation: &Conversation,
    providers: &[ProviderConfig],
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(4),
        ])
        .split(area);

    render_header(frame, chunks[0], state, providers, theme);
    render_transcript(frame, chunks[1], state, conversation, theme);
    render_input_bar(frame, chunks[2], state, theme);
}

fn render_header(
    frame: &mut Frame,
    area: Rect,
    state: &AgentViewState,
    providers: &[ProviderConfig],
    theme: &Theme,
) {
    let active = state
        .selected_provider_id
        .as_deref()
        .and_then(|id| providers.iter().find(|p| p.id == id));

    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" "));

    match active {
        Some(p) => {
            spans.push(Span::styled(
                p.display_name.clone(),
                Style::default().fg(theme.brand).add_modifier(Modifier::BOLD),
            ));
            let model = state
                .selected_model
                .clone()
                .unwrap_or_else(|| p.default_model.clone());
            spans.push(Span::styled("  ", Style::default()));
            spans.push(Span::styled(model, Style::default().fg(theme.accent)));
        }
        None => {
            spans.push(Span::styled(
                "No provider configured — press `:provider add` or Ctrl+P",
                Style::default().fg(theme.status_warning),
            ));
        }
    }

    spans.push(Span::raw("   "));
    spans.push(Span::styled(turn_status(state), Style::default().fg(theme.fg_dim)));

    if let Some(err) = &state.last_error {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("error: {}", truncate(err, 80)),
            Style::default().fg(theme.status_warning),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn turn_status(state: &AgentViewState) -> String {
    if !state.running {
        "idle".to_string()
    } else if let Some(tool) = &state.current_tool {
        format!("tool: {}", tool)
    } else {
        "thinking...".to_string()
    }
}

fn render_transcript(
    frame: &mut Frame,
    area: Rect,
    state: &AgentViewState,
    conversation: &Conversation,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    if conversation.messages.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  No messages yet. Type below and press Enter to start.",
            Style::default().fg(theme.fg_muted),
        )));
    } else {
        for (idx, message) in conversation.messages.iter().enumerate() {
            if idx > 0 {
                lines.push(Line::from(""));
            }
            append_message_lines(&mut lines, message, theme);
        }
    }

    // Auto-scroll to bottom while a turn is streaming, unless the user manually
    // scrolled up — honour `state.scroll_offset` in that case.
    let total = lines.len() as u16;
    let viewport = inner.height;
    let effective_scroll = if state.running && !state.user_scrolled {
        total.saturating_sub(viewport)
    } else {
        state.scroll_offset.min(total.saturating_sub(1).max(0))
    };

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((effective_scroll, 0));
    frame.render_widget(para, inner);
}

fn append_message_lines(lines: &mut Vec<Line<'static>>, message: &Message, theme: &Theme) {
    lines.push(role_header(message.role, theme));
    for part in &message.parts {
        match part {
            MessagePart::Text { text } => append_text_part(lines, text, message.role, theme),
            MessagePart::Reasoning { text } => append_reasoning_part(lines, text, theme),
            MessagePart::ToolCall(call) => append_tool_call(lines, call, theme),
            MessagePart::ToolResult(result) => append_tool_result(lines, result, theme),
        }
    }
}

fn role_header(role: Role, theme: &Theme) -> Line<'static> {
    let (label, color) = match role {
        Role::User => ("you", theme.accent),
        Role::Assistant => ("assistant", theme.brand),
        Role::System => ("system", theme.fg_dim),
    };
    Line::from(vec![
        Span::styled(
            format!("  {}", label),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

fn append_text_part(lines: &mut Vec<Line<'static>>, text: &str, role: Role, theme: &Theme) {
    let color = match role {
        Role::User => theme.accent,
        Role::Assistant | Role::System => theme.fg,
    };
    for line in text.lines() {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(line.to_string(), Style::default().fg(color)),
        ]));
    }
}

fn append_reasoning_part(lines: &mut Vec<Line<'static>>, text: &str, theme: &Theme) {
    let style = Style::default().fg(theme.fg_faint).add_modifier(Modifier::ITALIC);
    let preview: Vec<&str> = text.lines().take(REASONING_PREVIEW_LINES).collect();
    let remaining = text.lines().count().saturating_sub(preview.len());

    for (i, line) in preview.iter().enumerate() {
        let mut spans = vec![Span::raw("    ")];
        if i == 0 {
            spans.push(Span::styled("thinking: ", style));
        } else {
            spans.push(Span::styled("           ", style));
        }
        spans.push(Span::styled(line.to_string(), style));
        lines.push(Line::from(spans));
    }
    if remaining > 0 {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled("           ...", style),
        ]));
    }
}

fn append_tool_call(lines: &mut Vec<Line<'static>>, call: &ToolCall, theme: &Theme) {
    let summary = summarize_tool_input(&call.input);
    lines.push(Line::from(vec![
        Span::raw("    "),
        Span::styled(
            format!("⏺ {}({})", call.tool_name, summary),
            Style::default().fg(theme.accent),
        ),
    ]));
}

fn append_tool_result(lines: &mut Vec<Line<'static>>, result: &ToolResult, theme: &Theme) {
    let color = if result.is_error {
        theme.status_warning
    } else {
        theme.fg_dim
    };
    let content_lines: Vec<&str> = result.content.lines().collect();
    let shown = content_lines
        .iter()
        .take(TOOL_RESULT_PREVIEW_LINES)
        .copied()
        .collect::<Vec<_>>();
    let extra = content_lines.len().saturating_sub(shown.len());

    for line in shown {
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(line.to_string(), Style::default().fg(color)),
        ]));
    }
    if extra > 0 {
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(
                format!("({} more lines)", extra),
                Style::default().fg(theme.fg_muted),
            ),
        ]));
    }
}

fn summarize_tool_input(value: &serde_json::Value) -> String {
    let s = match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| format!("{}={}", k, compact_scalar(v)))
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    };
    truncate(&s, TOOL_INPUT_SUMMARY_MAX)
}

fn compact_scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => {
            let one_line = s.replace(['\n', '\r'], " ");
            truncate(&one_line, 32)
        }
        other => truncate(&other.to_string(), 32),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn render_input_bar(frame: &mut Frame, area: Rect, state: &AgentViewState, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1)])
        .split(area);

    let border_color = if state.focus == AgentFocus::Input {
        theme.brand
    } else {
        theme.border
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(chunks[0]);
    frame.render_widget(block, chunks[0]);

    let (before, after) = split_at_cursor(&state.input, state.cursor);
    let cursor_style = if state.focus == AgentFocus::Input {
        Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_dim)
    };

    let line = Line::from(vec![
        Span::styled("> ", Style::default().fg(theme.fg_dim)),
        Span::styled(before.to_string(), Style::default().fg(theme.fg)),
        Span::styled("│", cursor_style),
        Span::styled(after.to_string(), Style::default().fg(theme.fg)),
    ]);
    frame.render_widget(Paragraph::new(line), inner);

    let hint = Line::from(Span::styled(
        "  Enter: send   Ctrl+L: clear   Esc: return to Flight overlay (TBD)   Tab: focus transcript",
        Style::default().fg(theme.fg_muted),
    ));
    frame.render_widget(Paragraph::new(hint), chunks[1]);
}

fn split_at_cursor(s: &str, cursor: usize) -> (&str, &str) {
    let clamped = cursor.min(s.len());
    if !s.is_char_boundary(clamped) {
        return (s, "");
    }
    s.split_at(clamped)
}
