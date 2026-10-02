use crate::gui::app::App;
use crate::gui::color::{ratatui_color_to_gpui, resolve_line_style};
use crate::mode::app_mode::ModeRenderState;
use crate::theme::Theme as TuiTheme;
use crate::ui::TabState;
use gpui_kit::base::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{AnyElement, Context, Size, div, px};
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
fn line_row(tab: &TabState, line_idx: usize, selected: bool, theme: &TuiTheme) -> AnyElement {
    let owned_line_bytes = tab.file_reader.get_line(line_idx);
    let bytes: &[u8] = &owned_line_bytes;
    if selected {
        let text = String::from_utf8_lossy(bytes).into_owned();
        // `w_full()` matters beyond general tidiness: without it the row
        // shrink-wraps to its text content, so a selection `.bg()` only
        // colors a narrow strip behind the characters instead of the
        // whole row — easy to miss entirely as a "this line is selected"
        // signal.
        let mut row = div().w_full().truncate().child(text);
        if let Some(bg) = ratatui_color_to_gpui(theme.visual_select_bg) {
            row = row.bg(bg);
        }
        if let Some(fg) = ratatui_color_to_gpui(theme.visual_select_fg) {
            row = row.text_color(fg);
        }
        return row.into_any_element();
    }
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut row = div().w_full().truncate().child(text);
    // Splitting the line into separate per-span text children (so only
    // the matched substring, not the whole row, carries the color) made
    // every styled row render far taller than its 20px virtual-list slot
    // and bleed into the rows below it — reverted to coloring the whole
    // row until that layout issue is understood; see `LineStyle::spans`
    // for the still-correct match-only byte ranges, unused here for now.
    if let Some(style) = resolve_line_style(
        bytes,
        tab.log_manager.get_filters(),
        tab.log_manager.get_group_styles(),
    ) {
        if let Some(bg) = style.bg {
            row = row.bg(bg);
        }
        if let Some(fg) = style.fg {
            row = row.text_color(fg);
        }
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

    /// A tab backed by `Storage::Paged` (how real large files load), to
    /// guard against `line_row` reaching for `get_line_zero_copy`, which
    /// panics for this storage kind — reproduces the panic reported when
    /// opening a file large enough to trigger paged loading.
    async fn paged_tab() -> TabState {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"a\nb\nc\n").unwrap();
        f.flush().unwrap();
        let path = f.path().to_str().unwrap().to_string();
        let canonical = Arc::new(std::fs::canonicalize(&path).unwrap());
        let reader = FileReader::try_new_paged(&path, canonical)
            .unwrap()
            .expect("plain text file should build Storage::Paged");
        assert!(reader.is_paged());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(reader, log_manager, "test.log".to_string())
    }

    #[tokio::test]
    async fn line_row_does_not_panic_for_paged_storage() {
        let tab = paged_tab().await;
        let theme = TuiTheme::default();
        let _ = line_row(&tab, 0, false, &theme);
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
