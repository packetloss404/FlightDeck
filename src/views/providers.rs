//! Providers view: add/edit/delete LLM providers and test API keys.
//!
//! Owns its own local state (`ProvidersViewState`) which lives on `App`.
//! API keys flow through the form buffer only long enough to be handed to the
//! keyring — they are never persisted to JSON and are zeroed out when the form
//! is closed.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;
use zeroize::Zeroize;

use crate::core::provider_config::{
    self, ProviderConfig, ProviderKind, TestConnectionResult,
};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    DisplayName,
    DefaultModel,
    BaseUrl,
    ApiKey,
    Enabled,
    Submit,
}

impl FormField {
    fn next(self) -> Self {
        match self {
            FormField::DisplayName => FormField::DefaultModel,
            FormField::DefaultModel => FormField::BaseUrl,
            FormField::BaseUrl => FormField::ApiKey,
            FormField::ApiKey => FormField::Enabled,
            FormField::Enabled => FormField::Submit,
            FormField::Submit => FormField::DisplayName,
        }
    }

    fn prev(self) -> Self {
        match self {
            FormField::DisplayName => FormField::Submit,
            FormField::DefaultModel => FormField::DisplayName,
            FormField::BaseUrl => FormField::DefaultModel,
            FormField::ApiKey => FormField::BaseUrl,
            FormField::Enabled => FormField::ApiKey,
            FormField::Submit => FormField::Enabled,
        }
    }
}

/// Form state while adding or editing a provider. `api_key` is held only in
/// memory for the lifetime of the form and is zeroized on drop.
pub struct ProviderForm {
    pub editing_id: Option<String>,
    pub kind: ProviderKind,
    pub display_name: String,
    pub default_model: String,
    pub base_url: String,
    pub api_key: String,
    pub enabled: bool,
    pub focus: FormField,
}

impl Drop for ProviderForm {
    fn drop(&mut self) {
        self.api_key.zeroize();
    }
}

impl ProviderForm {
    pub fn new_add() -> Self {
        Self {
            editing_id: None,
            kind: ProviderKind::Anthropic,
            display_name: String::new(),
            default_model: ProviderKind::Anthropic.default_model().to_string(),
            base_url: String::new(),
            api_key: String::new(),
            enabled: true,
            focus: FormField::DisplayName,
        }
    }

    pub fn new_edit(existing: &ProviderConfig) -> Self {
        Self {
            editing_id: Some(existing.id.clone()),
            kind: existing.kind,
            display_name: existing.display_name.clone(),
            default_model: existing.default_model.clone(),
            base_url: existing.base_url.clone().unwrap_or_default(),
            api_key: String::new(),
            enabled: existing.enabled,
            focus: FormField::DisplayName,
        }
    }

    fn field_mut(&mut self, field: FormField) -> Option<&mut String> {
        match field {
            FormField::DisplayName => Some(&mut self.display_name),
            FormField::DefaultModel => Some(&mut self.default_model),
            FormField::BaseUrl => Some(&mut self.base_url),
            FormField::ApiKey => Some(&mut self.api_key),
            FormField::Enabled | FormField::Submit => None,
        }
    }
}

pub enum ProvidersMode {
    List,
    Form(ProviderForm),
    ConfirmDelete(String),
}

pub struct ProvidersViewState {
    pub selected_idx: usize,
    pub mode: ProvidersMode,
    pub last_status: Option<(bool, String)>,
}

impl Default for ProvidersViewState {
    fn default() -> Self {
        Self {
            selected_idx: 0,
            mode: ProvidersMode::List,
            last_status: None,
        }
    }
}

/// Outcome produced by `handle_key` that the host `App` acts on.
#[derive(Debug, Clone)]
pub enum ProvidersAction {
    None,
    SaveNew {
        id: String,
        kind: ProviderKind,
        display_name: String,
        default_model: String,
        base_url: Option<String>,
        enabled: bool,
        api_key: String,
    },
    SaveEdit {
        id: String,
        display_name: String,
        default_model: String,
        base_url: Option<String>,
        enabled: bool,
        replacement_api_key: Option<String>,
    },
    Delete(String),
    TestStored(String),
    TestWithKey {
        kind: ProviderKind,
        base_url: Option<String>,
        api_key: String,
    },
}

/// Derive a stable id from a display name: lowercase alphanumerics, hyphens
/// elsewhere, collapsed. Collisions are resolved by the caller by appending a
/// numeric suffix.
pub fn slug_from(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "provider".to_string()
    } else {
        out
    }
}

