#[derive(Default)]
pub struct ScrollState {
    pub scroll_offset: usize,
    pub viewport_offset: usize,
    pub horizontal_scroll: usize,
    pub visible_height: usize,
    pub visible_width: usize,
}
