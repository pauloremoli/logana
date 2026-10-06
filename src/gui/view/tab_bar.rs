use crate::gui::app::App;
use crate::gui::message::Message;
use crate::gui::state::GuiState;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, TestSupportExt};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::tab::Tab;
use gpui_kit::component::{Icon, tooltip::Tooltip};
use gpui_kit::gpui::prelude::*;
use gpui_kit::gpui::{Context, Window, div, px};

pub(crate) const TAB_BAR_HEIGHT: f32 = 40.0;
const TAB_HEIGHT: f32 = 36.0;

pub fn tab_bar(state: &GuiState, cx: &mut Context<App>) -> impl IntoElement {
    let mut bar = div()
        .flex()
        .items_center()
        .gap_1()
        .px(px(4.))
        .min_h(px(TAB_BAR_HEIGHT))
        .flex_wrap()
        .bg(cx.theme().tab)
        .overflow_hidden()
        .border_b_1()
        .border_color(cx.theme().border);
    for (idx, tab) in state.tabs.iter().enumerate() {
        bar = bar.child(tab_label(
            idx,
            tab.id,
            tab.title.clone(),
            tab.display_source_path().unwrap_or(&tab.title).to_owned(),
            idx == state.active_tab,
            cx,
        ));
    }
    bar.child(
        Button::new("open-file")
            .ghost()
            .icon(Icon::new(IconName::Plus))
            .tooltip("Open file")
            .on_click(cx.listener(|app: &mut App, _, window: &mut Window, cx| {
                app.dispatch(Message::OpenFileDialog, window, cx);
            })),
    )
    .child(super::search_bar::search_button(cx))
    .child(super::search_bar::filter_button(cx))
    .child(super::search_bar::filtering_button(state, cx))
    .child(super::search_bar::live_indicator(state, cx))
    .child(super::search_bar::view_mode_control(state, cx))
    .child(
        Button::new("open-commands")
            .ghost()
            .icon(Icon::new(IconName::SquareTerminal))
            .label("Actions")
            .tooltip("Actions (: or Ctrl+Shift+P)")
            .on_click(cx.listener(|app: &mut App, _, window, cx| {
                app.dispatch(Message::OpenCommands, window, cx)
            })),
    )
}

/// A tab chip: file icon, title, close button. The active tab is marked
/// with a background tint on the whole chip (matching the mockup) instead
/// of the TUI-era `[bracket]` convention this replaced.
fn tab_label(
    idx: usize,
    source: crate::shared::TabId,
    title: String,
    path: String,
    active: bool,
    cx: &mut Context<App>,
) -> impl IntoElement {
    let entity = cx.entity();
    div()
        .id(("file-tab", idx))
        .test_support()
        .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx))
        .context_menu(move |mut menu, _, cx| {
            use crate::gui::actions::{Action, CATALOG};
            for action in [
                Action::SaveLogs,
                Action::SaveMarked,
                Action::Export,
                Action::CopyPath,
                Action::FileInfo,
                Action::Merge,
                Action::CloseFile,
                Action::CloseOtherFiles,
                Action::CloseAllFiles,
            ] {
                let entity = entity.clone();
                let label = CATALOG.iter().find(|d| d.action == action).unwrap().label;
                let kb = crate::gui::keyboard::bindings(&entity.read(cx).state);
                let shortcut = crate::gui::keyboard::shortcut_for_action(&kb, action);
                let label = if shortcut.is_empty() {
                    label.to_owned()
                } else {
                    format!("{label}    {shortcut}")
                };
                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    entity.update(cx, |app, cx| {
                        if let Some(index) = app.state.tabs.iter().position(|tab| tab.id == source)
                        {
                            app.dispatch(Message::TabSelected(index), window, cx);
                            app.dispatch(Message::GuiAction(action), window, cx);
                        }
                    })
                }));
            }
            menu
        })
        .flex_shrink_0()
        .h(px(TAB_HEIGHT))
        .min_w(px(150.))
        .child(
            Tab::new()
                .underline()
                .selected(active)
                .prefix(Icon::new(IconName::FileText))
                .label(title)
                .on_click(
                    cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                        app.dispatch(Message::TabSelected(idx), window, cx);
                    }),
                )
                .suffix(
                    Button::new(("tab-close", idx))
                        .ghost()
                        .icon(Icon::new(IconName::X))
                        .tooltip("Close tab")
                        .on_click(
                            cx.listener(move |app: &mut App, _, window: &mut Window, cx| {
                                cx.stop_propagation();
                                app.dispatch(Message::TabClosed(idx), window, cx);
                            }),
                        ),
                ),
        )
}
