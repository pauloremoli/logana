use std::str::FromStr;

use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

/// Parse a color string supporting `[r,g,b]` RGB triplets in addition to
/// ratatui's built-in named colours (`Red`, `LightBlue`, ...) and hex (`#RRGGBB`).
pub fn parse_color(s: &str) -> Option<Color> {
    let trimmed = s.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() == 3
            && let (Ok(r), Ok(g), Ok(b)) = (
                parts[0].trim().parse::<u8>(),
                parts[1].trim().parse::<u8>(),
                parts[2].trim().parse::<u8>(),
            )
        {
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }
    Color::from_str(trimmed).ok()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum LogLevel {
    Info,
    Warning,
    Error,
    Debug,
    #[default]
    Unknown,
}

impl LogLevel {
    pub fn parse_level(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "info" => LogLevel::Info,
            "warn" | "warning" => LogLevel::Warning,
            "error" | "err" => LogLevel::Error,
            "debug" => LogLevel::Debug,
            _ => LogLevel::Unknown,
        }
    }

    /// Detect log level by scanning raw bytes for level keywords.
    pub fn detect_from_bytes(line: &[u8]) -> Self {
        // Fast path: scan for keywords case-insensitively using byte windows
        let mut i = 0;
        while i + 4 <= line.len() {
            let w4 = [
                line[i].to_ascii_uppercase(),
                line[i + 1].to_ascii_uppercase(),
                line[i + 2].to_ascii_uppercase(),
                line[i + 3].to_ascii_uppercase(),
            ];
            if w4 == *b"INFO" {
                return LogLevel::Info;
            }
            if w4 == *b"WARN" {
                return LogLevel::Warning;
            }
            if w4 == *b"DEBU" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'G') {
                return LogLevel::Debug;
            }
            if w4 == *b"ERRO" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'R') {
                return LogLevel::Error;
            }
            i += 1;
        }
        LogLevel::Unknown
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FilterType {
    Include,
    Exclude,
}

impl std::fmt::Display for FilterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterType::Include => write!(f, "Include"),
            FilterType::Exclude => write!(f, "Exclude"),
        }
    }
}

#[serde_as]
#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
pub struct ColorConfig {
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub fg: Option<Color>,
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub bg: Option<Color>,
    #[serde(default)]
    pub match_only: bool,
}

/// Persisted filter definition (stored in SQLite, displayed in sidebar).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterDef {
    pub id: usize,
    pub pattern: String,
    pub filter_type: FilterType,
    pub enabled: bool,
    pub color_config: Option<ColorConfig>,
}

/// A text comment attached to a group of log line indices.
/// The text may contain newlines for multi-line comments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub text: String,
    pub line_indices: Vec<usize>,
}

/// Result of a search operation: which line index it was on and where in that line.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub line_idx: usize,
    pub matches: Vec<(usize, usize)>,
}

/// Controls which structured columns are shown and in what order.
#[derive(Debug, Clone, Default)]
pub struct FieldLayout {
    /// When Some: show only these named columns in this order.
    /// Names: "timestamp"|"ts"|"time", "level"|"lvl", "target", "span",
    ///         "message"|"msg", or any extra-field key present in the line.
    /// When None: show all columns in default order.
    pub columns: Option<Vec<String>>,
    /// Full ordered field list (enabled + disabled) from the select-fields
    /// modal.  Used to restore the list order when the modal is reopened.
    pub columns_order: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::parse_level("info"), LogLevel::Info);
        assert_eq!(LogLevel::parse_level("INFO"), LogLevel::Info);
        assert_eq!(LogLevel::parse_level("warn"), LogLevel::Warning);
        assert_eq!(LogLevel::parse_level("WARNING"), LogLevel::Warning);
        assert_eq!(LogLevel::parse_level("error"), LogLevel::Error);
        assert_eq!(LogLevel::parse_level("ERR"), LogLevel::Error);
        assert_eq!(LogLevel::parse_level("debug"), LogLevel::Debug);
        assert_eq!(LogLevel::parse_level("unknown"), LogLevel::Unknown);
    }

    #[test]
    fn test_log_level_detect_from_bytes() {
        assert_eq!(
            LogLevel::detect_from_bytes(b"some INFO message"),
            LogLevel::Info
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"WARN: disk full"),
            LogLevel::Warning
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"ERROR: connection lost"),
            LogLevel::Error
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"DEBUG: value=5"),
            LogLevel::Debug
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"plain log line"),
            LogLevel::Unknown
        );
        // Case insensitive
        assert_eq!(
            LogLevel::detect_from_bytes(b"error happened"),
            LogLevel::Error
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"warn about something"),
            LogLevel::Warning
        );
    }

    #[test]
    fn test_filter_type_display() {
        assert_eq!(FilterType::Include.to_string(), "Include");
        assert_eq!(FilterType::Exclude.to_string(), "Exclude");
    }

    // ── parse_color ────────────────────────────────────────────────────────

    #[test]
    fn test_parse_color_rgb_triplet() {
        assert_eq!(parse_color("[255,0,0]"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_color("[0,255,0]"), Some(Color::Rgb(0, 255, 0)));
        assert_eq!(parse_color("[0,0,255]"), Some(Color::Rgb(0, 0, 255)));
    }

    #[test]
    fn test_parse_color_rgb_triplet_with_spaces() {
        assert_eq!(parse_color("[255, 128, 0]"), Some(Color::Rgb(255, 128, 0)));
        assert_eq!(
            parse_color("[ 10 , 20 , 30 ]"),
            Some(Color::Rgb(10, 20, 30))
        );
    }

    #[test]
    fn test_parse_color_named() {
        assert_eq!(parse_color("Red"), Some(Color::Red));
        assert_eq!(parse_color("LightBlue"), Some(Color::LightBlue));
        assert_eq!(parse_color("green"), Some(Color::Green));
    }

    #[test]
    fn test_parse_color_hex() {
        assert_eq!(parse_color("#FF0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_color("#00ff00"), Some(Color::Rgb(0, 255, 0)));
    }

    #[test]
    fn test_parse_color_invalid() {
        assert_eq!(parse_color("not_a_color"), None);
        assert_eq!(parse_color("[256,0,0]"), None);
        assert_eq!(parse_color("[1,2]"), None);
        assert_eq!(parse_color("[]"), None);
        assert_eq!(parse_color("[a,b,c]"), None);
    }
}
