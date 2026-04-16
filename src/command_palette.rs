use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::prelude::*;
use ratatui::widgets::*;

use super::theme::Theme;

#[derive(Debug, Clone)]
pub struct CommandEntry {
    pub id: &'static str,
    pub label: String,
    pub category: &'static str,
    pub shortcut: Option<&'static str>,
}

pub struct CommandPalette {
    pub visible: bool,
    pub input: String,
    pub cursor: usize,
    pub selected: usize,
    pub commands: Vec<CommandEntry>,
    pub filtered: Vec<(usize, i64, Vec<usize>)>, // (index, score, matched_indices)
    matcher: SkimMatcherV2,
}

impl CommandPalette {
    pub fn new() -> Self {
        Self {
            visible: false,
            input: String::new(),
            cursor: 0,
            selected: 0,
            commands: Vec::new(),
            filtered: Vec::new(),
            matcher: SkimMatcherV2::default(),
        }
    }

    pub fn open(&mut self, commands: Vec<CommandEntry>) {
        self.visible = true;
        self.input.clear();
        self.cursor = 0;
        self.selected = 0;
        self.commands = commands;
        self.refilter();
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.input.clear();
        self.commands.clear();
        self.filtered.clear();
    }

    pub fn selected_command_id(&self) -> Option<&'static str> {
        self.filtered
            .get(self.selected)
            .map(|(idx, _, _)| self.commands[*idx].id)
    }

    pub fn type_char(&mut self, ch: char) {
        self.input.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.selected = 0;
        self.refilter();
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.input[..self.cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.drain(prev..self.cursor);
            self.cursor = prev;
            self.selected = 0;
            self.refilter();
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    fn refilter(&mut self) {
        if self.input.is_empty() {
            self.filtered = self
                .commands
                .iter()
                .enumerate()
                .map(|(i, _)| (i, 0i64, Vec::new()))
                .collect();
        } else {
            let mut results: Vec<(usize, i64, Vec<usize>)> = self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(i, cmd)| {
                    let haystack = format!("{} {}", cmd.category, cmd.label);
                    self.matcher
                        .fuzzy_indices(&haystack, &self.input)
                        .map(|(score, indices)| (i, score, indices))
                })
                .collect();
            results.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = results;
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if !self.visible {
            return;
        }

        let width = 60u16.min(area.width.saturating_sub(4));
        let max_items = 15usize;
        let visible_items = self.filtered.len().min(max_items);
        let height = (visible_items as u16 + 3).min(area.height.saturating_sub(4)); // input + border + items

        let x = (area.width.saturating_sub(width)) / 2 + area.x;
        let y = area.height / 5 + area.y; // positioned in upper third
        let overlay = Rect::new(x, y, width, height);

        frame.render_widget(Clear, overlay);

        let block = Block::default()
            .title(Span::styled(
                " Commands ",
                Style::default().fg(theme.brand).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border))
            .style(Style::default().bg(theme.bg));

        let inner = block.inner(overlay);
        frame.render_widget(block, overlay);

        if inner.height < 2 {
            return;
        }

        // Input line
        let input_area = Rect::new(inner.x, inner.y, inner.width, 1);
        let input_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(theme.brand)),
            Span::styled(&self.input, Style::default().fg(theme.fg)),
            Span::styled("│", Style::default().fg(theme.fg_dim)),
        ]);
        frame.render_widget(Paragraph::new(input_line), input_area);

        // Items
        let items_area = Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1));

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .take(max_items)
            .enumerate()
            .map(|(display_idx, (cmd_idx, _score, matched_indices))| {
                let cmd = &self.commands[*cmd_idx];
                let is_selected = display_idx == self.selected;

                let mut spans = vec![
                    Span::styled(
                        format!(" {:<12}", cmd.category),
                        Style::default().fg(theme.fg_muted),
                    ),
                ];

                // Render label with highlighted matched chars
                let label_offset = cmd.category.len() + 1; // account for "category " prefix in haystack
                for (ci, ch) in cmd.label.chars().enumerate() {
                    let global_idx = label_offset + ci;
                    if matched_indices.contains(&global_idx) {
                        spans.push(Span::styled(
                            ch.to_string(),
                            Style::default().fg(theme.brand).add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        spans.push(Span::styled(
                            ch.to_string(),
                            if is_selected {
                                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(theme.fg)
                            },
                        ));
                    }
                }

                if let Some(shortcut) = cmd.shortcut {
                    let padding = (items_area.width as usize)
                        .saturating_sub(12 + cmd.label.len() + shortcut.len() + 3);
                    spans.push(Span::raw(" ".repeat(padding)));
                    spans.push(Span::styled(
                        shortcut,
                        Style::default().fg(theme.fg_dim),
                    ));
                }

                let style = if is_selected {
                    Style::default().bg(theme.bg_highlight)
                } else {
                    Style::default()
                };

                ListItem::new(Line::from(spans)).style(style)
            })
            .collect();

        let list = List::new(items);
        frame.render_widget(list, items_area);
    }
}

/// Build the command list based on current app state.
pub fn build_commands(
    theme_names: &[&str],
    current_theme: &str,
    has_flights: bool,
) -> Vec<CommandEntry> {
    let mut cmds = vec![
        CommandEntry {
            id: "nav.dashboard",
            label: "Go to Dashboard".to_string(),
            category: "Navigation",
            shortcut: Some("1"),
        },
        CommandEntry {
            id: "nav.sessions",
            label: "Go to Sessions".to_string(),
            category: "Navigation",
            shortcut: Some("2"),
        },
        CommandEntry {
            id: "nav.agents",
            label: "Go to Agents".to_string(),
            category: "Navigation",
            shortcut: Some("3"),
        },
        CommandEntry {
            id: "nav.settings",
            label: "Go to Settings".to_string(),
            category: "Navigation",
            shortcut: Some("4"),
        },
        CommandEntry {
            id: "flight.create",
            label: "Create New Flight".to_string(),
            category: "Flight",
            shortcut: Some("c"),
        },
        CommandEntry {
            id: "git.refresh",
            label: "Refresh Git Status".to_string(),
            category: "Git",
            shortcut: Some("g"),
        },
        CommandEntry {
            id: "git.pull",
            label: "Git Pull".to_string(),
            category: "Git",
            shortcut: None,
        },
        CommandEntry {
            id: "git.push",
            label: "Git Push".to_string(),
            category: "Git",
            shortcut: None,
        },
    ];

    if has_flights {
        cmds.push(CommandEntry {
            id: "flight.launch",
            label: "Launch Selected Flight".to_string(),
            category: "Flight",
            shortcut: Some("l"),
        });
        cmds.push(CommandEntry {
            id: "flight.pause",
            label: "Pause Selected Flight".to_string(),
            category: "Flight",
            shortcut: Some("p"),
        });
    }

    for name in theme_names {
        cmds.push(CommandEntry {
            id: if *name == "default_dark" {
                "theme.default_dark"
            } else if *name == "tokyonight" {
                "theme.tokyonight"
            } else if *name == "catppuccin_mocha" {
                "theme.catppuccin_mocha"
            } else if *name == "gruvbox_dark" {
                "theme.gruvbox_dark"
            } else {
                "theme.nord"
            },
            label: format!(
                "Theme: {}{}",
                name,
                if *name == current_theme { " (active)" } else { "" }
            ),
            category: "Theme",
            shortcut: None,
        });
    }

    cmds
}
