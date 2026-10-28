use crate::gui::app::App;
use crate::gui::color::{LineStyle, cell_style_spans, color_to_gpui};
use crate::gui::column_widths::{ResizableColumn, TableWidths};
use crate::gui::level::level_pill_colors;
use crate::gui::message::Message;
use crate::parser::LogLevel;
use crate::shared::TabState;
use crate::theme::Theme as TuiTheme;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tag::Tag;
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{
    AnyElement, Context, DragMoveEvent, Empty, FontWeight, ListOffset, ListState, MouseButton, div,
    list, px, rems,
};

/// Parsed row data shared by cell rendering and width measurement.
pub(crate) struct DisplayedRow {
    pub values: Vec<String>,
    pub level: LogLevel,
    pub target: Option<String>,
    value_spans: std::cell::RefCell<Option<ValueSpanCache>>,
    wrapped: std::cell::RefCell<Vec<Option<WrappedRanges>>>,
}
type WrappedRanges = (usize, std::rc::Rc<Vec<std::ops::Range<usize>>>);
struct CellText {
    data: std::rc::Rc<DisplayedRow>,
    column: usize,
    line: usize,
    position: usize,
    document: crate::gui::text_selection::SharedDocument,
    highlights: Vec<(std::ops::Range<usize>, gpui_kit::gpui::HighlightStyle)>,
    selection: Option<std::ops::Range<usize>>,
    selection_background: Option<gpui_kit::gpui::Hsla>,
    pill: Option<(gpui_kit::gpui::Rgba, gpui_kit::gpui::Rgba)>,
}
type ValueSpan = (usize, usize, crate::filters::manager::StyleId);
struct ValueSpanCache {
    disabled: std::collections::HashSet<String>,
    columns: Vec<Option<std::rc::Rc<Vec<ValueSpan>>>>,
}

impl DisplayedRow {
    fn wrapped_ranges(
        &self,
        column: usize,
        width: f32,
    ) -> std::rc::Rc<Vec<std::ops::Range<usize>>> {
        let limit = (width / 7.2).floor().max(1.) as usize;
        let mut cache = self.wrapped.borrow_mut();
        let cached = &mut cache[column];
        if let Some((old, ranges)) = cached
            && *old == limit
        {
            return ranges.clone();
        }
        let ranges = std::rc::Rc::new(wrap_ranges(&self.values[column], limit));
        *cached = Some((limit, ranges.clone()));
        ranges
    }

    fn wrapped_height(
        &self,
        fields: &[String],
        widths: &TableWidths,
        message_width: f32,
        font_scale: f32,
    ) -> f32 {
        let lines = fields
            .iter()
            .enumerate()
            .map(|(column, field)| {
                self.wrapped_ranges(
                    column,
                    (column_width(field, widths, message_width) - 16.).max(8.) / font_scale,
                )
                .len()
            })
            .max()
            .unwrap_or(1);
        ROW_HEIGHT_PX * lines as f32
    }

    fn value_spans(
        &self,
        column: usize,
        colors: &crate::theme::ValueColors,
    ) -> std::rc::Rc<Vec<ValueSpan>> {
        let mut cache = self.value_spans.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|cache| cache.disabled != colors.disabled)
        {
            *cache = Some(ValueSpanCache {
                disabled: colors.disabled.clone(),
                columns: vec![None; self.values.len()],
            });
        }
        let spans = &mut cache.as_mut().unwrap().columns[column];
        spans
            .get_or_insert_with(|| {
                std::rc::Rc::new(crate::shared::value_colors::collect_value_color_spans(
                    &self.values[column],
                    colors,
                ))
            })
            .clone()
    }
}

/// Minimum row height and initial height hint for offscreen rows.
/// 30px deliberately leaves a few
/// pixels of breathing room above/below each row's text.
const ROW_HEIGHT_PX: f32 = 30.0;
const ANNOTATION_HEIGHT_PX: f32 = 28.0;
/// Width of the draggable strip between two fixed columns — thin enough
/// to stay out of the way, wide enough to be a comfortable mouse target.
const RESIZE_HANDLE_WIDTH_PX: f32 = 6.0;

