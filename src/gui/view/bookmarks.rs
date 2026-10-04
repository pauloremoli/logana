//! Native bookmark management uses source indices, independent of filtering.
use crate::gui::{
    app::App,
    state::{NavPage, StatusMessage},
};
use gpui_kit::component::{ActiveTheme, button::Button};
use gpui_kit::gpui::{Context, ScrollStrategy, div, prelude::*, px};

pub fn jump(app: &mut App, line: usize) -> bool {
    let Some(tab) = app.state.active_tab_mut() else {
        return false;
    };
    let Some(row) = tab.filter.visible_indices.iter().position(|i| i == line) else {
        app.state.notify(StatusMessage::Info(
            "This row is hidden by the current filters.".into(),
        ));
        return false;
    };
    tab.scroll.scroll_offset = row;
    app.state.nav_page = NavPage::Logs;
    app.log_scroll.scroll_to_item(row, ScrollStrategy::Top);
    true
}

pub fn page(app: &App, cx: &mut Context<App>) -> impl IntoElement {
    let mut content = div()
        .id("bookmarks-page")
        .flex_1()
        .p_4()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_2();
    let Some(tab) = app.state.active_tab() else {
        return content.child("Open a file to manage bookmarks");
    };
    content = content.child(
        div()
            .flex()
            .gap_2()
            .child("Bookmarks")
            .child(
                Button::new("bookmark-current")
                    .label("Toggle current row")
                    .on_click(cx.listener(|app, _, _, cx| {
                        if let Some(tab) = app.state.active_tab_mut() {
                            if tab.scroll.scroll_offset < tab.filter.visible_indices.len() {
                                let line = tab.filter.visible_indices.get(tab.scroll.scroll_offset);
                                tab.mark_manager.toggle(line);
                            }
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("bookmark-clear")
                    .label("Clear bookmarks")
                    .on_click(cx.listener(|app, _, _, cx| {
                        if let Some(tab) = app.state.active_tab_mut() {
                            tab.mark_manager.clear();
                        }
                        cx.notify();
                    })),
            ),
    );
    let indices = tab.mark_manager.get_indices();
    if indices.is_empty() {
        content = content.child("No bookmarked rows. Select a row in Logs, then toggle it here.");
    }
    for line in indices {
        if line >= tab.file_reader.line_count() {
            continue;
        }
        let preview = String::from_utf8_lossy(&tab.file_reader.get_line(line)).into_owned();
        content = content.child(
            div()
                .flex()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(cx.theme().border)
                .py(px(8.))
                .child(
                    Button::new(("bookmark-jump", line))
                        .label(format!("Line {}", line + 1))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            jump(app, line);
                            cx.notify();
                        })),
                )
                .child(div().flex_1().min_w(px(0.)).truncate().child(preview))
                .child(
                    Button::new(("bookmark-remove", line))
                        .label("Remove")
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if let Some(tab) = app.state.active_tab_mut() {
                                tab.mark_manager.toggle(line);
                            }
                            cx.notify();
                        })),
                ),
        );
    }
    content
}
