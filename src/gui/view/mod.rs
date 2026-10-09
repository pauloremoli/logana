pub mod action_editor;
pub mod annotations;
pub mod bookmarks;
pub mod command_palette;
pub mod field_facets;
pub mod filter_pane;
pub mod group_pane;
pub mod loading;
pub mod log_pane;
pub mod notification;
pub mod row_inspector;
mod schema_preview;
pub mod search_bar;
pub mod sidebar;
pub mod status_bar;
pub mod tab_bar;

use crate::gui::app::App;
use crate::gui::state::NavPage;
use crate::gui::{actions::Action, message::Message};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::TitleBar;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::empty::{Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyTitle};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{h_resizable, resizable_panel};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

/// Leave enough room for the native editor's line height plus both borders.
/// The component's default 32px height with 8px vertical padding clips glyphs.
pub(super) fn input(
    state: &gpui_kit::gpui::Entity<gpui_kit::component::input::InputState>,
) -> gpui_kit::component::input::Input {
    gpui_kit::component::input::Input::new(state)
        .min_h(px(36.))
        .py(px(4.))
        .flex_shrink_0()
}

pub fn view(app: &mut App, window: &mut Window, cx: &mut Context<App>) -> impl IntoElement {
    let viewport = window.viewport_size();
    let mut root = div()
        .relative()
        .flex()
        .flex_col()
        .size_full()
        // Draws the real minimize/maximize/close controls (platform-gated
        // internally — native traffic lights on macOS, skipped entirely
        // under server-side decorations) and owns window dragging; see
        // `TitleBar::window_options()` in `logana_gui.rs`'s window setup.
        .child(
            TitleBar::new()
                .on_close_window(cx.listener(|app, _, _, cx| app.request_quit(cx)))
                .child("logana"),
        )
        .child(div().h(px(32.)).flex_shrink_0().child(app.menu_bar.clone()))
        .child(
            div().flex().flex_1().min_h(px(0.)).child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .child(tab_bar::tab_bar(&app.state, cx))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_1()
                            .min_h(px(0.))
                            .child(body(app, window, cx))
                            .children(loading::picker(app, f32::from(viewport.width), cx))
                            .children(row_inspector::panel(app, f32::from(viewport.width), cx)),
                    ),
            ),
        );
    if let Some(operations) = loading::operations(app, cx) {
        root = root.child(operations);
    }
    if let Some(search) = search_bar::search_popup(app, cx) {
        root = root.child(search);
    }
    if let Some(palette) = command_palette::command_palette(app, viewport, cx) {
        root = root.child(palette);
    }
    if let Some(editor) = action_editor::editor(app, viewport, cx) {
        root = root.child(editor);
    }
    if let Some(notification) = notification::notification(app, viewport, cx) {
        root = root.child(notification);
    }
    root
}

