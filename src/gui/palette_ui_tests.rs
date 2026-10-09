//! Native event tests: exercise the real Input, focus, hit testing and clipboard.
use super::app::{App, OpenCommands};
use super::{runtime, theme};
use crate::db::Database;
use crate::theme::Theme;
use gpui::{
    AppContext, Bounds, Entity, KeyBinding, TestAppContext, WindowBounds, WindowHandle,
    WindowOptions, point, px, size,
};
use gpui_kit::base::Root;
use gpui_kit::gpui;
use gpui_kit::test::TestWindowExt;
use std::sync::Arc;

fn palette_window(
    cx: &mut TestAppContext,
    dimensions: (f32, f32),
    name: &str,
) -> (WindowHandle<Root>, Entity<App>) {
    let db = Arc::new(runtime::handle().block_on(Database::in_memory()).unwrap());
    let theme = Theme::from_file(format!("{name}.json")).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(&theme, cx);
        cx.bind_keys([KeyBinding::new(
            "ctrl-shift-p",
            OpenCommands,
            Some("LoganaLogs"),
        )]);
        let (window, app) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(dimensions.0), px(dimensions.1)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let app = cx.new(|cx| App::new(db, theme, cx));
                app.update(cx, |app, cx| app.root_focus.focus(window, cx));
                app
            },
        )
        .unwrap();
        (window.downcast::<Root>().unwrap(), app)
    })
}
#[gpui_kit::test]
fn application_menus_dispatch_filter_view_export_and_close_actions(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 900.), "dracula");
    cx.update(|cx| add_log_tab(&app, cx, false));
    for (step, menu, item) in [
        (0, 2usize, 0usize),
        (1, 2, 0),
        (2, 3, 2),
        (3, 0, 7),
        (4, 0, 12),
    ] {
        if step == 4 {
            cx.update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                window.click("editor-cancel", cx);
            })
            .unwrap();
        }
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.within("app-menu-bar").within(menu).click("menu", cx);
            window.within("popup-menu").click(item, cx);
        })
        .unwrap();
        // Menu actions are dispatched by GPUI after the window update returns.
        cx.update(|cx| {
            let state = &app.read(cx).state;
            match step {
                0 => assert!(!state.tabs[0].filter.enabled),
                1 => assert!(state.tabs[0].filter.enabled),
                2 => assert!(state.tabs[0].filter.show_marks_only),
                3 => assert_eq!(
                    state.editor.as_ref().unwrap().action,
                    super::actions::Action::Export
                ),
                _ => assert!(state.tabs.is_empty()),
            }
        });
    }
}

