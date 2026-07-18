#[derive(Default)]
pub struct ScrollState {
    pub scroll_offset: usize,
    pub viewport_offset: usize,
    pub horizontal_scroll: usize,
    pub visible_height: usize,
    pub visible_width: usize,
    /// Width of the widest currently-rendered log line, written back each
    /// render pass by `prepare_log_panel` — used to size/clamp the
    /// horizontal scrollbar. `0` while wrapped (horizontal scroll is
    /// disabled in that mode).
    pub max_line_width: usize,
}
