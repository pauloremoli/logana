use crate::ui::TabState;

/// The active tab's counts/position, for the bottom status bar — pure data,
/// resolved once per render so the view layer never touches `TabState`
/// directly. Mirrors the `src/gui/color.rs` pattern: small, pure resolver
/// functions feeding dumb render functions.
pub struct StatusBarInfo {
    pub total: usize,
    pub showing: usize,
    pub filtered: usize,
    /// 1-based line number at the cursor, within the currently visible
    /// set — `None` when there's nothing visible to point at.
    pub current_line_1based: Option<usize>,
    /// Scroll position as a percentage through the visible set, 0-100.
    /// `None` alongside `current_line_1based` when nothing is visible.
    pub percent: Option<u8>,
}

pub fn status_bar_info(tab: &TabState) -> StatusBarInfo {
    let total = tab.file_reader.line_count();
    let showing = tab.filter.visible_indices.len();
    let filtered = total.saturating_sub(showing);
    let current_line_1based = tab
        .filter
        .visible_indices
        .get_opt(tab.scroll.scroll_offset)
        .map(|line_idx| line_idx + 1);
    let percent = if showing == 0 {
        None
    } else if showing == 1 {
        Some(100)
    } else {
        let offset = tab.scroll.scroll_offset.min(showing - 1);
        Some(((offset * 100) / (showing - 1)) as u8)
    };
    StatusBarInfo {
        total,
        showing,
        filtered,
        current_line_1based,
        percent,
    }
}

pub fn status_bar_left_text(info: &StatusBarInfo) -> String {
    format!(
        "Total: {}   Showing: {}   Filtered: {}",
        info.total, info.showing, info.filtered
    )
}

/// `logana` has no intra-line cursor/column concept (it's a line-oriented
/// viewer, not a text editor), so "Col" is always rendered as `1` — a
/// deliberate simplification, not a placeholder for state that should
/// exist, documented in the GUI redesign plan.
pub fn status_bar_right_text(info: &StatusBarInfo) -> String {
    match (info.current_line_1based, info.percent) {
        (Some(line), Some(percent)) => format!("Ln {line}, Col 1   •   {percent}%"),
        _ => "Ln -, Col 1".to_string(),
    }
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
    async fn no_filtering_shows_everything() {
        let tab = tab_with_lines(&["a", "b", "c"]).await;
        let info = status_bar_info(&tab);
        assert_eq!(info.total, 3);
        assert_eq!(info.showing, 3);
        assert_eq!(info.filtered, 0);
    }

    #[tokio::test]
    async fn filtered_lines_are_counted() {
        let mut tab = tab_with_lines(&["a", "b", "c"]).await;
        tab.filter.visible_indices = crate::ui::VisibleLines::Filtered(vec![0, 2]);
        let info = status_bar_info(&tab);
        assert_eq!(info.total, 3);
        assert_eq!(info.showing, 2);
        assert_eq!(info.filtered, 1);
    }

    #[tokio::test]
    async fn cursor_at_last_line_is_100_percent() {
        let mut tab = tab_with_lines(&["a", "b", "c"]).await;
        tab.scroll.scroll_offset = 2;
        let info = status_bar_info(&tab);
        assert_eq!(info.current_line_1based, Some(3));
        assert_eq!(info.percent, Some(100));
    }

    #[tokio::test]
    async fn cursor_at_first_line_is_0_percent() {
        let tab = tab_with_lines(&["a", "b", "c"]).await;
        let info = status_bar_info(&tab);
        assert_eq!(info.current_line_1based, Some(1));
        assert_eq!(info.percent, Some(0));
    }

    #[tokio::test]
    async fn empty_visible_set_has_no_position() {
        let mut tab = tab_with_lines(&["a", "b", "c"]).await;
        tab.filter.visible_indices = crate::ui::VisibleLines::Filtered(vec![]);
        let info = status_bar_info(&tab);
        assert_eq!(info.current_line_1based, None);
        assert_eq!(info.percent, None);
    }

    #[tokio::test]
    async fn left_text_matches_the_mockup_format() {
        let tab = tab_with_lines(&["a", "b"]).await;
        let info = status_bar_info(&tab);
        assert_eq!(
            status_bar_left_text(&info),
            "Total: 2   Showing: 2   Filtered: 0"
        );
    }

    #[tokio::test]
    async fn right_text_matches_the_mockup_format() {
        let tab = tab_with_lines(&["a", "b"]).await;
        let info = status_bar_info(&tab);
        assert_eq!(status_bar_right_text(&info), "Ln 1, Col 1   •   0%");
    }

    #[tokio::test]
    async fn right_text_falls_back_when_nothing_is_visible() {
        let mut tab = tab_with_lines(&["a", "b"]).await;
        tab.filter.visible_indices = crate::ui::VisibleLines::Filtered(vec![]);
        let info = status_bar_info(&tab);
        assert_eq!(status_bar_right_text(&info), "Ln -, Col 1");
    }
}
