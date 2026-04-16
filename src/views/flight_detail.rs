use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::core::flight::*;
use super::super::app::App;
use super::super::theme::{Theme, format_cost, format_tokens};

pub fn render(frame: &mut Frame, area: Rect, app: &App, flight_id: &str, theme: &Theme) {
    let flight = match app.flights.iter().find(|f| f.id == *flight_id) {
        Some(f) => f,
        None => {
            let msg = Paragraph::new("Flight not found. Press Esc to go back.")
                .style(Style::default().fg(theme.status_failed));
            frame.render_widget(msg, area);
            return;
        }
    };

    let (done, total) = flight.progress();
    let running = app.orchestrator.running_tasks_for_flight(&flight.id).len();
    let is_active = app.orchestrator.active_flight_ids.contains(&flight.id);
    let paused_at = app.orchestrator.paused_at_milestone.get(&flight.id);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // header
            Constraint::Length(3),  // controls
            Constraint::Min(0),    // milestones
        ])
        .split(area);

    // Header
    let status_color = match flight.status {
        FlightStatus::Active => theme.status_active,
        FlightStatus::Done => theme.status_done,
        FlightStatus::Failed => theme.status_failed,
        FlightStatus::Paused => theme.status_paused,
        FlightStatus::Review => theme.status_review,
        _ => theme.fg_dim,
    };

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("  ● ", Style::default().fg(status_color)),
            Span::styled(&flight.title, Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  [{}]", flight.status.label()), Style::default().fg(status_color)),
            Span::styled(format!("  {}", flight.priority.label()), Style::default().fg(theme.fg_dim)),
        ]),
        Line::from(vec![
            Span::styled("    ", Style::default()),
            Span::styled(
                if flight.objective.len() > 80 { format!("{}…", &flight.objective[..79]) } else { flight.objective.clone() },
                Style::default().fg(theme.fg_dim)
            ),
        ]),
        Line::from(vec![
            Span::styled(format!("    {}/{} tasks", done, total), Style::default().fg(theme.fg_dim)),
            Span::styled(format!("  │  {} ms", flight.milestones.len()), Style::default().fg(theme.fg_dim)),
            Span::styled(
                format!("  │  branch:{}", flight.git_branch.as_deref().unwrap_or("-")),
                Style::default().fg(theme.fg_dim),
            ),
            if running > 0 {
                Span::styled(format!("  │  {} agent(s) running", running), Style::default().fg(theme.status_active))
            } else {
                Span::raw("")
            },
            if flight.total_cost > 0.0 {
                Span::styled(
                    format!("  │  cost:{}  tok:{}", format_cost(flight.total_cost), format_tokens(flight.total_tokens)),
                    Style::default().fg(theme.status_info),
                )
            } else {
                Span::raw("")
            },
        ]),
    ])
    .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(theme.border)));
    frame.render_widget(header, layout[0]);

    // Controls
    let mut ctrl_spans = vec![Span::styled("  ", Style::default())];
    if !is_active && flight.status != FlightStatus::Done && flight.status != FlightStatus::Cancelled {
        ctrl_spans.push(Span::styled(" l:Launch ", Style::default().fg(theme.brand).bg(theme.bg_highlight)));
    }
    if is_active {
        ctrl_spans.push(Span::styled(" p:Pause ", Style::default().fg(theme.status_paused).bg(theme.bg_highlight)));
        ctrl_spans.push(Span::styled(" c:Cancel ", Style::default().fg(theme.status_failed).bg(theme.bg_highlight)));
    }
    if paused_at.is_some() {
        ctrl_spans.push(Span::styled(" l:Continue to next milestone ", Style::default().fg(theme.brand).bg(theme.bg_highlight)));
        ctrl_spans.push(Span::styled(" ⚠ Milestone complete — review before continuing ", Style::default().fg(theme.status_warning)));
    }
    if flight
        .milestones
        .iter()
        .flat_map(|ms| ms.tasks.iter())
        .any(|task| task.status == TaskStatus::ApprovalNeeded)
    {
        ctrl_spans.push(Span::styled(" y:Approve ", Style::default().fg(theme.brand).bg(theme.bg_highlight)));
        ctrl_spans.push(Span::styled(" n:Deny ", Style::default().fg(theme.status_paused).bg(theme.bg_highlight)));
        ctrl_spans.push(Span::styled(" a:Abort ", Style::default().fg(theme.status_failed).bg(theme.bg_highlight)));
    }
    ctrl_spans.push(Span::styled("  e:Edit", Style::default().fg(theme.fg_dim)));
    ctrl_spans.push(Span::styled("  s:Sessions", Style::default().fg(theme.fg_dim)));
    ctrl_spans.push(Span::styled("  Esc:Back", Style::default().fg(theme.fg_dim)));

    let controls = Paragraph::new(Line::from(ctrl_spans));
    frame.render_widget(controls, layout[1]);

    // Milestones & Tasks
    let mut lines: Vec<Line> = Vec::new();

    if flight.milestones.is_empty() {
        lines.push(Line::from(Span::styled("  No milestones yet", Style::default().fg(theme.fg_dim))));
    }

    for (mi, ms) in flight.milestones.iter().enumerate() {
        let ms_color = match ms.status {
            MilestoneStatus::Active => theme.status_active,
            MilestoneStatus::Done => theme.status_done,
            MilestoneStatus::Failed => theme.status_failed,
            MilestoneStatus::Pending => theme.fg_dim,
        };

        let ms_done = ms.tasks.iter().filter(|t| t.status == TaskStatus::Done).count();
        lines.push(Line::from(vec![
            Span::styled(format!("  M{} ", mi + 1), Style::default().fg(theme.fg_dim)),
            Span::styled("● ", Style::default().fg(ms_color)),
            Span::styled(&ms.title, Style::default().fg(theme.fg)),
            Span::styled(format!("  [{}/{}]", ms_done, ms.tasks.len()), Style::default().fg(theme.fg_dim)),
            Span::styled(format!("  {}", ms.status.label()), Style::default().fg(ms_color)),
        ]));

        for (ti, task) in ms.tasks.iter().enumerate() {
            let task_color = match task.status {
                TaskStatus::Running => theme.status_active,
                TaskStatus::Done => theme.status_done,
                TaskStatus::Failed => theme.status_failed,
                TaskStatus::ApprovalNeeded => theme.status_warning,
                TaskStatus::Queued => theme.status_info,
                TaskStatus::Paused => theme.status_paused,
                _ => theme.fg_dim,
            };

            let agent_name = app.agents.iter()
                .find(|a| a.id == task.agent_config_id)
                .map(|a| a.name.as_str())
                .unwrap_or("?");

            let runtime = task
                .session_id
                .as_ref()
                .and_then(|session_id| app.session_buffers.get(session_id));

            lines.push(Line::from(vec![
                Span::styled(format!("    T{} ", ti + 1), Style::default().fg(theme.fg_muted)),
                Span::styled("● ", Style::default().fg(task_color)),
                Span::styled(
                    if task.title.len() > 45 { format!("{}…", &task.title[..44]) } else { task.title.clone() },
                    Style::default().fg(theme.fg)
                ),
                Span::styled(format!("  {}", task.task_type.label()), Style::default().fg(theme.fg_muted)),
                Span::styled(format!("  {}", agent_name), Style::default().fg(theme.accent)),
                Span::styled(format!("  {}", task.status.label()), Style::default().fg(task_color)),
                if let Some(runtime) = runtime {
                    Span::styled(
                        format!("  {}{}", runtime.runtime_label(), runtime.current_tool.as_ref().map(|tool| format!(":{tool}")).unwrap_or_default()),
                        Style::default().fg(if runtime.needs_approval { theme.status_warning } else { theme.fg_muted }),
                    )
                } else {
                    Span::raw("")
                },
            ]));
        }

        lines.push(Line::from(""));
    }

    // Retrospective section (if flight is done/failed and a retrospective exists)
    if matches!(flight.status, FlightStatus::Done | FlightStatus::Failed) {
        if let Some(retro) = app.retrospectives.iter().find(|r| r.flight_id == flight.id) {
            lines.push(Line::from(vec![
                Span::styled("  ─── Retrospective ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                Span::styled("───", Style::default().fg(theme.border)),
            ]));
            lines.push(Line::from(Span::styled(
                format!("  {}", retro.summary),
                Style::default().fg(theme.fg),
            )));

            if !retro.lessons_learned.is_empty() {
                lines.push(Line::from(Span::styled("  Lessons:", Style::default().fg(theme.status_info))));
                for lesson in &retro.lessons_learned {
                    lines.push(Line::from(Span::styled(
                        format!("    • {}", lesson),
                        Style::default().fg(theme.fg_dim),
                    )));
                }
            }

            if !retro.what_worked.is_empty() {
                lines.push(Line::from(Span::styled("  What worked:", Style::default().fg(theme.status_done))));
                for item in &retro.what_worked {
                    lines.push(Line::from(Span::styled(
                        format!("    ✓ {}", item),
                        Style::default().fg(theme.fg_dim),
                    )));
                }
            }

            if !retro.what_failed.is_empty() {
                lines.push(Line::from(Span::styled("  What failed:", Style::default().fg(theme.status_failed))));
                for item in &retro.what_failed {
                    lines.push(Line::from(Span::styled(
                        format!("    ✗ {}", item),
                        Style::default().fg(theme.fg_dim),
                    )));
                }
            }

            lines.push(Line::from(""));
        }
    }

    let milestones = Paragraph::new(lines)
        .scroll((0, 0));
    frame.render_widget(milestones, layout[2]);
}
