use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::core::flight::*;
use super::super::app::{App, AttentionKind, DashboardFocus};
use super::super::theme::{format_cost, format_tokens};
use super::super::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // status strip
            Constraint::Min(0),    // body
        ])
        .split(area);

    render_status_strip(frame, layout[0], app, theme);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(64), Constraint::Percentage(36)])
        .split(layout[1]);

    render_flight_list(frame, body[0], app, theme);
    render_attention_queue(frame, body[1], app, theme);
}

fn render_status_strip(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let active = app.flights.iter().filter(|f| f.status == FlightStatus::Active).count();
    let draft = app.flights.iter().filter(|f| f.status == FlightStatus::Draft).count();
    let done = app.flights.iter().filter(|f| f.status == FlightStatus::Done).count();
    let failed = app.flights.iter().filter(|f| f.status == FlightStatus::Failed).count();
    let running_agents = app.orchestrator.running_tasks.len();
    let attention = app.flights.iter().filter(|f| f.needs_attention()).count();
    let sessions = app.session_order.len();
    let branch = app.git_branch.as_deref().unwrap_or("no-git");
    let git_status = app.git_status_summary.as_deref().unwrap_or("unknown");
    let total_cost: f64 = app.flights.iter().map(|f| f.total_cost).sum();
    let total_tokens: u64 = app.flights.iter().map(|f| f.total_tokens).sum();

    let text = Line::from(vec![
        Span::styled(format!(" ● {} active ", active), Style::default().fg(theme.status_active)),
        Span::raw("│"),
        Span::styled(format!(" {} draft ", draft), Style::default().fg(theme.fg_dim)),
        Span::raw("│"),
        Span::styled(format!(" ✓ {} done ", done), Style::default().fg(theme.status_done)),
        if failed > 0 {
            Span::styled(format!("│ ✗ {} failed ", failed), Style::default().fg(theme.status_failed))
        } else {
            Span::raw("")
        },
        Span::raw("│"),
        Span::styled(
            format!(" {} agent(s) ", running_agents),
            if running_agents > 0 { Style::default().fg(theme.status_active) } else { Style::default().fg(theme.fg_dim) }
        ),
        Span::raw("│"),
        Span::styled(
            format!(" {} session(s) ", sessions),
            if sessions > 0 { Style::default().fg(theme.status_info) } else { Style::default().fg(theme.fg_dim) }
        ),
        Span::raw("│"),
        Span::styled(
            format!(" {} ({}) ", branch, git_status),
            Style::default().fg(theme.fg_dim),
        ),
        if total_cost > 0.0 {
            Span::styled(format!("│ {} ({} tok) ", format_cost(total_cost), format_tokens(total_tokens)), Style::default().fg(theme.status_info))
        } else {
            Span::raw("")
        },
        if attention > 0 {
            Span::styled(format!("│ ⚠ {} need attention ", attention), Style::default().fg(theme.status_warning))
        } else {
            Span::raw("")
        },
    ]);

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(theme.border));

    let para = Paragraph::new(text).block(block);
    frame.render_widget(para, area);
}

