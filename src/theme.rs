use ratatui::prelude::Color;
use std::env;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    /// Parse theme mode from string (case-insensitive)
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    // Diff backgrounds
    pub diff_added_bg: Color,
    pub diff_removed_bg: Color,
    pub diff_hunk_bg: Color,

    // Diff prefix colors
    pub diff_added_fg: Color,
    pub diff_removed_fg: Color,

    // UI selection
    pub selected_bg: Color,
    pub selected_fg: Color,

    // Syntect theme name
    syntect_theme_name: &'static str,
}

impl Theme {
    /// Dark theme (Monokai-adjacent) - default
    pub fn dark() -> Self {
        Self {
            diff_added_bg: Color::Rgb(45, 74, 45),    // #2d4a2d - muted dark green
            diff_removed_bg: Color::Rgb(74, 45, 45),  // #4a2d2d - muted dark red
            diff_hunk_bg: Color::Rgb(45, 45, 74),     // #2d2d4a - muted dark blue
            diff_added_fg: Color::Green,
            diff_removed_fg: Color::Red,
            selected_bg: Color::Rgb(60, 60, 120),     // #3c3c78 - current selection color
            selected_fg: Color::White,
            syntect_theme_name: "base16-mocha.dark",
        }
    }

    /// Light theme - matches original colors
    pub fn light() -> Self {
        Self {
            diff_added_bg: Color::Rgb(200, 255, 200),
            diff_removed_bg: Color::Rgb(255, 220, 220),
            diff_hunk_bg: Color::Rgb(220, 220, 255),
            diff_added_fg: Color::Green,
            diff_removed_fg: Color::Red,
            selected_bg: Color::Rgb(60, 60, 120),
            selected_fg: Color::White,
            syntect_theme_name: "base16-ocean.light",
        }
    }

    /// Create theme from environment variable and/or CLI argument
    /// Priority: PRDIFF_THEME env var > CLI arg > default (dark)
    pub fn from_config(cli_theme: Option<ThemeMode>) -> Self {
        // Environment variable takes precedence
        if let Ok(env_theme) = env::var("PRDIFF_THEME") {
            if let Some(mode) = ThemeMode::from_str(&env_theme) {
                return Self::from_mode(mode);
            }
            // Invalid value in env var - fall through to CLI or default
        }

        // CLI argument
        if let Some(mode) = cli_theme {
            return Self::from_mode(mode);
        }

        // Default to dark
        Self::dark()
    }

    fn from_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => Self::light(),
            ThemeMode::Dark => Self::dark(),
        }
    }

    /// Get the syntect theme name for syntax highlighting
    pub fn syntect_theme(&self) -> &'static str {
        self.syntect_theme_name
    }
}
