//! Notes are attached to source rows so filters and column edits cannot move them.
use crate::gui::{actions::Action, app::App, message::Message, state::GuiState};
use gpui_kit::component::button::Button;
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

pub fn page(app: &App, cx: &mut Context<App>) -> impl IntoElement {
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
                .label("Add annotation to selected rows")
                .on_click(cx.listener(|app, _, window, cx| {
                    app.dispatch(Message::GuiAction(Action::Annotation), window, cx);
                })),
        );
    let Some(tab) = app.state.active_tab() else {
        return content.child("Open a file to add annotations");
    };
    if tab.comment_manager.get().is_empty() {
        content = content.child("No annotations. Select rows in Logs, then add a note.");
    }
    for (index, comment) in tab.comment_manager.get().iter().enumerate() {
        let line = comment.line_indices.first().copied();
        content = content.child(
            div()
                .flex()
                .gap_2()
                .items_center()
                .py(px(8.))
                .child(div().flex_1().child(comment.text.clone()))
                .child(
                    Button::new(("annotation-jump", index))
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
                        .label("Edit")
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.dispatch(Message::EditAnnotation(index), window, cx);
                        })),
                )
                .child(
                    Button::new(("annotation-delete", index))
                        .label("Delete")
                        .on_click(cx.listener(move |app, _, _, cx| {
                            if let Some(tab) = app.state.active_tab_mut() {
                                tab.comment_manager.remove(index);
                            }
                            cx.notify();
                        })),
                ),
        );
    }
    content
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
    }
}
