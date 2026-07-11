use ratatui::prelude::{Position, Rect};

use super::TabState;

pub struct InputHandler {
    pub log_panel_area: Rect,
    pub sidebar_area: Option<Rect>,
    pub last_click: Option<(std::time::Instant, u16, u16)>,
    pub scrollbar_dragging: bool,
}

impl InputHandler {
    pub fn hit_test_scrollbar(&self, col: u16, row: u16, tab: &TabState) -> Option<usize> {
        let area = self.log_panel_area;
        if row < area.y || row >= area.y + area.height {
            return None;
        }
        let show_borders = tab.display.show_borders;
        let scrollbar_col = if show_borders {
            area.right().saturating_sub(2)
        } else {
            area.right().saturating_sub(1)
        };
        if col != scrollbar_col {
            return None;
        }
        let total_visible = tab.filter.visible_indices.len();
        let max_scroll = total_visible.saturating_sub(tab.scroll.visible_height);
        let bar_height = area.height as usize;
        let pos = (row - area.y) as usize;
        Some(((pos * max_scroll) / bar_height.max(1)).min(max_scroll))
    }

    pub fn hit_test_sidebar(&self, col: u16, row: u16, tab: &TabState) -> Option<usize> {
        use crate::mode::app_mode::ModeRenderState;

        let area = self.sidebar_area?;
        if !area.contains(Position::new(col, row)) {
            return None;
        }
        let filters = tab.log_manager.get_filters();
        let num_filters = filters.len();
        if num_filters == 0 {
            return None;
        }
        let (inner_width, inner_height) = if tab.display.show_borders {
            (
                area.width.saturating_sub(2) as usize,
                area.height.saturating_sub(2) as usize,
            )
        } else {
            (
                area.width.saturating_sub(1) as usize,
                area.height.saturating_sub(1) as usize,
            )
        };
        let selected = match tab.interaction.mode.render_state() {
            ModeRenderState::FilterManagement { selected_index } => selected_index,
            _ => tab.filter.filter_context.unwrap_or(0),
        };
        // The sidebar auto-scrolls to keep `selected` in view; replicate that
        // scroll here so a click's screen row maps to the filter actually
        // rendered there, not the filter at that row assuming no scroll.
        let scroll = super::widgets::sidebar::compute_scroll_offset(
            filters,
            selected,
            &tab.filter.match_counts,
            inner_width,
            inner_height,
        );
        let target_row = row.saturating_sub(area.y + 1) as usize + scroll;
        let mut accumulated = 0usize;
        for (idx, filter) in filters.iter().enumerate() {
            let text = super::widgets::sidebar::filter_row_display_text(
                filter,
                idx,
                selected,
                &tab.filter.match_counts,
            );
            let rc = super::field_layout::line_row_count(text.as_bytes(), inner_width);
            if accumulated + rc > target_row {
                return Some(idx);
            }
            accumulated += rc;
        }
        Some(num_filters.saturating_sub(1))
    }

    pub fn hit_test_log_panel(&self, col: u16, row: u16, tab: &TabState) -> Option<usize> {
        let area = self.log_panel_area;
        let show_borders = tab.display.show_borders;
        let show_tab_bar = true;
        let x_off: u16 = if show_borders { 1 } else { 0 };
        let y_off: u16 = if show_borders && !show_tab_bar { 1 } else { 0 };
        let height_sub: u16 = if show_borders {
            if show_tab_bar { 1 } else { 2 }
        } else {
            0
        };
        let inner = Rect {
            x: area.x + x_off,
            y: area.y + y_off,
            width: area.width.saturating_sub(x_off * 2 + 1),
            height: area.height.saturating_sub(height_sub),
        };
        if !inner.contains(Position::new(col, row)) {
            return None;
        }
        let visual_row = (row - inner.y) as usize;
        if !tab.display.wrap {
            let visible_idx = tab.scroll.viewport_offset + visual_row;
            return if visible_idx < tab.filter.visible_indices.len() {
                Some(visible_idx)
            } else {
                None
            };
        }
        let inner_width = tab.scroll.visible_width;
        let parser = tab.display.format.as_deref();
        let field_layout = &tab.display.field_layout;
        let hidden_fields = &tab.display.hidden_fields;
        let show_keys = tab.display.show_keys;
        let visible_count = tab.filter.visible_indices.len();
        let mut accumulated = 0usize;
        let mut idx = tab.scroll.viewport_offset;
        while idx < visible_count {
            let line_bytes = tab
                .file_reader
                .get_line(tab.filter.visible_indices.get(idx));
            let rc = super::field_layout::effective_row_count(
                line_bytes,
                inner_width,
                parser,
                field_layout,
                hidden_fields,
                show_keys,
            );
            if accumulated + rc > visual_row {
                return Some(idx);
            }
            accumulated += rc;
            idx += 1;
        }
        None
    }

