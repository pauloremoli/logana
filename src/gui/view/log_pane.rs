use crate::gui::app::App;
use crate::gui::color::{ratatui_color_to_gpui, resolve_line_color};
use crate::mode::app_mode::ModeRenderState;
use crate::theme::Theme as TuiTheme;
use crate::ui::TabState;
use gpui_kit::base::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Size, div, px};
use std::rc::Rc;

/// Fixed row height, used for every visible log line — the virtual list
/// only needs height to lay out rows; width is inferred from content.
const ROW_HEIGHT_PX: f32 = 20.0;

/// Renders only the currently-visible window of `tab.filter.visible_indices`,
/// via gpui-component's virtual-scrolling list — no manual viewport math
/// or rendered-line cap, unlike the discarded iced prototype. `log_scroll`
/// is `App`'s own handle (not reachable through `cx`, which derefs to
/// gpui's own `App` platform type, not ours), passed in so `App::
/// sync_log_scroll` can drive the same handle after every dispatch.
pub fn log_pane(
    tab_idx: usize,
    tab: &TabState,
    log_scroll: &VirtualListScrollHandle,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let total = tab.filter.visible_indices.len();
    let item_sizes = Rc::new(vec![Size::new(px(0.0), px(ROW_HEIGHT_PX)); total]);
    let view = cx.entity();
    v_virtual_list(
        view,
        "log-pane",
        item_sizes,
        move |app: &mut App, range, _window, _cx| {
            let Some(tab) = app.state.tabs.get(tab_idx) else {
                return Vec::new();
            };
            let selection = visual_line_selection(tab);
            range
                .filter_map(|pos| {
                    tab.filter
                        .visible_indices
                        .get_opt(pos)
                        .map(|line_idx| (pos, line_idx))
                })
                .map(|(pos, line_idx)| {
                    line_row(
                        tab,
                        line_idx,
                        selection.is_some_and(|(lo, hi)| (lo..=hi).contains(&pos)),
                        &app.state.theme,
                    )
                })
                .collect()
        },
    )
    .track_scroll(log_scroll)
    .size_full()
}

/// `VisualLineMode`'s selected range, as `(low, high)` visible-position
/// indices inclusive — `VisualLineMode::anchor` and `tab.scroll.
/// scroll_offset` (the cursor) are both positions in the same visible-
/// index space the virtual list addresses rows by, so no translation is
/// needed beyond ordering the two endpoints.
fn visual_line_selection(tab: &TabState) -> Option<(usize, usize)> {
    match tab.interaction.mode.render_state() {
        ModeRenderState::VisualLine { anchor } => {
            let cursor = tab.scroll.scroll_offset;
            Some((anchor.min(cursor), anchor.max(cursor)))
        }
        _ => None,
    }
}

/// Each row renders as a single line, matching the TUI's default (`:wrap`
/// is off until a later phase implements it): wrapping a long line onto
/// multiple visual lines without the virtual list reserving extra height
/// for it just makes rows overlap, so overflow is clipped with an ellipsis
/// instead.
fn line_row(
    tab: &TabState,
    line_idx: usize,
    selected: bool,
    theme: &TuiTheme,
) -> gpui_kit::gpui::AnyElement {
    let bytes = tab.file_reader.get_line_zero_copy(line_idx);
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut row = div().truncate().child(text);
    if selected {
        if let Some(bg) = ratatui_color_to_gpui(theme.visual_select_bg) {
            row = row.bg(bg);
        }
        if let Some(fg) = ratatui_color_to_gpui(theme.visual_select_fg) {
            row = row.text_color(fg);
        }
    } else if let Some(color) = resolve_line_color(
        bytes,
        tab.log_manager.get_filters(),
        tab.log_manager.get_group_styles(),
    ) {
        row = row.text_color(color);
    }
    row.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, LogManager};
    use crate::ingestion::FileReader;
    use crate::mode::normal_mode::NormalMode;
    use crate::mode::visual_mode::VisualLineMode;
    use std::sync::Arc;

    async fn tab() -> TabState {
        let reader = FileReader::from_bytes(b"a\nb\nc\nd\ne\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(reader, log_manager, "test.log".to_string())
    }

    #[tokio::test]
    async fn no_selection_outside_visual_line_mode() {
        let mut t = tab().await;
        t.interaction.mode = Box::new(NormalMode::default());
        assert_eq!(visual_line_selection(&t), None);
    }

    #[tokio::test]
    async fn selection_spans_anchor_to_cursor_in_either_direction() {
        let mut t = tab().await;
        t.scroll.scroll_offset = 3;
        t.interaction.mode = Box::new(VisualLineMode {
            anchor: 1,
            count: None,
        });
        assert_eq!(visual_line_selection(&t), Some((1, 3)));

        t.scroll.scroll_offset = 0;
        t.interaction.mode = Box::new(VisualLineMode {
            anchor: 4,
            count: None,
        });
        assert_eq!(visual_line_selection(&t), Some((0, 4)));
    }

    #[tokio::test]
    async fn selection_is_a_single_line_when_anchor_equals_cursor() {
        let mut t = tab().await;
        t.scroll.scroll_offset = 2;
        t.interaction.mode = Box::new(VisualLineMode {
            anchor: 2,
            count: None,
        });
        assert_eq!(visual_line_selection(&t), Some((2, 2)));
    }
}
