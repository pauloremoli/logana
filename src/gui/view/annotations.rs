//! Notes are attached to source rows so filters and column edits cannot move them.
use crate::gui::{actions::Action, app::App, message::Message, state::GuiState};
use gpui_kit::base::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, list::ListItem};
use gpui_kit::gpui::{Context, div, prelude::*, px};

pub fn selected_lines(state: &GuiState) -> Vec<usize> {
    let Some(tab) = state.active_tab() else {
        return Vec::new();
    };
    let (first, last) = state
        .log_selection
        .filter(|(id, _, _)| *id == tab.id)
        .map(|(_, a, b)| (a.min(b), a.max(b)))
        .unwrap_or((tab.scroll.scroll_offset, tab.scroll.scroll_offset));
    tab.filter
        .visible_indices
        .iter()
        .skip(first)
        .take(last.saturating_sub(first) + 1)
        .collect()
}

pub fn page(app: &App, cx: &mut Context<App>) -> gpui_kit::gpui::AnyElement {
    let mut content = div()
        .id("annotations-page")
        .flex_1()
        .p_4()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            Button::new("annotation-add")
                .label("Add note to selected rows")
                .disabled(selected_lines(&app.state).is_empty())
                .on_click(cx.listener(|app, _, window, cx| {
                    app.dispatch(Message::GuiAction(Action::Annotation), window, cx);
                })),
        );
    let Some(tab) = app.state.active_tab() else {
        return content.child("Open a file to add notes").into_any_element();
    };
    if tab.comment_manager.get().is_empty() {
        content = content.child("No annotations. Select rows in Logs, then add a note.");
    }
    for (index, comment) in tab.comment_manager.get().iter().enumerate() {
        let line = comment.line_indices.first().copied();
        content = content.child(
            ListItem::new(("annotation-row", index)).child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .py(px(8.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().child(comment.text.clone()))
                            .child(
                                Button::new(("annotation-jump", index))
                                    .primary()
                                    .label("Go to row")
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        if let Some(line) = line {
                                            super::bookmarks::jump(app, line);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(("annotation-edit", index))
                                    .ghost()
                                    .label("Edit")
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        app.dispatch(Message::EditAnnotation(index), window, cx);
                                    })),
                            )
                            .child(
                                Button::new(("annotation-delete", index))
                                    .ghost()
                                    .label("Delete")
                                    .on_click(cx.listener(move |app, _, window, cx| {
                                        app.dispatch(Message::AnnotationRemoved(index), window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} · {} source rows · row {}",
                                tab.title,
                                comment.line_indices.len(),
                                line.map(|n| n + 1).unwrap_or(0)
                            )),
                    )
                    .when_some(line, |d, line| {
                        d.child(
                            div()
                                .truncate()
                                .text_sm()
                                .font_family("monospace")
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    String::from_utf8_lossy(&tab.file_reader.get_line(line))
                                        .into_owned(),
                                ),
                        )
                    }),
            ),
        );
    }
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(content.min_h(px(0.)))
        .child(super::status_bar::status_bar(
            Some(tab),
            crate::gui::state::NavPage::Annotations,
            cx,
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn selected_rows_are_source_indices_and_reverse_ranges_work() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db.clone());
        let manager = crate::db::LogManager::new(db, None).await;
        let tab = crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"a\nb\nc\nd\n".to_vec()),
            manager,
            "test".into(),
        );
        state.tabs.push(tab);
        state.log_selection = Some((state.tabs[0].id, 2, 1));
        assert_eq!(selected_lines(&state), vec![1, 2]);
        crate::gui::actions::open(&mut state, Action::Annotation, None, None);
        state.editor.as_mut().unwrap().values[0] = "Unicode note α".into();
        crate::gui::actions::apply(&mut state);
        assert_eq!(state.tabs[0].mark_manager.get_indices(), vec![1, 2]);
        assert_eq!(state.nav_page, crate::gui::state::NavPage::Logs);
        assert_eq!(
            state.tabs[0].comment_manager.get()[0].line_indices,
            vec![1, 2]
        );
        crate::gui::actions::open(&mut state, Action::Annotation, Some(0), None);
        state.editor.as_mut().unwrap().values[0] = "Edited".into();
        crate::gui::actions::apply(&mut state);
        assert_eq!(state.tabs[0].comment_manager.get().len(), 1);
        assert_eq!(state.tabs[0].comment_manager.get()[0].text, "Edited");
        state.log_selection = None;
        state.tabs[0].scroll.scroll_offset = 3;
        assert_eq!(selected_lines(&state), vec![3]);
        state.tabs[0].scroll.scroll_offset = 9;
        assert!(selected_lines(&state).is_empty());
        crate::gui::update::update(&mut state, Message::AnnotationRemoved(0));
        assert!(state.tabs[0].comment_manager.get().is_empty());
        crate::gui::update::update(&mut state, Message::GuiAction(Action::Undo));
        assert_eq!(state.tabs[0].comment_manager.get()[0].text, "Edited");
    }
}
