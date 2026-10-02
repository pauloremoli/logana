use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::{GuiState, NavPage};
use gpui_kit::assets::IconName;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

const NAV_ITEM_HEIGHT: f32 = 36.0;
// Wide enough for icon + the longest label ("Annotations") + a count
// badge to all sit on one line without truncating.
const NAV_RAIL_WIDTH: f32 = 168.0;

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
        .gap_1()
        .py(px(8.))
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
    // Icon, label, and badge all sit on one row now (not icon-above-label
    // like an earlier version) — `.truncate()` on the label (bundling
    // `overflow_hidden()`/`whitespace_nowrap()`/ellipsis) still matters:
    // without it a multi-word label like "Bookmarks" wraps onto multiple
    // lines and overlaps its neighbors, which is exactly what broke in an
    // earlier build when this was a bare text child.
    let mut item = div()
        .id(label)
        .flex()
        .items_center()
        .gap_2()
        .mx(px(4.))
        .px(px(8.))
        .rounded(px(6.))
        .h(px(NAV_ITEM_HEIGHT))
        .overflow_hidden()
        .child(Icon::new(icon))
        .child(div().flex_1().min_w(px(0.)).truncate().child(label))
        .on_click(
            cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                app.dispatch(Message::NavPageSelected(page), window, cx);
            }),
        );
    if let Some(count) = badge_count.filter(|c| *c > 0) {
        item = item.child(Badge::new().count(count));
    }
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