/// Renders the log table: a fixed header row (`#`/Time/Level/Message,
/// with draggable resize handles between the fixed-width columns) above
/// the currently-visible window of `tab.filter.visible_indices`, via
/// GPUI's list, which measures wrapped rows only in the viewport. `log_scroll`
/// is `App`'s own handle (not reachable through `cx`, which derefs to
/// gpui's own `App` platform type, not ours), passed in so `App::
/// sync_log_scroll` can drive the same handle after every dispatch.
/// `widths` is `GuiState::column_widths` — auto-fit to content whenever a
/// file loads, overridden per-column by dragging a header handle; see
/// `gui::column_widths` for both.
type LayoutCacheKey = (
    crate::shared::TabId,
    u64,
    u64,
    usize,
    usize,
    TableWidths,
    (u32, u32),
    Vec<String>,
    bool,
    bool,
    bool,
    Vec<Vec<usize>>,
);
pub struct LayoutCache {
    key: LayoutCacheKey,
}
#[cfg(test)]
pub fn prepare_list(
    cache: &mut Option<LayoutCache>,
    state: &ListState,
    tab: &TabState,
    widths: &TableWidths,
    fields: &[String],
    message_width: f32,
) {
    prepare_scaled_list(cache, state, tab, widths, fields, message_width, 1.);
}
#[allow(clippy::too_many_arguments)]
pub fn prepare_scaled_list(
    cache: &mut Option<LayoutCache>,
    state: &ListState,
    tab: &TabState,
    widths: &TableWidths,
    fields: &[String],
    message_width: f32,
    font_scale: f32,
) {
    let columns = visible_columns(tab, fields);
    let total = tab.filter.visible_indices.len();
    let key = (
        tab.id,
        tab.cache.parse_gen,
        tab.cache.render_gen,
        tab.file_reader.line_count(),
        total,
        widths.clone(),
        (message_width.to_bits(), font_scale.to_bits()),
        columns.clone(),
        tab.display.wrap,
        tab.display.show_keys,
        tab.display.raw_mode,
        tab.comment_manager
            .get()
            .iter()
            .map(|note| note.line_indices.clone())
            .collect(),
    );
    if let Some(cache) = cache
        && cache.key == key
    {
        return;
    }
    let same_rows = cache.as_ref().is_some_and(|cache| {
        cache.key.0 == key.0
            && cache.key.1 == key.1
            && cache.key.2 == key.2
            && cache.key.3 == key.3
            && cache.key.4 == key.4
    });
    if same_rows {
        // GPUI retains height hints and the scroll anchor, measuring only drawn rows.
        state.remeasure();
    } else {
        // Size the scrollbar from currently available visible rows. Publishing
        // another chunk changes `total`, so its height is updated on this frame.
        state.reset_with_uniform_height(total, px(ROW_HEIGHT_PX * font_scale));
        state.scroll_to(ListOffset {
            item_ix: tab.scroll.scroll_offset.min(total.saturating_sub(1)),
            offset_in_item: px(0.),
        });
    }
    *cache = Some(LayoutCache { key });
}
#[allow(clippy::too_many_arguments)]
pub fn log_pane(
    tab_idx: usize,
    tab: &TabState,
    widths: &TableWidths,
    log_scroll: &ListState,
    horizontal_scroll: &gpui_kit::gpui::ScrollHandle,
    fields: &[String],
    message_width: f32,
    font_scale: f32,
    text_handle: gpui_kit::base::TextSelectionHandle,
    text_document: crate::gui::text_selection::SharedDocument,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let columns = visible_columns(tab, fields);
    let table_width = if tab.display.show_line_numbers {
        widths.line_no + 6.
    } else {
        0.
    } + columns
        .iter()
        .map(|field| column_width(field, widths, message_width) + 6.)
        .sum::<f32>();
    let row_fields = columns.clone();
    let row_widths = widths.clone();
    let view = cx.entity();
    let row_document = text_document.clone();
    let table = div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .w(px(table_width + 22.))
        .min_w(gpui_kit::gpui::relative(1.))
        .h_full()
        .font_family("monospace")
        .text_size(rems(12. / 13.))
        .overflow_hidden()
        .child(div().pl(px(20.)).child(header_row(
            tab.id,
            widths,
            &columns,
            tab.display.show_line_numbers,
            message_width,
            cx,
        )))
        .child(
            div().relative().flex_1().min_h(px(0.)).child(
                crate::gui::text_selection::TableSelection {
                    document: text_document,
                    handle: text_handle,
                    scroll: log_scroll.clone(),
                    row_height: px(ROW_HEIGHT_PX * font_scale),
                    child: list(
                        log_scroll.clone(),
                        move |pos, _window, cx| view.update(cx, |app, cx| {
                            let Some(tab) = app.state.tabs.get(tab_idx) else {
                                return Empty.into_any_element();
                            };
                            let selection = app
                                .state
                                .log_selection
                                .filter(|(id, _, _)| *id == tab.id)
                                .map(|(_, a, b)| (a.min(b), a.max(b), b));
                            let Some(line_idx) = tab.filter.visible_indices.get_opt(pos) else {
                                return Empty.into_any_element();
                            };
                                    let output = raw_log_message(&tab.file_reader, line_idx);
                                    let data = app.row_cache.row(tab, line_idx, &row_fields);
                                    let displayed = data.values.join("\t");
                                    let hidden_lines = tab.active_continuation_map()
                                        .map_or(0, |map| continuation_count(map, line_idx));
                                    let collapsed = tab.display.collapse_continuations
                                        ^ tab.filter.overridden_groups.contains(&line_idx);
                                    let entity = cx.entity();
                                let source = tab.id;
                                    div()
                                        .id(("log-row", pos))
                                        .relative()
                                        .pl(px(20.))
                                        .test_support()
                                        .children(tab.comment_manager.get().iter().enumerate()
                                            .filter(|(_, note)| annotation_start(tab, &note.line_indices) == Some(pos))
                                            .map(|(index, note)| {
                                                let full_note = note.text.clone();
                                                div().id(("log-annotation", index)).test_support()
                                                    .h(rems(ANNOTATION_HEIGHT_PX / 13.)).w_full().flex().items_center().px_3().text_sm()
                                                    .text_color(color_to_gpui(app.state.theme.comment_fg).unwrap_or(gpui_kit::gpui::rgb(0xaaaaaa))).font_weight(FontWeight::BOLD).cursor_pointer()
                                                    .child(div().flex_1().min_w(px(0.)).truncate().child(note.text.replace('\n', " ")))
                                                    .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(full_note.clone()).build(window, cx))
                                                    .on_click(cx.listener(move |app, _, window, cx| {
                                                        cx.stop_propagation();
                                                        app.dispatch(Message::EditAnnotation(index), window, cx);
                                                    }))
                                                    .context_menu({
                                                        let entity = entity.clone();
                                                        move |menu, _, _| {
                                                            let entity = entity.clone();
                                                            menu.item(gpui_kit::component::menu::PopupMenuItem::new("Remove annotation")
                                                                .on_click(move |_, window, cx| entity.update(cx, |app, cx| {
                                                                    app.dispatch(Message::AnnotationRemoved(index), window, cx);
                                                                })))
                                                        }
                                                    })

                                            }))
                                                    .child(div().id(("log-row-actions", pos)).relative().child(line_row_with_selection(
                                            tab,
                                            line_idx,
                                            selection
                                                .is_some_and(|(a, b, _)| (a..=b).contains(&pos)),
                                            selection
                                                .map(|(_, _, cursor)| pos == cursor)
                                                .unwrap_or(pos == tab.scroll.scroll_offset),
                                            &app.state.theme,
                                            &row_widths,
                                            &row_fields,
                                            message_width,
                                            (&row_document, pos),
                                            font_scale,
                                            &data,
                                        ))
                                        .when(hidden_lines > 0, |row| {
                                            row.child(gpui_kit::component::button::Button::new(("toggle-multiline", line_idx))
                                                .label(if collapsed { "+" } else { "−" })
                                                .ghost()
                                                .small()
                                                .absolute()
                                                .left_0()
                                                .top_0()
                                                .w(px(20.))
                                                .h(rems(ROW_HEIGHT_PX / 13.))
                                                .p_0()
                                                .text_color(cx.theme().muted_foreground)
                                                .tooltip(if collapsed {
                                                    format!("Expand message ({hidden_lines} hidden lines)")
                                                } else {
                                                    "Collapse message".to_string()
                                                })
                                                .on_click(cx.listener(move |app, _, window, cx| {
                                                    cx.stop_propagation();
                                                    app.dispatch(Message::ToggleMultiline(source, line_idx), window, cx);
                                                })))
                                        })
                                        .when(tab.comment_manager.get().iter().any(|note| note.line_indices.contains(&line_idx)), |row| {
                                            row.border_l_1().border_color(color_to_gpui(app.state.theme.comment_fg).unwrap_or(gpui_kit::gpui::rgb(0xaaaaaa))).pl_2()
                                        })

                                        .on_click(cx.listener(
                                            move |app,
                                                  event: &gpui_kit::gpui::ClickEvent,
                                                  window,
                                                  cx| {
                                                app.text_document
                                                    .borrow_mut()
                                                    .sync(app.text_handle.snapshot(cx));
                                                let moved = matches!(event, gpui_kit::gpui::ClickEvent::Mouse(click)
                                                    if click.down.position != click.up.position);
                                                if app.text_document.borrow().dragged && moved {
                                                    return;
                                                }
                                                app.dispatch(
                                                    Message::SelectLog(
                                                        pos,
                                                        event.modifiers().shift,
                                                    ),
                                                    window,
                                                    cx,
                                                );
                                                if event.modifiers().shift {
                                                    gpui_kit::base::TextSelection::clear(window, cx);
                                                }
                                            },
                                        ))
                                        .context_menu(move |menu, _, cx| {
                                            let selected = {
                                                let app = entity.read(cx);
                                                app.text_document
                                                    .borrow_mut()
                                                    .sync(app.text_handle.snapshot(cx));
                                                app.state
                                                    .active_tab()
                                                    .map(|tab| app.text_document.borrow().copy(tab))
                                                    .unwrap_or_default()
                                            };
                                            let displayed = displayed.clone();
                                            let output = output.clone();
                                            let entity = entity.clone();
                                            let bookmark = entity.clone();
                                            let annotation = entity.clone();
                                            let filter_before = entity.clone();
                                            let filter_after = entity.clone();
                                            let inspector = entity.clone();
                                            menu.item(
                                                gpui_kit::component::menu::PopupMenuItem::new(
                                                    "Copy raw log message",
                                                )
                                                .on_click(move |_, _, cx| {
                                                    cx.write_to_clipboard(
                                                        gpui_kit::gpui::ClipboardItem::new_string(
                                                            output.clone(),
                                                        ),
                                                    )
                                                }),
                                            )
                                            .item(
                                                gpui_kit::component::menu::PopupMenuItem::new(
                                                    "Copy displayed row",
                                                )
                                                .on_click(move |_, _, cx| {
                                                    cx.write_to_clipboard(
                                                        gpui_kit::gpui::ClipboardItem::new_string(
                                                            displayed.clone(),
                                                        ),
                                                    )
                                                }),
                                            )
                                            .item(
                                                gpui_kit::component::menu::PopupMenuItem::new(
                                                    "Copy selected text",
                                                )
                                                .disabled(selected.is_empty())
                                                .on_click(move |_, _, cx| {
                                                    cx.write_to_clipboard(
                                                        gpui_kit::gpui::ClipboardItem::new_string(
                                                            selected.clone(),
                                                        ),
                                                    )
                                                }),
                                            )
                                            .item(gpui_kit::component::menu::PopupMenuItem::new("Toggle bookmark (Ctrl+B)")
                                                .on_click(move |_, window, cx| bookmark.update(cx, |app, cx| app.dispatch(Message::LogRowAction(source, line_idx, crate::gui::actions::Action::Bookmark), window, cx))))
                                            .item(gpui_kit::component::menu::PopupMenuItem::new("Add annotation (Ctrl+Shift+A)")
                                                .on_click(move |_, window, cx| annotation.update(cx, |app, cx| app.dispatch(Message::LogRowAction(source, line_idx, crate::gui::actions::Action::Annotation), window, cx))))
                                            .item(
                                                gpui_kit::component::menu::PopupMenuItem::new(
                                                    "Add filter",
                                                )
                                                .on_click(move |_, window, cx| {
                                                    entity.update(cx, |app, cx| {
                                                        app.dispatch(
                                                            Message::GuiAction(
                                                                crate::gui::actions::Action::Filter,
                                                            ),
                                                            window,
                                                            cx,
                                                        )
                                                    })
                                                }),
                                            )
                                            .item(
                                                gpui_kit::component::menu::PopupMenuItem::new(
                                                    "Filter out before this",
                                                )
                                                .on_click(move |_, window, cx| filter_before.update(cx, |app, cx| app.dispatch(Message::LogRowAction(source, line_idx, crate::gui::actions::Action::FilterOutBefore), window, cx))),
                                            )
                                            .item(
                                                gpui_kit::component::menu::PopupMenuItem::new(
                                                    "Filter out after this",
                                                )
                                                .on_click(move |_, window, cx| filter_after.update(cx, |app, cx| app.dispatch(Message::LogRowAction(source, line_idx, crate::gui::actions::Action::FilterOutAfter), window, cx))),
                                            )
                                            .item(gpui_kit::component::menu::PopupMenuItem::new("Inspect row")
                                                .on_click(move |_, _, cx| inspector.update(cx, |app, cx| {
                                                    app.inspected_row = Some((source, line_idx));
                                                    app.inspector_raw = false;
                                                    cx.notify();
                                                })))
                                        }))
                                        .into_any_element()
                        }),
                    )
                    .size_full()
                    .into_any_element(),
                },
            ),
        );
    // Keep the vertical scrollbar on the visible pane, outside the wide
    // horizontally scrolled table. Otherwise it moves off-screen with columns.
    div()
        .relative()
        .size_full()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_hidden()
        .child(
            div()
                .id("log-table-horizontal-scroll")
                .track_scroll(horizontal_scroll)
                .size_full()
                .overflow_x_scroll()
                .child(table)
                .horizontal_scrollbar(horizontal_scroll),
        )
        .child(
            div()
                .absolute()
                .top(rems((ROW_HEIGHT_PX + 4.) / 13.))
                .bottom_0()
                .left_0()
                .right_0()
                .vertical_scrollbar(log_scroll),
        )
}

fn annotation_start(tab: &TabState, lines: &[usize]) -> Option<usize> {
    lines
        .iter()
        .filter_map(|line| tab.filter.visible_indices.position_of(*line))
        .min()
}

