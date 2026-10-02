use crate::gui::app::App;
use crate::gui::color::{LineStyle, ratatui_color_to_gpui, resolve_line_style};
use crate::gui::level::{classify_line_level, level_pill_colors};
use crate::mode::app_mode::ModeRenderState;
use crate::parser::LogLevel;
use crate::theme::Theme as TuiTheme;
use crate::ui::TabState;
use gpui_kit::base::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{AnyElement, Context, FontWeight, Rgba, Size, div, px};
use std::rc::Rc;

/// Fixed row height, used for every visible log line — the virtual list
/// only needs height to lay out rows; width is inferred from content.
const ROW_HEIGHT_PX: f32 = 20.0;

/// Column widths, shared between the fixed header row and every data row
/// so they can never drift apart. Message has no fixed width — it's the
/// only cell allowed to grow (`.flex_1()`).
const COL_LINE_NO_WIDTH: f32 = 56.0;
// Wide enough for a full microsecond-precision ISO 8601 timestamp
// ("2026-04-11T19:39:02.389121Z", 27 chars) without truncating.
const COL_TIME_WIDTH: f32 = 230.0;
const COL_LEVEL_WIDTH: f32 = 64.0;

/// Renders the log table: a fixed header row (`#`/Time/Level/Message)
/// above the currently-visible window of `tab.filter.visible_indices`,
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
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(header_row())
        .child(
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
                                pos == tab.scroll.scroll_offset,
                                &app.state.theme,
                            )
                        })
                        .collect()
                },
            )
            .track_scroll(log_scroll)
            .flex_1(),
        )
}

