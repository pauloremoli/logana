//! Native bookmark management uses source indices, independent of filtering.
use crate::gui::{
    app::App,
    state::{NavPage, StatusMessage},
};

pub fn jump(app: &mut App, line: usize) -> bool {
    set_view(&mut app.state, NavPage::Logs);
    let Some(tab) = app.state.active_tab_mut() else {
        return false;
    };
    let Some(row) = tab.filter.visible_indices.position_of(line) else {
        app.state.notify(StatusMessage::Info(
            "This row is hidden by the current filters.".into(),
        ));
        return false;
    };
    tab.scroll.scroll_offset = row;
    let source = tab.id;
    app.state.log_selection = Some((source, row, row));
    app.state.nav_page = NavPage::Logs;
    app.log_scroll.scroll_to_reveal_item(row);
    true
}

/// Share the Logs table and its source-line mapping for the bookmarks view.
pub fn set_view(state: &mut crate::gui::state::GuiState, page: NavPage) {
    if state.nav_page != page {
        state.log_selection = None;
    }
    state.nav_page = page;
    if matches!(page, NavPage::Logs | NavPage::Bookmarks) {
        for tab in &mut state.tabs {
            let marks_only = page == NavPage::Bookmarks;
            if tab.filter.show_marks_only != marks_only {
                tab.filter.show_marks_only = marks_only;
                refresh(tab);
            }
        }
    }
}

pub fn sync(state: &mut crate::gui::state::GuiState) {
    if state.nav_page != NavPage::Bookmarks {
        return;
    }
    if let Some(tab) = state.active_tab_mut() {
        if tab.filter.show_marks_only && tab.filter.handle.is_some() {
            return;
        }
        let mut marks = tab.mark_manager.get_indices();
        marks.retain(|&line| line < tab.file_reader.line_count());
        if !tab.filter.show_marks_only || !tab.filter.visible_indices.iter().eq(marks) {
            tab.filter.show_marks_only = true;
            refresh(tab);
            state.log_selection = None;
        }
    }
}

fn refresh(tab: &mut crate::shared::TabState) {
    let handle = crate::gui::runtime::handle();
    let _guard = handle.enter();
    tab.begin_filter_refresh_background();
    if let Some(query) = tab.search.query.get_pattern().map(str::to_owned) {
        tab.begin_search(&query, true, false);
    }
}