pub fn handle_key(
    state: &mut ProvidersViewState,
    providers: &[ProviderConfig],
    key: KeyEvent,
) -> ProvidersAction {
    match &mut state.mode {
        ProvidersMode::List => handle_list_key(state, providers, key),
        ProvidersMode::Form(_) => handle_form_key(state, providers, key),
        ProvidersMode::ConfirmDelete(_) => handle_confirm_key(state, key),
    }
}

fn handle_list_key(
    state: &mut ProvidersViewState,
    providers: &[ProviderConfig],
    key: KeyEvent,
) -> ProvidersAction {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if state.selected_idx + 1 < providers.len() {
                state.selected_idx += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if state.selected_idx > 0 {
                state.selected_idx -= 1;
            }
        }
        KeyCode::Char('a') => {
            state.mode = ProvidersMode::Form(ProviderForm::new_add());
            state.last_status = None;
        }
        KeyCode::Char('e') => {
            if let Some(existing) = providers.get(state.selected_idx) {
                state.mode = ProvidersMode::Form(ProviderForm::new_edit(existing));
                state.last_status = None;
            }
        }
        KeyCode::Char('d') => {
            if let Some(existing) = providers.get(state.selected_idx) {
                state.mode = ProvidersMode::ConfirmDelete(existing.id.clone());
            }
        }
        KeyCode::Char('t') => {
            if let Some(existing) = providers.get(state.selected_idx) {
                return ProvidersAction::TestStored(existing.id.clone());
            }
        }
        _ => {}
    }
    ProvidersAction::None
}

fn handle_form_key(
    state: &mut ProvidersViewState,
    providers: &[ProviderConfig],
    key: KeyEvent,
) -> ProvidersAction {
    let form = match &mut state.mode {
        ProvidersMode::Form(f) => f,
        _ => return ProvidersAction::None,
    };

    match key.code {
        KeyCode::Esc => {
            state.mode = ProvidersMode::List;
            return ProvidersAction::None;
        }
        KeyCode::Tab => form.focus = form.focus.next(),
        KeyCode::BackTab => form.focus = form.focus.prev(),
        KeyCode::Char(' ') if form.focus == FormField::Enabled => {
            form.enabled = !form.enabled;
        }
        KeyCode::Enter if form.focus == FormField::Submit => {
            return submit_form(state, providers);
        }
        KeyCode::Enter => form.focus = form.focus.next(),
        KeyCode::Backspace => {
            if let Some(buf) = form.field_mut(form.focus) {
                buf.pop();
            }
        }
        KeyCode::Char(ch) => {
            if let Some(buf) = form.field_mut(form.focus) {
                // skip modifiers that are just Ctrl/Alt chords
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
                    buf.push(ch);
                }
            }
        }
        _ => {}
    }
    ProvidersAction::None
}

fn submit_form(
    state: &mut ProvidersViewState,
    providers: &[ProviderConfig],
) -> ProvidersAction {
    let form = match &state.mode {
        ProvidersMode::Form(f) => f,
        _ => return ProvidersAction::None,
    };

    if form.display_name.trim().is_empty() {
        state.last_status = Some((false, "display name is required".into()));
        return ProvidersAction::None;
    }
    if form.default_model.trim().is_empty() {
        state.last_status = Some((false, "default model is required".into()));
        return ProvidersAction::None;
    }

    let base_url = if form.base_url.trim().is_empty() {
        None
    } else {
        Some(form.base_url.trim().to_string())
    };

    match &form.editing_id {
        Some(id) => {
            let action = ProvidersAction::SaveEdit {
                id: id.clone(),
                display_name: form.display_name.trim().to_string(),
                default_model: form.default_model.trim().to_string(),
                base_url,
                enabled: form.enabled,
                replacement_api_key: if form.api_key.is_empty() {
                    None
                } else {
                    Some(form.api_key.clone())
                },
            };
            state.mode = ProvidersMode::List;
            action
        }
        None => {
            if form.api_key.is_empty() {
                state.last_status = Some((false, "api key is required on add".into()));
                return ProvidersAction::None;
            }
            let base = slug_from(form.display_name.trim());
            let id = unique_id(&base, providers);
            let action = ProvidersAction::SaveNew {
                id,
                kind: form.kind,
                display_name: form.display_name.trim().to_string(),
                default_model: form.default_model.trim().to_string(),
                base_url,
                enabled: form.enabled,
                api_key: form.api_key.clone(),
            };
            state.mode = ProvidersMode::List;
            action
        }
    }
}

