use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::prelude::*;
use ratatui::widgets::*;

use super::super::app::{App, format_elapsed};
use super::super::theme::Theme;
use super::super::widgets::markdown;

pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    render_session_list(frame, layout[0], app, theme);
    render_transcript(frame, layout[1], app, theme);
}

fn render_session_list(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    // Split area for filter bar if active
    let (filter_area, list_area) = if app.session_filter.is_some() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, area)
    };

    // Render filter bar
    if let Some(filter_area) = filter_area {
        let filter_text = app.session_filter.as_deref().unwrap_or("");
        let cursor = if app.session_filter_input { "│" } else { "" };
        let line = Line::from(vec![
            Span::styled(" / ", Style::default().fg(theme.brand)),
            Span::styled(filter_text, Style::default().fg(theme.fg)),
            Span::styled(cursor, Style::default().fg(theme.fg_dim)),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(theme.bg_highlight)),
            filter_area,
        );
    }

    // Filter sessions
    let matcher = SkimMatcherV2::default();
    let filtered_sessions: Vec<(usize, &str)> = if let Some(ref filter) = app.session_filter {
        if filter.is_empty() {
            app.session_order.iter().enumerate().map(|(i, id)| (i, id.as_str())).collect()
        } else {
            app.session_order
                .iter()
                .enumerate()
                .filter(|(_, session_id)| {
                    let session = app.session_buffers.get(session_id.as_str());
                    if let Some(session) = session {
                        let flight_title = app
                            .flights
                            .iter()
                            .find(|f| f.id == session.flight_id)
                            .map(|f| f.title.as_str())
                            .unwrap_or("");
                        let haystack = format!(
                            "{} {} {}",
                            session.title, session.agent_config_id, flight_title
                        );
                        matcher.fuzzy_match(&haystack, filter).is_some()
                    } else {
                        false
                    }
                })
                .map(|(i, id)| (i, id.as_str()))
                .collect()
        }
    } else {
        app.session_order.iter().enumerate().map(|(i, id)| (i, id.as_str())).collect()
    };

    let items: Vec<ListItem> = if filtered_sessions.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            if app.session_filter.is_some() {
                "  No matching sessions"
            } else {
                "  No sessions yet"
            },
            Style::default().fg(theme.fg_dim),
        )]))]
    } else {
        filtered_sessions
            .iter()
            .map(|(idx, session_id)| {
                let session = app.session_buffers.get(*session_id).expect("session buffer should exist");

                let flight_title = app
                    .flights
                    .iter()
                    .find(|flight| flight.id == session.flight_id)
                    .map(|flight| flight.title.as_str())
                    .unwrap_or("Unknown flight");

                let status_color = if !session.exited {
                    theme.status_active
                } else if session.killed {
                    theme.status_warning
                } else if session.success.unwrap_or(false) {
                    theme.status_done
                } else {
                    theme.status_failed
                };

                // Agent CLI badge color
                let agent_color = agent_badge_color(&session.agent_config_id);

                let mut spans = vec![
                    Span::styled("  ● ", Style::default().fg(status_color)),
                    Span::styled(
                        truncate(&session.title, 24),
                        if *idx == app.selected_session_idx {
                            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme.fg)
                        },
                    ),
                    Span::styled(
                        format!("  {}", agent_badge_label(&session.agent_config_id)),
                        Style::default().fg(agent_color),
                    ),
                    Span::styled(
                        format!("  {}", session.status_label()),
                        Style::default().fg(status_color),
                    ),
                    Span::styled(
                        format!("  {}", session.runtime_label()),
                        Style::default().fg(if session.needs_approval { theme.status_warning } else { theme.fg_dim }),
                    ),
                    Span::styled(
                        format!("  #{}", &session.session_id[..8]),
                        Style::default().fg(theme.fg_dim),
                    ),
                ];

                // Elapsed time
                if !session.exited {
                    spans.push(Span::styled(
                        format!("  {}", format_elapsed(session.started_at)),
                        Style::default().fg(theme.fg_muted),
                    ));
                }

                if session.unread_count > 0 {
                    spans.push(Span::styled(
                        format!("  +{}", session.unread_count),
                        Style::default().fg(theme.status_warning),
                    ));
                }

                if session.doom_loop_detected {
                    spans.push(Span::styled(
                        "  ⚠loop",
                        Style::default().fg(theme.status_failed),
                    ));
                }

                ListItem::new(Text::from(vec![
                    Line::from(spans),
                    Line::from(vec![Span::styled(
                        format!("     {}", truncate(flight_title, 28)),
                        Style::default().fg(theme.fg_dim),
                    )]),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .title(Span::styled(
                    " Sessions ",
                    Style::default().fg(theme.brand).add_modifier(Modifier::BOLD),
                ))
                .title_bottom(Span::styled(
                    " j/k:nav  x:kill  /:filter  ?:search  D:diff ",
                    Style::default().fg(theme.fg_dim),
                ))
                .borders(Borders::TOP | Borders::RIGHT),
        )
        .highlight_style(Style::default().bg(theme.bg_highlight))
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    if !filtered_sessions.is_empty() {
        state.select(Some(
            filtered_sessions
                .iter()
                .position(|(i, _)| *i == app.selected_session_idx)
                .unwrap_or(0),
        ));
    }

    frame.render_stateful_widget(list, list_area, &mut state);
}

fn render_transcript(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    // Split area for search bar at bottom if search is active
    let (transcript_area, search_area) = if app.session_search.is_some() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    // Render search bar
    if let Some(search_area) = search_area {
        let search_text = app.session_search.as_deref().unwrap_or("");
        let cursor = if app.session_search_input { "│" } else { "" };
        let match_info = if app.session_search_matches.is_empty() {
            if search_text.is_empty() {
                String::new()
            } else {
                " [no matches]".to_string()
            }
        } else {
            format!(
                " [{}/{}]",
                app.session_search_index + 1,
                app.session_search_matches.len()
            )
        };
        let line = Line::from(vec![
            Span::styled(" ? ", Style::default().fg(theme.brand)),
            Span::styled(search_text, Style::default().fg(theme.fg)),
            Span::styled(cursor, Style::default().fg(theme.fg_dim)),
            Span::styled(match_info, Style::default().fg(theme.fg_dim)),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(theme.bg_highlight)),
            search_area,
        );
    }

    let Some(session_id) = app.session_order.get(app.selected_session_idx) else {
        let empty = Paragraph::new("No session selected")
            .block(Block::default().title(" Transcript ").borders(Borders::TOP));
        frame.render_widget(empty, transcript_area);
        return;
    };

    let Some(session) = app.session_buffers.get(session_id) else {
        return;
    };

    let agent_name = app
        .agents
        .iter()
        .find(|agent| agent.id == session.agent_config_id)
        .map(|agent| agent.name.as_str())
        .unwrap_or("Unknown agent");

    let header = format!(
        " {} [{}] {} {} #{} ",
        session.title,
        agent_name,
        session.status_label(),
        session.runtime_label(),
        &session.task_id[..8]
    );

    let block = Block::default()
        .title(Span::styled(
            header,
            Style::default().fg(theme.brand).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            format!(
                " {}  tool:{}  file:{}  approval:{}  PgUp/PgDn:scroll  g/G:top/bottom  ?:search ",
                session.project_path,
                session.current_tool.as_deref().unwrap_or("-"),
                session.current_file.as_deref().unwrap_or("-"),
                if session.needs_approval { "pending" } else { "no" }
            ),
            Style::default().fg(theme.fg_dim),
        ))
        .borders(Borders::TOP);

    if session.output.is_empty() {
        let transcript = Paragraph::new("\n  Waiting for output...")
            .block(block)
            .style(Style::default().fg(theme.fg));
        frame.render_widget(transcript, transcript_area);
        return;
    }

    // Build lines with search highlighting
    let search_query = app.session_search.as_deref().unwrap_or("");
    let has_search = !search_query.is_empty();

    if has_search {
        let highlight_style = Style::default().bg(Color::Yellow).fg(Color::Black);
        let current_match_style = Style::default().bg(Color::Rgb(255, 165, 0)).fg(Color::Black);
        let query_lower = search_query.to_lowercase();
        let mut lines: Vec<Line> = Vec::new();

        for (line_idx, raw_line) in session.output.lines().enumerate() {
            let is_match_line = app.session_search_matches.contains(&line_idx);
            let is_current = is_match_line
                && !app.session_search_matches.is_empty()
                && app.session_search_matches.get(app.session_search_index) == Some(&line_idx);

            if is_match_line {
                // Highlight matching substrings
                let mut spans: Vec<Span> = Vec::new();
                let line_lower = raw_line.to_lowercase();
                let mut pos = 0;
                while let Some(found) = line_lower[pos..].find(&query_lower) {
                    let abs_start = pos + found;
                    let abs_end = abs_start + query_lower.len();
                    if abs_start > pos {
                        spans.push(Span::styled(
                            raw_line[pos..abs_start].to_string(),
                            Style::default().fg(theme.fg),
                        ));
                    }
                    spans.push(Span::styled(
                        raw_line[abs_start..abs_end].to_string(),
                        if is_current { current_match_style } else { highlight_style },
                    ));
                    pos = abs_end;
                }
                if pos < raw_line.len() {
                    spans.push(Span::styled(
                        raw_line[pos..].to_string(),
                        Style::default().fg(theme.fg),
                    ));
                }
                lines.push(Line::from(spans));
            } else {
                lines.push(Line::from(Span::styled(
                    raw_line.to_string(),
                    Style::default().fg(theme.fg),
                )));
            }
        }

        let transcript = Paragraph::new(lines)
            .block(block)
            .scroll((session.scroll, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(transcript, transcript_area);
    } else {
        // No search — use markdown rendering as before
        let lines = markdown::markdown_to_lines(&session.output, theme);

        let transcript = Paragraph::new(lines)
            .block(block)
            .scroll((session.scroll, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(transcript, transcript_area);
    }
}

fn truncate(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        format!("{value:<width$}", width = max_len)
    } else {
        let truncated: String = value.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}

/// Color badge for CLI agent type in session list.
fn agent_badge_color(agent_config_id: &str) -> Color {
    match agent_config_id {
        "claude-code" => Color::Rgb(240, 180, 0),   // amber
        "codex" => Color::Rgb(88, 166, 255),         // blue
        "gemini" => Color::Rgb(138, 180, 248),       // light blue
        "opencode" => Color::Rgb(63, 185, 80),       // green
        _ => Color::Rgb(139, 148, 158),              // muted
    }
}

/// Short label for CLI agent type in session list.
fn agent_badge_label(agent_config_id: &str) -> &str {
    match agent_config_id {
        "claude-code" => "Claude",
        "codex" => "Codex",
        "gemini" => "Gemini",
        "opencode" => "OpenCode",
        _ => "Agent",
    }
}
