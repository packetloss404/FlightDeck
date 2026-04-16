use ratatui::prelude::*;
use ratatui::widgets::*;

use super::super::app::{App, EditorFocus};
use super::super::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let Some(editor) = app.flight_editor.as_ref() else {
        let empty = Paragraph::new("Flight editor unavailable")
            .style(Style::default().fg(theme.status_failed));
        frame.render_widget(empty, area);
        return;
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    let title = if editor.original_flight_id.is_some() {
        " Edit Flight "
    } else {
        " New Flight "
    };

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(title, Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(" {}", if editor.draft.title.is_empty() { "Untitled" } else { &editor.draft.title }),
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(Span::styled(
            "  Tab switches panes. Enter/e edits, o edits descriptions, v edits milestone checks, [/] reorder, t cycles task type, g cycles task agent, m edits model, r edits deps.",
            Style::default().fg(theme.fg_dim),
        )),
    ])
    .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, layout[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(32), Constraint::Percentage(28), Constraint::Percentage(40)])
        .split(layout[1]);

    render_meta(frame, body[0], app, theme);
    render_milestones(frame, body[1], app, theme);
    render_tasks(frame, body[2], app, theme);

    let footer_text = if let Some(input) = editor.input.as_ref() {
        format!(" Editing: {}", input.value)
    } else if let Some(task) = editor
        .draft
        .milestones
        .get(editor.selected_milestone_idx)
        .and_then(|ms| ms.tasks.get(editor.selected_task_idx))
    {
        let dependency_numbers = editor
            .draft
            .milestones
            .get(editor.selected_milestone_idx)
            .map(|ms| format_dependency_numbers(ms, task))
            .unwrap_or_else(|| "-".to_string());
        let validation = editor
            .draft
            .milestones
            .get(editor.selected_milestone_idx)
            .map(|ms| format_validation_criteria(ms))
            .unwrap_or_else(|| "-".to_string());
        format!(
            " Task metadata │ agent:{} │ model:{} │ deps:{} │ checks:{} │ project:{}{}",
            task.agent_config_id,
            task.model.as_deref().unwrap_or("-"),
            dependency_numbers,
            truncate(&validation, 18),
            editor.draft.project_path,
            editor
                .draft
                .git_branch
                .as_ref()
                .map(|branch| format!("  │  Branch: {branch}"))
                .unwrap_or_default()
        )
    } else {
        format!(
            " Project: {}{}",
            editor.draft.project_path,
            editor.draft.git_branch.as_ref().map(|branch| format!("  │  Branch: {branch}")).unwrap_or_default()
        )
    };

    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(theme.fg_dim))
        .block(Block::default().borders(Borders::TOP));
    frame.render_widget(footer, layout[2]);
}

