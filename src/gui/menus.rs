//! The native application menus and in-window menu bar use the same actions.
use super::{
    actions::{Action, CATALOG},
    app::{OpenCommands, Quit},
};
use gpui_kit::{
    component::GlobalState,
    gpui::{App, Menu, MenuItem},
};

#[derive(Clone, PartialEq, gpui_kit::gpui::Action)]
#[action(namespace = logana_gui, no_json)]
pub struct InvokeAction {
    pub action: Action,
}

pub fn install(cx: &mut App) {
    cx.set_menus(menus());
    GlobalState::global_mut(cx).set_app_menus(menus().into_iter().map(Menu::owned).collect());
}

fn menus() -> Vec<Menu> {
    let mut menus = Vec::new();
    for (name, actions) in [
        (
            "File",
            &[
                Action::Open,
                Action::OpenDirectory,
                Action::SwitchFile,
                Action::Merge,
                Action::SaveLogs,
                Action::SaveMarked,
                Action::Export,
                Action::CloseFile,
                Action::CloseAllFiles,
            ][..],
        ),
        ("Edit", &[Action::Undo, Action::Redo][..]),
        (
            "Filters",
            &[
                Action::ToggleFiltering,
                Action::Filter,
                Action::Group,
                Action::EnableFilters,
                Action::DisableFilters,
                Action::ClearFilters,
                Action::LoadFilters,
                Action::AppendFilters,
                Action::ImportFilters,
                Action::SaveFilters,
                Action::DefaultFilters,
            ][..],
        ),
        (
            "View",
            &[
                Action::ShowLogs,
                Action::ShowBookmarks,
                Action::MarksOnly,
                Action::Bookmark,
                Action::Annotation,
                Action::View,
                Action::Columns,
                Action::Schema,
                Action::LevelColors,
                Action::ValueColors,
                Action::Settings,
            ][..],
        ),
        (
            "Sources",
            &[
                Action::Docker,
                Action::Dlt,
                Action::Otel,
                Action::Run,
                Action::Pause,
                Action::Tail,
                Action::Stop,
                Action::EnableMcp,
                Action::DisableMcp,
            ][..],
        ),
        ("Help", &[Action::Help, Action::FileInfo][..]),
    ] {
        let mut items = actions
            .iter()
            .map(|action| {
                let descriptor = CATALOG
                    .iter()
                    .find(|d| d.action == *action)
                    .expect("menu action in catalog");
                MenuItem::action(descriptor.label, InvokeAction { action: *action })
            })
            .collect::<Vec<_>>();
        if name == "File" {
            items.extend([MenuItem::separator(), MenuItem::action("Quit", Quit)]);
        }
        if name == "Help" {
            items.push(MenuItem::action("All actions…", OpenCommands));
        }
        menus.push(Menu::new(name).items(items));
    }
    menus
}
