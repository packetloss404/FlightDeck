use ratatui::prelude::*;
use ratatui::widgets::*;

use super::super::theme::Theme;

#[derive(Debug, Clone)]
pub enum DiffLineKind {
    Context,
    Addition,
    Deletion,
    HunkHeader,
    FileHeader,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct DiffFile {
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Default)]
pub struct DiffViewState {
    pub scroll: u16,
    pub visible: bool,
}

/// Parse unified diff text into structured diff files.
pub fn parse_unified_diff(input: &str) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut current_file: Option<DiffFile> = None;
    let mut old_line: usize = 0;
    let mut new_line: usize = 0;

    for line in input.lines() {
        if line.starts_with("diff --git ") {
            if let Some(file) = current_file.take() {
                files.push(file);
            }
            // Extract path from "diff --git a/path b/path"
            let path = line
                .strip_prefix("diff --git a/")
                .and_then(|s| s.split(" b/").next())
                .unwrap_or("unknown")
                .to_string();
            current_file = Some(DiffFile {
                lines: vec![DiffLine {
                    kind: DiffLineKind::FileHeader,
                    old_lineno: None,
                    new_lineno: None,
                    content: path,
                }],
            });
        } else if line.starts_with("@@") {
            // Parse hunk header: @@ -old_start,count +new_start,count @@
            if let Some(ref mut file) = current_file {
                file.lines.push(DiffLine {
                    kind: DiffLineKind::HunkHeader,
                    old_lineno: None,
                    new_lineno: None,
                    content: line.to_string(),
                });
                // Parse line numbers
                let parts: Vec<&str> = line.split(|c| c == '-' || c == '+' || c == ',').collect();
                if parts.len() >= 2 {
                    old_line = parts[1].trim().split(' ').next()
                        .and_then(|s| s.split(',').next())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                }
                if let Some(plus_part) = line.split('+').nth(1) {
                    new_line = plus_part.split(',').next()
                        .and_then(|s| s.split(' ').next())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                }
            }
        } else if line.starts_with("---") || line.starts_with("+++") {
            // Skip file header lines (already captured)
        } else if let Some(ref mut file) = current_file {
            if line.starts_with('+') {
                file.lines.push(DiffLine {
                    kind: DiffLineKind::Addition,
                    old_lineno: None,
                    new_lineno: Some(new_line),
                    content: line[1..].to_string(),
                });
                new_line += 1;
            } else if line.starts_with('-') {
                file.lines.push(DiffLine {
                    kind: DiffLineKind::Deletion,
                    old_lineno: Some(old_line),
                    new_lineno: None,
                    content: line[1..].to_string(),
                });
                old_line += 1;
            } else if line.starts_with(' ') || line.is_empty() {
                let content = if line.is_empty() { "" } else { &line[1..] };
                file.lines.push(DiffLine {
                    kind: DiffLineKind::Context,
                    old_lineno: Some(old_line),
                    new_lineno: Some(new_line),
                    content: content.to_string(),
                });
                old_line += 1;
                new_line += 1;
            }
        }
    }

    if let Some(file) = current_file {
        files.push(file);
    }

    files
}

/// Detect diff blocks in raw session output.
pub fn extract_diffs_from_output(output: &str) -> Vec<DiffFile> {
    let mut diff_blocks = Vec::new();
    let mut current_block = String::new();
    let mut in_diff = false;

    for line in output.lines() {
        if line.starts_with("diff --git ") {
            if in_diff && !current_block.is_empty() {
                diff_blocks.push(current_block.clone());
                current_block.clear();
            }
            in_diff = true;
            current_block.push_str(line);
            current_block.push('\n');
        } else if in_diff {
            if line.starts_with("diff --git ")
                || (!line.starts_with('+')
                    && !line.starts_with('-')
                    && !line.starts_with(' ')
                    && !line.starts_with('@')
                    && !line.is_empty()
                    && !line.starts_with("index ")
                    && !line.starts_with("new file")
                    && !line.starts_with("deleted file")
                    && !line.starts_with("similarity")
                    && !line.starts_with("rename "))
            {
                diff_blocks.push(current_block.clone());
                current_block.clear();
                in_diff = false;
            } else {
                current_block.push_str(line);
                current_block.push('\n');
            }
        }
    }

    if in_diff && !current_block.is_empty() {
        diff_blocks.push(current_block);
    }

    diff_blocks
        .iter()
        .flat_map(|block| parse_unified_diff(block))
        .collect()
}

/// Render a diff file overlay.
pub fn render_diff_overlay(
    frame: &mut Frame,
    area: Rect,
    files: &[DiffFile],
    scroll: u16,
    theme: &Theme,
) {
    if files.is_empty() {
        return;
    }

    // Use most of the screen
    let margin = 2u16;
    let overlay = Rect::new(
        area.x + margin,
        area.y + 1,
        area.width.saturating_sub(margin * 2),
        area.height.saturating_sub(2),
    );

    frame.render_widget(Clear, overlay);

    let block = Block::default()
        .title(Span::styled(
            format!(" Diff: {} file(s) ", files.len()),
            Style::default().fg(theme.brand).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " PgUp/PgDn:scroll  Esc/d:close ",
            Style::default().fg(theme.fg_dim),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.bg));

    let inner = block.inner(overlay);
    frame.render_widget(block, overlay);

    // Flatten all diff files into lines
    let mut lines: Vec<Line> = Vec::new();
    for file in files {
        for dl in &file.lines {
            let line = match dl.kind {
                DiffLineKind::FileHeader => Line::from(vec![
                    Span::styled(
                        format!(" {} ", dl.content),
                        Style::default()
                            .fg(theme.fg)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                DiffLineKind::HunkHeader => Line::from(vec![
                    Span::styled(
                        &dl.content,
                        Style::default().fg(theme.status_info),
                    ),
                ]),
                DiffLineKind::Addition => {
                    let gutter = format!(
                        "    │{:>4}│",
                        dl.new_lineno.map(|n| n.to_string()).unwrap_or_default(),
                    );
                    Line::from(vec![
                        Span::styled(gutter, Style::default().fg(theme.fg_muted)),
                        Span::styled("+", Style::default().fg(theme.status_done)),
                        Span::styled(
                            &dl.content,
                            Style::default().fg(theme.status_done),
                        ),
                    ])
                }
                DiffLineKind::Deletion => {
                    let gutter = format!(
                        "{:>4}│    │",
                        dl.old_lineno.map(|n| n.to_string()).unwrap_or_default(),
                    );
                    Line::from(vec![
                        Span::styled(gutter, Style::default().fg(theme.fg_muted)),
                        Span::styled("-", Style::default().fg(theme.status_failed)),
                        Span::styled(
                            &dl.content,
                            Style::default().fg(theme.status_failed),
                        ),
                    ])
                }
                DiffLineKind::Context => {
                    let gutter = format!(
                        "{:>4}│{:>4}│",
                        dl.old_lineno.map(|n| n.to_string()).unwrap_or_default(),
                        dl.new_lineno.map(|n| n.to_string()).unwrap_or_default(),
                    );
                    Line::from(vec![
                        Span::styled(gutter, Style::default().fg(theme.fg_muted)),
                        Span::styled(" ", Style::default()),
                        Span::styled(
                            &dl.content,
                            Style::default().fg(theme.fg_dim),
                        ),
                    ])
                }
            };
            lines.push(line);
        }
        lines.push(Line::from("")); // blank between files
    }

    let para = Paragraph::new(lines)
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(para, inner);
}
