use ratatui::style::Color;
use serde::Deserialize;
use tracing::warn;

/// A complete TUI color theme.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub bg: Color,
    pub bg_highlight: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_muted: Color,
    pub fg_faint: Color,
    pub border: Color,
    pub accent: Color,
    pub brand: Color,
    pub status_active: Color,
    pub status_done: Color,
    pub status_failed: Color,
    pub status_paused: Color,
    pub status_review: Color,
    pub status_warning: Color,
    pub status_info: Color,
    pub progress_filled: Color,
    pub progress_empty: Color,
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    name: String,
    bg: String,
    bg_highlight: String,
    fg: String,
    fg_dim: String,
    fg_muted: String,
    fg_faint: String,
    border: String,
    accent: String,
    brand: String,
    status_active: String,
    status_done: String,
    status_failed: String,
    status_paused: String,
    status_review: String,
    status_warning: String,
    status_info: String,
    progress_filled: String,
    progress_empty: String,
}

fn hex_to_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return Color::White;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
    Color::Rgb(r, g, b)
}

impl From<ThemeFile> for Theme {
    fn from(tf: ThemeFile) -> Self {
        Theme {
            name: tf.name,
            bg: hex_to_color(&tf.bg),
            bg_highlight: hex_to_color(&tf.bg_highlight),
            fg: hex_to_color(&tf.fg),
            fg_dim: hex_to_color(&tf.fg_dim),
            fg_muted: hex_to_color(&tf.fg_muted),
            fg_faint: hex_to_color(&tf.fg_faint),
            border: hex_to_color(&tf.border),
            accent: hex_to_color(&tf.accent),
            brand: hex_to_color(&tf.brand),
            status_active: hex_to_color(&tf.status_active),
            status_done: hex_to_color(&tf.status_done),
            status_failed: hex_to_color(&tf.status_failed),
            status_paused: hex_to_color(&tf.status_paused),
            status_review: hex_to_color(&tf.status_review),
            status_warning: hex_to_color(&tf.status_warning),
            status_info: hex_to_color(&tf.status_info),
            progress_filled: hex_to_color(&tf.progress_filled),
            progress_empty: hex_to_color(&tf.progress_empty),
        }
    }
}

// Embedded theme JSON files
const THEME_DEFAULT_DARK: &str = include_str!("themes/default_dark.json");
const THEME_TOKYONIGHT: &str = include_str!("themes/tokyonight.json");
const THEME_CATPPUCCIN: &str = include_str!("themes/catppuccin_mocha.json");
const THEME_GRUVBOX: &str = include_str!("themes/gruvbox_dark.json");
const THEME_NORD: &str = include_str!("themes/nord.json");

fn parse_theme(json: &str) -> Option<Theme> {
    serde_json::from_str::<ThemeFile>(json).ok().map(Theme::from)
}

/// All built-in theme names in display order.
pub fn theme_names() -> Vec<&'static str> {
    vec!["default_dark", "tokyonight", "catppuccin_mocha", "gruvbox_dark", "nord"]
}

/// Load a theme by name. Checks user themes dir first, then built-ins.
pub fn load_theme(name: Option<&str>) -> Theme {
    let name = name.unwrap_or("default_dark");

    // Try user themes directory first
    let user_dir = crate::core::storage::data_dir().join("themes");
    let user_file = user_dir.join(format!("{}.json", name));
    if user_file.exists() {
        if let Ok(contents) = std::fs::read_to_string(&user_file) {
            if let Some(theme) = parse_theme(&contents) {
                return theme;
            }
            warn!("Failed to parse user theme: {}", user_file.display());
        }
    }

    // Fall back to built-in themes
    match name {
        "tokyonight" => parse_theme(THEME_TOKYONIGHT),
        "catppuccin_mocha" => parse_theme(THEME_CATPPUCCIN),
        "gruvbox_dark" => parse_theme(THEME_GRUVBOX),
        "nord" => parse_theme(THEME_NORD),
        _ => parse_theme(THEME_DEFAULT_DARK),
    }
    .unwrap_or_else(|| parse_theme(THEME_DEFAULT_DARK).expect("default theme must parse"))
}

/// Cycle to the next theme name after the current one.
pub fn next_theme_name(current: &str) -> &'static str {
    let names = theme_names();
    let idx = names.iter().position(|n| *n == current).unwrap_or(0);
    names[(idx + 1) % names.len()]
}

// Formatting helpers for cost/token display

pub fn format_cost(cost: f64) -> String {
    if cost < 0.01 {
        "$0.00".to_string()
    } else if cost < 10.0 {
        format!("${:.2}", cost)
    } else {
        format!("${:.0}", cost)
    }
}

pub fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}