/// Multiline schemas keep their source layout; parsed fields remain available
/// to filtering and inspection rather than becoming table columns.
fn uses_raw_layout(tab: &TabState) -> bool {
    tab.display.raw_mode
        || tab
            .display
            .format
            .as_deref()
            .is_some_and(|parser| parser.wants_continuation_walk())
}

/// All fields are visible by default; command preferences control visibility and order.
pub(crate) fn visible_columns(tab: &TabState, fields: &[String]) -> Vec<String> {
    if uses_raw_layout(tab) {
        return vec!["message".into()];
    }
    let mut ordered = tab.display.field_layout.columns.clone().unwrap_or_default();
    for field in fields {
        if !ordered.contains(field) {
            ordered.push(field.clone());
        }
    }
    if tab.display.format.is_none() && !ordered.iter().any(|field| field == "message") {
        ordered.push("message".to_string());
    }
    ordered.retain(|field| {
        !tab.display.hidden_fields.contains(field)
            && (fields.contains(field) || (field == "message" && tab.display.format.is_none()))
    });
    ordered
}
/// Fill the viewport after reserving every visible fixed column and separator.
/// Keep a usable minimum in narrow windows, where the table scrolls horizontally.
pub fn message_width(
    tab: &TabState,
    fields: &[String],
    widths: &TableWidths,
    available: f32,
) -> f32 {
    let columns = visible_columns(tab, fields);
    let messages = columns
        .iter()
        .filter(|field| matches!(field.as_str(), "message" | "msg"))
        .count();
    if messages == 0 {
        return 160.;
    }
    let fixed = columns
        .iter()
        .filter(|field| !matches!(field.as_str(), "message" | "msg"))
        .map(|field| column_width(field, widths, 0.))
        .sum::<f32>();
    let numbers = if tab.display.show_line_numbers {
        widths.line_no + RESIZE_HANDLE_WIDTH_PX
    } else {
        0.
    };
    ((available - 2. - numbers - fixed - columns.len() as f32 * RESIZE_HANDLE_WIDTH_PX)
        / messages as f32)
        .max(160.)
}
/// Measure only the rows that can be mounted, so a long message anywhere in
/// the file gets a scrollable cell without scanning the file on each frame.
pub(super) fn fit_visible_messages(
    tab: &TabState,
    state: &ListState,
    widths: &mut TableWidths,
    fields: &[String],
    message_width: f32,
    window: &gpui_kit::gpui::Window,
    cache: &mut crate::gui::row_cache::RowCache,
) {
    if tab.display.wrap {
        return;
    }
    let columns = visible_columns(tab, fields);
    if !columns
        .iter()
        .any(|field| matches!(field.as_str(), "message" | "msg"))
    {
        return;
    }
    let start = state.logical_scroll_top().item_ix;
    let scale = f32::from(window.rem_size()) / 13.;
    let count =
        (f32::from(window.viewport_size().height) / (ROW_HEIGHT_PX * scale)).ceil() as usize + 2;
    // GPUI clamps an end-of-list anchor during layout; include that final
    // viewport now so the next frame does not have to discover its rows again.
    let start = start.min(tab.filter.visible_indices.len().saturating_sub(count));
    let lines = (start
        ..start
            .saturating_add(count)
            .min(tab.filter.visible_indices.len()))
        .filter_map(|pos| tab.filter.visible_indices.get_opt(pos))
        .collect();
    *widths = cache.fit(tab, lines, &columns, widths, message_width, window);
}

fn column_width(field: &str, widths: &TableWidths, message_width: f32) -> f32 {
    match field {
        "timestamp" | "time" | "ts" => widths.time,
        "level" | "lvl" => widths.level,
        "message" | "msg" => widths.fields.get(field).copied().unwrap_or(message_width),
        _ => widths
            .fields
            .get(field)
            .copied()
            .unwrap_or(field.len() as f32 * 7.2 + 16.),
    }
}
fn header_cell(
    source: crate::shared::TabId,
    field: Option<String>,
    width: f32,
    label: String,
    cx: &mut Context<App>,
) -> AnyElement {
    let entity = cx.entity();
    div()
        .id(gpui_kit::gpui::SharedString::from(format!(
            "column-header-{label}"
        )))
        .test_support()
        .child(fixed_cell(width, label))
        .context_menu(move |menu, _, _| {
            let choose = entity.clone();
            let hide = entity.clone();
            let field = field.clone();
            menu.item(
                gpui_kit::component::menu::PopupMenuItem::new("Choose columns…").on_click(
                    move |_, window, cx| {
                        choose.update(cx, |app, cx| {
                            if let Some(index) = app.state.tabs.iter().position(|t| t.id == source)
                            {
                                app.dispatch(Message::TabSelected(index), window, cx);
                                app.dispatch(
                                    Message::GuiAction(crate::gui::actions::Action::Columns),
                                    window,
                                    cx,
                                );
                            }
                        });
                    },
                ),
            )
            .item(
                gpui_kit::component::menu::PopupMenuItem::new("Hide column").on_click(
                    move |_, _, cx| {
                        hide.update(cx, |app, cx| {
                            if let Some(tab) = app.state.tabs.iter_mut().find(|t| t.id == source) {
                                if let Some(field) = &field {
                                    tab.display.hidden_fields.insert(field.clone());
                                } else {
                                    tab.display.show_line_numbers = false;
                                }
                                tab.invalidate_parse_cache();
                                app.save_session_state();
                                cx.notify();
                            }
                        });
                    },
                ),
            )
        })
        .into_any_element()
}

fn header_row(
    source: crate::shared::TabId,
    widths: &TableWidths,
    fields: &[String],
    show_numbers: bool,
    message_width: f32,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let mut row = div()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(cx.theme().border)
        .text_color(cx.theme().muted_foreground)
        .h(rems((ROW_HEIGHT_PX + 4.) / 13.))
        .overflow_hidden();
    if show_numbers {
        row = row
            .child(header_cell(source, None, widths.line_no, "#".into(), cx))
            .child(resize_handle(
                ResizableColumn::Field("#".into(), widths.line_no),
                cx,
            ));
    }
    for field in fields {
        let label = match field.as_str() {
            "timestamp" => "Time",
            "level" => "Level",
            "message" => "Message",
            _ => field,
        };
        row = row.child(header_cell(
            source,
            Some(field.clone()),
            column_width(field, widths, message_width),
            label.to_string(),
            cx,
        ));
        row = row.child(resize_handle(
            ResizableColumn::Field(field.clone(), column_width(field, widths, message_width)),
            cx,
        ));
    }
    row
}

/// A fixed, non-interactive spacer matching `resize_handle`'s width —
/// used in `line_row` so data rows reserve exactly the same horizontal
/// space the header's resize handles occupy, keeping columns aligned
/// without data rows needing (or wanting) their own drag handles.
fn column_spacer() -> AnyElement {
    div()
        .flex_shrink_0()
        .w(px(RESIZE_HANDLE_WIDTH_PX))
        .h(rems(ROW_HEIGHT_PX / 13.))
        .into_any_element()
}

/// One column boundary's drag handle: `on_drag` starts gpui's native
/// drag-and-drop with `column` as the payload (an invisible `Empty` ghost
/// — the resize cursor is feedback enough, no floating preview needed);
/// `on_drag_move` fires on every pointer move for the duration of that
/// drag, dispatching this step's absolute mouse x so the reducer
/// (`Message::ColumnResizeMoved`) can diff it against the previous
/// event's x and apply the delta — see `GuiState::column_resize_last_x`'s
/// doc comment for why a delta instead of a start/end pair.
/// `on_mouse_up_out` (not `on_mouse_up`) ends the drag even when the
/// pointer is released outside the thin handle itself, which is the
/// common case once a drag is underway.
fn resize_handle(column: ResizableColumn, cx: &mut Context<App>) -> AnyElement {
    let id: gpui_kit::gpui::SharedString = match &column {
        ResizableColumn::LineNo => "resize-line-no".into(),
        ResizableColumn::Time => "resize-time".into(),
        ResizableColumn::Level => "resize-level".into(),
        ResizableColumn::Field(field, _) => format!("resize-{field}").into(),
    };
    div()
        .id(id)
        .test_support()
        .flex_shrink_0()
        .w(px(RESIZE_HANDLE_WIDTH_PX))
        .h(rems(ROW_HEIGHT_PX / 13.))
        .cursor_col_resize()
        .hover(|style| style.bg(cx.theme().border))
        .on_drag(column, |_column, _offset, _window, cx| cx.new(|_| Empty))
        .on_drag_move(cx.listener(
            move |app: &mut App, e: &DragMoveEvent<ResizableColumn>, window, cx| {
                let mouse_x = f32::from(e.event.position.x);
                app.dispatch(
                    Message::ColumnResizeMoved(e.drag(cx).clone(), mouse_x),
                    window,
                    cx,
                );
            },
        ))
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|app: &mut App, _, window, cx| {
                app.dispatch(Message::ColumnResizeEnded, window, cx);
            }),
        )
        .into_any_element()
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

/// `YYYY-MM-DD HH:MM:SS` timestamp for `line_idx`, resolving a year-less
/// (e.g. BSD syslog) timestamp against the tab's year map — the exact bound
/// format `:date-filter`'s own parser accepts (it has no fractional-second
/// syntax, unlike `canonical_timestamp`'s `.fff` suffix), used to build a
/// "filter out before/after this line" expression from a right-clicked row.
pub(crate) fn line_canonical_timestamp(tab: &TabState, line_idx: usize) -> Option<String> {
    let bytes = tab.file_reader.get_line(line_idx);
    let timestamp = row_time_text(tab, &bytes)?;
    let year_override = tab.year_map.as_deref().map(|ym| ym.year_for_line(line_idx));
    let canonical = crate::filters::canonical_timestamp(&timestamp, year_override)?;
    // `canonical_timestamp` always returns the fixed `YYYY-MM-DD
    // HH:MM:SS.fff` shape (23 ASCII bytes); keep only the first 19 —
    // `:date-filter`'s parser has no syntax for the `.fff` suffix.
    Some(canonical[..19].to_string())
}