fn render_flight_list(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if app.flights.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled("  No flights yet", Style::default().fg(theme.fg_dim))),
            Line::from(""),
            Line::from(Span::styled("  Press c to create your first flight here in the TUI.", Style::default().fg(theme.fg_dim))),
        ]);
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = app.flights.iter().enumerate().map(|(idx, f)| {
        let (done, total) = f.progress();
        let status_color = match f.status {
            FlightStatus::Active => theme.status_active,
            FlightStatus::Done => theme.status_done,
            FlightStatus::Failed | FlightStatus::Cancelled => theme.status_failed,
            FlightStatus::Review => theme.status_review,
            FlightStatus::Paused => theme.status_paused,
            _ => theme.fg_dim,
        };

        let running = app.orchestrator.running_tasks_for_flight(&f.id).len();

        let mut spans = vec![
            Span::styled("  ● ", Style::default().fg(status_color)),
            Span::styled(
                format!("{:<40}", if f.title.len() > 38 { format!("{}…", &f.title[..37]) } else { f.title.clone() }),
                if idx == app.selected_flight_idx {
                    Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                }
            ),
            Span::styled(format!(" {:>8} ", f.status.label()), Style::default().fg(status_color)),
        ];

        if total > 0 {
            // Progress bar
            let pct = if total > 0 { (done * 10) / total } else { 0 };
            let filled = "█".repeat(pct);
            let empty = "░".repeat(10 - pct);
            spans.push(Span::styled(filled, Style::default().fg(theme.progress_filled)));
            spans.push(Span::styled(empty, Style::default().fg(theme.progress_empty)));
            spans.push(Span::styled(format!(" {}/{} ", done, total), Style::default().fg(theme.fg_dim)));
        }

        if running > 0 {
            spans.push(Span::styled(format!(" {}⚡", running), Style::default().fg(theme.status_active)));
        }

        if f.total_cost > 0.0 {
            spans.push(Span::styled(format!(" {}", format_cost(f.total_cost)), Style::default().fg(theme.fg_dim)));
        }

        let content = Line::from(spans);
        ListItem::new(content)
    }).collect();

    let title = if app.dashboard_focus == DashboardFocus::Flights {
        " Flights "
    } else {
        " Flights "
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(
                    title,
                    if app.dashboard_focus == DashboardFocus::Flights {
                        Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.fg_dim)
                    },
                ))
                .borders(Borders::RIGHT),
        )
        .highlight_style(Style::default().bg(theme.bg_highlight))
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    state.select(Some(app.selected_flight_idx));

    frame.render_stateful_widget(list, area, &mut state);
}

fn render_attention_queue(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let attention_items = app.attention_items();
    let items: Vec<ListItem> = if attention_items.is_empty() {
        vec![ListItem::new(Text::from(vec![
            Line::from(""),
            Line::from(Span::styled("  No attention items", Style::default().fg(theme.fg_dim))),
            Line::from(Span::styled("  Approvals, failures, reviews, and paused flights land here.", Style::default().fg(theme.fg_dim))),
        ]))]
    } else {
        attention_items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let color = match item.kind {
                    AttentionKind::Approval => theme.status_warning,
                    AttentionKind::FailedTask => theme.status_failed,
                    AttentionKind::FlightReview => theme.status_review,
                    AttentionKind::FlightPaused => theme.status_active,
                };

                let title = item
                    .task_title
                    .clone()
                    .unwrap_or_else(|| item.milestone_title.clone());

                ListItem::new(Text::from(vec![
                    Line::from(vec![
                        Span::styled("  ● ", Style::default().fg(color)),
                        Span::styled(
                            truncate(&title, 24),
                            if app.dashboard_focus == DashboardFocus::Attention && idx == app.selected_attention_idx {
                                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(theme.fg)
                            },
                        ),
                        Span::styled(format!("  {}", item.kind.label()), Style::default().fg(color)),
                    ]),
                    Line::from(Span::styled(
                        format!("    {}", truncate(&item.detail, 34)),
                        Style::default().fg(theme.fg_dim),
                    )),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(
                    " Attention ",
                    if app.dashboard_focus == DashboardFocus::Attention {
                        Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.fg_dim)
                    },
                ))
                .title_bottom(Span::styled(
                    " Tab:switch  Enter:open  l:jump ",
                    Style::default().fg(theme.fg_dim),
                )),
        )
        .highlight_style(Style::default().bg(theme.bg_highlight))
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    if !attention_items.is_empty() {
        state.select(Some(app.selected_attention_idx.min(attention_items.len().saturating_sub(1))));
    }

    frame.render_stateful_widget(list, area, &mut state);
}

fn truncate(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        value.to_string()
    } else {
        let truncated: String = value.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}