/// `#`/Time/Level/Message column titles, fixed above the scrolling list —
/// same width constants as `line_row`'s cells, so columns stay aligned.
fn header_row() -> impl IntoElement {
    div()
        .flex()
        .gap_2()
        .pb(px(4.))
        .h(px(ROW_HEIGHT_PX + 4.))
        .overflow_hidden()
        .child(fixed_cell(COL_LINE_NO_WIDTH, "#"))
        .child(fixed_cell(COL_TIME_WIDTH, "Time"))
        .child(fixed_cell(COL_LEVEL_WIDTH, "Level"))
        .child(div().flex_1().overflow_hidden().child("Message"))
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

/// The Time column's text for `bytes` — the detected format's parsed
/// timestamp, or blank for a tab with no detected format. Pure and
/// gpui-free so the unstructured-file fallback is unit-testable without
/// touching rendering.
fn row_time_text(tab: &TabState, bytes: &[u8]) -> Option<String> {
    let parser = tab.display.format.as_deref()?;
    let timestamp = parser.parse_line(bytes).and_then(|parts| parts.timestamp)?;
    Some(timestamp.to_string())
}

/// Each row renders as a `#`/Time/Level/Message table row, matching the
/// TUI's default (`:wrap` is off until a later phase implements it):
/// wrapping a long line onto multiple visual lines without the virtual
/// list reserving extra height for it just makes rows overlap, so
/// overflow is clipped instead. `.flex()` on the outer row is load-
/// bearing, not cosmetic — a plain `div()` defaults to block layout,
/// which stacks children (the four cells) vertically instead of flowing
/// them left to right; see `styled_line_row`'s doc comment for the exact
/// failure mode that caused in an earlier version of this file.
fn line_row(
    tab: &TabState,
    line_idx: usize,
    selected: bool,
    is_cursor_row: bool,
    theme: &TuiTheme,
) -> AnyElement {
    let owned_line_bytes = tab.file_reader.get_line(line_idx);
    let bytes: &[u8] = &owned_line_bytes;

    let number_cell = fixed_cell(COL_LINE_NO_WIDTH, (line_idx + 1).to_string());
    let time_cell = fixed_cell(
        COL_TIME_WIDTH,
        row_time_text(tab, bytes).unwrap_or_default(),
    );
    // Selection highlighting (below) already colors the whole row, so a
    // selected row skips the level pill and match-only highlighting and
    // just shows plain text in every cell — same as the TUI's visual
    // selection, which doesn't layer filter colors under the selection
    // tint either.
    let level_cell = if selected {
        fixed_cell(COL_LEVEL_WIDTH, "")
    } else {
        level_cell(classify_line_level(tab, bytes), theme)
    };
    let message_cell = if selected {
        div()
            .flex_1()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(String::from_utf8_lossy(bytes).into_owned())
            .into_any_element()
    } else {
        message_cell(tab, bytes)
    };

    let mut row = div()
        .flex()
        .gap_2()
        .h(px(ROW_HEIGHT_PX))
        .overflow_hidden()
        .child(number_cell)
        .child(time_cell)
        .child(level_cell)
        .child(message_cell);

    if selected {
        if let Some(bg) = ratatui_color_to_gpui(theme.visual_select_bg) {
            row = row.bg(bg);
        }
        if let Some(fg) = ratatui_color_to_gpui(theme.visual_select_fg) {
            row = row.text_color(fg);
        }
    }
    // Matches the TUI's own visual-selection render: every selected row
    // gets the bg/fg above, but only the one at the cursor (`tab.scroll.
    // scroll_offset`, one end of the selected range) also gets bold +
    // underline, marking which end is "active".
    if is_cursor_row {
        row = row.font_weight(FontWeight::BOLD).underline();
    }
    row.into_any_element()
}

/// A fixed-width, single-line text cell — `#`, Time, and the header row's
/// titles all use this. `.flex_shrink_0()` keeps it at exactly `width`
/// regardless of its own or siblings' content (a long Message must never
/// squeeze this column), matching `text_segment`'s existing rationale.
fn fixed_cell(width: f32, text: impl Into<gpui_kit::gpui::SharedString>) -> AnyElement {
    div()
        .flex_shrink_0()
        .w(px(width))
        .h(px(ROW_HEIGHT_PX))
        .overflow_hidden()
        .whitespace_nowrap()
        .child(text.into())
        .into_any_element()
}

/// The Level column: a small colored pill for a classified level, or a
/// blank cell (`LogLevel::Unknown` — always true for an unstructured
/// file, per `classify_line_level`).
fn level_cell(level: LogLevel, theme: &TuiTheme) -> AnyElement {
    let mut cell = div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .w(px(COL_LEVEL_WIDTH))
        .h(px(ROW_HEIGHT_PX))
        .overflow_hidden();
    if let Some((bg, fg)) = level_pill_colors(&level, theme) {
        cell = cell.child(
            div()
                .flex_shrink_0()
                .px(px(6.))
                .rounded(px(4.))
                .bg(bg)
                .text_color(fg)
                .whitespace_nowrap()
                .child(format!("{level:?}").to_uppercase()),
        );
    }
    cell.into_any_element()
}

/// The Message column: match-only (or line-mode) highlight spans via
/// `resolve_line_style`, same logic `styled_line_row` already had, just
/// nested one cell deeper (`.flex_1()` instead of `.w_full()`) instead of
/// being the whole row.
fn message_cell(tab: &TabState, bytes: &[u8]) -> AnyElement {
    match resolve_line_style(
        bytes,
        tab.log_manager.get_filters(),
        tab.log_manager.get_group_styles(),
    ) {
        Some(style) => styled_message_cell(bytes, &style),
        None => div()
            .flex_1()
            .overflow_hidden()
            .whitespace_nowrap()
            .child(String::from_utf8_lossy(bytes).into_owned())
            .into_any_element(),
    }
}

/// Splits a line into alternating plain/highlighted text segments so
/// `style`'s color applies only to its spans — one span per match by
/// default, or the whole line for a line-mode (`-l`) filter — leaving the
/// rest of the line in the default text color. gpui has no single-element
/// "highlight this substring" primitive, so this builds one child per
/// segment instead of styling the cell as a whole.
///
/// `.flex()` is load-bearing here, not cosmetic: a plain `div()` defaults
/// to block layout, which stacks children vertically one per line — an
/// earlier version of this function omitted it, and every styled row
/// rendered several line-heights tall and bled into the rows below it in
/// the virtual list (each row's slot is a fixed `ROW_HEIGHT_PX`). `.flex()`
/// (default direction row) lays the segments out inline instead; the
/// explicit `.h()` + `.overflow_hidden()` are a second, independent
/// safety net capping the cell at its slot height regardless.
fn styled_message_cell(bytes: &[u8], style: &LineStyle) -> AnyElement {
    let mut cell = div()
        .flex_1()
        .h(px(ROW_HEIGHT_PX))
        .flex()
        .overflow_hidden()
        .whitespace_nowrap();
    let mut pos = 0usize;
    for &(start, end) in &style.spans {
        if start > pos {
            cell = cell.child(text_segment(&bytes[pos..start], None, None));
        }
        cell = cell.child(text_segment(&bytes[start..end], style.fg, style.bg));
        pos = end.max(pos);
    }
    if pos < bytes.len() {
        cell = cell.child(text_segment(&bytes[pos..], None, None));
    }
    cell.into_any_element()
}

/// One inline text segment of a `styled_message_cell` — `flex_shrink_0()`
/// keeps it at its natural content width rather than letting flex squeeze
/// (and potentially wrap) it to fit, since the cell clips overflow instead.
fn text_segment(bytes: &[u8], fg: Option<Rgba>, bg: Option<Rgba>) -> AnyElement {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut el = div().flex_shrink_0().whitespace_nowrap().child(text);
    if let Some(bg) = bg {
        el = el.bg(bg);
    }
    if let Some(fg) = fg {
        el = el.text_color(fg);
    }
    el.into_any_element()
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
        let _ = line_row(&tab, 0, false, false, &theme);
    }

    #[tokio::test]
    async fn row_time_text_is_blank_for_unstructured_files() {
        let t = tab().await;
        assert!(t.display.format.is_none());
        assert_eq!(row_time_text(&t, b"a"), None);
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