/// The Message column's bytes: a detected format's parsed `message` field,
/// so the timestamp/level/target already shown in their own columns don't
/// also repeat inline — or the full raw line when there's no detected
/// format, parsing fails, or the format doesn't capture a `message` field
/// (an unstructured file's only option). `LogFormatParser::parse_line`'s
/// contract guarantees `message` borrows from `bytes` itself, so this
/// recovers its byte range via the same pointer-offset technique
/// `ui::tab_state::merged_message_range` already uses for the same
/// invariant, rather than reparsing or copying.
fn row_message_bytes<'a>(tab: &TabState, bytes: &'a [u8]) -> &'a [u8] {
    if uses_raw_layout(tab) {
        return bytes;
    }
    let Some(parser) = tab.display.format.as_deref() else {
        return bytes;
    };
    let Some(message) = parser.parse_line(bytes).and_then(|parts| parts.message) else {
        return bytes;
    };
    let start = message.as_ptr() as usize - bytes.as_ptr() as usize;
    &bytes[start..start + message.len()]
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
#[cfg(test)]
fn wrap_text(text: &str, width: f32) -> Vec<String> {
    wrap_ranges(text, (width / 7.2).floor().max(1.) as usize)
        .into_iter()
        .map(|range| text[range].to_owned())
        .collect()
}

/// Cache byte ranges rather than allocating a String for every wrapped line.
fn wrap_ranges(text: &str, limit: usize) -> Vec<std::ops::Range<usize>> {
    use unicode_width::UnicodeWidthChar;
    let mut lines = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (offset, c) in text.char_indices() {
        if c == '\n' {
            lines.push(start..offset);
            start = offset + 1;
            used = 0;
            continue;
        }
        let size = c.width().unwrap_or(0);
        if used + size > limit && offset > start {
            lines.push(start..offset);
            start = offset;
            used = 0;
        }
        used += size;
    }
    lines.push(start..text.len());
    lines
}

#[cfg(test)]
fn wrapped_height(
    tab: &TabState,
    line: usize,
    fields: &[String],
    widths: &TableWidths,
    message_width: f32,
) -> f32 {
    scaled_wrapped_height(tab, line, fields, widths, message_width, 1.)
}
#[cfg(test)]
fn scaled_wrapped_height(
    tab: &TabState,
    line: usize,
    fields: &[String],
    widths: &TableWidths,
    message_width: f32,
    font_scale: f32,
) -> f32 {
    wrapped_height_for_values(
        &displayed_values(tab, line, fields),
        fields,
        widths,
        message_width,
        font_scale,
    )
}
#[cfg(test)]
fn wrapped_height_for_values(
    values: &[String],
    fields: &[String],
    widths: &TableWidths,
    message_width: f32,
    font_scale: f32,
) -> f32 {
    let count = values
        .iter()
        .zip(fields)
        .map(|(text, field)| {
            wrap_text(
                text,
                (column_width(field, widths, message_width) - 16.).max(8.) / font_scale,
            )
            .len()
        })
        .max()
        .unwrap_or(1);
    ROW_HEIGHT_PX * count as f32
}
#[allow(clippy::too_many_arguments)]
fn line_row_with_selection(
    tab: &TabState,
    line_idx: usize,
    selected: bool,
    is_cursor_row: bool,
    theme: &TuiTheme,
    widths: &TableWidths,
    fields: &[String],
    message_width: f32,
    text_selection: (&crate::gui::text_selection::SharedDocument, usize),
    font_scale: f32,
    data: &std::rc::Rc<DisplayedRow>,
) -> AnyElement {
    let palette = crate::gui::theme::GuiPalette::from_theme(theme);
    let rgb = |(r, g, b): (u8, u8, u8)| {
        gpui_kit::gpui::rgb((r as u32) << 16 | (g as u32) << 8 | b as u32)
    };

    let filter_styles = crate::gui::color::cached_row_styles(
        tab,
        line_idx,
        &tab.file_reader.get_raw_line(line_idx),
    );
    let filter_style = filter_styles.iter().find(|style| style.whole_line);
    let number_cell = fixed_cell(
        widths.line_no,
        format!(
            "{}",
            if tab.display.relative_line_numbers {
                let selected = tab
                    .filter
                    .visible_indices
                    .get_opt(tab.scroll.scroll_offset)
                    .unwrap_or(line_idx);
                if selected == line_idx {
                    line_idx + 1
                } else {
                    line_idx.abs_diff(selected)
                }
            } else {
                line_idx + 1
            }
        ),
    );
    let mut row = div()
        .id(("log-row-content", text_selection.1))
        .test_support()
        .flex()
        .w_full()
        .h(rems(
            (if tab.display.wrap {
                data.wrapped_height(fields, widths, message_width, font_scale)
            } else {
                ROW_HEIGHT_PX
            }) / 13.,
        ))
        .items_start()
        .border_b_1()
        .border_color(rgb(palette.border));
    if tab.display.show_line_numbers {
        row = row.child(number_cell).child(column_spacer());
    }
    for (column, field) in fields.iter().enumerate() {
        let width = column_width(field, widths, message_width);
        let (document, position) = text_selection;
        let cell = selectable_cell(
            tab,
            line_idx,
            column,
            position,
            field,
            &data.values[column],
            width,
            theme,
            &filter_styles,
            document,
            font_scale,
            data,
        );
        row = row.child(cell).child(column_spacer());
    }

    if let Some(style) = &filter_style
        && style.whole_line
    {
        if let Some(bg) = style.bg {
            row = row.bg(bg);
        }
        if let Some(fg) = style.fg {
            row = row.text_color(fg);
        }
    }
    if filter_style
        .as_ref()
        .filter(|style| style.whole_line)
        .and_then(|style| style.bg)
        .is_none()
        && tab.mark_manager.is_marked(line_idx)
        && !tab.filter.show_marks_only
        && let Some(background) = color_to_gpui(theme.mark_bg)
    {
        row = row.bg(background);
    }
    if selected {
        row = row
            .bg(rgb(palette.selected))
            .text_color(rgb(palette.foreground));
        // Bold + underline marks which end of an active selection is
        // "the cursor", matching the TUI's own visual-selection render.
        if is_cursor_row {
            row = row.font_weight(FontWeight::BOLD).underline();
        }
    } else if is_cursor_row && !filter_style.as_ref().is_some_and(|style| style.whole_line) {
        // Outside an active selection, the cursor row still needs its
        // own visual indicator — a GUI table has no other way to show
        // "you are here" the way a terminal's native cursor does for the
        // TUI (see `theme.cursor_bg`/`cursor_fg`'s other GUI-only uses:
        // popups, the command bar's text cursor). A plain background
        // tint, not bold+underline, which `selected` above reserves for
        // marking an actual selection's active end.
        row = row
            .bg(rgb(palette.selected))
            .text_color(rgb(palette.foreground));
    }
    row.into_any_element()
}

/// Text used for both table rendering and displayed-row/selection copying.
pub(crate) fn displayed_values(tab: &TabState, line: usize, fields: &[String]) -> Vec<String> {
    displayed_row(tab, line, fields).values
}

pub(crate) fn displayed_row(tab: &TabState, line: usize, fields: &[String]) -> DisplayedRow {
    let bytes = tab.file_reader.get_line(line);
    let parts = tab
        .display
        .format
        .as_ref()
        .and_then(|p| p.parse_line(&bytes));
    let level = parts
        .as_ref()
        .and_then(|parts| parts.level)
        .map(|raw| crate::parser::classify_level(tab.display.format.as_deref(), raw))
        .unwrap_or(LogLevel::Unknown);
    let target = parts
        .as_ref()
        .and_then(|parts| parts.target)
        .map(str::to_owned);
    let raw_layout = uses_raw_layout(tab);
    let values: Vec<String> = fields
        .iter()
        .map(|field| match field.as_str() {
            "timestamp" | "time" | "ts" => parts
                .as_ref()
                .and_then(|parts| parts.timestamp)
                .unwrap_or_default()
                .to_owned(),
            "level" | "lvl" => match &level {
                LogLevel::Unknown => String::new(),
                LogLevel::Warning => "WARN".into(),
                other => format!("{other:?}").to_uppercase(),
            },
            "message" | "msg" => {
                let message = if raw_layout {
                    &bytes
                } else {
                    parts
                        .as_ref()
                        .and_then(|parts| parts.message)
                        .map(str::as_bytes)
                        .unwrap_or(&bytes)
                };
                raw_log_text(&String::from_utf8_lossy(message))
            }
            _ => {
                // `get_col` (shared with the TUI) also resolves span/fields.*
                // sub-fields and indexed repeated-group columns, which plain
                // `resolve_field` does not.
                let value: String = if field == "source" {
                    tab.file_reader
                        .merged_entries()
                        .and_then(|entries| entries.get(line))
                        .and_then(|entry| {
                            tab.merged
                                .as_ref()
                                .and_then(|m| m.source_labels.get(entry.source_idx))
                        })
                        .cloned()
                        .unwrap_or_default()
                } else {
                    parts
                        .as_ref()
                        .and_then(|parts| {
                            crate::shared::field_layout::get_col(parts, field, false, None)
                        })
                        .unwrap_or_default()
                };
                if tab.display.show_keys && !value.is_empty() {
                    format!("{field}={value}")
                } else {
                    value
                }
            }
        })
        .collect();
    DisplayedRow {
        wrapped: std::cell::RefCell::new(vec![None; values.len()]),
        values,
        level,
        target,
        value_spans: Default::default(),
    }
}
pub(crate) fn raw_log_message(reader: &crate::ingestion::FileReader, line: usize) -> String {
    String::from_utf8_lossy(&reader.get_raw_line(line)).into_owned()
}

