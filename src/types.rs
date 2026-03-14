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
    if let Some(c) = parse_extended_named_color(trimmed) {
        return Some(c);
    }
    Color::from_str(trimmed).ok()
}

fn parse_extended_named_color(s: &str) -> Option<Color> {
    match s.to_lowercase().as_str() {
        "orange" => Some(Color::Rgb(255, 165, 0)),
        "pink" => Some(Color::Rgb(255, 105, 180)),
        "purple" => Some(Color::Rgb(128, 0, 128)),
        "violet" => Some(Color::Rgb(238, 130, 238)),
        "indigo" => Some(Color::Rgb(75, 0, 130)),
        "teal" => Some(Color::Rgb(0, 128, 128)),
        "turquoise" => Some(Color::Rgb(64, 224, 208)),
        "coral" => Some(Color::Rgb(255, 127, 80)),
        "salmon" => Some(Color::Rgb(250, 128, 114)),
        "gold" => Some(Color::Rgb(255, 215, 0)),
        "lime" => Some(Color::Rgb(0, 255, 0)),
        "maroon" => Some(Color::Rgb(128, 0, 0)),
        "navy" => Some(Color::Rgb(0, 0, 128)),
        "olive" => Some(Color::Rgb(128, 128, 0)),
        "brown" => Some(Color::Rgb(165, 42, 42)),
        _ => None,
    }
}

