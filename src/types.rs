use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

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
    pub fn from_str(s: &str) -> Self {
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
            if w4 == *b"DEBU" {
                if i + 5 <= line.len() && line[i + 4].to_ascii_uppercase() == b'G' {
                    return LogLevel::Debug;
                }
            }
            if w4 == *b"ERRO" {
                if i + 5 <= line.len() && line[i + 4].to_ascii_uppercase() == b'R' {
                    return LogLevel::Error;
                }
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

/// Result of a search operation: which line index it was on and where in that line.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub line_idx: usize,
    pub matches: Vec<(usize, usize)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::from_str("info"), LogLevel::Info);
        assert_eq!(LogLevel::from_str("INFO"), LogLevel::Info);
        assert_eq!(LogLevel::from_str("warn"), LogLevel::Warning);
        assert_eq!(LogLevel::from_str("WARNING"), LogLevel::Warning);
        assert_eq!(LogLevel::from_str("error"), LogLevel::Error);
        assert_eq!(LogLevel::from_str("ERR"), LogLevel::Error);
        assert_eq!(LogLevel::from_str("debug"), LogLevel::Debug);
        assert_eq!(LogLevel::from_str("unknown"), LogLevel::Unknown);
    }

    #[test]
    fn test_log_level_detect_from_bytes() {
        assert_eq!(LogLevel::detect_from_bytes(b"some INFO message"), LogLevel::Info);
        assert_eq!(LogLevel::detect_from_bytes(b"WARN: disk full"), LogLevel::Warning);
        assert_eq!(LogLevel::detect_from_bytes(b"ERROR: connection lost"), LogLevel::Error);
        assert_eq!(LogLevel::detect_from_bytes(b"DEBUG: value=5"), LogLevel::Debug);
        assert_eq!(LogLevel::detect_from_bytes(b"plain log line"), LogLevel::Unknown);
        // Case insensitive
        assert_eq!(LogLevel::detect_from_bytes(b"error happened"), LogLevel::Error);
        assert_eq!(LogLevel::detect_from_bytes(b"warn about something"), LogLevel::Warning);
    }

    #[test]
    fn test_filter_type_display() {
        assert_eq!(FilterType::Include.to_string(), "Include");
        assert_eq!(FilterType::Exclude.to_string(), "Exclude");
    }
}