pub(crate) fn raw_log_text(text: &str) -> String {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
        .to_string()
}

/// Map the active raw-text occurrence to this cell, even when another column
/// contains identical text. Parser slices retain their original byte offsets.
fn current_search_range(
    tab: &TabState,
    line: usize,
    field: &str,
    text: &str,
) -> Option<std::ops::Range<usize>> {
    let result = tab
        .search
        .query
        .get_current_match()
        .filter(|r| r.line_idx == line)?;
    let &(start, end) = result
        .matches
        .get(tab.search.query.get_current_occurrence_index())?;
    // Searches and displayed values both use normalized source bytes.
    let raw = tab.file_reader.get_line(line);
    let parts = tab.display.format.as_ref().and_then(|p| p.parse_line(&raw));
    let value = if matches!(field, "message" | "msg") {
        row_message_bytes(tab, &raw)
    } else {
        crate::filters::resolve_field(field, parts.as_ref()?)?.as_bytes()
    };
    let offset = (value.as_ptr() as usize).checked_sub(raw.as_ptr() as usize)?;
    if offset.checked_add(value.len())? > raw.len()
        || start >= offset + value.len()
        || end <= offset
    {
        return None;
    }
    if matches!(field, "level" | "lvl") {
        return Some(0..text.len());
    }
    let prefix = if tab.display.show_keys
        && !matches!(field, "timestamp" | "time" | "ts" | "message" | "msg")
    {
        field.len() + 1
    } else {
        0
    };
    let start = prefix + start.saturating_sub(offset);
    let end = (prefix + end.saturating_sub(offset)).min(text.len());
    (start < end && text.is_char_boundary(start) && text.is_char_boundary(end))
        .then_some(start..end)
}