#[gpui_kit::test]
fn single_line_inputs_leave_room_for_text_and_padding(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 900.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        window.render_frame(cx);
        window.click("open-search", cx);
        window.render_frame(cx);
        assert!(window.find("log-search-input").bounds().size.height >= px(36.));
        if !app.read(cx).state.search_open {
            window.click("open-search", cx);
            window.render_frame(cx);
        }
        window.click("log-search-input", cx);
        window.input("gyp Åé 東京", cx);
        assert_eq!(
            app.read(cx).log_search.as_ref().unwrap().read(cx).value(),
            "gyp Åé 東京"
        );
        app.update(cx, |app, cx| {
            app.dispatch(super::message::Message::OpenCommands, window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("command-input").bounds().size.height >= px(36.));
        app.update(cx, |app, cx| {
            app.dispatch(super::message::Message::PaletteClose, window, cx);
            app.dispatch(
                super::message::Message::GuiAction(super::actions::Action::Filter),
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert!(window.find(("editor-input", 0usize)).bounds().size.height >= px(36.));
        app.update(cx, |app, cx| {
            app.dispatch(super::message::Message::EditorCancel, window, cx);
            app.state.tabs[0].display.show_sidebar = true;
            app.dispatch(super::message::Message::FacetBrowserToggled, window, cx);
        });
        window.render_frame(cx);
        assert!(window.find("facet-search-input").bounds().size.height >= px(36.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn keybindings_editor_captures_a_new_binding_for_the_first_grouped_item(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        app.update(cx, |app, cx| {
            app.dispatch(
                super::message::Message::GuiAction(super::actions::Action::Keybindings),
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().action,
            super::actions::Action::Keybindings
        );
        window.click(("capture-binding", 0usize), cx);
        assert_eq!(app.read(cx).state.binding_capture, Some(0));
        window.press("f", cx);
        assert_eq!(app.read(cx).state.binding_capture, None);
        assert_eq!(app.read(cx).state.editor.as_ref().unwrap().values[0], "f");
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_open_without_a_file_and_save_restore_preferences(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        app.update(cx, |app, cx| {
            app.dispatch(
                super::message::Message::NavPageSelected(super::state::NavPage::Settings),
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().action,
            super::actions::Action::Settings
        );
        window.click(("restore-policy", 1usize), cx);
        window.click(("restore-policy", 5usize), cx);
        window.click("editor-apply", cx);
        assert_eq!(
            app.read(cx).state.preferences.restore_session,
            crate::config::RestoreSessionPolicy::Ask
        );
        assert_eq!(
            app.read(cx).state.preferences.restore_file,
            crate::config::RestoreSessionPolicy::Never
        );
        assert!(app.read(cx).state.editor.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn auto_filter_prefills_from_a_manual_text_selection_only(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 600.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| {
            // "plain"[1..4] == "lai" — a manual selection of part of the line.
            let mut doc = app.text_document.borrow_mut();
            doc.columns = vec!["message".into()];
            doc.selection = Some((
                crate::gui::text_selection::Endpoint {
                    line: 0,
                    row: 0,
                    column: 0,
                    offset: 1,
                },
                crate::gui::text_selection::Endpoint {
                    line: 0,
                    row: 0,
                    column: 0,
                    offset: 4,
                },
            ));
            drop(doc);
            app.dispatch(
                super::message::Message::KeyPressed(
                    crate::input::KeyCode::Char('a'),
                    crate::input::KeyModifiers::NONE,
                ),
                window,
                cx,
            );
        });
    })
    .unwrap();
    let editor = app
        .read_with(cx, |app, _| app.state.editor.clone())
        .unwrap();
    assert_eq!(editor.values[0], "lai");
}

#[gpui_kit::test]
fn auto_filter_does_not_prefill_from_the_cursor_line_without_a_selection(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 600.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| {
            app.dispatch(
                super::message::Message::KeyPressed(
                    crate::input::KeyCode::Char('a'),
                    crate::input::KeyModifiers::NONE,
                ),
                window,
                cx,
            );
        });
    })
    .unwrap();
    let editor = app
        .read_with(cx, |app, _| app.state.editor.clone())
        .unwrap();
    assert_eq!(editor.values[0], "");
}

#[gpui_kit::test]
fn actions_scroll_without_moving_background_logs(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 600.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| {
            let tab = &mut app.state.tabs[0];
            tab.file_reader = crate::ingestion::FileReader::from_bytes(b"row\n".repeat(1000));
            tab.filter.visible_indices = crate::shared::VisibleLines::All(1000);
            app.dispatch(super::message::Message::OpenCommands, window, cx);
        });
        window.render_frame(cx);
        let logs = app.read(cx).log_scroll.logical_scroll_top();
        for delta in [
            gpui::ScrollDelta::Lines(point(0., -3.)),
            gpui::ScrollDelta::Pixels(point(px(0.), px(-500.))),
        ] {
            window.scroll("command-suggestions", delta, cx);
            assert!(app.read(cx).command_scroll.offset().y < px(0.));
            let current = app.read(cx).log_scroll.logical_scroll_top();
            assert_eq!(
                (current.item_ix, current.offset_in_item),
                (logs.item_ix, logs.offset_in_item)
            );
        }
        app.update(cx, |app, cx| {
            app.dispatch(super::message::Message::PaletteClose, window, cx);
            app.dispatch(
                super::message::Message::GuiAction(super::actions::Action::Appearance),
                window,
                cx,
            );
        });
        window.render_frame(cx);
        window.scroll(
            "editor-content",
            gpui::ScrollDelta::Lines(point(0., -8.)),
            cx,
        );
        assert!(app.read(cx).command_scroll.offset().y < px(0.));
        let current = app.read(cx).log_scroll.logical_scroll_top();
        assert_eq!(
            (current.item_ix, current.offset_in_item),
            (logs.item_ix, logs.offset_in_item)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn sidebar_tabs_switch_between_filters_and_groups(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| {
            app.state.tabs[0].display.show_sidebar = true;
            cx.notify();
        });
        for (id, tab) in [
            ("sidebar-groups", super::state::SidebarTab::Groups),
            ("sidebar-filters", super::state::SidebarTab::Filters),
            ("sidebar-groups", super::state::SidebarTab::Groups),
        ] {
            window.render_frame(cx);
            window.click(id, cx);
            assert_eq!(app.read(cx).state.sidebar_tab, tab);
        }
        window.render_frame(cx);
        window.click("add-group", cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().action,
            super::actions::Action::Group
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn clicking_tab_names_switches_tabs(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (900., 600.), "dracula");
    cx.update(|cx| {
        add_log_tab(&app, cx, false);
        add_log_tab(&app, cx, false);
    });
    cx.update_window(handle.into(), |_, window, cx| {
        for index in [1usize, 0, 1] {
            window.render_frame(cx);
            window.click_at(("file-tab", index), point(px(45.), px(18.)), cx);
            assert_eq!(app.read(cx).state.active_tab, index);
        }
        window.render_frame(cx);
        window.click(("tab-close", 0usize), cx);
        assert_eq!(app.read(cx).state.tabs.len(), 1);
        assert_eq!(app.read(cx).state.active_tab, 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn view_mode_switches_between_logs_and_bookmarks(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update(|cx| add_log_tab(&app, cx, false));
    app.update(cx, |app, _| {
        let tab = &mut app.state.tabs[0];
        tab.file_reader =
            crate::ingestion::FileReader::from_bytes(b"first\nsecond\nthird\n".to_vec());
        tab.filter.visible_indices = crate::shared::VisibleLines::All(3);
        tab.filter.enabled = false;
        tab.mark_manager.mark(1);
    });
    for (item, page) in [
        (1usize, super::state::NavPage::Bookmarks),
        (0, super::state::NavPage::Logs),
    ] {
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("time-range-toggle").is_none());
            window.click("view-mode", cx);
            window.within("popup-menu").click(item, cx);
        })
        .unwrap();
        cx.update(|cx| {
            let state = &app.read(cx).state;
            assert_eq!(state.nav_page, page);
            let lines: Vec<_> = state.tabs[0].filter.visible_indices.iter().collect();
            assert_eq!(
                lines,
                if page == super::state::NavPage::Bookmarks {
                    vec![1]
                } else {
                    vec![0, 1, 2]
                }
            );
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find(("log-row", 0usize)).is_some());
            if page == super::state::NavPage::Bookmarks {
                assert!(window.try_find(("log-row", 1usize)).is_none());
                app.update(cx, |app, cx| {
                    app.dispatch(super::message::Message::SelectLog(0, false), window, cx);
                    app.dispatch(
                        super::message::Message::GuiAction(super::actions::Action::Bookmark),
                        window,
                        cx,
                    );
                    assert!(app.state.tabs[0].filter.visible_indices.is_empty());
                });
                window.render_frame(cx);
                assert!(window.try_find(("log-row", 0usize)).is_none());
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn slash_opens_search_popup_and_escape_clears_query(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| app.root_focus.focus(window, cx));
        window.render_frame(cx);
        assert!(window.try_find("log-search-input").is_none());
        assert!(window.try_find("filter-panel-toggle").is_some());
        window.press("/", cx);
        window.render_frame(cx);
        assert!(app.read(cx).state.search_open);
        assert_eq!(window.find("log-search-input").focused(), Some(true));
        window.input("plain/line", cx);
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!app.read(cx).state.search_open);
        assert!(window.try_find("log-search-input").is_none());
        assert_eq!(app.read(cx).state.tabs[0].search.query.get_pattern(), None);
        window.press("/", cx);
        window.render_frame(cx);
        assert_eq!(window.find("log-search-input").value(), Some(""));
        window.input("plain", cx);
        window.render_frame(cx);
        window.click("search-clear", cx);
        window.render_frame(cx);
        assert!(app.read(cx).state.search_open);
        assert_eq!(app.read(cx).state.tabs[0].search.query.get_pattern(), None);
        assert_eq!(window.find("log-search-input").value(), Some(""));
        window.click("log-search-input", cx);
        window.press("enter", cx);
        assert!(!app.read(cx).state.search_open);
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_clears_completed_search_after_enter(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| app.root_focus.focus(window, cx));
        window.render_frame(cx);
        window.press("/", cx);
        window.render_frame(cx);
        window.input("plain", cx);
        window.press("enter", cx);
        app.update(cx, |app, _| {
            let tab = &mut app.state.tabs[0];
            tab.cancel_search();
            tab.search
                .query
                .search("plain", [0].into_iter(), |_| Some("plain".into()))
                .unwrap();
        });
        assert!(!app.read(cx).state.search_open);
        assert_eq!(
            app.read(cx).state.tabs[0]
                .search
                .query
                .get_total_match_count(),
            1
        );
        window.press("escape", cx);
        window.render_frame(cx);
        let search = &app.read(cx).state.tabs[0].search;
        assert!(search.handle.is_none());
        assert!(search.query.get_results().is_empty());
        assert!(search.query.get_pattern().is_none());
        window.press("/", cx);
        window.render_frame(cx);
        assert_eq!(window.find("log-search-input").value(), Some(""));
    })
    .unwrap();
}

fn add_log_tab(app: &Entity<App>, cx: &mut gpui::App, structured: bool) {
    let manager = runtime::handle().block_on(crate::db::LogManager::new(
        app.read(cx).state.db.clone(),
        None,
    ));
    let reader = crate::ingestion::FileReader::from_bytes(if structured {
        br#"{"level":"INFO","message":"hello","service":"payment"}"#.to_vec()
    } else {
        b"plain\n".to_vec()
    });
    app.update(cx, |app, cx| {
        app.state.tabs.push(crate::shared::TabState::new(
            reader,
            manager,
            "test.log".into(),
        ));
        let tab = &app.state.tabs[0];
        let key = (
            tab.file_reader.line_count(),
            tab.display
                .format
                .as_ref()
                .map(|p| p.name().to_string())
                .unwrap_or_default(),
        );
        let counts = crate::shared::TabState::field_value_counts_for(
            &tab.file_reader,
            tab.display.format.as_deref(),
        );
        app.facet_counts.insert(
            tab.id,
            super::app::FacetCache {
                key,
                counts: Some(counts),
            },
        );
        cx.notify();
    });
}

#[gpui_kit::test]
fn launcher_is_native_search_and_tui_keys_do_not_execute(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("q", cx);
        window.press("j", cx);
        window.press("f", cx);
        assert!(!app.read(cx).state.palette.open);
        assert!(app.read(cx).state.editor.is_none());
        window.press("ctrl-shift-p", cx);
        assert_eq!(window.find("command-input").focused(), Some(true));
        window.input("theme", cx);
        assert_eq!(app.read(cx).state.palette.selected, 0);
        window.press("enter", cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().action,
            crate::gui::actions::Action::Appearance
        );
        window.press("ctrl-shift-p", cx);
        assert!(!app.read(cx).state.palette.open);
        window.click("editor-cancel", cx);
        assert!(!app.read(cx).state.palette.open);
        window.press("ctrl-shift-p", cx);
        assert!(app.read(cx).state.palette.open);
        window.press("escape", cx);
        assert!(!app.read(cx).state.palette.open);
    })
    .unwrap();
}
#[gpui_kit::test]
fn colon_types_in_native_inputs_and_opens_launcher_on_logs(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        window.render_frame(cx);
        if !app.read(cx).state.search_open {
            window.click("open-search", cx);
            window.render_frame(cx);
        }
        window.click("log-search-input", cx);
        window.input("value:東京", cx);
        assert!(!app.read(cx).state.palette.open);
        assert_eq!(window.find("log-search-input").value(), Some("value:東京"));
        app.update(cx, |app, cx| app.root_focus.focus(window, cx));
        window.press(":", cx);
        assert!(app.read(cx).state.palette.open);
        window.press("escape", cx);
        window.press("q", cx);
        assert!(!app.read(cx).state.palette.open);
    })
    .unwrap();
}
#[gpui_kit::test]
fn columns_are_mouse_operated_and_cancel_preserves_preferences(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        window.render_frame(cx);
        window.click("open-commands", cx);
        window.input("select-fields", cx);
        window.click(("command-suggestion", 0usize), cx);
        assert!(app.read(cx).state.editor.is_some());
        window.click("editor-none", cx);
        window.click("editor-cancel", cx);
        assert!(app.read(cx).state.tabs[0].display.hidden_fields.is_empty());
        assert!(!app.read(cx).state.palette.open);
        window.click("open-commands", cx);
        window.input("select-fields", cx);
        window.click(("command-suggestion", 0usize), cx);
        window.click("editor-none", cx);
        window.click("editor-apply", cx);
        assert!(!app.read(cx).state.tabs[0].display.hidden_fields.is_empty());
        assert!(!app.read(cx).state.palette.open);
        assert!(app.read(cx).state.editor.is_none());
    })
    .unwrap();
}
#[gpui_kit::test]
fn previous_command_output_is_absent_from_launcher_and_all_editors(cx: &mut TestAppContext) {
    use crate::gui::{actions::Action, message::Message};
    for name in ["dracula", "github-light"] {
        let (handle, app) = palette_window(cx, (1000., 900.), name);
        cx.update_window(handle.into(), |_, window, cx| {
            add_log_tab(&app, cx, false);
            app.update(cx, |app, _| {
                app.state.tabs[0].filter.visible_indices = crate::shared::VisibleLines::All(1)
            });
            app.update(cx, |app, _| {
                crate::gui::update::gui_outcome(
                    &mut app.state,
                    "Old command".into(),
                    crate::gui::palette::ExecutionState::Success,
                    "Previous output".into(),
                )
            });
            window.render_frame(cx);
            window.click("open-commands", cx);
            assert!(window.try_find("command-copy").is_none());
            assert!(window.try_find("command-clear").is_none());
            window.press("escape", cx);
            for action in [
                Action::Annotation,
                Action::Filter,
                Action::Group,
                Action::Columns,
                Action::Appearance,
                Action::ValueColors,
                Action::LevelColors,
                Action::Schema,
                Action::FileInfo,
                Action::View,
                Action::Export,
            ] {
                app.update(cx, |app, cx| {
                    app.dispatch(Message::GuiAction(action), window, cx)
                });
                window.render_frame(cx);
                assert!(app.read(cx).state.editor.is_some(), "{action:?}");
                assert!(window.try_find("editor-copy").is_none(), "{action:?}");
                assert!(window.try_find("editor-clear").is_none(), "{action:?}");
                window.press("escape", cx);
            }
        })
        .unwrap();
    }
}
#[gpui_kit::test]
fn filter_form_shows_validation_without_terminal_mode(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "github-light");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        window.render_frame(cx);
        window.click("add-filter", cx);
        window.click("editor-apply", cx);
        assert!(app.read(cx).state.editor.as_ref().unwrap().error.is_some());
        assert!(matches!(
            app.read(cx).state.tabs[0].interaction.mode.render_state(),
            crate::mode::app_mode::ModeRenderState::Normal
        ));
        window.click("editor-cancel", cx);
        assert!(app.read(cx).state.editor.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn errors_persist_until_dismissed_and_information_expires(cx: &mut TestAppContext) {
    use crate::gui::message::Message;
    use std::time::Duration;
    let (handle, app) = palette_window(cx, (1000., 800.), "github-light");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        window.render_frame(cx);
        if !app.read(cx).state.search_open {
            window.click("open-search", cx);
            window.render_frame(cx);
        }
        window.click("log-search-input", cx);
        app.update(cx, |app, cx| {
            app.dispatch(
                Message::FilterPersistenceFailed("first line\n東京".into()),
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert_eq!(window.find("log-search-input").focused(), Some(true));
        assert!(window.find("notification-popup").bounds().size.width <= px(448.));
        window.click("notification-dismiss", cx);
        assert!(app.read(cx).state.status.is_none());
        app.update(cx, |app, cx| {
            app.dispatch(
                Message::FilterPersistenceFailed("repeated".into()),
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.background_executor.advance_clock(Duration::from_secs(4));
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(app.read(cx).state.status.is_some());
        app.update(cx, |app, cx| {
            app.dispatch(
                Message::FilterPersistenceFailed("repeated".into()),
                window,
                cx,
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, _, cx| {
        assert!(app.read(cx).state.status.is_some());
    })
    .unwrap();
    cx.background_executor
        .advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(app.read(cx).state.status.is_some());
        window.render_frame(cx);
        window.click("notification-copy", cx);
        assert!(
            cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .contains("repeated")
        );
        window.click("notification-dismiss", cx);
        app.update(cx, |app, cx| {
            app.state
                .notify(crate::gui::state::StatusMessage::Info("Saved".into()));
            app.sync_notification_timer(cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.background_executor.advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    cx.update(|cx| assert!(app.read(cx).state.status.is_none()));
}

#[gpui_kit::test]
fn filter_editor_prefills_native_inputs_and_color_pickers_without_command_output(
    cx: &mut TestAppContext,
) {
    use crate::gui::{actions, message::Message};
    let (handle, app) = palette_window(cx, (1100., 1100.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        app.update(cx, |app, cx| {
            let pattern = crate::filters::encode_field_filter(
                &[("level".into(), "INFO".into())],
                Some("hello"),
            );
            app.state.tabs[0].log_manager = runtime::handle()
                .block_on(actions::save_filter(
                    app.state.tabs[0].log_manager.clone(),
                    None,
                    pattern,
                    crate::filters::FilterType::Highlight,
                    crate::filters::FilterOptions {
                        fg: Some("#123456".into()),
                        bg: Some("#654321".into()),
                        ..Default::default()
                    },
                ))
                .unwrap();
            let id = app.state.tabs[0].log_manager.get_filters()[0].id;
            crate::gui::update::gui_outcome(
                &mut app.state,
                "Old command".into(),
                crate::gui::palette::ExecutionState::Success,
                "Unrelated output".into(),
            );
            app.dispatch(Message::EditFilter(id), window, cx);
        });
        window.render_frame(cx);
        window.click("filter-custom-style", cx);
        window.render_frame(cx);
        for (index, expected) in [(0usize, "hello"), (2, "INFO")] {
            assert_eq!(window.find(("editor-input", index)).value(), Some(expected));
        }
        assert_eq!(
            app.read(cx)
                .editor_field_select
                .as_ref()
                .unwrap()
                .read(cx)
                .selected_value()
                .map(String::as_str),
            Some("level")
        );
        assert!(window.try_find(("editor-input", 1usize)).is_none());
        window.click(("filter-kind", 1usize), cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().kind,
            crate::filters::FilterType::Exclude
        );
        window.render_frame(cx);
        assert!(window.try_find(("color-reset", 0usize)).is_none());
        window.click(("filter-kind", 2usize), cx);
        window.render_frame(cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().kind,
            crate::filters::FilterType::Highlight
        );
        assert!(window.try_find("editor-copy").is_none());
        assert!(window.try_find("editor-clear").is_none());
        assert_eq!(app.read(cx).editor_colors.len(), 2);
        for (index, expected) in [(0usize, "#123456"), (1, "#654321")] {
            let color = app.read(cx).editor_colors[index].read(cx).value().unwrap();
            let color: gpui::Rgba = color.into();
            assert_eq!(
                format!(
                    "#{:02X}{:02X}{:02X}",
                    (color.r * 255.).round() as u8,
                    (color.g * 255.).round() as u8,
                    (color.b * 255.).round() as u8
                ),
                expected
            );
        }
        let picker = app.read(cx).editor_colors[0].clone();
        picker.update(cx, |picker, cx| {
            picker.select_color(gpui::rgb(0xabcdef).into(), window, cx)
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let editor = app.read(cx).state.editor.as_ref().unwrap();
        assert_eq!(
            crate::theme::parse_color(&editor.values[4]),
            Some(crate::style::Color::Rgb(0xab, 0xcd, 0xef))
        );
        app.update(cx, |app, _| {
            let effect = crate::gui::update::update(&mut app.state, Message::EditorApply);
            assert!(matches!(
                effect,
                crate::gui::effect::Effect::SaveGuiFilter { .. }
            ));
            assert!(app.state.editor.is_none());
            assert!(!app.state.palette.open);
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn filter_editor_selects_fields_and_clears_groups(cx: &mut TestAppContext) {
    use crate::gui::{actions::Action, message::Message};
    let (handle, app) = palette_window(cx, (1100., 1100.), "github-light");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        app.update(cx, |app, cx| {
            app.dispatch(Message::GuiAction(Action::Filter), window, cx);
            let editor = app.state.editor.as_mut().unwrap();
            editor.values[3] = "Existing group".into();
            editor.discovery.names = vec!["level".into(), "service".into()];
            cx.notify();
        });
        window.render_frame(cx);
        window.click("filter-advanced", cx);
        window.render_frame(cx);
        assert_eq!(
            app.read(cx)
                .editor_group_select
                .as_ref()
                .unwrap()
                .read(cx)
                .selected_value()
                .map(String::as_str),
            Some("Existing group")
        );
        window.within("filter-group-select").click("clean", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(app.read(cx).state.editor.as_ref().unwrap().values[3].is_empty());
        window.within("filter-field-select").click("input", cx);
        window.render_frame(cx);
        window.press("escape", cx);
        assert!(app.read(cx).state.editor.is_some());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.within("filter-field-select").click("input", cx);
        window.render_frame(cx);
        window.input("service", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().values[1],
            "service"
        );
        assert_eq!(
            app.read(cx)
                .editor_field_select
                .as_ref()
                .unwrap()
                .read(cx)
                .selected_value()
                .map(String::as_str),
            Some("service")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn filter_editor_timestamp_components_prefill_and_save(cx: &mut TestAppContext) {
    use crate::gui::{actions::Action, message::Message};
    use gpui_kit::component::date_picker::{DatePickerEvent, DateTime};
    let (handle, app) = palette_window(cx, (1100., 900.), "dracula");
    let original = format!(
        "{}2024-06-15 12:30:00 .. 2024-06-16 13:45:00",
        crate::filters::DATE_PREFIX
    );
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        app.update(cx, |app, cx| {
            app.dispatch(Message::GuiAction(Action::Filter), window, cx);
            app.state.editor.as_mut().unwrap().values[0] = original.clone();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("timestamp-comparison").is_some());
        assert!(window.try_find(("timestamp-date", 0usize)).is_some());
        assert!(window.try_find(("timestamp-date", 1usize)).is_some());
        assert!(window.try_find("filter-field-select").is_none());
        for (index, expected) in ["2024-06-15 12:30:00", "2024-06-16 13:45:00"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                app.read(cx).editor_dates[index]
                    .read(cx)
                    .date_time()
                    .format("%Y-%m-%d %H:%M:%S")
                    .unwrap()
                    .as_ref(),
                expected
            );
        }
        let picker = app.read(cx).editor_dates[0].clone();
        picker.update(cx, |_, cx| {
            cx.emit(DatePickerEvent::Change(DateTime::Single(
                "2024-06-15T14:00:00".parse().ok(),
            )));
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let expected = format!(
            "{}2024-06-15 14:00:00 .. 2024-06-16 13:45:00",
            crate::filters::DATE_PREFIX
        );
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().values[0],
            expected
        );
        app.update(cx, |app, _| {
            let crate::gui::effect::Effect::SaveGuiFilter { pattern, .. } =
                crate::gui::update::update(&mut app.state, Message::EditorApply)
            else {
                panic!("expected date filter save")
            };
            assert_eq!(pattern, expected);
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn filter_editor_timestamp_time_only_uses_segmented_time_control(cx: &mut TestAppContext) {
    use crate::gui::{actions::Action, message::Message};
    let (handle, app) = palette_window(cx, (900., 700.), "github-light");
    let original = format!("{}>= 09:30:00", crate::filters::DATE_PREFIX);
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        app.update(cx, |app, cx| {
            app.dispatch(Message::GuiAction(Action::Filter), window, cx);
            app.state.editor.as_mut().unwrap().values[0] = original.clone();
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find(("timestamp-time", 0usize)).is_some());
        assert!(window.try_find(("timestamp-date", 0usize)).is_none());
        assert_eq!(
            app.read(cx).editor_times[0]
                .read(cx)
                .time()
                .format("%H:%M:%S")
                .to_string(),
            "09:30:00"
        );
        window.click(("timestamp-time", 0usize), cx);
        window.press("up", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let pattern = &app.read(cx).state.editor.as_ref().unwrap().values[0];
        assert_ne!(pattern, &original);
        assert!(
            crate::filters::parse_date_filter(
                pattern.strip_prefix(crate::filters::DATE_PREFIX).unwrap()
            )
            .is_ok()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn filter_editor_common_case_is_compact_and_supports_auto_style(cx: &mut TestAppContext) {
    use crate::gui::{actions::Action, message::Message};
    let (handle, app) = palette_window(cx, (800., 650.), "github-light");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        app.update(cx, |app, cx| {
            app.dispatch(Message::GuiAction(Action::Filter), window, cx)
        });
        window.render_frame(cx);
        assert!(window.find("editor-content").bounds().size.height < px(350.));
        assert!(window.try_find("filter-field-select").is_some());
        assert!(window.try_find("filter-group-select").is_none());
        assert!(window.try_find(("color-reset", 0usize)).is_none());
        assert!(window.try_find(("filter-option", 0usize)).is_none());
        assert!(!app.read(cx).state.editor.as_ref().unwrap().auto_style);
        window.click("filter-auto-style", cx);
        assert!(app.read(cx).state.editor.as_ref().unwrap().auto_style);
        assert_eq!(app.read(cx).state.editor.as_ref().unwrap().style_seed, 1);
        window.click("filter-advanced", cx);
        window.render_frame(cx);
        assert!(window.try_find("filter-group-select").is_some());
        window.click(("filter-option", 0usize), cx);
        assert!(app.read(cx).state.editor.as_ref().unwrap().regex);
    })
    .unwrap();
}

// Keep synchronous fixtures from launching Tokio work on the deterministic GPUI scheduler.
fn cache_log_facets(app: &mut App) {
    let tab = &app.state.tabs[0];
    let key = (
        tab.file_reader.line_count(),
        tab.display
            .format
            .as_ref()
            .map(|p| p.name().to_string())
            .unwrap_or_default(),
    );
    let counts = crate::shared::TabState::field_value_counts_for(
        &tab.file_reader,
        tab.display.format.as_deref(),
    );
    app.facet_counts.insert(
        tab.id,
        super::app::FacetCache {
            key,
            counts: Some(counts),
        },
    );
}

#[gpui_kit::test]
fn log_mouse_text_selection_copies_unicode_and_preserves_row_selection(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1100., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, _| {
            app.state.tabs[0].file_reader = crate::ingestion::FileReader::from_bytes(
                "hello 東京\nsecond\nthird\n".as_bytes().to_vec(),
            );
            app.state.tabs[0].filter.visible_indices = crate::shared::VisibleLines::All(3);
            cache_log_facets(app);
        });
        window.render_frame(cx);
        let from = app.read(cx).text_document.borrow().test_point(0, 0, 6);
        let to = app.read(cx).text_document.borrow().test_point(2, 0, 3);
        window.drag(from, to, cx);
        window.render_frame(cx);
        assert_eq!(
            gpui_kit::base::TextSelection::selected_text(window, cx),
            "東京\nsecond\nthi"
        );
        assert!(
            app.read(cx).state.log_selection.is_none(),
            "drag must not select rows"
        );
        window.press("ctrl-c", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "東京\nsecond\nthi"
        );
        window.drag(to, from, cx);
        window.render_frame(cx);
        assert_eq!(
            gpui_kit::base::TextSelection::selected_text(window, cx),
            "東京\nsecond\nthi"
        );
        window.click("open-commands", cx);
        assert!(app.read(cx).text_document.borrow().selection.is_none());
        window.press("escape", cx);
        window.click(("log-row", 1usize), cx);
        let id = app.read(cx).state.tabs[0].id;
        assert_eq!(app.read(cx).state.log_selection, Some((id, 1, 1)));
        let position = app.read(cx).text_document.borrow().test_point(2, 0, 1);
        use gpui::InputEvent;
        let modifiers = gpui::Modifiers {
            shift: true,
            ..Default::default()
        };
        window.dispatch_event(
            gpui::MouseDownEvent {
                position,
                button: gpui::MouseButton::Left,
                click_count: 1,
                first_mouse: false,
                modifiers,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui::MouseUpEvent {
                position,
                button: gpui::MouseButton::Left,
                click_count: 1,
                modifiers,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(app.read(cx).state.log_selection, Some((id, 1, 2)));
        // The "#" column renders as a generic resizable field (see
        // `header_row`), so its dragged width lands in the per-format
        // `preferences.column_widths` map, not the legacy fixed
        // `state.column_widths.line_no` (which only `ColumnWidths`'s other,
        // now-unused `LineNo` variant would write to).
        let format = crate::gui::column_widths::format_key(&app.read(cx).state.tabs[0]);
        let before = app.read(cx).state.column_widths.line_no;
        let handle = window.find("resize-#").bounds().center();
        window.drag(handle, handle + point(px(30.), px(0.)), cx);
        let after = app
            .read(cx)
            .state
            .preferences
            .column_widths
            .get(&format)
            .and_then(|widths| widths.get("#"))
            .copied();
        assert!(after.is_some_and(|w| w > before));
        assert!(app.read(cx).text_document.borrow().selection.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn log_text_selection_crosses_columns_and_raw_menu_uses_clicked_row(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1100., 800.), "github-light");
    let raw = "{\"message\":\"next\",\"tenant\":\"api\",\"secret\":\"hidden\"}";
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, _| {
            let tab = &mut app.state.tabs[0];
            tab.file_reader = crate::ingestion::FileReader::from_bytes(format!("{{\"message\":\"hello world\",\"tenant\":\"pay\",\"secret\":\"hidden\"}}\n{raw}\n").into_bytes());
            tab.filter.visible_indices = crate::shared::VisibleLines::All(2);
            tab.display.format = Some(Arc::from(crate::parser::find_builtin_parser("json").unwrap()));
            tab.display.hidden_fields.insert("secret".into());
            tab.display.field_layout.columns = Some(vec!["tenant".into(), "message".into()]);
            tab.invalidate_parse_cache();
            cache_log_facets(app);
        });
        window.render_frame(cx);
        let from = app.read(cx).text_document.borrow().test_point(0, 0, 1);
        let to = app.read(cx).text_document.borrow().test_point(1, 1, 2);
        window.drag(from, to, cx);
        window.press("ctrl-c", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "ay\thello world\napi\tne");
    }).unwrap();
    for (row, item, expected) in [
        (1usize, 0, raw),
        (1, 1, "api\tnext"),
        (0, 2, "ay\thello world\napi\tne"),
    ] {
        cx.update_window(handle.into(), |_, window, cx| {
            window.right_click(("log-row", row), cx)
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("popup-menu").is_some());
            for _ in 0..=item {
                window.press("down", cx);
            }
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), expected));
    }
    cx.update_window(handle.into(), |_, window, cx| {
        app.update(cx, |app, _| app.state.tabs[0].invalidate_parse_cache());
        window.render_frame(cx);
        assert!(app.read(cx).text_document.borrow().selection.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn log_drag_autoscroll_keeps_virtualized_anchor_and_stops_on_release(cx: &mut TestAppContext) {
    use gpui::{InputEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent};
    let (handle, app) = palette_window(cx, (1000., 500.), "dracula");
    let (outside, initial_scroll) = cx
        .update_window(handle.into(), |_, window, cx| {
            add_log_tab(&app, cx, false);
            app.update(cx, |app, _| {
                app.state.tabs[0].file_reader = crate::ingestion::FileReader::from_bytes(
                    (0..150)
                        .map(|i| format!("row {i:03}\n"))
                        .collect::<String>()
                        .into_bytes(),
                );
                app.state.tabs[0].filter.visible_indices = crate::shared::VisibleLines::All(150);
                cache_log_facets(app);
            });
            window.render_frame(cx);
            let from = app.read(cx).text_document.borrow().test_point(0, 0, 2);
            let outside = point(
                from.x,
                app.read(cx).log_scroll.viewport_bounds().bottom() + px(40.),
            );
            window.dispatch_event(
                MouseMoveEvent {
                    position: from,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseDownEvent {
                    position: from,
                    button: MouseButton::Left,
                    click_count: 1,
                    first_mouse: false,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseMoveEvent {
                    position: outside,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            (
                outside,
                app.read(cx).log_scroll.scroll_px_offset_for_scrollbar().y,
            )
        })
        .unwrap();
    cx.run_until_parked();
    for _ in 0..24 {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
    let released_scroll = cx
        .update_window(handle.into(), |_, window, cx| {
            assert!(
                app.read(cx).log_scroll.scroll_px_offset_for_scrollbar().y < initial_scroll,
                "dragging beyond the viewport must scroll"
            );
            window.dispatch_event(
                MouseUpEvent {
                    position: outside,
                    button: MouseButton::Left,
                    click_count: 1,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            let document = app.read(cx).text_document.borrow();
            let (anchor, cursor) = document.selection.as_ref().unwrap();
            assert_eq!(anchor.line, 0);
            assert!(cursor.line > 10);
            drop(document);
            let selected = gpui_kit::base::TextSelection::selected_text(window, cx);
            assert!(selected.starts_with("w 000\nrow 001\n"));
            app.read(cx).log_scroll.scroll_px_offset_for_scrollbar().y
        })
        .unwrap();
    for _ in 0..8 {
        cx.background_executor
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            app.read(cx).log_scroll.scroll_px_offset_for_scrollbar().y,
            released_scroll
        );
        app.read(cx).log_scroll.scroll_to_reveal_item(100);
        window.render_frame(cx);
        assert!(
            gpui_kit::base::TextSelection::selected_text(window, cx)
                .starts_with("w 000\nrow 001\n")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn log_bookmark_background_covers_wrapped_rows_and_selection_takes_precedence(
    cx: &mut TestAppContext,
) {
    for theme_name in ["dracula", "github-light"] {
        let (handle, app) = palette_window(cx, (700., 600.), theme_name);
        cx.update_window(handle.into(), |_, window, cx| {
            add_log_tab(&app, cx, false);
            app.update(cx, |app, _| {
                let tab = &mut app.state.tabs[0];
                tab.file_reader = crate::ingestion::FileReader::from_bytes(
                    format!("first\n{}\nlast\n", "long message ".repeat(20)).into_bytes(),
                );
                tab.filter.visible_indices = crate::shared::VisibleLines::All(3);
                tab.display.show_line_numbers = false;
                tab.display.show_sidebar = false;
                tab.display.wrap = true;
                tab.mark_manager.toggle(1);
                cache_log_facets(app);
            });
            window.render_frame(cx);
            let row = window.find(("log-row", 1usize)).bounds();
            assert!(row.size.height > px(30.));
            let background =
                crate::gui::color::color_to_gpui(app.read(cx).state.theme.mark_bg).unwrap();
            let color: gpui::Hsla = background.into();
            let scale = window.scale_factor();
            let painted = window.painted_quads().into_iter().any(|q| {
                q.background.as_solid() == Some(color)
                    && (q.bounds.origin.y.0 / scale - f32::from(row.origin.y)).abs() < 2.
                    && q.bounds.size.width.0 / scale >= f32::from(row.size.width) - 2.
                    && q.bounds.size.height.0 / scale >= f32::from(row.size.height) - 2.
            });
            assert!(
                painted,
                "{theme_name}: bookmark background must span the whole wrapped row"
            );
            let from = app.read(cx).text_document.borrow().test_point(1, 0, 3);
            let to = app.read(cx).text_document.borrow().test_point(1, 0, 90);
            window.drag(from, to, cx);
            window.press("ctrl-c", cx);
            let expected = "long message ".repeat(20)[3..90].to_string();
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                expected,
                "wrapping must not insert copied newlines"
            );
            app.update(cx, |app, _| {
                let id = app.state.tabs[0].id;
                app.state.log_selection = Some((id, 1, 1));
            });
            window.render_frame(cx);
            assert!(
                !window.painted_quads().into_iter().any(|q| {
                    q.background.as_solid() == Some(color)
                        && (q.bounds.origin.y.0 / scale - f32::from(row.origin.y)).abs() < 2.
                }),
                "row selection must take precedence over bookmark background"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn bookmark_annotation_shortcuts_and_enter_are_scoped_to_their_interfaces(cx: &mut TestAppContext) {
    use crate::gui::{actions::Action, state::NavPage};
    let (handle, app) = palette_window(cx, (1100., 900.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, _| {
            app.state.tabs[0].filter.visible_indices = crate::shared::VisibleLines::All(1)
        });
        window.render_frame(cx);
        window.click(("log-row", 0usize), cx);
        window.press("ctrl-b", cx);
        let line = app.read(cx).state.tabs[0].filter.visible_indices.get(0);
        assert!(app.read(cx).state.tabs[0].mark_manager.is_marked(line));
        window.press("ctrl-shift-b", cx);
        assert_eq!(app.read(cx).state.nav_page, NavPage::Bookmarks);
        window.press("ctrl-shift-b", cx);
        assert_eq!(app.read(cx).state.nav_page, NavPage::Logs);
        window.press("ctrl-shift-a", cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().action,
            Action::Annotation
        );
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().members[0].0,
            line
        );
        window.press("ctrl-b", cx);
        assert!(app.read(cx).state.tabs[0].mark_manager.is_marked(line));
        window.press("escape", cx);
        if !app.read(cx).state.search_open {
            window.click("open-search", cx);
            window.render_frame(cx);
        }
        window.click("log-search-input", cx);
        window.press("ctrl-b", cx);
        assert!(app.read(cx).state.tabs[0].mark_manager.is_marked(line));
        app.update(cx, |app, cx| app.root_focus.focus(window, cx));
        window.press("ctrl-shift-p", cx);
        window.input("bookmarks", cx);
        assert_eq!(app.read(cx).state.palette.selected, 0);
        window.press("down", cx);
        assert!(app.read(cx).state.palette.selected > 0);
        window.press("up", cx);
        assert_eq!(app.read(cx).state.palette.selected, 0);
        window.press("enter", cx);
        assert_eq!(app.read(cx).state.nav_page, NavPage::Bookmarks);
        assert!(!app.read(cx).state.palette.open);
        window.press("ctrl-shift-b", cx);
        window.right_click(("log-row", 0usize), cx);
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        let line = app.read(cx).state.tabs[0].filter.visible_indices.get(0);
        window.click(3usize, cx);
        assert!(!app.read(cx).state.tabs[0].mark_manager.is_marked(line));
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.right_click(("log-row", 0usize), cx);
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        let line = app.read(cx).state.tabs[0].filter.visible_indices.get(0);
        window.click(4usize, cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().action,
            Action::Annotation
        );
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().members[0].0,
            line
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn minimal_filter_sidebar_hides_fields_and_opens_prefilled_conditions(cx: &mut TestAppContext) {
    use crate::filters::{FilterOptions, FilterType};
    use crate::gui::state::NavPage;
    for name in ["dracula", "github-light"] {
        let (handle, app) = palette_window(cx, (800., 650.), name);
        let mut id = 0;
        cx.update_window(handle.into(), |_, window, cx| {
            add_log_tab(&app, cx, true);
            app.update(cx, |app, cx| {
                runtime::handle().block_on(app.state.tabs[0].log_manager.add_filter_with_color(
                    "payment".into(),
                    FilterType::Include,
                    FilterOptions::default(),
                ));
                id = app.state.tabs[0].log_manager.get_filters()[0].id;
                app.state.nav_page = NavPage::Logs;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.try_find("facet-search-input").is_none());
            assert!(window.try_find("clear-all-filters").is_none());
            assert!(window.try_find(("filter-remove", id)).is_some());
            assert!(window.try_find(("filter-edit", id)).is_some());
            window.click("filter-menu", cx);
        })
        .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.within("popup-menu").click(0usize, cx);
        })
        .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(app.read(cx).state.facet_browser_open);
            assert!(window.try_find("facet-search-input").is_some());
            window.click("close-field-browser", cx);
            window.render_frame(cx);
            assert!(!app.read(cx).state.facet_browser_open);
            assert!(window.try_find("facet-search-input").is_none());
            window.click(("filter-condition", id), cx);
            assert_eq!(
                app.read(cx).state.editor.as_ref().unwrap().values[0],
                "payment"
            );
            window.press("escape", cx);
            window.click(("filter-edit", id), cx);
            assert_eq!(
                app.read(cx).state.editor.as_ref().unwrap().values[0],
                "payment"
            );
            window.press("escape", cx);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn filter_sidebar_keyboard_navigation_scrolls_selection_into_view(cx: &mut TestAppContext) {
    use crate::filters::{FilterOptions, FilterType};
    use crate::input::{KeyCode, KeyModifiers};
    let (handle, app) = palette_window(cx, (1000., 500.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| {
            for i in 0..40 {
                runtime::handle().block_on(app.state.tabs[0].log_manager.add_filter_with_color(
                    format!("pattern{i}"),
                    FilterType::Highlight,
                    FilterOptions::default(),
                ));
            }
            app.state.tabs[0].display.show_sidebar = true;
            cx.notify();
        });
        window.render_frame(cx);
        app.update(cx, |app, cx| {
            app.dispatch(
                super::message::Message::KeyPressed(KeyCode::Char('f'), KeyModifiers::NONE),
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert_eq!(app.read(cx).state.sidebar_cursor, 0);
        assert_eq!(app.read(cx).filter_sidebar_scroll.offset().y, px(0.));
        app.update(cx, |app, cx| {
            for _ in 0..30 {
                app.dispatch(
                    super::message::Message::KeyPressed(KeyCode::Char('j'), KeyModifiers::NONE),
                    window,
                    cx,
                );
            }
        });
        window.render_frame(cx);
        assert_eq!(app.read(cx).state.sidebar_cursor, 30);
        assert!(
            app.read(cx).filter_sidebar_scroll.offset().y < px(0.),
            "selecting a filter far down the list must scroll it into view"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn annotations_are_visible_in_logs_and_bookmark_their_lines(cx: &mut TestAppContext) {
    use crate::gui::{actions::Action, message::Message, state::NavPage};
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| {
            app.state.tabs[0].filter.visible_indices = crate::shared::VisibleLines::All(1);
            app.dispatch(Message::GuiAction(Action::Annotation), window, cx);
            app.state.editor.as_mut().unwrap().values[0] =
                "Investigate shutdown\nFollow up tomorrow".into();
            app.dispatch(Message::EditorApply, window, cx);
        });
        window.render_frame(cx);
        assert_eq!(app.read(cx).state.nav_page, NavPage::Logs);
        assert!(app.read(cx).state.tabs[0].mark_manager.is_marked(0));
        assert!(window.find(("log-row", 0usize)).bounds().size.height > px(30.));
        window
            .within(("log-row", 0usize))
            .click(("log-annotation", 0usize), cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().values[0],
            "Investigate shutdown\nFollow up tomorrow"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn merge_picker_has_native_selection_search_and_cancel_controls(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        let manager = runtime::handle().block_on(crate::db::LogManager::new(
            app.read(cx).state.db.clone(),
            None,
        ));
        app.update(cx, |app, cx| {
            app.state.tabs.push(crate::shared::TabState::new(
                crate::ingestion::FileReader::from_bytes(
                    b"Jan  1 00:00:01 host tag: second\n".to_vec(),
                ),
                manager,
                "second.log".into(),
            ));
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("merge-files").is_none());
        window.click("open-commands", cx);
        window.input("merge", cx);
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().action,
            super::actions::Action::Merge
        );
        window.click("merge-all", cx);
        assert!(
            app.read(cx)
                .state
                .editor
                .as_ref()
                .unwrap()
                .merge_sources
                .iter()
                .all(|(_, on)| *on)
        );
        window.click("merge-none", cx);
        assert!(
            app.read(cx)
                .state
                .editor
                .as_ref()
                .unwrap()
                .merge_sources
                .iter()
                .all(|(_, on)| !*on)
        );
        window.click(("merge-source", 0usize), cx);
        assert!(app.read(cx).state.editor.as_ref().unwrap().merge_sources[0].1);
        window.click(("editor-input", 0usize), cx);
        window.input("second", cx);
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("merge-source", 0usize)).is_none());
        assert!(window.try_find(("merge-source", 1usize)).is_some());
        window.press("escape", cx);
        assert!(app.read(cx).state.editor.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn facet_counts_finish_on_the_gpui_executor(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        app.update(cx, |app, cx| {
            app.state.tabs[0].display.format = Some(Arc::from(
                crate::parser::find_builtin_parser("json").unwrap(),
            ));
            app.facet_counts.clear();
            cx.notify();
        });
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let app = app.read(cx);
        let id = app.state.tabs[0].id;
        let counts = app.facet_counts[&id]
            .counts
            .as_ref()
            .expect("facet worker completed");
        assert_eq!(counts.counts["level"], vec![("INFO".into(), 1)]);
    });
}

#[gpui_kit::test]
fn font_size_shortcuts_scale_logs_and_preserve_input(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        window.render_frame(cx);
        let original = f32::from(window.rem_size());
        let row_height = window.find(("log-row", 0usize)).bounds().size.height;
        window.press("ctrl-=", cx);
        window.render_frame(cx);
        assert_eq!(f32::from(window.rem_size()), original + 1.);
        assert!(window.find(("log-row", 0usize)).bounds().size.height > row_height);
        window.press("ctrl--", cx);
        window.render_frame(cx);
        assert_eq!(f32::from(window.rem_size()), original);
        app.update(cx, |app, cx| {
            app.dispatch(crate::gui::message::Message::SearchPopup(true), window, cx)
        });
        window.render_frame(cx);
        window.press("-", cx);
        assert_eq!(f32::from(window.rem_size()), original);
        assert_eq!(
            app.read(cx)
                .log_search
                .as_ref()
                .unwrap()
                .read(cx)
                .value()
                .as_str(),
            "-"
        );
        window.press("ctrl-=", cx);
        window.render_frame(cx);
        assert_eq!(f32::from(window.rem_size()), original + 1.);
        gpui_kit::component::theme::Theme::update(cx, |theme| theme.font_size = px(32.));
        window.render_frame(cx);
        window.press("ctrl-=", cx);
        assert_eq!(f32::from(window.rem_size()), 32.);
        gpui_kit::component::theme::Theme::update(cx, |theme| theme.font_size = px(10.));
        window.render_frame(cx);
        window.press("ctrl--", cx);
        assert_eq!(f32::from(window.rem_size()), 10.);
    })
    .unwrap();
}

#[gpui_kit::test]
fn font_size_is_saved_and_restored_in_a_new_window(cx: &mut TestAppContext) {
    use crate::gui::persistence::Preferences;
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    let db = app.read_with(cx, |app, _| app.state.db.clone());
    let flushed = cx
        .update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-=", cx);
            window.press("ctrl-=", cx);
            app.update(cx, |app, _| app.flush_persistence())
        })
        .unwrap();
    runtime::handle().block_on(flushed).unwrap();
    let preferences = runtime::handle()
        .block_on(Preferences::load(&db, &crate::config::Config::default()))
        .unwrap();
    assert_eq!(preferences.font_size, 15.);
    let restored = cx.update(|cx| {
        let source = Theme::from_file("dracula.json").unwrap();
        theme::install(&source, cx);
        gpui_kit::open_window(Default::default(), cx, |window, cx| {
            let app = cx.new(|cx| App::new(db, source, cx));
            app.update(cx, |app, cx| {
                app.restore_session(preferences, vec![], window, cx)
            });
            app
        })
        .unwrap()
        .0
    });
    cx.update_window(restored, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(f32::from(window.rem_size()), 15.);
    })
    .unwrap();
}

#[gpui_kit::test]
fn narrow_workspace_retains_log_canvas_with_sidebar_open(cx: &mut TestAppContext) {
    for width in [600., 800., 1200.] {
        let (handle, app) = palette_window(cx, (width, 700.), "dracula");
        cx.update_window(handle.into(), |_, window, cx| {
            add_log_tab(&app, cx, false);
            app.update(cx, |app, _| app.state.tabs[0].display.show_sidebar = true);
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.find("log-workspace").bounds().size.width >= px(280.));
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn file_switcher_searches_source_paths_and_selects_stable_tab(cx: &mut TestAppContext) {
    use super::actions::Action;
    use super::message::Message;
    let (handle, app) = palette_window(cx, (1000., 900.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| {
            app.state.tabs[0].source_path = Some("/services/api/server.log".into());
            app.state.tabs[1].source_path = Some("/services/worker/server.log".into());
            app.dispatch(Message::GuiAction(Action::SwitchFile), window, cx);
            app.dispatch(Message::EditorText(0, "worker".into()), window, cx);
        });
        window.render_frame(cx);
        window.click(("switch-file", 1usize), cx);
        assert_eq!(app.read(cx).state.active_tab, 1);
        assert!(app.read(cx).state.editor.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn saved_device_search_and_otel_transport_controls_update_drafts(cx: &mut TestAppContext) {
    use super::actions::Action;
    use super::message::Message;
    let (handle, app) = palette_window(cx, (1000., 900.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        app.update(cx, |app, cx| {
            app.dispatch(Message::GuiAction(Action::Dlt), window, cx);
            let editor = app.state.editor.as_mut().unwrap();
            editor.members = vec![(0, "Gateway".into(), false), (1, "Worker".into(), false)];
            editor.conditions = vec![
                ("gateway.local".into(), "3490".into()),
                ("worker.local".into(), "3491".into()),
            ];
            app.dispatch(Message::EditorText(2, "worker".into()), window, cx);
        });
        window.render_frame(cx);
        window.click(("dlt-device", 1usize), cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().values[0],
            "worker.local"
        );
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().values[1],
            "3491"
        );
        app.update(cx, |app, cx| {
            app.dispatch(Message::EditorCancel, window, cx);
            app.dispatch(Message::GuiAction(Action::Otel), window, cx);
        });
        window.render_frame(cx);
        window.click("otel-grpc", cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().values[1],
            "grpc"
        );
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().values[0],
            "4317"
        );
        window.click("otel-http", cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().values[0],
            "4318"
        );
        app.update(cx, |app, cx| {
            app.dispatch(Message::EditorText(0, "9999".into()), window, cx);
            app.dispatch(Message::EditorText(1, "grpc".into()), window, cx);
        });
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().values[0],
            "9999",
            "a manually chosen port must not be clobbered by a transport switch"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn schema_and_export_render_searchable_choice_controls(cx: &mut TestAppContext) {
    use super::actions::Action;
    use super::message::Message;
    let (handle, app) = palette_window(cx, (1000., 900.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        for (action, id) in [
            (Action::Schema, "schema-select"),
            (Action::Export, "export-template-select"),
        ] {
            app.update(cx, |app, cx| {
                app.dispatch(Message::GuiAction(action), window, cx);
            });
            window.render_frame(cx);
            assert!(app.read(cx).editor_choice_select.is_some());
            window.within(id).click("input", cx);
            window.render_frame(cx);
            window.press("escape", cx);
            assert!(app.read(cx).state.editor.is_some());
            app.update(cx, |app, cx| {
                app.dispatch(Message::EditorCancel, window, cx)
            });
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn hidden_note_row_can_be_revealed_without_filters(cx: &mut TestAppContext) {
    use super::{actions::Action, message::Message, state::NavPage};
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| {
            app.dispatch(Message::GuiAction(Action::Annotation), window, cx);
            app.dispatch(
                Message::EditorText(0, "Investigate this row".into()),
                window,
                cx,
            );
            app.dispatch(Message::EditorApply, window, cx);
            app.state.tabs[0].filter.visible_indices = crate::shared::VisibleLines::All(0);
            app.state.nav_page = NavPage::Annotations;
        });
        window.render_frame(cx);
        window.click(("annotation-jump", 0usize), cx);
        window.render_frame(cx);
        assert!(!app.read(cx).state.tabs[0].filter.enabled);
        assert_eq!(app.read(cx).state.nav_page, NavPage::Logs);
        assert!(app.read(cx).state.log_selection.is_some());
        assert!(app.read(cx).pending_source_jump.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn perf_probe_click_select_row(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1200., 900.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        let total_lines = 2000;
        let num_filters = 30;
        let mut manager = runtime::handle().block_on(crate::db::LogManager::new(
            app.read(cx).state.db.clone(),
            None,
        ));
        runtime::handle().block_on(async {
            for i in 0..num_filters {
                manager
                    .add_filter_with_color(
                        format!("line {}", i * 7),
                        crate::filters::FilterType::Include,
                        crate::filters::FilterOptions::default(),
                    )
                    .await;
            }
        });
        let mut bytes = Vec::new();
        for i in 0..total_lines {
            bytes.extend_from_slice(format!("2024-01-01 INFO line {i}\n").as_bytes());
        }
        let reader = crate::ingestion::FileReader::from_bytes(bytes);
        app.update(cx, |app, cx| {
            let mut tab = crate::shared::TabState::new(reader, manager, "perf.log".into());
            let _runtime = runtime::handle().enter();
            tab.begin_filter_refresh();
            app.state.tabs.push(tab);
            app.root_focus.focus(window, cx);
        });
        window.render_frame(cx);
        // Drain the async filter computation the same way the 16ms poll loop would.
        for _ in 0..500 {
            let done = app.update(cx, |app, _| {
                let tab = &mut app.state.tabs[0];
                if tab.filter.handle.is_some() {
                    tab.advance_filter_computation();
                }
                tab.filter.handle.is_none()
            });
            window.render_frame(cx);
            if done {
                break;
            }
        }
        eprintln!(
            "PERF_PROBE visible_after_filters={}",
            app.read(cx).state.tabs[0].filter.visible_indices.len()
        );
        let n = 50;
        let start = std::time::Instant::now();
        for i in 0..n {
            app.update(cx, |app, cx| {
                app.dispatch(
                    super::message::Message::SelectLog(i % 50, false),
                    window,
                    cx,
                );
            });
            window.render_frame(cx);
        }
        let elapsed = start.elapsed();
        eprintln!(
            "PERF_PROBE total={:?} per_click={:?}",
            elapsed,
            elapsed / n as u32
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn notes_search_preserves_source_row_actions(cx: &mut TestAppContext) {
    use super::state::NavPage;
    let (handle, app) = palette_window(cx, (1000., 800.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        app.update(cx, |app, cx| {
            app.state.tabs[0]
                .comment_manager
                .add("First note".into(), vec![0]);
            app.state.tabs[0]
                .comment_manager
                .add("Second note".into(), vec![0]);
            app.state.nav_page = NavPage::Annotations;
            cx.notify();
        });
        window.render_frame(cx);
        window.click("notes-search", cx);
        window.input("Second", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(("annotation-row", 0usize)).is_none());
        assert!(window.try_find(("annotation-row", 1usize)).is_some());
        window.click(("annotation-edit", 1usize), cx);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().values[0],
            "Second note"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn field_facets_page_values_and_search_every_page(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1100., 1100.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        app.update(cx, |app, cx| {
            app.state.facet_browser_open = true;
            let id = app.state.tabs[0].id;
            let cache = app.facet_counts.get_mut(&id).unwrap();
            cache.counts = Some(crate::shared::FieldValueCounts {
                names: vec!["service".into()],
                counts: std::collections::HashMap::from([(
                    "service".into(),
                    (0..100).map(|i| (format!("value{i:03}"), 1)).collect(),
                )]),
            });
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("facet-next:service").is_some());
        assert!(window.try_find("facet-value:service:value000").is_some());
        assert!(window.try_find("facet-value:service:value040").is_none());
        window.click("facet-next:service", cx);
        window.render_frame(cx);
        assert!(window.try_find("facet-value:service:value000").is_none());
        assert!(window.try_find("facet-value:service:value040").is_some());
        window.click("facet-previous:service", cx);
        window.render_frame(cx);
        assert!(window.try_find("facet-value:service:value000").is_some());
        window.click("facet-next:service", cx);
        window.render_frame(cx);
        window.click("facet-next:service", cx);
        window.render_frame(cx);
        assert!(window.try_find("facet-value:service:value080").is_some());

        window.click("facet-search-input", cx);
        window.input("value099", cx);
        window.render_frame(cx);
        assert!(window.try_find("facet-value:service:value099").is_some());
        assert!(window.try_find("facet-next:service").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn columns_drag_reorders_draft_and_cancel_restores_order(cx: &mut TestAppContext) {
    let (handle, app) = palette_window(cx, (1000., 900.), "dracula");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, true);
        app.update(cx, |app, cx| {
            app.dispatch(
                crate::gui::message::Message::GuiAction(crate::gui::actions::Action::Columns),
                window,
                cx,
            );
        });
        window.render_frame(cx);
        let initial = app.read(cx).state.editor.as_ref().unwrap().fields.clone();
        assert!(initial.len() >= 3);
        window.drag_to(("column-drag", 0usize), ("column-row", 2usize), cx);
        let mut reordered = initial.clone();
        let first = reordered.remove(0);
        reordered.insert(2, first);
        assert_eq!(
            app.read(cx).state.editor.as_ref().unwrap().fields,
            reordered
        );
        window.render_frame(cx);
        window.drag_to(("column-drag", 2usize), ("column-row", 0usize), cx);
        assert_eq!(app.read(cx).state.editor.as_ref().unwrap().fields, initial);
        window.render_frame(cx);
        window.drag_to(("column-drag", 0usize), ("column-row", 2usize), cx);

        window.click("editor-cancel", cx);
        app.update(cx, |app, cx| {
            app.dispatch(
                crate::gui::message::Message::GuiAction(crate::gui::actions::Action::Columns),
                window,
                cx,
            );
        });
        assert_eq!(app.read(cx).state.editor.as_ref().unwrap().fields, initial);
    })
    .unwrap();
}
