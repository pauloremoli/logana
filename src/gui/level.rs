use crate::parser::LogLevel;
use crate::theme::Theme as TuiTheme;
use crate::ui::TabState;
use crate::ui::widgets::log_panel::classify_level;
use gpui_kit::gpui::Rgba;

/// Classifies a log line's level for the table's Level column. Only
/// meaningful for a tab with a detected structured format — a plain
/// unstructured file has no reliable "level" field to read, so it always
/// classifies as `Unknown` (rendered as a blank cell), rather than
/// guessing from line content the way the TUI's own log panel does for
/// plain text.
pub fn classify_line_level(tab: &TabState, bytes: &[u8]) -> LogLevel {
    let Some(parser) = tab.display.format.as_deref() else {
        return LogLevel::Unknown;
    };
    let Some(raw_level) = parser.parse_line(bytes).and_then(|parts| parts.level) else {
        return LogLevel::Unknown;
    };
    classify_level(Some(parser), raw_level)
}

/// `(background, foreground)` for a level pill, or `None` for a blank cell
/// (`LogLevel::Unknown`). Background reuses the TUI's own per-level theme
/// color as a solid swatch (the TUI itself only ever uses these as text
/// colors or line backgrounds, never both at once); foreground is chosen
/// for contrast against it via the TUI's own `readable_foreground_for`.
pub fn level_pill_colors(level: &LogLevel, theme: &TuiTheme) -> Option<(Rgba, Rgba)> {
    let bg_color = match level {
        LogLevel::Trace => theme.trace_fg,
        LogLevel::Debug => theme.debug_fg,
        LogLevel::Info => theme.info_fg,
        LogLevel::Notice => theme.notice_fg,
        LogLevel::Warning => theme.warning_fg,
        LogLevel::Error => theme.error_fg,
        LogLevel::Fatal => theme.fatal_fg,
        LogLevel::Unknown => return None,
    };
    let bg_rgb = crate::theme::color_to_rgb(bg_color);
    let fg_rgb = crate::ui::theme::readable_foreground_for(bg_rgb);
    let bg = super::color::ratatui_color_to_gpui(bg_color)?;
    let fg = super::color::ratatui_color_to_gpui(ratatui::style::Color::Rgb(
        fg_rgb.0, fg_rgb.1, fg_rgb.2,
    ))?;
    Some((bg, fg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, LogManager};
    use crate::ingestion::FileReader;
    use std::sync::Arc;

    async fn tab_with_lines(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(reader, log_manager, "test.log".to_string())
    }

    #[tokio::test]
    async fn unstructured_file_is_always_unknown() {
        let tab = tab_with_lines(&["plain text line"]).await;
        assert!(tab.display.format.is_none());
        assert_eq!(
            classify_line_level(&tab, b"plain text line"),
            LogLevel::Unknown
        );
    }

    #[test]
    fn unknown_level_has_no_pill() {
        let theme = TuiTheme::default();
        assert_eq!(level_pill_colors(&LogLevel::Unknown, &theme), None);
    }

    #[test]
    fn every_known_level_has_a_pill() {
        let theme = TuiTheme::default();
        for level in [
            LogLevel::Trace,
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Notice,
            LogLevel::Warning,
            LogLevel::Error,
            LogLevel::Fatal,
        ] {
            assert!(
                level_pill_colors(&level, &theme).is_some(),
                "{level:?} should have a pill color"
            );
        }
    }

    #[test]
    fn pill_foreground_is_readable_against_its_background() {
        let theme = TuiTheme::default();
        for level in [LogLevel::Error, LogLevel::Info, LogLevel::Warning] {
            let (bg, fg) = level_pill_colors(&level, &theme).unwrap();
            let bg_rgb = (
                (bg.r * 255.0).round() as u8,
                (bg.g * 255.0).round() as u8,
                (bg.b * 255.0).round() as u8,
            );
            let fg_rgb = (
                (fg.r * 255.0).round() as u8,
                (fg.g * 255.0).round() as u8,
                (fg.b * 255.0).round() as u8,
            );
            let readable = crate::ui::theme::readable_foreground_for(bg_rgb);
            assert_eq!(fg_rgb, readable, "{level:?}");
        }
    }
}
