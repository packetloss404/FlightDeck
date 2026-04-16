use ratatui::prelude::*;
use ratatui::widgets::*;

use super::super::theme::Theme;

pub struct HelpOverlay {
    pub visible: bool,
    pub scroll: u16,
}

impl HelpOverlay {
    pub fn new() -> Self {
        Self {
            visible: false,
            scroll: 0,
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        self.scroll = 0;
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme, view_name: &str) {
        if !self.visible {
            return;
        }

        let margin = 4u16;
        let overlay = Rect::new(
            area.x + margin,
            area.y + 1,
            area.width.saturating_sub(margin * 2),
            area.height.saturating_sub(2),
        );

        frame.render_widget(Clear, overlay);

        let block = Block::default()
            .title(Span::styled(
                " Keybindings ",
                Style::default().fg(theme.brand).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Span::styled(
                " ?:close  PgUp/PgDn:scroll ",
                Style::default().fg(theme.fg_dim),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.bg));

        let inner = block.inner(overlay);
        frame.render_widget(block, overlay);

        let mut lines: Vec<Line> = Vec::new();

        // Global section
        lines.push(section_header("Global", theme));
        lines.push(keybind("1 / 2 / 3 / 4", "Switch view: Dashboard / Sessions / Agents / Settings", theme));
        lines.push(keybind("Ctrl+P", "Open command palette", theme));
        lines.push(keybind("Ctrl+X", "Leader key (then: n=new, s=sessions, t=theme, g=git, p=palette)", theme));
        lines.push(keybind("q", "Quit", theme));
        lines.push(keybind("Esc", "Back / close overlay", theme));
        lines.push(keybind("?", "Toggle this help", theme));
        lines.push(Line::from(""));

        match view_name {
            "dashboard" => {
                lines.push(section_header("Dashboard", theme));
                lines.push(keybind("j / k", "Navigate flights", theme));
                lines.push(keybind("Tab", "Switch focus: Flights / Attention", theme));
                lines.push(keybind("Enter", "Open flight detail", theme));
                lines.push(keybind("c", "Create new flight", theme));
                lines.push(keybind("e", "Edit selected flight", theme));
                lines.push(keybind("l", "Launch selected flight", theme));
                lines.push(keybind("s", "Go to sessions", theme));
            }
            "flight_detail" => {
                lines.push(section_header("Flight Detail", theme));
                lines.push(keybind("l", "Launch / continue flight", theme));
                lines.push(keybind("p", "Pause flight", theme));
                lines.push(keybind("c", "Cancel flight", theme));
                lines.push(keybind("y / n / a", "Approve / Deny / Abort (approval)", theme));
                lines.push(keybind("e", "Edit flight", theme));
                lines.push(keybind("s", "Go to sessions", theme));
            }
            "flight_editor" => {
                lines.push(section_header("Flight Editor", theme));
                lines.push(keybind("Tab", "Cycle focus: Meta / Milestones / Tasks", theme));
                lines.push(keybind("Enter / e", "Edit field", theme));
                lines.push(keybind("o", "Edit description", theme));
                lines.push(keybind("a", "Add milestone or task", theme));
                lines.push(keybind("d", "Delete selected", theme));
                lines.push(keybind("t", "Cycle task type", theme));
                lines.push(keybind("g", "Cycle task agent", theme));
                lines.push(keybind("m", "Edit model override", theme));
                lines.push(keybind("r", "Edit task dependencies", theme));
                lines.push(keybind("p", "Cycle priority", theme));
                lines.push(keybind("v", "Edit milestone validation", theme));
                lines.push(keybind("[ / ]", "Reorder items", theme));
                lines.push(keybind("Shift+S", "Save flight", theme));
            }
            "sessions" => {
                lines.push(section_header("Sessions", theme));
                lines.push(keybind("j / k", "Navigate sessions", theme));
                lines.push(keybind("x", "Kill selected session", theme));
                lines.push(keybind("y / n / a", "Approve / Deny / Abort", theme));
                lines.push(keybind("/", "Filter sessions (fuzzy search)", theme));
                lines.push(keybind("D", "Open diff viewer", theme));
                lines.push(keybind("E", "Export session to markdown", theme));
                lines.push(keybind("g / G", "Scroll to top / bottom", theme));
                lines.push(keybind("PgUp / PgDn", "Scroll transcript", theme));
                lines.push(keybind("Ctrl+U / Ctrl+D", "Half-page scroll", theme));
                lines.push(keybind("Ctrl+Up/Down", "Navigate flight hierarchy", theme));
            }
            "agents" => {
                lines.push(section_header("Agents", theme));
                lines.push(keybind("j / k", "Navigate agents", theme));
                lines.push(keybind("r", "Refresh agent detection", theme));
            }
            "settings" => {
                lines.push(section_header("Settings", theme));
                lines.push(keybind("e", "Edit project path", theme));
                lines.push(keybind("c", "Set project path to cwd", theme));
                lines.push(keybind("Left / Right", "Adjust max parallel sessions", theme));
                lines.push(keybind("m", "Toggle milestone gating", theme));
                lines.push(keybind("t", "Cycle theme", theme));
                lines.push(keybind("g", "Refresh git status", theme));
                lines.push(keybind("C", "Git commit all", theme));
                lines.push(keybind("L", "Git pull", theme));
                lines.push(keybind("P", "Git push", theme));
            }
            _ => {}
        }

        let para = Paragraph::new(lines)
            .scroll((self.scroll, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(para, inner);
    }
}

fn section_header<'a>(title: &str, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("  {} ", title),
            Style::default()
                .fg(theme.brand)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
    ])
}

fn keybind<'a>(key: &str, desc: &str, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("    {:<20}", key), Style::default().fg(theme.status_info)),
        Span::styled(desc.to_string(), Style::default().fg(theme.fg)),
    ])
}