/// Convert a `Color` to its display string.
/// Extended named colors (those added beyond ratatui's built-ins) are returned
/// as their human-readable name rather than ratatui's hex representation.
pub fn color_to_string(c: Color) -> String {
    match c {
        Color::Rgb(255, 165, 0) => "Orange".to_string(),
        Color::Rgb(255, 105, 180) => "Pink".to_string(),
        Color::Rgb(128, 0, 128) => "Purple".to_string(),
        Color::Rgb(238, 130, 238) => "Violet".to_string(),
        Color::Rgb(75, 0, 130) => "Indigo".to_string(),
        Color::Rgb(0, 128, 128) => "Teal".to_string(),
        Color::Rgb(64, 224, 208) => "Turquoise".to_string(),
        Color::Rgb(255, 127, 80) => "Coral".to_string(),
        Color::Rgb(250, 128, 114) => "Salmon".to_string(),
        Color::Rgb(255, 215, 0) => "Gold".to_string(),
        Color::Rgb(0, 255, 0) => "Lime".to_string(),
        Color::Rgb(128, 0, 0) => "Maroon".to_string(),
        Color::Rgb(0, 0, 128) => "Navy".to_string(),
        Color::Rgb(128, 128, 0) => "Olive".to_string(),
        Color::Rgb(165, 42, 42) => "Brown".to_string(),
        other => format!("{other}"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Fatal,
    #[default]
    Unknown,
}

impl LogLevel {
    pub fn parse_level(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "trace" | "trc" => LogLevel::Trace,
            "debug" | "dbg" => LogLevel::Debug,
            "info" | "inf" => LogLevel::Info,
            "notice" => LogLevel::Notice,
            "warn" | "warning" | "wrn" => LogLevel::Warning,
            "error" | "err" => LogLevel::Error,
            "fatal" | "ftl" | "critical" | "crit" | "emerg" | "alert" => LogLevel::Fatal,
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
            if w4 == *b"FATA" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'L') {
                return LogLevel::Fatal;
            }
            if w4 == *b"CRIT" {
                return LogLevel::Fatal;
            }
            if w4 == *b"EMER" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'G') {
                return LogLevel::Fatal;
            }
            if w4 == *b"ALER" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'T') {
                return LogLevel::Fatal;
            }
            if w4 == *b"ERRO" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'R') {
                return LogLevel::Error;
            }
            if w4 == *b"WARN" {
                return LogLevel::Warning;
            }
            if w4 == *b"NOTI"
                && i + 6 <= line.len()
                && line[i + 4].eq_ignore_ascii_case(&b'C')
                && line[i + 5].eq_ignore_ascii_case(&b'E')
            {
                return LogLevel::Notice;
            }
            if w4 == *b"INFO" {
                return LogLevel::Info;
            }
            if w4 == *b"DEBU" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'G') {
                return LogLevel::Debug;
            }
            if w4 == *b"TRAC" && i + 5 <= line.len() && line[i + 4].eq_ignore_ascii_case(&b'E') {
                return LogLevel::Trace;
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

/// A running Docker container discovered by `docker ps`.
#[derive(Debug, Clone)]
pub struct DockerContainer {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
}

/// Controls which structured columns are shown and in what order.
#[derive(Debug, Clone, Default)]
pub struct FieldLayout {
    /// When Some: ordered list of all column names (visible and hidden).
    /// Names: "timestamp"|"ts"|"time", "level"|"lvl", "target", "span",
    ///         "message"|"msg", or any extra-field key present in the line.
    /// Visibility of each column is controlled by `hidden_fields` in `TabState`.
    /// When None: show all columns in default order.
    pub columns: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::parse_level("trace"), LogLevel::Trace);
        assert_eq!(LogLevel::parse_level("TRC"), LogLevel::Trace);
        assert_eq!(LogLevel::parse_level("debug"), LogLevel::Debug);
        assert_eq!(LogLevel::parse_level("DBG"), LogLevel::Debug);
        assert_eq!(LogLevel::parse_level("info"), LogLevel::Info);
        assert_eq!(LogLevel::parse_level("INFO"), LogLevel::Info);
        assert_eq!(LogLevel::parse_level("INF"), LogLevel::Info);
        assert_eq!(LogLevel::parse_level("notice"), LogLevel::Notice);
        assert_eq!(LogLevel::parse_level("warn"), LogLevel::Warning);
        assert_eq!(LogLevel::parse_level("WARNING"), LogLevel::Warning);
        assert_eq!(LogLevel::parse_level("WRN"), LogLevel::Warning);
        assert_eq!(LogLevel::parse_level("error"), LogLevel::Error);
        assert_eq!(LogLevel::parse_level("ERR"), LogLevel::Error);
        assert_eq!(LogLevel::parse_level("fatal"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("FTL"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("critical"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("CRIT"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("emerg"), LogLevel::Fatal);
        assert_eq!(LogLevel::parse_level("alert"), LogLevel::Fatal);
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
        // New levels
        assert_eq!(
            LogLevel::detect_from_bytes(b"TRACE entering function"),
            LogLevel::Trace
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"NOTICE system event"),
            LogLevel::Notice
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"FATAL system crash"),
            LogLevel::Fatal
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"CRITICAL out of memory"),
            LogLevel::Fatal
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"EMERG kernel panic"),
            LogLevel::Fatal
        );
        assert_eq!(
            LogLevel::detect_from_bytes(b"ALERT security breach"),
            LogLevel::Fatal
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

    #[test]
    fn test_parse_color_extended_named() {
        assert_eq!(parse_color("orange"), Some(Color::Rgb(255, 165, 0)));
        assert_eq!(parse_color("pink"), Some(Color::Rgb(255, 105, 180)));
        assert_eq!(parse_color("purple"), Some(Color::Rgb(128, 0, 128)));
        assert_eq!(parse_color("violet"), Some(Color::Rgb(238, 130, 238)));
        assert_eq!(parse_color("indigo"), Some(Color::Rgb(75, 0, 130)));
        assert_eq!(parse_color("teal"), Some(Color::Rgb(0, 128, 128)));
        assert_eq!(parse_color("turquoise"), Some(Color::Rgb(64, 224, 208)));
        assert_eq!(parse_color("coral"), Some(Color::Rgb(255, 127, 80)));
        assert_eq!(parse_color("salmon"), Some(Color::Rgb(250, 128, 114)));
        assert_eq!(parse_color("gold"), Some(Color::Rgb(255, 215, 0)));
        assert_eq!(parse_color("lime"), Some(Color::Rgb(0, 255, 0)));
        assert_eq!(parse_color("maroon"), Some(Color::Rgb(128, 0, 0)));
        assert_eq!(parse_color("navy"), Some(Color::Rgb(0, 0, 128)));
        assert_eq!(parse_color("olive"), Some(Color::Rgb(128, 128, 0)));
        assert_eq!(parse_color("brown"), Some(Color::Rgb(165, 42, 42)));
    }

    #[test]
    fn test_color_to_string_extended_named() {
        assert_eq!(color_to_string(Color::Rgb(255, 165, 0)), "Orange");
        assert_eq!(color_to_string(Color::Rgb(255, 105, 180)), "Pink");
        assert_eq!(color_to_string(Color::Rgb(128, 0, 128)), "Purple");
        assert_eq!(color_to_string(Color::Rgb(0, 128, 128)), "Teal");
        assert_eq!(color_to_string(Color::Rgb(165, 42, 42)), "Brown");
        assert_eq!(color_to_string(Color::Rgb(0, 0, 128)), "Navy");
    }

    #[test]
    fn test_color_to_string_ratatui_named_passes_through() {
        assert_eq!(color_to_string(Color::Red), "Red");
        assert_eq!(color_to_string(Color::LightBlue), "LightBlue");
    }

    #[test]
    fn test_color_to_string_roundtrip() {
        for name in &["Orange", "Pink", "Purple", "Teal", "Navy", "Brown"] {
            let color = parse_color(name).unwrap();
            assert_eq!(color_to_string(color).to_lowercase(), name.to_lowercase());
        }
    }

    #[test]
    fn test_parse_color_extended_named_case_insensitive() {
        assert_eq!(parse_color("Orange"), Some(Color::Rgb(255, 165, 0)));
        assert_eq!(parse_color("ORANGE"), Some(Color::Rgb(255, 165, 0)));
        assert_eq!(parse_color("Teal"), Some(Color::Rgb(0, 128, 128)));
        assert_eq!(parse_color("NAVY"), Some(Color::Rgb(0, 0, 128)));
        assert_eq!(parse_color("Brown"), Some(Color::Rgb(165, 42, 42)));
    }
}
