use crate::model::HighlightedLine;
use crate::theme::Theme;
use ratatui::prelude::Color;
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, Theme as SyntectTheme, ThemeSet},
    parsing::SyntaxSet,
};

#[derive(Clone, Copy, PartialEq)]
enum DiffLineType {
    Header,
    Hunk,
    Added,
    Removed,
    Context,
}

pub struct Highlighter {
    syntax_set: SyntaxSet,
    syntect_theme: Option<SyntectTheme>,
    theme: Theme,
}

impl Highlighter {
    pub fn new(theme: Theme) -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();

        // Keep this robust: missing themes should never crash the TUI.
        // Try the theme specified by our Theme, then fall back to alternatives.
        let syntect_theme = theme_set
            .themes
            .get(theme.syntect_theme())
            .or_else(|| theme_set.themes.get("base16-ocean.light"))
            .or_else(|| theme_set.themes.get("base16-ocean.dark"))
            .or_else(|| theme_set.themes.values().next())
            .cloned();

        Self {
            syntax_set,
            syntect_theme,
            theme,
        }
    }

    pub fn highlight_diff(&self, diff_lines: &[String], file_path: &str) -> Vec<HighlightedLine> {
        let extension = file_path.rsplit('.').next().unwrap_or("");
        // Map common extensions that syntect doesn't recognize directly
        let mapped_ext = match extension {
            "tsx" | "jsx" => "js", // syntect's JS syntax handles JSX
            "ts" => "js",          // TypeScript close enough to JS for highlighting
            "scss" => "css",
            _ => extension,
        };
        let syntax = self
            .syntax_set
            .find_syntax_by_extension(mapped_ext)
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let mut highlighter = self
            .syntect_theme
            .as_ref()
            .map(|theme| HighlightLines::new(syntax, theme));

        let mut result = Vec::new();

        for line in diff_lines {
            let stripped = strip_ansi(line);
            let line_type = classify_diff_line(&stripped);

            let (bg_color, code_to_highlight) = match line_type {
                DiffLineType::Added => (
                    self.theme.diff_added_bg,
                    stripped.get(1..).unwrap_or("").to_string(),
                ),
                DiffLineType::Removed => (
                    self.theme.diff_removed_bg,
                    stripped.get(1..).unwrap_or("").to_string(),
                ),
                DiffLineType::Hunk => (self.theme.diff_hunk_bg, stripped.clone()),
                DiffLineType::Header => (Color::Reset, stripped.clone()),
                DiffLineType::Context => {
                    if stripped.starts_with(' ') {
                        (Color::Reset, stripped.get(1..).unwrap_or("").to_string())
                    } else {
                        (Color::Reset, stripped.clone())
                    }
                }
            };

            // For header/hunk lines, just use plain styling
            if matches!(line_type, DiffLineType::Header | DiffLineType::Hunk) {
                let fg = match line_type {
                    DiffLineType::Hunk => Color::Cyan,
                    _ => Color::DarkGray,
                };
                result.push(HighlightedLine {
                    spans: vec![(stripped.clone(), fg, bg_color)],
                });
                continue;
            }

            // For code lines, apply syntax highlighting (if theme exists).
            let prefix = match line_type {
                DiffLineType::Added => "+",
                DiffLineType::Removed => "-",
                DiffLineType::Context if stripped.starts_with(' ') => " ",
                _ => "",
            };

            let mut spans = Vec::new();

            // Add the prefix with appropriate color from theme
            let prefix_fg = match line_type {
                DiffLineType::Added => self.theme.diff_added_fg,
                DiffLineType::Removed => self.theme.diff_removed_fg,
                _ => Color::DarkGray,
            };
            if !prefix.is_empty() {
                spans.push((prefix.to_string(), prefix_fg, bg_color));
            }

            // Highlight the code
            if let Some(ref mut hl) = highlighter {
                let code_with_newline = format!("{code_to_highlight}\n");
                if let Ok(highlighted) = hl.highlight_line(&code_with_newline, &self.syntax_set) {
                    for (style, text) in highlighted {
                        let fg = syntect_to_ratatui_color(style);
                        let clean_text = text.trim_end_matches('\n').to_string();
                        if !clean_text.is_empty() {
                            spans.push((clean_text, fg, bg_color));
                        }
                    }
                } else {
                    spans.push((code_to_highlight.to_string(), Color::White, bg_color));
                }
            } else {
                spans.push((code_to_highlight.to_string(), Color::White, bg_color));
            }

            result.push(HighlightedLine { spans });
        }

        result
    }
}

fn syntect_to_ratatui_color(style: SyntectStyle) -> Color {
    Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b)
}

fn classify_diff_line(line: &str) -> DiffLineType {
    if line.starts_with("@@") {
        DiffLineType::Hunk
    } else if line.starts_with('+') && !line.starts_with("+++") {
        DiffLineType::Added
    } else if line.starts_with('-') && !line.starts_with("---") {
        DiffLineType::Removed
    } else if line.starts_with("diff ")
        || line.starts_with("index ")
        || line.starts_with("+++")
        || line.starts_with("---")
        || line.starts_with("new file mode")
        || line.starts_with("deleted file mode")
        || line.starts_with("old mode")
        || line.starts_with("new mode")
        || line.starts_with("similarity index")
        || line.starts_with("dissimilarity index")
        || line.starts_with("rename from")
        || line.starts_with("rename to")
        || line.starts_with("copy from")
        || line.starts_with("copy to")
        || line.starts_with("Binary files ")
        || line.starts_with("\\ No newline at end of file")
    {
        DiffLineType::Header
    } else {
        DiffLineType::Context
    }
}

fn strip_ansi(s: &str) -> String {
    let mut result = String::new();
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else {
            result.push(c);
        }
    }
    result
}