fn body(app: &mut App, window: &mut Window, cx: &mut Context<App>) -> impl IntoElement {
    if app.state.nav_page == NavPage::Annotations {
        return annotations::page(app, cx).into_any_element();
    }
    if !matches!(app.state.nav_page, NavPage::Logs | NavPage::Bookmarks) {
        return div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child("Not yet implemented")
            .into_any_element();
    }
    let active_tab = app.state.active_tab;
    match app.state.tabs.get(active_tab) {
        Some(_) => {
            // Re-borrow after the match's immutable borrow of `app.state`
            // ends, so `log_pane`'s `cx.entity()` call (which needs
            // `&mut Context<App>`) and `sidebar`'s panes aren't fighting
            // the outer borrow.
            let fields = app.state.tabs[active_tab].collect_field_names();
            let tab = &app.state.tabs[active_tab];
            let mut widths = crate::gui::column_widths::table_widths(
                &mut app.field_widths,
                tab,
                &fields,
                app.state.column_widths,
            );
            if let Some(saved) = app
                .state
                .preferences
                .column_widths
                .get(&crate::gui::column_widths::format_key(tab))
            {
                widths.restore(saved);
            }
            let viewport_width = f32::from(window.viewport_size().width);
            let compact = viewport_width < 720.;
            let sidebar_width = app.sidebar_width.min((viewport_width * 0.45).max(240.));
            let available_width = viewport_width
                - if tab.display.show_sidebar && !compact {
                    sidebar_width
                } else {
                    0.
                }
                - if viewport_width >= 1100. && app
                    .loading
                    .selected_picker
                    .and_then(|id| app.loading.operations.iter().find(|op| op.id == id))
                    .is_some_and(|op| op.picker.is_some())
                {
                    420_f32.min(f32::from(window.viewport_size().width) * 0.5)
                } else {
                    0.
                }
                - if viewport_width >= 1100. && app.inspected_row.is_some() { 420. } else { 0. }
                - 24.; // Table container's horizontal padding.
            let message_width = log_pane::message_width(tab, &fields, &widths, available_width);
            log_pane::prepare_scaled_list(
                &mut app.table_layout,
                &app.log_scroll,
                &app.state.tabs[active_tab],
                &widths,
                &fields,
                message_width,
                f32::from(window.rem_size()) / 13.,
            );
            let tab = &app.state.tabs[active_tab];
            let sidebar_on_left = tab.display.sidebar_side.is_left();
            let show_sidebar = tab.display.show_sidebar;
            let no_rows = tab.filter.visible_indices.is_empty() && tab.filter.handle.is_none();
            let log_content = if no_rows {
                let bookmarks = app.state.nav_page == NavPage::Bookmarks;
                let no_data = tab.file_reader.line_count() == 0;
                Empty::new()
                    .header(EmptyHeader::new()
                        .title(EmptyTitle::new().child(if bookmarks { "No bookmarks yet" } else if no_data { "No log rows yet" } else { "No rows match" }))
                        .description(EmptyDescription::new().child(if bookmarks { "Select a row in Logs and bookmark it to keep useful evidence here." } else if no_data { "This source has no rows. Live sources will appear as data arrives." } else { "The current filters hide every row. Turn filtering off to see the source." })))
                    .content(EmptyContent::new().child(
                        Button::new("empty-logs-recover").label(if bookmarks { "Return to Logs" } else if no_data { "Open another file" } else { "Turn filtering off" })
                            .on_click(cx.listener(move |app, _, window, cx| {
                                if bookmarks { app.dispatch(Message::NavPageSelected(NavPage::Logs), window, cx); }
                                else { app.dispatch(Message::GuiAction(if no_data { Action::Open } else { Action::ToggleFiltering }), window, cx); }
                            }))))
                    .into_any_element()
            } else {
                log_pane::log_pane(active_tab, tab, &widths, &app.log_scroll, &app.horizontal_scroll, &fields, message_width,
                    f32::from(window.rem_size()) / 13., app.text_handle.clone(), app.text_document.clone(), cx).into_any_element()
            };
            let table = div()
                .id("log-workspace")
                .test_support()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h(px(0.))
                        .px(px(12.))
                        .pb(px(8.))
                        .child(log_content),
                )
                .child(Button::new("inspect-selected-row").ghost().label("Inspect selected row")
                    .on_click(cx.listener(|app, _, _, cx| {
                        if let Some(line) = annotations::selected_lines(&app.state).first().copied()
                            && let Some(tab) = app.state.active_tab() {
                            app.inspected_row = Some((tab.id, line));
                            app.inspector_raw = false;
                            cx.notify();
                        }
                    })))
                .child(status_bar::status_bar(Some(tab), app.state.nav_page, cx))
                .into_any_element();
            let sidebar = {
                let sidebar_tab = app.state.sidebar_tab;
                let tab = &app.state.tabs[active_tab];
                sidebar::sidebar(
                    active_tab,
                    tab,
                    sidebar_tab,
                    app.state
                        .keyboard_sidebar
                        .map(|scope| (scope, app.state.sidebar_cursor)),
                    app.state.facet_browser_open,
                    &app.state.facet_expanded,
                    &app.facet_pages,
                    app.facet_search.as_ref(),
                    app.facet_counts
                        .get(&tab.id)
                        .and_then(|entry| entry.counts.as_ref()),
                    &app.filter_sidebar_scroll,
                    &app.group_sidebar_scroll,
                    cx,
                )
                .into_any_element()
            };
            let mut row = div().relative().flex().flex_1().min_w(px(0.)).min_h(px(0.));
            if !show_sidebar {
                return row.child(table).into_any_element();
            }
            if compact {
                return row.child(table).child(div().absolute().top_0().bottom_0()
                    .when(sidebar_on_left, |d| d.left_0())
                    .when(!sidebar_on_left, |d| d.right_0())
                    .w(px(sidebar_width)).max_w(gpui_kit::gpui::relative(0.9))
                    .occlude().shadow_lg()
                    .on_mouse_down(gpui_kit::gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(sidebar)).into_any_element();
            }
            let entity = cx.entity();
            let sidebar_index = usize::from(!sidebar_on_left);
            let panels = h_resizable(if sidebar_on_left { "workspace-left-panels" } else { "workspace-right-panels" })
                .on_resize(move |state, _, cx| {
                    if let Some(size) = state.read(cx).sizes().get(sidebar_index) {
                        let width = f32::from(*size);
                        entity.update(cx, |app, cx| { app.sidebar_width = width; cx.notify(); });
                    }
                });
            let sidebar = resizable_panel().size(px(sidebar_width)).size_range(px(240.)..px(560.)).child(sidebar);
            let table = resizable_panel().size(px((available_width + 24.).max(280.))).size_range(px(280.)..px(10000.)).child(table);
            row = row.child(if sidebar_on_left { panels.child(sidebar).child(table) } else { panels.child(table).child(sidebar) });
            row.into_any_element()
        }
        None => Empty::new()
            .header(EmptyHeader::new()
                .title(EmptyTitle::new().child("Start investigating logs"))
                .description(EmptyDescription::new().child("Open a log file, directory, or archive. Or connect a live source to watch new events.")))
            .content(EmptyContent::new().child(div().flex().flex_wrap().gap_2()
                .child(Button::new("start-open-file").primary().label("Open files…")
                    .on_click(cx.listener(|app, _, window, cx| app.dispatch(Message::OpenFileDialog, window, cx))))
                .child(Button::new("start-open-directory").label("Open directory…")
                    .on_click(cx.listener(|app, _, window, cx| app.dispatch(Message::GuiAction(Action::OpenDirectory), window, cx))))
                .child({
                    let entity = cx.entity();
                    Button::new("start-connect-source").label("Connect source…")
                        .dropdown_menu(move |mut menu, _, _| {
                            for (label, action) in [("Docker container", Action::Docker), ("DLT device", Action::Dlt), ("OpenTelemetry receiver", Action::Otel), ("Process output", Action::Run)] {
                                let entity = entity.clone();
                                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                                    entity.update(cx, |app, cx| app.dispatch(Message::GuiAction(action), window, cx));
                                }));
                            }
                            menu
                        })
                })))
            .into_any_element(),
    }
}
