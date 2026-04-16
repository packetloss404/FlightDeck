use std::time::{Duration, Instant};

use ratatui::prelude::*;
use ratatui::widgets::*;

use super::super::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
    pub created_at: Instant,
    pub duration: Duration,
}

pub struct ToastManager {
    pub toasts: Vec<Toast>,
    pub max_visible: usize,
}

impl ToastManager {
    pub fn new() -> Self {
        Self {
            toasts: Vec::new(),
            max_visible: 3,
        }
    }

    pub fn push(&mut self, message: impl Into<String>, level: ToastLevel) {
        self.toasts.push(Toast {
            message: message.into(),
            level,
            created_at: Instant::now(),
            duration: match level {
                ToastLevel::Error => Duration::from_secs(6),
                ToastLevel::Warning => Duration::from_secs(5),
                _ => Duration::from_secs(3),
            },
        });
    }

    /// Remove expired toasts.
    pub fn gc(&mut self) {
        self.toasts
            .retain(|t| t.created_at.elapsed() < t.duration);
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let visible: Vec<&Toast> = self
            .toasts
            .iter()
            .rev()
            .take(self.max_visible)
            .collect();

        if visible.is_empty() {
            return;
        }

        for (i, toast) in visible.iter().enumerate() {
            let width = (toast.message.len() as u16 + 4).min(50).min(area.width.saturating_sub(2));
            let height = 3u16;
            let x = area.x + area.width.saturating_sub(width + 2);
            let y = area.y + area.height.saturating_sub(2 + (i as u16) * 4);

            if y < area.y + 1 {
                break;
            }

            let toast_area = Rect::new(x, y.saturating_sub(height), width, height);

            let border_color = match toast.level {
                ToastLevel::Success => theme.status_done,
                ToastLevel::Warning => theme.status_warning,
                ToastLevel::Error => theme.status_failed,
            };

            frame.render_widget(Clear, toast_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(theme.bg));

            let inner = block.inner(toast_area);
            frame.render_widget(block, toast_area);

            let truncated = if toast.message.len() > inner.width as usize {
                format!("{}…", &toast.message[..inner.width as usize - 1])
            } else {
                toast.message.clone()
            };

            let para = Paragraph::new(Span::styled(
                truncated,
                Style::default().fg(theme.fg),
            ));
            frame.render_widget(para, inner);
        }
    }
}
