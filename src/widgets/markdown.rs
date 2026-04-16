use ratatui::prelude::*;

use super::super::theme::Theme;

/// Convert markdown text to styled ratatui Lines.
///
/// Supports: headings, bold, italic, inline code, code blocks, lists, blockquotes.
/// This is a lightweight parser — not full CommonMark, but covers what agents typically output.
pub fn markdown_to_lines<'a>(input: &str, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for raw_line in input.lines() {
        if raw_line.starts_with("```") {
            if in_code_block {
                in_code_block = false;
                code_lang.clear();
            } else {
                in_code_block = true;
                code_lang = raw_line.trim_start_matches('`').to_string();
                let header = if code_lang.is_empty() {
                    " ─── code ───".to_string()
                } else {
                    format!(" ─── {} ───", code_lang)
                };
                lines.push(Line::from(Span::styled(
                    header,
                    Style::default().fg(theme.fg_muted),
                )));
            }
            continue;
        }

        if in_code_block {
            lines.push(Line::from(vec![
                Span::styled(" │ ", Style::default().fg(theme.fg_muted)),
                Span::styled(
                    raw_line.to_string(),
                    Style::default().fg(theme.status_info),
                ),
            ]));
            continue;
        }

        // Headings
        if raw_line.starts_with("### ") {
            lines.push(Line::from(Span::styled(
                raw_line[4..].to_string(),
                Style::default()
                    .fg(theme.fg)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if raw_line.starts_with("## ") {
            lines.push(Line::from(Span::styled(
                raw_line[3..].to_string(),
                Style::default()
                    .fg(theme.fg)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if raw_line.starts_with("# ") {
            lines.push(Line::from(Span::styled(
                raw_line[2..].to_string(),
                Style::default()
                    .fg(theme.brand)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        // Blockquotes
        if raw_line.starts_with("> ") {
            lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(theme.fg_muted)),
                Span::styled(
                    raw_line[2..].to_string(),
                    Style::default().fg(theme.fg_dim),
                ),
            ]));
            continue;
        }

        // Unordered lists
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            let indent = raw_line.len() - trimmed.len();
            let prefix = " ".repeat(indent);
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled("  • ", Style::default().fg(theme.fg_dim)),
                Span::styled(
                    trimmed[2..].to_string(),
                    Style::default().fg(theme.fg),
                ),
            ]));
            continue;
        }

        // Ordered lists (simple: "1. ", "2. ", etc.)
        if let Some(rest) = try_parse_ordered_list(trimmed) {
            let indent = raw_line.len() - trimmed.len();
            let prefix = " ".repeat(indent);
            let number = &trimmed[..trimmed.find('.').unwrap_or(0)];
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(format!("  {}. ", number), Style::default().fg(theme.fg_dim)),
                Span::styled(
                    rest.to_string(),
                    Style::default().fg(theme.fg),
                ),
            ]));
            continue;
        }

        // Regular text with inline formatting
        lines.push(parse_inline(raw_line, theme));
    }

    lines
}

fn try_parse_ordered_list(line: &str) -> Option<&str> {
    let dot_pos = line.find(". ")?;
    let number_part = &line[..dot_pos];
    if number_part.chars().all(|c| c.is_ascii_digit()) && !number_part.is_empty() {
        Some(&line[dot_pos + 2..])
    } else {
        None
    }
}

/// Parse inline markdown formatting: **bold**, *italic*, `code`
fn parse_inline<'a>(text: &str, theme: &Theme) -> Line<'a> {
    let mut spans: Vec<Span> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut current = String::new();

    while i < len {
        // Inline code: `...`
        if chars[i] == '`' {
            if !current.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut current),
                    Style::default().fg(theme.fg),
                ));
            }
            i += 1;
            let mut code = String::new();
            while i < len && chars[i] != '`' {
                code.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1; // skip closing `
            }
            spans.push(Span::styled(
                code,
                Style::default().fg(theme.status_info),
            ));
            continue;
        }

        // Bold: **...**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if !current.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut current),
                    Style::default().fg(theme.fg),
                ));
            }
            i += 2;
            let mut bold = String::new();
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                bold.push(chars[i]);
                i += 1;
            }
            if i + 1 < len {
                i += 2; // skip closing **
            }
            spans.push(Span::styled(
                bold,
                Style::default()
                    .fg(theme.fg)
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }

        // Italic: *...*
        if chars[i] == '*' && (i + 1 >= len || chars[i + 1] != '*') {
            if !current.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut current),
                    Style::default().fg(theme.fg),
                ));
            }
            i += 1;
            let mut italic = String::new();
            while i < len && chars[i] != '*' {
                italic.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1; // skip closing *
            }
            spans.push(Span::styled(
                italic,
                Style::default()
                    .fg(theme.fg)
                    .add_modifier(Modifier::ITALIC),
            ));
            continue;
        }

        current.push(chars[i]);
        i += 1;
    }

    if !current.is_empty() {
        spans.push(Span::styled(current, Style::default().fg(theme.fg)));
    }

    if spans.is_empty() {
        Line::from("")
    } else {
        Line::from(spans)
    }
}
