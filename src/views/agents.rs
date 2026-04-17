use ratatui::prelude::*;
use ratatui::widgets::*;

use super::super::app::App;
use super::super::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let items: Vec<ListItem> = app.agents.iter().enumerate().map(|(idx, agent)| {
        let installed_indicator = if agent.installed {
            Span::styled(" ✓ ", Style::default().fg(theme.status_done))
        } else {
            Span::styled(" ✗ ", Style::default().fg(theme.fg_dim))
        };

        let name_style = if idx == app.selected_agent_idx {
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg)
        };

        let line = Line::from(vec![
            Span::raw("  "),
            installed_indicator,
            Span::styled(format!("{:<20}", agent.name), name_style),
            Span::styled(format!("  {:<12}", agent.display_command()), Style::default().fg(theme.accent)),
            Span::styled(
                &agent.description,
                Style::default().fg(theme.fg_dim),
            ),
            if agent.is_builtin {
                Span::styled("  [built-in]", Style::default().fg(theme.fg_faint))
            } else {
                Span::styled("  [custom]", Style::default().fg(theme.fg_faint))
            },
        ]);

        ListItem::new(line)
    }).collect();

    let block = Block::default()
        .title(Span::styled(" Agents ", Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)))
        .title_bottom(Span::styled(" r:refresh detection  j/k:navigate ", Style::default().fg(theme.fg_dim)))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(theme.bg_highlight))
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    state.select(Some(app.selected_agent_idx));

    frame.render_stateful_widget(list, area, &mut state);
}