fn search_highlights(
    text: &str,
    search: &crate::utils::search::Search,
    theme: &TuiTheme,
    current: Option<std::ops::Range<usize>>,
    highlights: Vec<(std::ops::Range<usize>, gpui_kit::gpui::HighlightStyle)>,
) -> Vec<(std::ops::Range<usize>, gpui_kit::gpui::HighlightStyle)> {
    let Some(pattern) = search.get_compiled_pattern() else {
        return highlights;
    };
    let matches: Vec<_> = pattern
        .find_iter(text)
        .filter(|m| m.start() < m.end())
        .map(|m| m.range())
        .collect();
    if matches.is_empty() && current.is_none() {
        return highlights;
    }
    let mut boundaries = vec![0, text.len()];
    for range in highlights
        .iter()
        .map(|(range, _)| range)
        .chain(&matches)
        .chain(current.iter())
    {
        boundaries.extend([range.start, range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .map(|pair| {
            let mut style = highlights
                .iter()
                .find(|(range, _)| range.contains(&pair[0]))
                .map(|(_, style)| *style)
                .unwrap_or_default();
            if matches.iter().any(|range| range.contains(&pair[0])) {
                style.color = color_to_gpui(theme.search_fg).map(Into::into);
                style.background_color = color_to_gpui(theme.text_highlight_fg).map(Into::into);
            }
            if current
                .as_ref()
                .is_some_and(|range| range.contains(&pair[0]))
            {
                style.color = color_to_gpui(theme.text_highlight_fg).map(Into::into);
                style.background_color = color_to_gpui(theme.search_fg).map(Into::into);
                style.font_weight = Some(FontWeight::BOLD);
            }
            (pair[0]..pair[1], style)
        })
        .collect()
}

/// Value colors are the base layer for every displayed column. Filters override
/// only their matched spans and only the color attributes they actually set.
#[cfg(test)]
fn cell_highlights(
    raw: &[u8],
    text: &str,
    theme: &TuiTheme,
    filter_style: Option<&LineStyle>,
    process_color: Option<crate::style::Color>,
) -> Vec<(std::ops::Range<usize>, gpui_kit::gpui::HighlightStyle)> {
    let spans = crate::shared::value_colors::collect_value_color_spans(text, &theme.value_colors);
    cell_highlights_with_spans(raw, text, theme, filter_style, process_color, &spans)
}
fn cell_highlights_with_spans(
    raw: &[u8],
    text: &str,
    theme: &TuiTheme,
    filter_style: Option<&LineStyle>,
    process_color: Option<crate::style::Color>,
    spans: &[ValueSpan],
) -> Vec<(std::ops::Range<usize>, gpui_kit::gpui::HighlightStyle)> {
    use gpui_kit::gpui::HighlightStyle;
    let mut highlights = Vec::new();
    use crate::shared::value_colors::*;
    for &(start, end, style) in spans {
        let colors = &theme.value_colors;
        let color = match style {
            VALUE_STYLE_HTTP_GET => colors.http_get,
            VALUE_STYLE_HTTP_POST => colors.http_post,
            VALUE_STYLE_HTTP_PUT => colors.http_put,
            VALUE_STYLE_HTTP_DELETE => colors.http_delete,
            VALUE_STYLE_HTTP_PATCH => colors.http_patch,
            VALUE_STYLE_HTTP_OTHER => colors.http_other,
            VALUE_STYLE_STATUS_2XX => colors.status_2xx,
            VALUE_STYLE_STATUS_3XX => colors.status_3xx,
            VALUE_STYLE_STATUS_4XX => colors.status_4xx,
            VALUE_STYLE_STATUS_5XX => colors.status_5xx,
            VALUE_STYLE_IP => colors.ip_address,
            VALUE_STYLE_UUID => colors.uuid,
            _ => theme.text,
        };
        highlights.push((
            start..end,
            HighlightStyle {
                color: color_to_gpui(color).map(Into::into),
                ..Default::default()
            },
        ));
    }

    // collect_value_color_spans already returns sorted, disjoint ranges.

    if let Some(color) = process_color {
        highlights = vec![(
            0..text.len(),
            HighlightStyle {
                color: color_to_gpui(color).map(Into::into),
                ..Default::default()
            },
        )];
    }

    overlay_filter(raw, text, filter_style, highlights)
}
fn overlay_filter(
    raw: &[u8],
    text: &str,
    filter_style: Option<&LineStyle>,
    highlights: Vec<(std::ops::Range<usize>, gpui_kit::gpui::HighlightStyle)>,
) -> Vec<(std::ops::Range<usize>, gpui_kit::gpui::HighlightStyle)> {
    let Some(filter) = filter_style else {
        return highlights;
    };
    let spans = cell_style_spans(raw, text, filter);
    if spans.is_empty() {
        return highlights;
    }
    let mut boundaries = Vec::new();
    for range in highlights.iter().map(|(range, _)| range).chain(&spans) {
        boundaries.extend([range.start, range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .map(|pair| {
            let mut style = highlights
                .iter()
                .find(|(range, _)| range.contains(&pair[0]))
                .map(|(_, style)| *style)
                .unwrap_or_default();
            if spans.iter().any(|range| range.contains(&pair[0])) {
                if let Some(fg) = filter.fg {
                    style.color = Some(fg.into());
                }
                if !filter.whole_line
                    && let Some(bg) = filter.bg
                {
                    style.background_color = Some(bg.into());
                }
            }
            (pair[0]..pair[1], style)
        })
        .collect()
}

fn process_color(
    theme: &TuiTheme,
    field: &str,
    value: &str,
    target: Option<&str>,
) -> Option<crate::style::Color> {
    if !matches!(field, "pid" | "target") || value.is_empty() {
        return None;
    }
    let key = target.filter(|target| !target.is_empty()).unwrap_or(value);
    theme.process_color(key)
}

#[allow(clippy::too_many_arguments)]
fn selectable_cell(
    tab: &TabState,
    line: usize,
    column: usize,
    position: usize,
    field: &str,
    text: &str,
    width: f32,
    theme: &TuiTheme,
    filter_styles: &[LineStyle],
    document: &crate::gui::text_selection::SharedDocument,
    font_scale: f32,
    data: &std::rc::Rc<DisplayedRow>,
) -> AnyElement {
    let raw = tab.file_reader.get_raw_line(line);
    let color = process_color(theme, field, text, data.target.as_deref());
    let spans = data.value_spans(column, &theme.value_colors);
    let mut highlights = cell_highlights_with_spans(&raw, text, theme, None, color, &spans);
    for filter in filter_styles {
        highlights = overlay_filter(&raw, text, Some(filter), highlights);
    }
    let filter_style = filter_styles.iter().find(|style| style.whole_line);
    let current = current_search_range(tab, line, field, text);
    highlights = search_highlights(text, &tab.search.query, theme, current, highlights);
    let selection = document.borrow().range(position, column, text.len());
    let pill = if !filter_style.is_some_and(|style| style.whole_line)
        && matches!(field, "level" | "lvl")
        && !tab
            .display
            .level_colors_disabled
            .contains(&format!("{:?}", data.level).to_lowercase())
    {
        level_pill_colors(&data.level, theme)
    } else {
        None
    };
    let content = CellText {
        data: data.clone(),
        column,
        line,
        position,
        document: document.clone(),
        highlights,
        selection,
        selection_background: color_to_gpui(theme.cursor_bg).map(Into::into),
        pill,
    };
    let cell = div()
        .flex_shrink_0()
        .w(px(width))
        .pl(px(8.))
        .overflow_hidden()
        .flex()
        .flex_col();
    if !tab.display.wrap {
        return cell
            .child(content.chunk(0, 0..text.len()))
            .into_any_element();
    }
    let ranges = data.wrapped_ranges(column, (width - 16.).max(8.) / font_scale);
    cell.child(content.wrapped(ranges, font_scale))
        .into_any_element()
}

impl CellText {
    /// Determine visibility after layout, using the same clip as the parent
    /// list. This also handles scrolling partway through one oversized row.
    fn wrapped(
        self,
        ranges: std::rc::Rc<Vec<std::ops::Range<usize>>>,
        font_scale: f32,
    ) -> impl IntoElement {
        let height = ROW_HEIGHT_PX * font_scale;
        let count = ranges.len();
        gpui_kit::gpui::canvas(
            move |bounds, window, cx| {
                let visible = bounds.intersect(&window.content_mask().bounds);
                let mut children = Vec::new();
                if visible.size.width <= px(0.) || visible.size.height <= px(0.) {
                    return children;
                }
                let first = ((f32::from(visible.top() - bounds.top()) / height)
                    .floor()
                    .max(0.) as usize)
                    .min(count);
                let last = ((f32::from(visible.bottom() - bounds.top()) / height)
                    .ceil()
                    .max(0.) as usize)
                    .min(count);
                for index in first..last {
                    let mut child = self.chunk(index, ranges[index].clone());
                    child.layout_as_root(
                        gpui_kit::gpui::size(
                            gpui_kit::gpui::AvailableSpace::Definite(bounds.size.width),
                            gpui_kit::gpui::AvailableSpace::Definite(px(height)),
                        ),
                        window,
                        cx,
                    );
                    child.prepaint_at(
                        gpui_kit::gpui::point(
                            bounds.left(),
                            bounds.top() + px(index as f32 * height),
                        ),
                        window,
                        cx,
                    );
                    children.push(child);
                }
                children
            },
            |_, mut children, window, cx| {
                for child in &mut children {
                    child.paint(window, cx);
                }
            },
        )
        .w_full()
        .h(rems(count as f32 * ROW_HEIGHT_PX / 13.))
        .flex_shrink_0()
    }

    fn chunk(&self, chunk_index: usize, range: std::ops::Range<usize>) -> AnyElement {
        use crate::gui::text_selection::{Endpoint, SelectionText};
        use gpui_kit::gpui::StyledText;
        let local: Vec<_> = self
            .highlights
            .iter()
            .filter_map(|(highlight, style)| {
                chunk_range(highlight, range.start, range.end).map(|r| (r, *style))
            })
            .collect();
        let selected = self
            .selection
            .as_ref()
            .and_then(|selection| chunk_range(selection, range.start, range.end));
        let shared: gpui_kit::gpui::SharedString = self.data.values[self.column][range.clone()]
            .to_owned()
            .into();
        let mut styled = StyledText::new(shared.clone()).with_highlights(local.clone());
        if let Some(selected) = &selected {
            let mut boundaries = vec![0, shared.len(), selected.start, selected.end];
            for (r, _) in &local {
                boundaries.extend([r.start, r.end]);
            }
            boundaries.sort_unstable();
            boundaries.dedup();
            let merged = boundaries
                .windows(2)
                .filter(|p| p[0] < p[1])
                .map(|p| {
                    let mut style = local
                        .iter()
                        .find(|(r, _)| r.contains(&p[0]))
                        .map(|(_, s)| *s)
                        .unwrap_or_default();
                    if selected.contains(&p[0]) {
                        style.background_color = self.selection_background;
                    }
                    (p[0]..p[1], style)
                })
                .collect::<Vec<_>>();
            styled = StyledText::new(shared.clone()).with_highlights(merged);
        }
        #[cfg(test)]
        {
            self.document.borrow_mut().prepared_chunks += 1;
        }
        let content = SelectionText {
            text: shared,
            styled,
            document: self.document.clone(),
            endpoint: Endpoint {
                line: self.line,
                row: self.position,
                column: self.column,
                offset: range.start,
            },
            order: ((self.position as u64) << 32)
                | ((self.column as u64) << 16)
                | chunk_index as u64,
        };
        let row = div()
            .h(rems(ROW_HEIGHT_PX / 13.))
            .flex()
            .items_center()
            .whitespace_nowrap();
        if let Some((bg, fg)) = self.pill {
            row.child(
                Tag::custom(bg.into(), fg.into(), bg.into())
                    .small()
                    .rounded(px(5.))
                    .child(content),
            )
            .into_any_element()
        } else {
            row.child(content).into_any_element()
        }
    }
}

/// Convert an overlapping global span to chunk-local offsets. A span outside
/// the chunk must be rejected before subtracting its start (unsigned offsets).
fn chunk_range(
    range: &std::ops::Range<usize>,
    start: usize,
    end: usize,
) -> Option<std::ops::Range<usize>> {
    let lo = range.start.max(start);
    let hi = range.end.min(end);
    if lo >= hi {
        return None;
    }
    Some(lo.checked_sub(start)?..hi.checked_sub(start)?)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn line_row(
    tab: &TabState,
    line_idx: usize,
    selected: bool,
    is_cursor_row: bool,
    theme: &TuiTheme,
    widths: &TableWidths,
    fields: &[String],
    message_width: f32,
) -> AnyElement {
    let document = Default::default();
    line_row_with_selection(
        tab,
        line_idx,
        selected,
        is_cursor_row,
        theme,
        widths,
        fields,
        message_width,
        (&document, line_idx),
        1.,
        &std::rc::Rc::new(displayed_row(tab, line_idx, fields)),
    )
}

/// A fixed-width, single-line text cell — `#`, Time, and the header row's
/// titles all use this. `.flex_shrink_0()` keeps it at exactly `width`
/// regardless of its own or siblings' content (a long Message must never
/// squeeze this column), matching `text_segment`'s existing rationale.
fn fixed_cell(width: f32, text: impl Into<gpui_kit::gpui::SharedString>) -> AnyElement {
    div()
        .flex_shrink_0()
        .w(px(width))
        .h(rems(ROW_HEIGHT_PX / 13.))
        .flex()
        .items_center()
        .pl(px(8.))
        .overflow_hidden()
        .whitespace_nowrap()
        .child(text.into())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, LogManager};
    use crate::gui::column_widths::ColumnWidths;
    use crate::ingestion::FileReader;
    use std::sync::Arc;

    #[tokio::test]
    async fn wrapped_ranges_keep_unicode_offsets_and_reuse_unchanged_widths() {
        let t = tab().await;
        let mut data = displayed_row(&t, 0, &["message".into()]);
        data.values[0] = "a界b\n\nc".into();
        let ranges = data.wrapped_ranges(0, 22.);
        let text: Vec<_> = ranges.iter().map(|r| &data.values[0][r.clone()]).collect();
        assert_eq!(text, ["a界", "b", "", "c"]);
        assert!(
            std::rc::Rc::ptr_eq(&ranges, &data.wrapped_ranges(0, 28.)),
            "widths with the same character capacity must reuse line boundaries"
        );
        let narrower = data.wrapped_ranges(0, 15.);
        assert!(!std::rc::Rc::ptr_eq(&ranges, &narrower));
        assert_eq!(
            narrower
                .iter()
                .map(|r| &data.values[0][r.clone()])
                .collect::<Vec<_>>(),
            ["a", "界", "b", "", "c"]
        );
    }

    #[tokio::test]
    async fn value_detection_cache_reuses_ranges_and_tracks_disabled_categories() {
        use crate::shared::value_colors::VALUE_STYLE_HTTP_GET;
        let t = tab().await;
        let mut data = displayed_row(&t, 0, &["message".into()]);
        data.values[0] = "GET /example 200".into();
        let mut colors = crate::theme::ValueColors::default();
        let first = data.value_spans(0, &colors);
        assert!(
            first
                .iter()
                .any(|(_, _, style)| *style == VALUE_STYLE_HTTP_GET)
        );
        assert!(std::rc::Rc::ptr_eq(&first, &data.value_spans(0, &colors)));
        colors.http_get = crate::style::Color::Rgb(1, 2, 3);
        assert!(
            std::rc::Rc::ptr_eq(&first, &data.value_spans(0, &colors)),
            "changing colors must reuse positions and resolve the new color at render time"
        );
        colors.disabled.insert("http_get".into());
        assert!(
            !data
                .value_spans(0, &colors)
                .iter()
                .any(|(_, _, style)| *style == VALUE_STYLE_HTTP_GET)
        );
        colors.disabled.clear();
        assert_eq!(data.value_spans(0, &colors).as_ref(), first.as_ref());
    }

    #[test]
    fn highlight_spans_outside_wrapped_chunks_do_not_underflow() {
        assert_eq!(chunk_range(&(0..4), 10, 20), None);
        assert_eq!(chunk_range(&(25..30), 10, 20), None);
        assert_eq!(chunk_range(&(10..10), 10, 20), None);
        assert_eq!(chunk_range(&(0..15), 10, 20), Some(0..5));
        assert_eq!(chunk_range(&(15..25), 10, 20), Some(5..10));
        assert_eq!(chunk_range(&(0..30), 10, 20), Some(0..10));
    }

    async fn tab() -> TabState {
        let reader = FileReader::from_bytes(b"a\nb\nc\nd\ne\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(reader, log_manager, "test.log".to_string())
    }

    /// Always parses to a fixed [`DisplayParts`] with one repeated-group
    /// item, regardless of the raw line — exercises `displayed_values`'s
    /// field-group lookup without needing a real custom schema.
    #[derive(Debug)]
    struct GroupFieldParser;
    impl crate::parser::LogFormatParser for GroupFieldParser {
        fn parse_line<'a>(&self, _line: &'a [u8]) -> Option<crate::parser::DisplayParts<'a>> {
            Some(crate::parser::DisplayParts {
                field_groups: vec![(
                    "mygroup",
                    vec![crate::parser::GroupItem {
                        fields: vec![(crate::parser::FieldSemantic::Extra, "key1", "value1")],
                    }],
                )],
                ..Default::default()
            })
        }
        fn name(&self) -> &str {
            "group-field-test"
        }
        fn collect_field_names(&self, _lines: &[&[u8]]) -> Vec<String> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn displayed_values_resolves_indexed_group_fields() {
        let mut t = tab().await;
        t.display.format = Some(std::sync::Arc::new(GroupFieldParser));
        let values = displayed_values(&t, 0, &["mygroup.0.key1".to_string()]);
        assert_eq!(values, vec!["value1".to_string()]);
    }

    #[test]
    fn process_colors_do_not_cycle_through_the_theme_palette() {
        let theme = TuiTheme::default();
        let colors: std::collections::HashSet<_> = (0..128)
            .map(|i| process_color(&theme, "target", &format!("process-{i}"), None).unwrap())
            .collect();
        assert_eq!(colors.len(), 128);
    }

    #[test]
    fn process_ids_share_logger_colors_and_respect_toggle() {
        let mut theme = TuiTheme::default();
        let color = process_color(&theme, "pid", "4321", Some("myapp")).unwrap();
        assert_eq!(
            Some(color),
            process_color(&theme, "target", "myapp", Some("myapp"))
        );
        let highlights = cell_highlights(b"4321", "4321", &theme, None, Some(color));
        assert_eq!(highlights[0].0, 0..4);
        assert_eq!(highlights[0].1.color, color_to_gpui(color).map(Into::into));
        assert!(process_color(&theme, "pid", "4321", None).is_some());
        assert!(process_color(&theme, "message", "4321", None).is_none());
        theme.value_colors.disabled.insert("process_colors".into());
        assert!(process_color(&theme, "pid", "4321", Some("myapp")).is_none());
    }

    #[test]
    fn value_colors_apply_to_structured_values_and_respect_disabled_categories() {
        let mut theme = TuiTheme::default();
        let text = "192.168.1.1";
        let highlights = cell_highlights(text.as_bytes(), text, &theme, None, None);
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].0, 0..text.len());
        assert_eq!(
            highlights[0].1.color,
            color_to_gpui(theme.value_colors.ip_address).map(Into::into)
        );
        theme.value_colors.disabled.insert("ip_address".into());
        assert!(cell_highlights(text.as_bytes(), text, &theme, None, None).is_empty());
    }

    #[test]
    fn filter_highlights_preserve_value_colors_outside_the_match() {
        let theme = TuiTheme::default();
        let text = "GET failed from 192.168.1.1";
        let foreground = gpui_kit::gpui::rgb(0xff1234);
        let filter = LineStyle {
            fg: Some(foreground),
            bg: None,
            spans: vec![(4, 10)],
            whole_line: false,
        };
        let highlights = cell_highlights(text.as_bytes(), text, &theme, Some(&filter), None);
        let color_at = |offset| {
            highlights
                .iter()
                .find(|(range, _)| range.contains(&offset))
                .unwrap()
                .1
                .color
        };
        assert_eq!(
            color_at(0),
            color_to_gpui(theme.value_colors.http_get).map(Into::into)
        );
        assert_eq!(color_at(4), Some(foreground.into()));
        assert_eq!(
            color_at(16),
            color_to_gpui(theme.value_colors.ip_address).map(Into::into)
        );
    }

    #[test]
    fn background_only_filter_preserves_value_foreground() {
        let theme = TuiTheme::default();
        let text = "GET /api";
        let background = gpui_kit::gpui::rgb(0x123456);
        let filter = LineStyle {
            fg: None,
            bg: Some(background),
            spans: vec![(0, 3)],
            whole_line: false,
        };
        let highlights = cell_highlights(text.as_bytes(), text, &theme, Some(&filter), None);
        assert_eq!(
            highlights[0].1.color,
            color_to_gpui(theme.value_colors.http_get).map(Into::into)
        );
        assert_eq!(highlights[0].1.background_color, Some(background.into()));
    }

    #[test]
    fn search_highlights_regex_matches_and_preserves_other_styles() {
        use gpui_kit::gpui::HighlightStyle;
        let theme = TuiTheme::default();
        let original = HighlightStyle {
            color: color_to_gpui(theme.text).map(Into::into),
            ..Default::default()
        };
        let mut search = crate::utils::search::Search::new();
        search.set_pattern(regex::Regex::new("(?i)érr(or)?").unwrap(), true);
        let text = "éRRor plain érr";
        let highlights =
            search_highlights(text, &search, &theme, None, vec![(0..text.len(), original)]);
        for (range, style) in &highlights {
            if text[range.clone()].starts_with('é') {
                assert_eq!(style.color, color_to_gpui(theme.search_fg).map(Into::into));
                assert_eq!(
                    style.background_color,
                    color_to_gpui(theme.text_highlight_fg).map(Into::into)
                );
            } else {
                assert_eq!(style.color, original.color);
                assert_eq!(style.background_color, None);
            }
        }
        assert_eq!(highlights.len(), 3);
        search.clear();
        assert_eq!(
            search_highlights(text, &search, &theme, None, vec![(0..text.len(), original)]).len(),
            1
        );
    }

    #[tokio::test]
    async fn current_search_highlight_tracks_occurrences_and_raw_column_offsets() {
        let mut tab = tab().await;
        let raw = br#"{"level":"INFO","target":"needle","message":"needle needle"}"#;
        tab.file_reader = FileReader::from_bytes(raw.to_vec());
        tab.display.format = crate::parser::detect_format(&[raw.as_slice()]).map(Arc::from);
        tab.search
            .query
            .search("needle", std::iter::once(0), |_| {
                Some(String::from_utf8_lossy(raw).into_owned())
            })
            .unwrap();
        assert_eq!(
            current_search_range(&tab, 0, "target", "needle"),
            Some(0..6)
        );
        assert_eq!(
            current_search_range(&tab, 0, "message", "needle needle"),
            None
        );
        tab.search.query.next_match();
        assert_eq!(current_search_range(&tab, 0, "target", "needle"), None);
        assert_eq!(
            current_search_range(&tab, 0, "message", "needle needle"),
            Some(0..6)
        );
        tab.search.query.next_match();
        let active = current_search_range(&tab, 0, "message", "needle needle");
        assert_eq!(active, Some(7..13));
        assert_eq!(
            current_search_range(&tab, 1, "message", "needle needle"),
            None
        );
        let theme = TuiTheme::default();
        let highlights =
            search_highlights("needle needle", &tab.search.query, &theme, active, vec![]);
        let ordinary = highlights.iter().find(|(r, _)| r.start == 0).unwrap().1;
        let current = highlights.iter().find(|(r, _)| r.start == 7).unwrap().1;
        assert_ne!(ordinary.background_color, current.background_color);
        assert_eq!(current.color, ordinary.background_color);
        assert_eq!(current.background_color, ordinary.color);
        assert_eq!(current.font_weight, Some(FontWeight::BOLD));
        tab.search.query.next_match();
        tab.display.show_keys = true;
        assert_eq!(
            current_search_range(&tab, 0, "target", "target=needle"),
            Some(7..13)
        );
    }

    #[tokio::test]
    async fn message_column_fills_remaining_space_and_tracks_visible_columns() {
        let mut tab = tab().await;
        tab.display.raw_mode = true;
        tab.display.show_line_numbers = false;
        let widths = TableWidths::from(ColumnWidths::default());
        assert_eq!(message_width(&tab, &[], &widths, 1200.), 1192.);
        assert_eq!(message_width(&tab, &[], &widths, 800.), 792.);
        tab.display.show_line_numbers = true;
        assert_eq!(
            message_width(&tab, &[], &widths, 1200.),
            1186. - widths.line_no
        );
        assert_eq!(message_width(&tab, &[], &widths, 80.), 160.);
    }

    #[tokio::test]
    async fn preparing_wrapped_layout_does_not_parse_offscreen_rows() {
        #[derive(Debug)]
        struct NoEagerParsing;
        impl crate::parser::LogFormatParser for NoEagerParsing {
            fn parse_line<'a>(&self, _: &'a [u8]) -> Option<crate::parser::DisplayParts<'a>> {
                panic!("list preparation must not parse rows");
            }
            fn collect_field_names(&self, _: &[&[u8]]) -> Vec<String> {
                vec!["message".into()]
            }
            fn name(&self) -> &str {
                "no-eager-parsing"
            }
        }
        let mut tab = tab().await;
        tab.file_reader = FileReader::from_bytes(b"row\n".repeat(1000));
        tab.filter.visible_indices = crate::shared::VisibleLines::All(1000);
        tab.display.format = Some(Arc::new(NoEagerParsing));
        tab.display.wrap = true;
        let state = ListState::new(0, gpui_kit::gpui::ListAlignment::Top, px(0.));
        prepare_list(
            &mut None,
            &state,
            &tab,
            &ColumnWidths::default().into(),
            &["message".into()],
            400.,
        );
        assert_eq!(state.item_count(), 1000);
    }

    #[tokio::test]
    async fn scrollbar_extent_tracks_loaded_visible_rows() {
        let mut tab = tab().await;
        let mut cache = None;
        let widths = TableWidths::from(ColumnWidths::default());
        let state = ListState::new(0, gpui_kit::gpui::ListAlignment::Top, px(0.));
        for count in [100, 1000, 250, 0] {
            tab.file_reader = FileReader::from_bytes(b"row\n".repeat(count));
            tab.filter.visible_indices = crate::shared::VisibleLines::All(count);
            prepare_list(&mut cache, &state, &tab, &widths, &[], 600.);
            assert_eq!(state.item_count(), count);
            assert_eq!(
                state.max_offset_for_scrollbar().y,
                px(count as f32 * ROW_HEIGHT_PX)
            );
        }
    }

    #[tokio::test]
    async fn resizing_recalculates_wrapped_row_heights() {
        let mut tab = tab().await;
        tab.file_reader =
            FileReader::from_bytes(format!("{}\n", "x".repeat(120)).repeat(5).into_bytes());
        tab.display.raw_mode = true;
        tab.display.wrap = true;
        let mut cache = None;
        let widths = TableWidths::from(ColumnWidths::default());
        let state = ListState::new(0, gpui_kit::gpui::ListAlignment::Top, px(0.));
        prepare_list(&mut cache, &state, &tab, &widths, &[], 600.);
        let narrow_key = cache.as_ref().unwrap().key.clone();
        prepare_list(&mut cache, &state, &tab, &widths, &[], 1000.);
        assert_ne!(narrow_key, cache.as_ref().unwrap().key);
        assert_eq!(
            wrapped_height(&tab, 0, &["message".into()], &widths, 600.),
            ROW_HEIGHT_PX * 2.
        );
        assert_eq!(
            wrapped_height(&tab, 0, &["message".into()], &widths, 1000.),
            ROW_HEIGHT_PX
        );
        state.scroll_to(ListOffset {
            item_ix: 2,
            offset_in_item: px(3.),
        });
        prepare_list(&mut cache, &state, &tab, &widths, &[], 1000.);
        assert_eq!(state.logical_scroll_top().item_ix, 2);
        assert_eq!(state.logical_scroll_top().offset_in_item, px(3.));
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
        let _ = line_row(
            &tab,
            0,
            false,
            false,
            &theme,
            &TableWidths::from(ColumnWidths::default()),
            &[],
            600.,
        );
    }

    #[tokio::test]
    async fn line_row_does_not_panic_for_the_plain_cursor_row() {
        // Outside an active visual-line selection, the cursor row still
        // gets its own (cursor_bg/cursor_fg) highlight — this just
        // exercises that path for a panic, since there's no element-tree
        // introspection in this test module to assert the exact colors.
        let tab = paged_tab().await;
        let theme = TuiTheme::default();
        let _ = line_row(
            &tab,
            0,
            false,
            true,
            &theme,
            &TableWidths::from(ColumnWidths::default()),
            &[],
            600.,
        );
    }

    #[tokio::test]
    async fn row_time_text_is_blank_for_unstructured_files() {
        let t = tab().await;
        assert!(t.display.format.is_none());
        assert_eq!(row_time_text(&t, b"a"), None);
    }

    #[tokio::test]
    async fn line_canonical_timestamp_resolves_bsd_year_from_the_current_year() {
        let mut t = tab().await;
        t.display.format = Some(Arc::new(crate::parser::SyslogParser::default()));
        t.file_reader = FileReader::from_bytes(
            b"<134>Oct 11 22:14:15 myhost sshd[1234]: Accepted password for user\n".to_vec(),
        );
        let current_year = time::OffsetDateTime::now_utc().year();
        let ts = line_canonical_timestamp(&t, 0);
        assert_eq!(ts, Some(format!("{current_year}-10-11 22:14:15")));
        // The whole point of this shape is that `:date-filter`'s own
        // parser accepts it as a bound — `canonical_timestamp`'s `.fff`
        // suffix, which this strips, is not valid date-filter syntax.
        crate::filters::parse_date_filter(&format!(">= {}", ts.unwrap())).unwrap();
    }

    #[tokio::test]
    async fn line_canonical_timestamp_is_none_without_a_detected_format() {
        let t = tab().await;
        assert_eq!(line_canonical_timestamp(&t, 0), None);
    }

    #[tokio::test]
    async fn row_message_bytes_is_the_full_line_without_a_detected_format() {
        let t = tab().await;
        let line = b"<134>Oct 11 22:14:15 myhost sshd[1234]: Accepted password for user";
        assert_eq!(row_message_bytes(&t, line), line.as_slice());
    }

    #[tokio::test]
    async fn row_message_bytes_strips_the_syslog_prefix_so_time_is_not_duplicated() {
        let mut t = tab().await;
        t.display.format = Some(Arc::new(crate::parser::SyslogParser::default()));
        let line = b"<134>Oct 11 22:14:15 myhost sshd[1234]: Accepted password for user";
        assert_eq!(
            row_message_bytes(&t, line),
            b"Accepted password for user".as_slice()
        );
    }

    #[tokio::test]
    async fn all_fields_default_to_visible_and_commands_can_hide_and_reorder() {
        let mut tab = tab().await;
        tab.display.format = Some(Arc::new(crate::parser::SyslogParser::default()));
        let bytes = b"<134>Oct 11 22:14:15 myhost sshd[1234]: Accepted password";
        let fields = tab
            .display
            .format
            .as_ref()
            .unwrap()
            .collect_field_names(&[bytes.as_slice()]);
        assert_eq!(visible_columns(&tab, &fields), fields);
        assert!(fields.contains(&"target".to_string()));
        tab.display.hidden_fields.insert("target".to_string());
        assert!(!visible_columns(&tab, &fields).contains(&"target".to_string()));
        tab.display.field_layout.columns =
            Some(vec!["message".to_string(), "timestamp".to_string()]);
        assert_eq!(visible_columns(&tab, &fields)[0], "message");
    }

    #[test]
    fn wrapping_preserves_unicode_and_allocates_multiple_visual_lines() {
        assert_eq!(wrap_text("東京abc", 22.), vec!["東", "京a", "bc"]);
        assert_eq!(wrap_text("a\nb", 100.), vec!["a", "b"]);
    }
    #[tokio::test]
    async fn raw_display_bypasses_parser_and_columns() {
        let mut t = tab().await;
        t.display.raw_mode = true;
        assert_eq!(visible_columns(&t, &["level".into()]), vec!["message"]);
        assert_eq!(row_message_bytes(&t, b"raw text"), b"raw text");
    }
    #[tokio::test]
    async fn multiline_schemas_preserve_source_lines_in_one_column() {
        use crate::config::CustomSchemaConfig;
        use crate::parser::{CustomParser, LogFormatParser};

        let mut t = tab().await;
        for schema in [
            include_str!("../../../examples/transactions.json"),
            r#"{"name":"folded", "template":"{level} {message}", "multiline":true}"#,
        ] {
            let config: CustomSchemaConfig = serde_json::from_str(schema).unwrap();
            let parser = CustomParser::from_config(&config).unwrap();
            assert!(parser.wants_continuation_walk());
            t.display.format = Some(std::sync::Arc::new(parser));
            t.display.hidden_fields.insert("message".into());
            t.display.show_keys = true;
            assert!(!t.display.raw_mode);
            assert_eq!(
                visible_columns(&t, &["timestamp".into(), "level".into()]),
                vec!["message"]
            );
            let source = "### Transaction 1610 started at: 2026-10-21 11:37:49.874\nStatus: COMPLETED\n  object_name: \"FanPwmConfig0\"\nINFO original message\n";
            t.file_reader = FileReader::from_bytes(source.as_bytes().to_vec());
            for (line, expected) in source.lines().enumerate() {
                assert_eq!(
                    displayed_values(&t, line, &visible_columns(&t, &[])),
                    vec![expected]
                );
            }
        }
    }
    #[tokio::test]
    async fn unstructured_and_merged_field_lists_keep_the_raw_message_column() {
        let tab = tab().await;
        assert_eq!(visible_columns(&tab, &[]), vec!["message"]);
        assert_eq!(
            visible_columns(&tab, &["source".to_string()]),
            vec!["source", "message"]
        );
    }
}

#[cfg(test)]
mod multiline_tests {
    #[test]
    fn only_multiline_parents_have_controls() {
        let map = [0, 0, 0, 3, 4, 4];
        assert_eq!(super::continuation_count(&map, 0), 2);
        assert_eq!(super::continuation_count(&map, 1), 0);
        assert_eq!(super::continuation_count(&map, 3), 0);
        assert_eq!(super::continuation_count(&map, 4), 1);
        assert_eq!(super::continuation_count(&map, 6), 0);
    }
}

fn continuation_count(map: &[usize], line: usize) -> usize {
    if map.get(line) != Some(&line) {
        return 0;
    }
    map.partition_point(|parent| *parent <= line)
        .saturating_sub(line + 1)
}
