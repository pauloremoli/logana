#[derive(Default)]
pub struct ScrollState {
    pub scroll_offset: usize,
    pub viewport_offset: usize,
    pub viewport_wrapped_row: usize,
    pub horizontal_scroll: usize,
    /// Wrapped row within the selected log entry.
    pub wrapped_row: usize,
    pub wrapped_line: usize,
    pub wrapped_width: usize,
    pub wrapped_prefix_width: usize,
    /// Visible screen rows mapped back to the original log-entry positions.
    pub rendered_rows: Option<Vec<Option<usize>>>,
    pub visible_height: usize,
    pub visible_width: usize,
}

impl super::TabState {
    pub(crate) fn wrapped_rows_for_line(&self, line_idx: usize) -> usize {
        let text = self.get_display_text(line_idx);
        let text = format!("{}{}", " ".repeat(self.scroll.wrapped_prefix_width), text);
        ratatui::widgets::Paragraph::new(text)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .line_count(self.scroll.wrapped_width as u16)
            .max(1)
    }

    fn current_wrapped_rows(&self) -> usize {
        self.filter
            .visible_indices
            .get_opt(self.scroll.scroll_offset)
            .map(|line_idx| self.wrapped_rows_for_line(line_idx))
            .unwrap_or(1)
    }

    /// Move by terminal rows in wrap mode, retaining log-entry selection semantics.
    pub fn scroll_rows(&mut self, down: bool, mut rows: usize) {
        if !self.display.wrap || self.scroll.wrapped_width == 0 {
            self.scroll.wrapped_row = 0;
            self.scroll.scroll_offset = if down {
                self.scroll.scroll_offset.saturating_add(rows)
            } else {
                self.scroll.scroll_offset.saturating_sub(rows)
            };
            return;
        }
        self.clamp_scroll_offset();
        if self.scroll.wrapped_line != self.scroll.scroll_offset {
            self.scroll.wrapped_row = 0;
        }
        self.scroll.wrapped_row = self.scroll.wrapped_row.min(self.current_wrapped_rows() - 1);
        while rows > 0 && !self.filter.visible_indices.is_empty() {
            let last_row = self.current_wrapped_rows() - 1;
            self.scroll.wrapped_row = self.scroll.wrapped_row.min(last_row);
            let available = if down {
                last_row - self.scroll.wrapped_row
            } else {
                self.scroll.wrapped_row
            };
            let step = rows.min(available);
            if down {
                self.scroll.wrapped_row += step;
            } else {
                self.scroll.wrapped_row -= step;
            }
            rows -= step;
            if rows == 0 {
                break;
            }
            if down {
                if self.scroll.scroll_offset + 1 >= self.filter.visible_indices.len() {
                    break;
                }
                self.scroll.scroll_offset += 1;
                self.scroll.wrapped_row = 0;
            } else {
                if self.scroll.scroll_offset == 0 {
                    break;
                }
                self.scroll.scroll_offset -= 1;
                self.scroll.wrapped_row = self.current_wrapped_rows() - 1;
            }
            rows -= 1;
        }
        self.scroll.wrapped_line = self.scroll.scroll_offset;
    }
}
