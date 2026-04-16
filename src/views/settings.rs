use ratatui::prelude::*;
use ratatui::widgets::*;

use super::super::app::App;
use super::super::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled(" Settings ", Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));

    let max = app.settings.max_parallel_sessions;
    let gating = app.settings.milestone_gating;
    let project_path = &app.settings.project_path;
    let git_branch = app.git_branch.as_deref().unwrap_or("-");
    let git_status = app.git_status_summary.as_deref().unwrap_or("unknown");
    let git_message = app.git_last_message.as_deref().unwrap_or("No git action run yet.");
    let input_line = app
        .settings_input
        .as_ref()
        .map(|input| format!("  Input: {}", input.value))
        .unwrap_or_default();

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Project Path:         ", Style::default().fg(theme.fg)),
            Span::styled(project_path, Style::default().fg(theme.status_info)),
        ]),
        Line::from(vec![
            Span::styled("                        ", Style::default()),
            Span::styled("(e edit, c current cwd)", Style::default().fg(theme.fg_muted)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Git Branch:           ", Style::default().fg(theme.fg)),
            Span::styled(git_branch, Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)),
            Span::styled(format!("    status: {}", git_status), Style::default().fg(theme.fg_dim)),
        ]),
        Line::from(vec![
            Span::styled("                        ", Style::default()),
            Span::styled("(g refresh, C commit-all, L pull, P push)", Style::default().fg(theme.fg_muted)),
        ]),
        Line::from(Span::styled(format!("  {}", git_message), Style::default().fg(theme.fg_muted))),
        if input_line.is_empty() {
            Line::from("")
        } else {
            Line::from(Span::styled(input_line, Style::default().fg(theme.status_warning)))
        },
        Line::from(""),
        Line::from(vec![
            Span::styled("  Max Parallel Sessions: ", Style::default().fg(theme.fg)),
            Span::styled("◀ ", Style::default().fg(theme.fg_dim)),
            Span::styled(
                format!(" {} ", max),
                Style::default().fg(theme.brand).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ▶", Style::default().fg(theme.fg_dim)),
            Span::styled("    (←/→ to adjust)", Style::default().fg(theme.fg_muted)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Milestone Gating:      ", Style::default().fg(theme.fg)),
            if gating {
                Span::styled(" ON  ", Style::default().fg(theme.brand).add_modifier(Modifier::BOLD))
            } else {
                Span::styled(" OFF ", Style::default().fg(theme.fg_dim))
            },
            Span::styled("    (m to toggle)", Style::default().fg(theme.fg_muted)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "    Pause between milestones for human review before continuing.",
            Style::default().fg(theme.fg_muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Theme:                 ", Style::default().fg(theme.fg)),
            Span::styled(&theme.name, Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)),
            Span::styled("    (t to cycle)", Style::default().fg(theme.fg_muted)),
        ]),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "  PacketCode v0.1.0 — Provider-agnostic agent orchestration",
            Style::default().fg(theme.fg_faint),
        )),
    ];

    let para = Paragraph::new(text).block(block);
    frame.render_widget(para, area);
}
