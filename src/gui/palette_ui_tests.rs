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
fn notifications_are_dismissible_and_expire_five_seconds_after_each_event(cx: &mut TestAppContext) {
    use crate::gui::message::Message;
    use std::time::Duration;
    let (handle, app) = palette_window(cx, (1000., 800.), "github-light");
    cx.update_window(handle.into(), |_, window, cx| {
        add_log_tab(&app, cx, false);
        window.render_frame(cx);
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
    cx.background_executor.advance_clock(Duration::from_secs(4));
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, _, cx| {
        assert!(app.read(cx).state.status.is_none());
    })
    .unwrap();
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
        for (index, expected) in [(0usize, "hello"), (1, "level"), (2, "INFO")] {
            assert_eq!(window.find(("editor-input", index)).value(), Some(expected));
        }
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
        let before = app.read(cx).state.column_widths.line_no;
        let handle = window.find("resize-line-no").bounds().center();
        window.drag(handle, handle + point(px(30.), px(0.)), cx);
        assert!(app.read(cx).state.column_widths.line_no > before);
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
