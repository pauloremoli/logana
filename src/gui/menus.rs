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

#[derive(Clone, PartialEq, gpui_kit::gpui::Action)]
#[action(namespace = logana_gui, no_json)]
pub struct OpenRecent {
    pub path: std::path::PathBuf,
}

const MENU_ACTIONS: &[(&str, &[Action])] = &[
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
            Action::CopyPath,
            Action::CloseOtherFiles,
            Action::CloseFile,
            Action::CloseAllFiles,
        ],
    ),
    ("Edit", &[Action::Undo, Action::Redo]),
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
        ],
    ),
    (
        "View",
        &[
            Action::ShowLogs,
            Action::ShowBookmarks,
            Action::Bookmark,
            Action::Annotation,
            Action::View,
            Action::Columns,
            Action::CollapseMessages,
            Action::ExpandMessages,
            Action::Schema,
            Action::LevelColors,
            Action::ValueColors,
            Action::Settings,
        ],
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
        ],
    ),
    ("Help", &[Action::Help, Action::FileInfo, Action::About]),
];

pub fn install(cx: &mut App, recent: &[std::path::PathBuf]) {
    cx.set_menus(menus(recent));
    GlobalState::global_mut(cx).set_app_menus(menus(recent).into_iter().map(Menu::owned).collect());
}

fn menus(recent: &[std::path::PathBuf]) -> Vec<Menu> {
    let mut menus = Vec::new();
    let kb = crate::config::Config::load()
        .unwrap_or_default()
        .keybindings;
    for &(name, actions) in MENU_ACTIONS {
        let mut items = Vec::new();
        for action in actions {
            if matches!(
                action,
                Action::SaveLogs
                    | Action::CopyPath
                    | Action::CloseOtherFiles
                    | Action::Filter
                    | Action::EnableFilters
                    | Action::LoadFilters
                    | Action::DefaultFilters
                    | Action::Bookmark
                    | Action::View
                    | Action::LevelColors
                    | Action::Settings
                    | Action::Pause
                    | Action::EnableMcp
            ) {
                items.push(MenuItem::separator());
            }
            let descriptor = CATALOG
                .iter()
                .find(|d| d.action == *action)
                .expect("menu action in catalog");
            let shortcut = super::keyboard::shortcut_for_action(&kb, *action);
            let label = if shortcut.is_empty() {
                descriptor.label.to_owned()
            } else {
                format!("{}    {}", descriptor.label, shortcut)
            };
            items.push(MenuItem::action(label, InvokeAction { action: *action }));
            if *action == Action::Open {
                items.push(MenuItem::submenu(Menu::new("Open Recent").items(
                    recent.iter().take(5).map(|path| {
                        MenuItem::action(
                            path.display().to_string(),
                            OpenRecent { path: path.clone() },
                        )
                    }),
                )));
            }
        }
        if name == "File" {
            items.extend([MenuItem::separator(), MenuItem::action("Quit", Quit)]);
        }
        if name == "Help" {
            items.extend([
                MenuItem::separator(),
                MenuItem::action("All actions…", OpenCommands),
            ]);
        }
        menus.push(Menu::new(name).items(items));
    }
    menus
}

impl super::app::App {
    pub(super) fn refresh_menus(&mut self, cx: &mut gpui_kit::gpui::Context<Self>) {
        install(cx, &self.state.preferences.recent_files);
        self.menu_bar.update(cx, |menu, cx| menu.reload(cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_view_has_one_menu_entry_and_a_shortcut() {
        let (_, actions) = MENU_ACTIONS
            .iter()
            .find(|(name, _)| *name == "View")
            .unwrap();
        assert!(actions.contains(&Action::ShowBookmarks));
        assert!(!actions.contains(&Action::MarksOnly));
        assert_eq!(
            crate::gui::keyboard::shortcut_for_action(
                &crate::config::Keybindings::default(),
                Action::ShowBookmarks,
            ),
            "Ctrl+Shift+B",
        );
    }
}
