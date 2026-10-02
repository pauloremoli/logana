use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::{GuiState, NavPage};
use gpui_kit::assets::IconName;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

const NAV_ITEM_HEIGHT: f32 = 56.0;
const NAV_RAIL_WIDTH: f32 = 88.0;

/// The active tab's bookmark count, for the nav rail's `Bookmarks` badge —
/// scoped to the active tab, same as every other per-tab panel (filters,
/// groups). `None`/no tabs open renders as `0`.
fn bookmark_count(state: &GuiState) -> usize {
    state
        .active_tab()
        .map(|tab| tab.mark_manager.get_indices().len())
        .unwrap_or(0)
}

/// The left icon nav rail. Only `NavPage::Logs` has real content today
/// (`gui::view::mod::body` branches on `state.nav_page`); the rest are
/// placeholders — see the GUI redesign plan's "out of scope" section.
pub fn nav_rail(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .w(px(NAV_RAIL_WIDTH))
        .h_full()
        .overflow_hidden()
        .border_r_1()
        .border_color(cx.theme().border)
        .child(nav_item(
            IconName::FileText,
            "Logs",
            NavPage::Logs,
            state.nav_page == NavPage::Logs,
            None,
            cx,
        ))
        .child(nav_item(
            IconName::Bookmark,
            "Bookmarks",
            NavPage::Bookmarks,
            state.nav_page == NavPage::Bookmarks,
            Some(bookmark_count(state)),
            cx,
        ))
        .child(nav_item(
            IconName::MessageSquare,
            "Annotations",
            NavPage::Annotations,
            state.nav_page == NavPage::Annotations,
            None,
            cx,
        ))
        .child(nav_item(
            IconName::Search,
            "Searches",
            NavPage::Searches,
            state.nav_page == NavPage::Searches,
            None,
            cx,
        ))
        .child(nav_item(
            IconName::Settings,
            "Settings",
            NavPage::Settings,
            state.nav_page == NavPage::Settings,
            None,
            cx,
        ))
}

#[allow(clippy::too_many_arguments)]
fn nav_item(
    icon: IconName,
    label: &'static str,
    page: NavPage,
    active: bool,
    badge_count: Option<usize>,
    cx: &mut Context<App>,
) -> impl IntoElement {
    // The label row (text + optional badge) is its own nested flex row,
    // not a bare text child: without an explicit `.truncate()` (which
    // bundles `overflow_hidden()`/`whitespace_nowrap()`/ellipsis), a
    // multi-word label like "Bookmarks" or "Annotations" wraps onto
    // multiple lines inside this narrow column and visually overlaps the
    // badge/next item — this is exactly what broke in an earlier build.
    let mut label_row = div()
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .w_full()
        .overflow_hidden()
        .child(div().truncate().child(label));
    if let Some(count) = badge_count.filter(|c| *c > 0) {
        label_row = label_row.child(Badge::new().count(count));
    }

    let mut item = div()
        .id(label)
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .h(px(NAV_ITEM_HEIGHT))
        .w_full()
        .overflow_hidden()
        .child(Icon::new(icon))
        .child(label_row)
        .on_click(
            cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                app.dispatch(Message::NavPageSelected(page), window, cx);
            }),
        );
    if active {
        item = item.bg(cx.theme().accent);
    }
    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::ingestion::FileReader;
    use std::sync::Arc;

    #[tokio::test]
    async fn bookmark_count_is_zero_with_no_active_tab() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let state = GuiState::new(db);
        assert_eq!(bookmark_count(&state), 0);
    }

    #[tokio::test]
    async fn bookmark_count_reflects_the_active_tabs_marks() {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let mut state = GuiState::new(db);
        let reader = FileReader::from_bytes(b"a\nb\nc\n".to_vec());
        let log_manager = crate::db::LogManager::new(Arc::clone(&state.db), None).await;
        let mut tab = crate::ui::TabState::new(reader, log_manager, "test.log".to_string());
        tab.mark_manager.toggle(0);
        tab.mark_manager.toggle(2);
        state.tabs.push(tab);
        assert_eq!(bookmark_count(&state), 2);
    }
}