    pub fn col_to_char_offset(&self, col: u16, tab: &TabState) -> usize {
        let area = self.log_panel_area;
        let x_off: u16 = if tab.display.show_borders { 1 } else { 0 };
        let total_lines = tab.file_reader.line_count();
        let ln_prefix: u16 = if tab.display.show_line_numbers {
            (total_lines.max(1).to_string().len() + 2) as u16
        } else {
            0
        };
        col.saturating_sub(area.x + x_off + ln_prefix) as usize + tab.scroll.horizontal_scroll
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ratatui::prelude::Rect;

    use crate::db::LogManager;
    use crate::ingestion::FileReader;

    use super::*;

    async fn fixture(
        visible_lines: usize,
        visible_height: usize,
        log_area: Rect,
        sidebar_area: Option<Rect>,
    ) -> (InputHandler, TabState) {
        let db = Arc::new(crate::db::Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let data: Vec<u8> = (0..visible_lines)
            .map(|i| format!("line {}\n", i))
            .collect::<String>()
            .into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let mut tab = TabState::new(file_reader, log_manager, "test".to_string());
        tab.filter.visible_indices = crate::ui::VisibleLines::All(visible_lines);
        tab.display.show_borders = false;
        tab.scroll.visible_height = visible_height;
        let handler = InputHandler {
            log_panel_area: log_area,
            sidebar_area,
            last_click: None,
            scrollbar_dragging: false,
        };
        (handler, tab)
    }

    #[tokio::test]
    async fn test_hit_test_scrollbar_correct_column_no_borders() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 40,
        };
        let (h, tab) = fixture(200, 40, area, None).await;
        assert!(h.hit_test_scrollbar(79, 10, &tab).is_some());
        assert!(h.hit_test_scrollbar(78, 10, &tab).is_none());
    }

    #[tokio::test]
    async fn test_hit_test_scrollbar_out_of_row_range() {
        let area = Rect {
            x: 0,
            y: 5,
            width: 80,
            height: 20,
        };
        let (h, tab) = fixture(200, 20, area, None).await;
        assert!(h.hit_test_scrollbar(79, 4, &tab).is_none());
        assert!(h.hit_test_scrollbar(79, 25, &tab).is_none());
        assert!(h.hit_test_scrollbar(79, 5, &tab).is_some());
        assert!(h.hit_test_scrollbar(79, 24, &tab).is_some());
    }

    #[tokio::test]
    async fn test_hit_test_scrollbar_proportional_position() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 10,
        };
        let (h, tab) = fixture(100, 10, area, None).await;
        assert_eq!(h.hit_test_scrollbar(79, 5, &tab).unwrap(), 45);
        assert_eq!(h.hit_test_scrollbar(79, 0, &tab).unwrap(), 0);
        assert!(h.hit_test_scrollbar(79, 9, &tab).unwrap() <= 90);
    }

    #[tokio::test]
    async fn test_hit_test_sidebar_outside_returns_none() {
        let log_area = Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 40,
        };
        let sidebar_area = Rect {
            x: 60,
            y: 0,
            width: 20,
            height: 40,
        };
        let (h, tab) = fixture(10, 10, log_area, Some(sidebar_area)).await;
        assert!(h.hit_test_sidebar(59, 0, &tab).is_none());
        assert!(h.hit_test_sidebar(80, 0, &tab).is_none());
    }

    #[tokio::test]
    async fn test_hit_test_sidebar_no_sidebar_returns_none() {
        let log_area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 40,
        };
        let (h, tab) = fixture(10, 10, log_area, None).await;
        assert!(h.hit_test_sidebar(70, 5, &tab).is_none());
    }

    #[tokio::test]
    async fn test_hit_test_log_panel_maps_row_to_visible_idx() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let (h, mut tab) = fixture(50, 20, area, None).await;
        tab.scroll.viewport_offset = 5;
        assert_eq!(h.hit_test_log_panel(10, 3, &tab), Some(8));
        assert_eq!(h.hit_test_log_panel(10, 0, &tab), Some(5));
    }

    #[tokio::test]
    async fn test_hit_test_log_panel_with_borders_no_top_border() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let (h, mut tab) = fixture(50, 20, area, None).await;
        tab.display.show_borders = true;
        tab.scroll.viewport_offset = 5;
        assert_eq!(h.hit_test_log_panel(10, 0, &tab), Some(5));
        assert_eq!(h.hit_test_log_panel(10, 3, &tab), Some(8));
        assert_eq!(h.hit_test_log_panel(10, 19, &tab), None);
    }

    #[tokio::test]
    async fn test_hit_test_log_panel_outside_returns_none() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let (h, tab) = fixture(50, 20, area, None).await;
        assert!(h.hit_test_log_panel(79, 5, &tab).is_none());
        assert!(h.hit_test_log_panel(10, 20, &tab).is_none());
    }

    #[tokio::test]
    async fn test_hit_test_log_panel_beyond_visible_lines_returns_none() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 20,
        };
        let (h, mut tab) = fixture(5, 20, area, None).await;
        tab.scroll.scroll_offset = 0;
        assert!(h.hit_test_log_panel(10, 5, &tab).is_none());
        assert!(h.hit_test_log_panel(10, 4, &tab).is_some());
    }
}