fn unique_id(base: &str, providers: &[ProviderConfig]) -> String {
    if !providers.iter().any(|p| p.id == base) {
        return base.to_string();
    }
    for n in 2..u32::MAX {
        let candidate = format!("{}-{}", base, n);
        if !providers.iter().any(|p| p.id == candidate) {
            return candidate;
        }
    }
    base.to_string()
}

fn handle_confirm_key(
    state: &mut ProvidersViewState,
    key: KeyEvent,
) -> ProvidersAction {
    let id = match &state.mode {
        ProvidersMode::ConfirmDelete(id) => id.clone(),
        _ => return ProvidersAction::None,
    };
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter => {
            state.mode = ProvidersMode::List;
            ProvidersAction::Delete(id)
        }
        _ => {
            state.mode = ProvidersMode::List;
            ProvidersAction::None
        }
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &ProvidersViewState,
    providers: &[ProviderConfig],
    theme: &Theme,
) {
    match &state.mode {
        ProvidersMode::List => render_list(frame, area, state, providers, theme),
        ProvidersMode::Form(form) => render_form(frame, area, form, state.last_status.as_ref(), theme),
        ProvidersMode::ConfirmDelete(id) => render_confirm(frame, area, id, theme),
    }
}

fn render_list(
    frame: &mut Frame,
    area: Rect,
    state: &ProvidersViewState,
    providers: &[ProviderConfig],
    theme: &Theme,
) {
    let title_top = Span::styled(
        " Providers ",
        Style::default().fg(theme.brand).add_modifier(Modifier::BOLD),
    );
    let title_bottom = Span::styled(
        " a:add  e:edit  d:delete  t:test  j/k:navigate ",
        Style::default().fg(theme.fg_dim),
    );

    let block = Block::default()
        .title(title_top)
        .title_bottom(title_bottom)
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(block.inner(area));

    frame.render_widget(block, area);

    if providers.is_empty() {
        let para = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No providers configured yet.",
                Style::default().fg(theme.fg_dim),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press `a` to add one — you'll need an Anthropic API key.",
                Style::default().fg(theme.fg_muted),
            )),
        ]);
        frame.render_widget(para, chunks[0]);
    } else {
        let items: Vec<ListItem> = providers
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                let has_key = provider_config::has_api_key(&p.id);
                let marker = if has_key {
                    Span::styled(" ✓ ", Style::default().fg(theme.status_done))
                } else {
                    Span::styled(" ✗ ", Style::default().fg(theme.status_warning))
                };
                let enabled_tag = if p.enabled {
                    Span::styled(" enabled ", Style::default().fg(theme.status_done))
                } else {
                    Span::styled(" disabled", Style::default().fg(theme.fg_dim))
                };
                let name_style = if idx == state.selected_idx {
                    Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                let line = Line::from(vec![
                    Span::raw("  "),
                    marker,
                    Span::styled(format!("{:<22}", p.display_name), name_style),
                    Span::styled(
                        format!("  {:<10}", p.kind.display_name()),
                        Style::default().fg(theme.accent),
                    ),
                    Span::styled(
                        format!("  {:<28}", p.default_model),
                        Style::default().fg(theme.fg_dim),
                    ),
                    enabled_tag,
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .highlight_style(Style::default().bg(theme.bg_highlight))
            .highlight_symbol("▸ ");
        let mut list_state = ListState::default();
        list_state.select(Some(state.selected_idx));
        frame.render_stateful_widget(list, chunks[0], &mut list_state);
    }

    if let Some((ok, msg)) = &state.last_status {
        let style = if *ok {
            Style::default().fg(theme.status_done)
        } else {
            Style::default().fg(theme.status_warning)
        };
        let line = Line::from(vec![
            Span::raw("  "),
            Span::styled(msg.clone(), style),
        ]);
        frame.render_widget(Paragraph::new(line), chunks[1]);
    }
}

fn render_form(
    frame: &mut Frame,
    area: Rect,
    form: &ProviderForm,
    last_status: Option<&(bool, String)>,
    theme: &Theme,
) {
    let title_top = Span::styled(
        if form.editing_id.is_some() { " Edit Provider " } else { " Add Provider " },
        Style::default().fg(theme.brand).add_modifier(Modifier::BOLD),
    );
    let title_bottom = Span::styled(
        " Tab:next  Shift+Tab:prev  Enter (on Save):submit  Esc:cancel ",
        Style::default().fg(theme.fg_dim),
    );
    let block = Block::default()
        .title(title_top)
        .title_bottom(title_bottom)
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let masked_key = "•".repeat(form.api_key.chars().count());
    let api_key_hint = if form.editing_id.is_some() && form.api_key.is_empty() {
        "(leave blank to keep existing)".to_string()
    } else {
        masked_key
    };

    let rows = vec![
        field_line(theme, "Kind              ", form.kind.display_name(), false, form.focus == FormField::DisplayName && false),
        field_line(theme, "Display name      ", &form.display_name, form.focus == FormField::DisplayName, false),
        field_line(theme, "Default model     ", &form.default_model, form.focus == FormField::DefaultModel, false),
        field_line(theme, "Base URL          ", if form.base_url.is_empty() { "(default)".into() } else { form.base_url.clone() }.as_str(), form.focus == FormField::BaseUrl, false),
        field_line(theme, "API key           ", &api_key_hint, form.focus == FormField::ApiKey, false),
        field_line(theme, "Enabled           ", if form.enabled { "yes" } else { "no" }, form.focus == FormField::Enabled, true),
    ];

    let mut lines: Vec<Line> = vec![Line::from("")];
    lines.extend(rows);
    lines.push(Line::from(""));
    lines.push(submit_line(theme, form.focus == FormField::Submit, form.editing_id.is_some()));

    if let Some((ok, msg)) = last_status {
        lines.push(Line::from(""));
        let style = if *ok {
            Style::default().fg(theme.status_done)
        } else {
            Style::default().fg(theme.status_warning)
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(msg.clone(), style),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn field_line<'a>(
    theme: &Theme,
    label: &'static str,
    value: &str,
    focused: bool,
    toggle: bool,
) -> Line<'a> {
    let value_style = if focused {
        Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)
    } else if toggle {
        Style::default().fg(theme.fg)
    } else {
        Style::default().fg(theme.fg)
    };
    let marker = if focused { "▸ " } else { "  " };
    Line::from(vec![
        Span::raw(marker),
        Span::styled(label, Style::default().fg(theme.fg_muted)),
        Span::styled(value.to_string(), value_style),
    ])
}

fn submit_line<'a>(theme: &Theme, focused: bool, editing: bool) -> Line<'a> {
    let marker = if focused { "▸ " } else { "  " };
    let label = if editing { "[ Save changes ]" } else { "[ Save provider ]" };
    let style = if focused {
        Style::default().fg(theme.brand).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.fg_dim)
    };
    Line::from(vec![
        Span::raw(marker),
        Span::styled(label, style),
    ])
}

fn render_confirm(frame: &mut Frame, area: Rect, id: &str, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled(
            " Delete provider? ",
            Style::default().fg(theme.status_warning).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.border));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Provider: ", Style::default().fg(theme.fg_dim)),
            Span::styled(id.to_string(), Style::default().fg(theme.fg).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  The JSON record and the keyring entry will both be removed.",
            Style::default().fg(theme.fg_muted),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("[y] confirm", Style::default().fg(theme.status_warning).add_modifier(Modifier::BOLD)),
            Span::raw("     "),
            Span::styled("[any other key] cancel", Style::default().fg(theme.fg_dim)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_from_handles_spaces_and_punct() {
        assert_eq!(slug_from("Anthropic (primary)"), "anthropic-primary");
        assert_eq!(slug_from("My  Test!!"), "my-test");
        assert_eq!(slug_from(""), "provider");
    }

    #[test]
    fn form_field_cycle_wraps() {
        let mut f = FormField::DisplayName;
        for _ in 0..6 {
            f = f.next();
        }
        assert_eq!(f, FormField::DisplayName);
    }

    #[test]
    fn unique_id_appends_suffix() {
        let existing = vec![ProviderConfig {
            id: "anthropic".into(),
            kind: ProviderKind::Anthropic,
            display_name: "Anthropic".into(),
            base_url: None,
            default_model: "claude-sonnet-4-6".into(),
            enabled: true,
        }];
        assert_eq!(unique_id("anthropic", &existing), "anthropic-2");
        assert_eq!(unique_id("other", &existing), "other");
    }
}