fn render_meta(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let editor = app.flight_editor.as_ref().unwrap();
    let rows = [
        ("Title", editor.draft.title.as_str()),
        ("Objective", editor.draft.objective.as_str()),
        ("Priority", editor.draft.priority.label()),
        ("Git Branch", editor.draft.git_branch.as_deref().unwrap_or("-")),
        ("Project Path", editor.draft.project_path.as_str()),
    ];

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(idx, (label, value))| {
            let style = if editor.focus == EditorFocus::Meta && editor.selected_meta_idx == idx {
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {:<12}", label), Style::default().fg(theme.fg_dim)),
                Span::styled(truncate(value, 28), style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block_with_focus(" Flight ", editor.focus == EditorFocus::Meta, theme))
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    state.select(Some(editor.selected_meta_idx));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_milestones(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let editor = app.flight_editor.as_ref().unwrap();
    let items: Vec<ListItem> = if editor.draft.milestones.is_empty() {
        vec![ListItem::new(Span::styled("  No milestones", Style::default().fg(theme.fg_dim)))]
    } else {
        editor
            .draft
            .milestones
            .iter()
            .enumerate()
            .map(|(idx, milestone)| {
                let selected = editor.focus == EditorFocus::Milestones && editor.selected_milestone_idx == idx;
                ListItem::new(Text::from(vec![
                    Line::from(vec![
                        Span::styled(
                            format!(" M{} ", idx + 1),
                            Style::default().fg(theme.fg_dim),
                        ),
                        Span::styled(
                            truncate(&milestone.title, 20),
                            if selected {
                                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(theme.fg)
                            },
                        ),
                    ]),
                    Line::from(Span::styled(
                        format!(
                            "    {} task(s)  │  checks:{}",
                            milestone.tasks.len(),
                            truncate(&format_validation_criteria(milestone), 18)
                        ),
                        Style::default().fg(theme.fg_dim),
                    )),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(block_with_focus(" Milestones ", editor.focus == EditorFocus::Milestones, theme))
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    if !editor.draft.milestones.is_empty() {
        state.select(Some(editor.selected_milestone_idx));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_tasks(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let editor = app.flight_editor.as_ref().unwrap();
    let tasks = editor
        .draft
        .milestones
        .get(editor.selected_milestone_idx)
        .map(|ms| ms.tasks.as_slice())
        .unwrap_or(&[]);

    let items: Vec<ListItem> = if tasks.is_empty() {
        vec![ListItem::new(Span::styled("  No tasks", Style::default().fg(theme.fg_dim)))]
    } else {
        tasks
            .iter()
            .enumerate()
            .map(|(idx, task)| {
                let selected = editor.focus == EditorFocus::Tasks && editor.selected_task_idx == idx;
                ListItem::new(Text::from(vec![
                    Line::from(vec![
                        Span::styled(
                            format!(" T{} ", idx + 1),
                            Style::default().fg(theme.fg_dim),
                        ),
                        Span::styled(
                            truncate(&task.title, 26),
                            if selected {
                                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(theme.fg)
                            },
                        ),
                        Span::styled(
                            format!("  {}", task.task_type.label()),
                            Style::default().fg(theme.fg_dim),
                        ),
                        Span::styled(
                            format!("  {}", truncate(&task.agent_config_id, 12)),
                            Style::default().fg(theme.accent),
                        ),
                    ]),
                    Line::from(Span::styled(
                        format!(
                            "    {}  │  model:{}  │  deps:{}",
                            truncate(&task.description, 18),
                            truncate(task.model.as_deref().unwrap_or("-"), 10),
                            editor
                                .draft
                                .milestones
                                .get(editor.selected_milestone_idx)
                                .map(|ms| format_dependency_numbers(ms, task))
                                .unwrap_or_else(|| "-".to_string())
                        ),
                        Style::default().fg(theme.fg_dim),
                    )),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(block_with_focus(" Tasks ", editor.focus == EditorFocus::Tasks, theme))
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    if !tasks.is_empty() {
        state.select(Some(editor.selected_task_idx));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn block_with_focus<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    Block::default()
        .title(Span::styled(
            title,
            if focused {
                Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.fg_dim)
            },
        ))
        .borders(Borders::ALL)
}

fn truncate(value: &str, max_len: usize) -> String {
    if value.is_empty() {
        return "-".to_string();
    }
    if value.chars().count() <= max_len {
        value.to_string()
    } else {
        let truncated: String = value.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}

fn format_dependency_numbers(ms: &crate::core::flight::Milestone, task: &crate::core::flight::Task) -> String {
    let mut numbers = Vec::new();
    for dep_id in &task.depends_on {
        if let Some(idx) = ms.tasks.iter().position(|candidate| candidate.id == *dep_id) {
            numbers.push((idx + 1).to_string());
        }
    }

    if numbers.is_empty() {
        "-".to_string()
    } else {
        numbers.join(",")
    }
}

fn format_validation_criteria(ms: &crate::core::flight::Milestone) -> String {
    if ms.validation_criteria.is_empty() {
        "-".to_string()
    } else {
        ms.validation_criteria.join(" | ")
    }
}
