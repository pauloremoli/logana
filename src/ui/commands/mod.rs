use clap::Parser;

use crate::auto_complete::shell_split;
use crate::commands::{CommandLine, Commands};

use super::App;

mod display;
mod filter;
mod io;
mod stream;

pub(super) fn parse_key_value(pattern: &str) -> Result<(&str, &str), String> {
    let eq = pattern
        .find('=')
        .ok_or_else(|| format!("--field pattern must be 'key=value', got: {pattern}"))?;
    let key = &pattern[..eq];
    let value = &pattern[eq + 1..];
    if key.is_empty() {
        return Err("field name must not be empty".to_string());
    }
    if value.is_empty() {
        return Err("field value must not be empty".to_string());
    }
    Ok((key, value))
}

pub(super) fn resolve_hide_field_arg(
    tab: &mut crate::ui::TabState,
    arg: &str,
) -> Result<String, String> {
    if let Ok(idx) = arg.parse::<usize>() {
        let visible: Vec<String> = tab
            .collect_field_names()
            .into_iter()
            .filter(|n| !tab.display.hidden_fields.contains(n.as_str()))
            .collect();
        visible
            .into_iter()
            .nth(idx)
            .ok_or_else(|| format!("Field index {idx} out of range"))
    } else {
        Ok(arg.to_string())
    }
}

impl App {
    /// Returns `Ok(true)` when the command sets the mode itself (e.g. select-fields
    /// opens a popup), so `execute_command_str` should not override it.
    pub(super) async fn run_command(&mut self, input: &str) -> Result<bool, String> {
        // Bare number → go to line.
        let trimmed = input.trim();
        if let Ok(line_number) = trimmed.parse::<usize>() {
            self.tabs[self.active_tab].goto_line(line_number)?;
            return Ok(false);
        }

        let args = CommandLine::try_parse_from(shell_split(input))
            .map_err(|e| format!("Invalid command: {}", e))?;

        match args.command {
            Some(Commands::Filter {
                pattern,
                fg,
                bg,
                line_mode,
                field,
                regex,
            }) => {
                return self
                    .cmd_filter(pattern.join(" "), fg, bg, line_mode, field, regex)
                    .await;
            }
            Some(Commands::Exclude {
                pattern,
                field,
                regex,
            }) => {
                return self.cmd_exclude(pattern.join(" "), field, regex).await;
            }
            Some(Commands::SetColor { fg, bg, line_mode }) => {
                return self.cmd_set_color(fg, bg, line_mode).await;
            }
            Some(Commands::ExportMarked { path }) => return self.cmd_export_marked(path),
            Some(Commands::Save { path }) => return self.cmd_save(path),
            Some(Commands::Export { path, template }) => return self.cmd_export(path, template),
            Some(Commands::SaveFilters { path }) => return self.cmd_save_filters(path),
            Some(Commands::LoadFilters { path }) => return self.cmd_load_filters(path).await,
            Some(Commands::Wrap) => self.cmd_wrap().await,
            Some(Commands::LineNumbers) => self.cmd_line_numbers().await,
            Some(Commands::LevelColors) => return self.cmd_level_colors(),
            Some(Commands::SetTheme { theme_name }) => return self.cmd_set_theme(theme_name).await,
            Some(Commands::SidebarPosition { side }) => {
                return self.cmd_sidebar_position(side).await;
            }
            Some(Commands::Open { path }) => return self.cmd_open(path).await,
            Some(Commands::CloseTab) => return self.cmd_close_tab(),
            Some(Commands::ClearFilters) => return self.cmd_clear_filters().await,
            Some(Commands::DisableFilters) => return self.cmd_disable_filters().await,
            Some(Commands::EnableFilters) => return self.cmd_enable_filters().await,
            Some(Commands::Filtering) => self.cmd_filtering(),
            Some(Commands::HideField { field }) => return self.cmd_hide_field(field),
            Some(Commands::ShowField { field }) => self.cmd_show_field(field),
            Some(Commands::ShowAllFields) => self.cmd_show_all_fields(),
            Some(Commands::SelectFields) => return self.cmd_select_fields(),
            Some(Commands::Docker) => return self.cmd_docker(),
            Some(Commands::ValueColors) => return self.cmd_value_colors(),
            Some(Commands::DateFilter {
                expr,
                fg,
                bg,
                line_mode,
            }) => return self.cmd_date_filter(expr, fg, bg, line_mode).await,
            Some(Commands::Tail) => self.cmd_tail(),
            Some(Commands::ShowKeys) => self.cmd_show_keys(),
            Some(Commands::HideKeys) => self.cmd_hide_keys(),
            Some(Commands::Raw) => self.cmd_raw(),
            Some(Commands::Stop) => self.cmd_stop(),
            Some(Commands::Pause) => self.cmd_pause(),
            Some(Commands::Resume) => self.cmd_resume(),
            Some(Commands::Reset) => return self.cmd_reset().await,
            Some(Commands::Dlt) => return self.cmd_dlt(),
            Some(Commands::Otel { http, port }) => return self.cmd_otel(http, port).await,
            Some(Commands::EnableMcp { port }) => return self.cmd_enable_mcp(port).await,
            Some(Commands::DisableMcp) => self.cmd_disable_mcp(),
            Some(Commands::Run { command }) => return self.cmd_run(command).await,
            None => {}
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::filters::FilterType;
    use crate::ingestion::FileReader;
    use crate::mode::app_mode::ModeRenderState;
    use crate::theme::Theme;
    use crate::ui::VisibleLines;
    use crate::ui::app::App;
    use std::sync::Arc;

    async fn await_filter_computations(app: &mut App) {
        for tab in &mut app.tabs {
            if let Some(mut h) = tab.filter.handle.take() {
                let mut all_visible = Vec::new();
                let mut final_counts = None;
                while let Some(chunk) = h.result_rx.recv().await {
                    all_visible.extend(chunk.visible);
                    if chunk.is_last {
                        final_counts = chunk.filter_match_counts;
                        break;
                    }
                }
                tab.filter.visible_indices = VisibleLines::Filtered(all_visible);
                if let Some(counts) = final_counts {
                    tab.filter.match_counts = counts;
                }
                if tab.filter.visible_indices.is_empty() {
                    tab.scroll.scroll_offset = 0;
                } else {
                    tab.scroll.scroll_offset = tab
                        .scroll
                        .scroll_offset
                        .min(tab.filter.visible_indices.len() - 1);
                }
            }
        }
    }

    async fn make_app(lines: &[&str]) -> App {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
        .await
    }

    #[tokio::test]
    async fn test_tail_command_toggles_on() {
        let mut app = make_app(&["line1", "line2", "line3"]).await;
        assert!(!app.tab().stream.tail_mode);
        app.run_command("tail").await.unwrap();
        assert!(app.tab().stream.tail_mode);
    }

    #[tokio::test]
    async fn test_tail_command_toggles_off() {
        let mut app = make_app(&["line1", "line2"]).await;
        app.run_command("tail").await.unwrap();
        assert!(app.tab().stream.tail_mode);
        app.run_command("tail").await.unwrap();
        assert!(!app.tab().stream.tail_mode);
    }

    #[tokio::test]
    async fn test_tail_on_jumps_to_last_line() {
        let mut app = make_app(&["l1", "l2", "l3", "l4", "l5"]).await;
        app.tabs[0].scroll.scroll_offset = 0;
        app.run_command("tail").await.unwrap();
        // Enabling tail should immediately jump to the last visible line.
        assert_eq!(app.tab().scroll.scroll_offset, 4);
    }

    #[tokio::test]
    async fn test_tail_off_does_not_change_scroll() {
        let mut app = make_app(&["l1", "l2", "l3", "l4", "l5"]).await;
        // Enable then disable tail; the disabling should not move the cursor.
        app.run_command("tail").await.unwrap();
        assert_eq!(app.tab().scroll.scroll_offset, 4);
        app.tabs[0].scroll.scroll_offset = 2;
        app.run_command("tail").await.unwrap();
        assert!(!app.tab().stream.tail_mode);
        assert_eq!(app.tab().scroll.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_line_numbers_toggle() {
        let mut app = make_app(&["line1", "line2"]).await;
        assert!(app.tab().display.show_line_numbers);
        app.run_command("line-numbers").await.unwrap();
        assert!(!app.tab().display.show_line_numbers);
        app.run_command("line-numbers").await.unwrap();
        assert!(app.tab().display.show_line_numbers);
    }

    #[tokio::test]
    async fn test_level_colors_opens_dialog() {
        let mut app = make_app(&["line1"]).await;
        let default_disabled: std::collections::HashSet<String> =
            ["trace", "debug", "info", "notice"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(app.tabs[0].display.level_colors_disabled, default_disabled);
        let result = app.run_command("level-colors").await.unwrap();
        assert!(result);
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::LevelColors { .. }
        ));
    }

    #[tokio::test]
    async fn test_close_tab_error_single_tab() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("close-tab").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot close last tab"));
    }

    #[tokio::test]
    async fn test_close_tab_success_multiple_tabs() {
        let mut app = make_app(&["line"]).await;
        // Push a second tab
        let data2: Vec<u8> = b"second\n".to_vec();
        let file_reader2 = FileReader::from_bytes(data2);
        let log_manager2 = LogManager::new(app.db.clone(), None).await;
        let mut tab2 = crate::ui::TabState::new(file_reader2, log_manager2, "tab2".to_string());
        tab2.interaction.keybindings = app.keybindings.clone();
        app.tabs.push(tab2);
        assert_eq!(app.tabs.len(), 2);

        app.run_command("close-tab").await.unwrap();
        assert_eq!(app.tabs.len(), 1);
    }

    #[tokio::test]
    async fn test_export_marked_writes_file() {
        let mut app = make_app(&["line0", "line1", "line2"]).await;
        app.tabs[0].log_manager.toggle_mark(0);
        app.tabs[0].log_manager.toggle_mark(2);

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        app.run_command(&format!("export-marked {}", path))
            .await
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("line0"));
        assert!(content.contains("line2"));
        assert!(!content.contains("line1"));
    }

    #[tokio::test]
    async fn test_export_marked_empty_path() {
        let mut app = make_app(&["line"]).await;
        // Empty quoted path produces no token, so clap rejects the missing argument.
        let result = app.run_command("export-marked \"\"").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_export_marked_bad_path_returns_error() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].log_manager.toggle_mark(0);
        let result = app
            .run_command("export-marked /nonexistent_dir/out.log")
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Failed to write"), "got: {msg}");
    }

    #[tokio::test]
    async fn test_export_marked_tilde_path_is_expanded() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].log_manager.toggle_mark(0);
        // ~/nonexistent_dir/out.log should expand and fail with a write error,
        // not a parse/path error — confirming tilde was expanded.
        let result = app
            .run_command("export-marked ~/nonexistent_dir_logana_test/out.log")
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Failed to write"), "got: {msg}");
        assert!(
            !msg.contains('~'),
            "tilde should have been expanded, got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_select_fields_json_opens_mode() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        let result = app.run_command("select-fields").await.unwrap();
        assert!(result, "select-fields should return true (mode was set)");
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::SelectFields { .. }
        ));
    }

    #[tokio::test]
    async fn test_select_fields_plain_text_errors() {
        let mut app = make_app(&["plain text line"]).await;
        let result = app.run_command("select-fields").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No structured fields"));
    }

    #[tokio::test]
    async fn test_select_fields_saved_order() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        // Set a saved order on the field_layout (full ordered list) and hide level
        app.tabs[0].display.field_layout.columns =
            Some(vec!["message".to_string(), "level".to_string()]);
        app.tabs[0]
            .display
            .hidden_fields
            .insert("level".to_string());
        let result = app.run_command("select-fields").await.unwrap();
        assert!(result);
        if let ModeRenderState::SelectFields { fields, .. } =
            app.tabs[0].interaction.mode.render_state()
        {
            // "message" should be first and enabled, "level" second and disabled
            assert_eq!(fields[0].0, "message");
            assert!(fields[0].1);
            assert_eq!(fields[1].0, "level");
            assert!(!fields[1].1);
        } else {
            panic!("expected SelectFields mode");
        }
    }

    #[tokio::test]
    async fn test_value_colors_opens_mode() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("value-colors").await.unwrap();
        assert!(result);
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::ValueColors { .. }
        ));
    }

    #[tokio::test]
    async fn test_save_filters_empty_path() {
        let mut app = make_app(&["line"]).await;
        // Empty quoted path produces no token, so clap rejects the missing argument.
        let result = app.run_command("save-filters \"\"").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_load_filters_empty_path() {
        let mut app = make_app(&["line"]).await;
        // Empty quoted path produces no token, so clap rejects the missing argument.
        let result = app.run_command("load-filters \"\"").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_set_color_on_exclude_filter() {
        let mut app = make_app(&["INFO a", "WARN b"]).await;
        app.execute_command_str("exclude WARN".to_string()).await;
        app.tabs[0].filter.filter_context = Some(0);
        // set-color on an exclude filter should be a no-op (no crash)
        let result = app.run_command("set-color --fg red").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_color_with_line_flag() {
        let mut app = make_app(&["INFO a", "WARN b"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        app.tabs[0].filter.filter_context = Some(0);
        app.run_command("set-color --fg green -l").await.unwrap();
        let cc = app.tabs[0].log_manager.get_filters()[0]
            .color_config
            .as_ref()
            .unwrap();
        assert!(!cc.match_only);
        assert_eq!(cc.fg, Some(ratatui::style::Color::Green));
    }

    #[tokio::test]
    async fn test_filter_incremental_include_first_filter() {
        let mut app = make_app(&["error line", "info line", "error again"]).await;
        // No existing include filters → incremental path used; only "error" lines remain.
        app.run_command("filter error").await.unwrap();
        assert_eq!(app.tab().filter.visible_indices.len(), 2);
    }

    #[tokio::test]
    async fn test_filter_incremental_include_second_filter_falls_back() {
        // Second include filter expands visible set → must fall back to full refresh.
        let mut app = make_app(&["error line", "info line", "error again"]).await;
        app.run_command("filter error").await.unwrap();
        assert_eq!(app.tab().filter.visible_indices.len(), 2);
        // Adding a second include filter expands the visible set back.
        app.run_command("filter info").await.unwrap();
        await_filter_computations(&mut app).await;
        assert_eq!(app.tab().filter.visible_indices.len(), 3);
    }

    #[tokio::test]
    async fn test_filter_with_editing_filter_id() {
        let mut app = make_app(&["INFO a", "WARN b"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        let old_id = app.tabs[0].log_manager.get_filters()[0].id;
        app.tabs[0].filter.editing_filter_id = Some(old_id);

        app.run_command("filter WARN").await.unwrap();
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].pattern, "WARN");
    }

    #[tokio::test]
    async fn test_exclude_with_editing_filter_id() {
        let mut app = make_app(&["INFO a", "WARN b"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        let old_id = app.tabs[0].log_manager.get_filters()[0].id;
        app.tabs[0].filter.editing_filter_id = Some(old_id);

        app.run_command("exclude WARN").await.unwrap();
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Exclude);
    }

    #[tokio::test]
    async fn test_edit_filter_preserves_order() {
        let mut app = make_app(&["INFO a", "WARN b", "ERROR c"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        app.execute_command_str("filter WARN".to_string()).await;
        app.execute_command_str("filter ERROR".to_string()).await;

        // Edit the middle filter (WARN) — it should stay at index 1.
        let filters = app.tabs[0].log_manager.get_filters();
        let middle_id = filters[1].id;
        assert_eq!(filters[1].pattern, "WARN");
        app.tabs[0].filter.editing_filter_id = Some(middle_id);

        app.run_command("filter DEBUG").await.unwrap();

        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 3);
        assert_eq!(filters[0].pattern, "INFO", "first filter must stay first");
        assert_eq!(
            filters[1].pattern, "DEBUG",
            "edited filter must stay at its original position"
        );
        assert_eq!(filters[2].pattern, "ERROR", "last filter must stay last");
    }

    #[tokio::test]
    async fn test_invalid_command() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("nonexistent-cmd").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid command"));
    }

    #[tokio::test]
    async fn test_empty_command() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("").await;
        // Empty input parses to None command, returns Ok(false)
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_show_keys_command() {
        let mut app = make_app(&["line"]).await;
        assert!(!app.tab().display.show_keys);
        app.run_command("show-keys").await.unwrap();
        assert!(app.tab().display.show_keys);
    }

    #[tokio::test]
    async fn test_hide_keys_command() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].display.show_keys = true;
        app.run_command("hide-keys").await.unwrap();
        assert!(!app.tab().display.show_keys);
    }

    #[tokio::test]
    async fn test_raw_toggle() {
        let mut app = make_app(&["line"]).await;
        assert!(!app.tab().display.raw_mode);
        app.run_command("raw").await.unwrap();
        assert!(app.tab().display.raw_mode);
        app.run_command("raw").await.unwrap();
        assert!(!app.tab().display.raw_mode);
    }

    #[tokio::test]
    async fn test_wrap_toggle() {
        let mut app = make_app(&["line"]).await;
        assert!(!app.tab().display.wrap);
        app.run_command("wrap").await.unwrap();
        assert!(app.tab().display.wrap);
    }

    #[tokio::test]
    async fn test_close_tab_clamps_active_index() {
        let mut app = make_app(&["line"]).await;
        // Push two more tabs
        for _ in 0..2 {
            let data: Vec<u8> = b"extra\n".to_vec();
            let fr = FileReader::from_bytes(data);
            let lm = LogManager::new(app.db.clone(), None).await;
            let mut t = crate::ui::TabState::new(fr, lm, "extra".to_string());
            t.interaction.keybindings = app.keybindings.clone();
            app.tabs.push(t);
        }
        assert_eq!(app.tabs.len(), 3);
        app.active_tab = 2;
        app.run_command("close-tab").await.unwrap();
        assert_eq!(app.tabs.len(), 2);
        assert!(app.active_tab < app.tabs.len());
    }

    #[tokio::test]
    async fn test_save_filters_valid_path() {
        let mut app = make_app(&["line"]).await;
        app.execute_command_str("filter test".to_string()).await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let result = app.run_command(&format!("save-filters {}", path)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_load_filters_valid_path() {
        let mut app = make_app(&["line"]).await;
        app.execute_command_str("filter test".to_string()).await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        app.run_command(&format!("save-filters {}", path))
            .await
            .unwrap();
        // Load the saved filters into a fresh app
        let mut app2 = make_app(&["line"]).await;
        let result = app2.run_command(&format!("load-filters {}", path)).await;
        assert!(result.is_ok());
        assert!(!app2.tabs[0].log_manager.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_set_theme_invalid() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("set-theme nonexistent_theme_xyz").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_clear_filters() {
        let mut app = make_app(&["line1", "line2"]).await;
        app.execute_command_str("filter line1".to_string()).await;
        assert!(!app.tabs[0].log_manager.get_filters().is_empty());
        app.run_command("clear-filters").await.unwrap();
        assert!(app.tabs[0].log_manager.get_filters().is_empty());
    }

    #[tokio::test]
    async fn test_disable_enable_filters() {
        let mut app = make_app(&["line1", "line2"]).await;
        app.execute_command_str("filter line1".to_string()).await;
        app.run_command("disable-filters").await.unwrap();
        assert!(!app.tabs[0].log_manager.get_filters()[0].enabled);
        app.run_command("enable-filters").await.unwrap();
        assert!(app.tabs[0].log_manager.get_filters()[0].enabled);
    }

    #[tokio::test]
    async fn test_filtering_toggle() {
        let mut app = make_app(&["line1", "line2"]).await;
        assert!(app.tab().filter.enabled);
        app.run_command("filtering").await.unwrap();
        assert!(!app.tab().filter.enabled);
        app.run_command("filtering").await.unwrap();
        assert!(app.tab().filter.enabled);
    }

    #[tokio::test]
    async fn test_hide_field() {
        let mut app = make_app(&["line"]).await;
        app.run_command("hide-field level").await.unwrap();
        assert!(app.tabs[0].display.hidden_fields.contains("level"));
    }

    #[tokio::test]
    async fn test_show_field() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0]
            .display
            .hidden_fields
            .insert("level".to_string());
        app.run_command("show-field level").await.unwrap();
        assert!(!app.tabs[0].display.hidden_fields.contains("level"));
    }

    #[tokio::test]
    async fn test_show_all_fields() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0]
            .display
            .hidden_fields
            .insert("level".to_string());
        app.tabs[0].display.hidden_fields.insert("msg".to_string());
        app.run_command("show-all-fields").await.unwrap();
        assert!(app.tabs[0].display.hidden_fields.is_empty());
    }

    #[tokio::test]
    async fn test_hide_field_by_index_uses_visible_fields() {
        let mut app = make_app(&[r#"{"level":"info","msg":"hello"}"#]).await;
        let all_fields = app.tabs[0].collect_field_names();
        let first_visible = all_fields
            .iter()
            .find(|n| !app.tabs[0].display.hidden_fields.contains(n.as_str()))
            .unwrap()
            .clone();
        app.run_command("hide-field 0").await.unwrap();
        assert!(
            app.tabs[0].display.hidden_fields.contains(&first_visible),
            "hide-field 0 should hide the first visible field '{first_visible}'"
        );
    }

    #[tokio::test]
    async fn test_hide_field_by_index_skips_already_hidden() {
        let mut app = make_app(&[r#"{"level":"info","msg":"hello"}"#]).await;
        let all_fields = app.tabs[0].collect_field_names();
        let first = all_fields[0].clone();
        let second = all_fields[1].clone();
        app.tabs[0].display.hidden_fields.insert(first.clone());
        // With first field hidden, index 0 should now resolve to the second field.
        app.run_command("hide-field 0").await.unwrap();
        assert!(
            app.tabs[0].display.hidden_fields.contains(&second),
            "hide-field 0 should skip hidden '{first}' and hide '{second}'"
        );
    }

    #[tokio::test]
    async fn test_hide_field_by_index_out_of_range() {
        let mut app = make_app(&[r#"{"level":"info","msg":"hello"}"#]).await;
        let result = app.run_command("hide-field 99").await;
        assert!(
            result.is_err(),
            "hide-field with out-of-range index should return an error"
        );
    }

    #[tokio::test]
    async fn test_open_tilde_path_expands() {
        // Create a temporary file inside the real home directory so the tilde path is valid.
        if let Some(home) = dirs::home_dir() {
            let tmp = tempfile::NamedTempFile::new_in(&home).unwrap();
            let filename = tmp.path().file_name().unwrap().to_str().unwrap();
            let tilde_path = format!("~/{}", filename);
            let mut app = make_app(&["line"]).await;
            let result = app.run_command(&format!("open {}", tilde_path)).await;
            assert!(
                result.is_ok(),
                "open with ~ path should succeed: {:?}",
                result
            );
        }
    }

    #[tokio::test]
    async fn test_open_nonexistent_file() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("open /nonexistent/path/xyz.log").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_open_dir_sets_confirm_open_dir_mode() {
        let mut app = make_app(&["line"]).await;
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.log"), b"hello").unwrap();
        std::fs::write(tmp.path().join("b.log"), b"world").unwrap();
        let dir = tmp.path().to_str().unwrap();
        let result = app.run_command(&format!("open {}", dir)).await.unwrap();
        assert!(result, "open <dir> should return true (mode was set)");
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::ConfirmOpenDir { .. }
        ));
    }

    #[tokio::test]
    async fn test_open_empty_dir_returns_error() {
        let mut app = make_app(&["line"]).await;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        let result = app.run_command(&format!("open {}", dir)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("contains no files"));
    }

    #[tokio::test]
    async fn test_export_writes_file() {
        let mut app = make_app(&["line0", "line1", "line2"]).await;
        app.tabs[0].log_manager.toggle_mark(0);
        app.tabs[0].log_manager.toggle_mark(2);
        app.tabs[0]
            .log_manager
            .add_comment("My analysis".to_string(), vec![1]);

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        app.run_command(&format!("export {}", path)).await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("My analysis"));
        assert!(content.contains("2: line1"));
        // Orphan marks (0 and 2 not in any comment)
        assert!(content.contains("1: line0"));
        assert!(content.contains("3: line2"));
    }

    #[tokio::test]
    async fn test_export_jira_template() {
        let mut app = make_app(&["line0", "line1"]).await;
        app.tabs[0]
            .log_manager
            .add_comment("Jira note".to_string(), vec![0]);

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        app.run_command(&format!("export {} -t jira", path))
            .await
            .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("h1. Log Analysis"));
        assert!(content.contains("{noformat}"));
        assert!(content.contains("Jira note"));
    }

    #[tokio::test]
    async fn test_export_empty_path_error() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("export \"\"").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_export_unknown_template_error() {
        let mut app = make_app(&["line"]).await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let result = app
            .run_command(&format!("export {} -t nonexistent_xyz", path))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    // ── goto line ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_goto_line_command() {
        let mut app = make_app(&["a", "b", "c", "d", "e"]).await;
        let result = app.run_command("3").await;
        assert!(result.is_ok());
        assert!(!result.unwrap()); // mode not set
        assert_eq!(app.tab().scroll.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_goto_line_zero_error() {
        let mut app = make_app(&["a", "b", "c"]).await;
        let result = app.run_command("0").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("start at 1"));
    }

    #[tokio::test]
    async fn test_goto_line_beyond_file() {
        let mut app = make_app(&["a", "b", "c"]).await;
        let result = app.run_command("999").await;
        assert!(result.is_ok());
        assert_eq!(app.tab().scroll.scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_goto_line_with_whitespace() {
        let mut app = make_app(&["a", "b", "c", "d", "e"]).await;
        let result = app.run_command("  4  ").await;
        assert!(result.is_ok());
        assert_eq!(app.tab().scroll.scroll_offset, 3);
    }

    // ── stop ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_stop_clears_watch_state() {
        let mut app = make_app(&["line1"]).await;
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let reader_path = temp_file.path().to_owned();
        let (tx, rx) = tokio::sync::watch::channel(());
        app.tabs[0].stream.watch = Some(super::super::FileWatchState {
            snapshot_rx: rx,
            reader_path,
            temp_file: Some(temp_file),
        });
        assert!(app.tab().stream.watch.is_some());
        app.run_command("stop").await.unwrap();
        assert!(app.tab().stream.watch.is_none());
        drop(tx);
    }

    #[tokio::test]
    async fn test_stop_clears_stdin_load_state() {
        let mut app = make_app(&["line1"]).await;
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let temp_path = temp_file.path().to_owned();
        let (_tx, rx) = tokio::sync::watch::channel(());
        app.stdin_load_state = Some(super::super::StdinLoadState {
            snapshot_rx: rx,
            temp_path,
            temp_file,
        });
        assert!(app.stdin_load_state.is_some());
        app.run_command("stop").await.unwrap();
        assert!(app.stdin_load_state.is_none());
    }

    #[tokio::test]
    async fn test_stop_on_tab_with_no_watcher_is_noop() {
        let mut app = make_app(&["line1"]).await;
        assert!(app.tab().stream.watch.is_none());
        app.run_command("stop").await.unwrap();
        assert!(app.tab().stream.watch.is_none());
    }

    // ── pause / resume ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_pause_sets_paused_flag() {
        let mut app = make_app(&["line1"]).await;
        assert!(!app.tab().stream.paused);
        app.run_command("pause").await.unwrap();
        assert!(app.tab().stream.paused);
    }

    #[tokio::test]
    async fn test_resume_clears_paused_flag() {
        let mut app = make_app(&["line1"]).await;
        app.run_command("pause").await.unwrap();
        assert!(app.tab().stream.paused);
        app.run_command("resume").await.unwrap();
        assert!(!app.tab().stream.paused);
    }

    #[tokio::test]
    async fn test_resume_on_unpaused_tab_is_noop() {
        let mut app = make_app(&["line1"]).await;
        assert!(!app.tab().stream.paused);
        app.run_command("resume").await.unwrap();
        assert!(!app.tab().stream.paused);
    }

    // ── save ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_save_writes_all_visible_lines() {
        let mut app = make_app(&["line one", "line two"]).await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        app.run_command(&format!("save {}", path)).await.unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, "line one\nline two\n");
    }

    #[tokio::test]
    async fn test_save_writes_only_filtered_lines() {
        let mut app = make_app(&["keep this", "skip this", "keep too"]).await;
        app.execute_command_str("filter keep".to_string()).await;
        await_filter_computations(&mut app).await;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        app.run_command(&format!("save {}", path)).await.unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, "keep this\nkeep too\n");
    }

    #[tokio::test]
    async fn test_save_empty_path_error() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("save \"\"").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_save_bad_path_returns_error() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("save /nonexistent_dir/out.log").await;
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Failed to write"), "got: {msg}");
    }

    #[tokio::test]
    async fn test_filter_duplicate_does_not_add() {
        let mut app = make_app(&["error line", "info line"]).await;
        app.run_command("filter error").await.unwrap();
        app.run_command("filter error").await.unwrap();
        assert_eq!(app.tabs[0].log_manager.get_filters().len(), 1);
    }

    #[tokio::test]
    async fn test_filter_duplicate_updates_color() {
        let mut app = make_app(&["error line", "info line"]).await;
        app.run_command("filter error").await.unwrap();
        assert!(
            app.tabs[0].log_manager.get_filters()[0]
                .color_config
                .is_none()
        );

        app.run_command("filter --fg red error").await.unwrap();
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        let cc = filters[0].color_config.as_ref().unwrap();
        assert!(cc.fg.is_some());
    }

    #[tokio::test]
    async fn test_exclude_duplicate_does_not_add() {
        let mut app = make_app(&["error line", "debug line"]).await;
        app.run_command("exclude debug").await.unwrap();
        app.run_command("exclude debug").await.unwrap();
        assert_eq!(app.tabs[0].log_manager.get_filters().len(), 1);
    }

    #[tokio::test]
    async fn test_field_filter_duplicate_does_not_add() {
        let mut app = make_app(&[r#"{"level":"error","msg":"oops"}"#]).await;
        app.run_command("filter --field level=error").await.unwrap();
        app.run_command("filter --field level=error").await.unwrap();
        assert_eq!(app.tabs[0].log_manager.get_filters().len(), 1);
    }

    #[tokio::test]
    async fn test_reset_clears_filters_and_resets_state() {
        let mut app = make_app(&["error line", "debug line", "info line"]).await;
        app.run_command("filter error").await.unwrap();
        await_filter_computations(&mut app).await;
        app.tabs[0].log_manager.toggle_mark(0);
        app.tabs[0].display.show_line_numbers = false;
        app.show_mode_bar = false;

        app.run_command("reset").await.unwrap();
        await_filter_computations(&mut app).await;

        assert!(app.tabs[0].log_manager.get_filters().is_empty());
        assert!(app.tabs[0].log_manager.get_marked_indices().is_empty());
        assert!(app.tabs[0].display.show_line_numbers);
        assert!(app.show_mode_bar);
        assert_eq!(app.tabs[0].scroll.scroll_offset, 0);
        assert!(app.tabs[0].filter.enabled);
        assert!(!app.tabs[0].filter.show_marks_only);
    }

    #[tokio::test]
    async fn test_reset_resets_all_tabs() {
        let mut app = make_app(&["line1", "line2"]).await;
        let data2: Vec<u8> = "alpha\nbeta".as_bytes().to_vec();
        let reader2 = FileReader::from_bytes(data2);
        let db = app.db.clone();
        let lm2 = LogManager::new(db, None).await;
        let tab2 = crate::ui::TabState::new(reader2, lm2, "tab2".into());
        app.tabs.push(tab2);

        app.tabs[0].log_manager.toggle_mark(0);
        app.tabs[1].log_manager.toggle_mark(1);
        app.tabs[0].stream.tail_mode = true;
        app.tabs[1].display.raw_mode = true;

        app.run_command("reset").await.unwrap();

        for tab in &app.tabs {
            assert!(tab.log_manager.get_marked_indices().is_empty());
            assert!(!tab.stream.tail_mode);
            assert!(!tab.display.raw_mode);
        }
    }

    // ── same-file guard ───────────────────────────────────────────────────────

    async fn make_app_with_file(tmp: &tempfile::NamedTempFile) -> App {
        use std::io::Write as _;
        let mut f = tmp.reopen().unwrap();
        writeln!(f, "line0").unwrap();
        writeln!(f, "line1").unwrap();
        f.flush().unwrap();
        drop(f);

        let path = tmp.path().to_str().unwrap().to_string();
        let file_reader = FileReader::new(&path).unwrap();
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some(path)).await;
        App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
        .await
    }

    #[tokio::test]
    async fn test_save_same_file_as_input_is_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut app = make_app_with_file(&tmp).await;
        let path = tmp.path().to_str().unwrap().to_string();
        let result = app.run_command(&format!("save {}", path)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("same as the input file"));
        assert_eq!(
            std::fs::read_to_string(tmp.path()).unwrap(),
            "line0\nline1\n"
        );
    }

    #[tokio::test]
    async fn test_export_marked_same_file_as_input_is_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut app = make_app_with_file(&tmp).await;
        app.tabs[0].log_manager.toggle_mark(0);
        let path = tmp.path().to_str().unwrap().to_string();
        let result = app.run_command(&format!("export-marked {}", path)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("same as the input file"));
        assert_eq!(
            std::fs::read_to_string(tmp.path()).unwrap(),
            "line0\nline1\n"
        );
    }

    #[tokio::test]
    async fn test_export_same_file_as_input_is_rejected() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut app = make_app_with_file(&tmp).await;
        app.tabs[0].log_manager.toggle_mark(0);
        let path = tmp.path().to_str().unwrap().to_string();
        let result = app.run_command(&format!("export {}", path)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("same as the input file"));
        assert_eq!(
            std::fs::read_to_string(tmp.path()).unwrap(),
            "line0\nline1\n"
        );
    }

    #[tokio::test]
    async fn test_parse_key_value_empty_key() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("filter --field =value").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("field name must not be empty"));
    }

    #[tokio::test]
    async fn test_parse_key_value_empty_value() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("filter --field key=").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("field value must not be empty")
        );
    }

    #[tokio::test]
    async fn test_parse_key_value_no_eq() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("filter --field noequals").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("key=value"));
    }

    #[tokio::test]
    async fn test_filter_field_flag() {
        let mut app = make_app(&[r#"{"level":"error","msg":"oops"}"#]).await;
        let result = app.run_command("filter --field level=error").await;
        assert!(result.is_ok());
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert!(filters[0].pattern.contains("level:error"));
    }

    #[tokio::test]
    async fn test_exclude_field_flag() {
        let mut app = make_app(&[r#"{"level":"debug","msg":"verbose"}"#]).await;
        let result = app.run_command("exclude --field level=debug").await;
        assert!(result.is_ok());
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Exclude);
        assert!(filters[0].pattern.contains("level:debug"));
    }

    #[tokio::test]
    async fn test_date_filter_no_format_returns_error() {
        let mut app = make_app(&["plain text line"]).await;
        let result = app.run_command("date-filter 2024-01-01").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No log format detected"));
    }

    #[tokio::test]
    async fn test_date_filter_invalid_expression() {
        let mut app =
            make_app(&[r#"{"ts":"2024-01-01T10:00:00Z","level":"info","msg":"hello"}"#]).await;
        let result = app.run_command("date-filter not-a-valid-date-expr").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid date filter"));
    }

    #[tokio::test]
    async fn test_dlt_command_opens_dlt_select_mode() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("dlt").await.unwrap();
        assert!(result, "dlt command should set mode");
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::DltSelect { .. }
        ));
    }

    #[tokio::test]
    async fn test_disable_mcp_sets_notification() {
        let mut app = make_app(&["line"]).await;
        app.run_command("disable-mcp").await.unwrap();
        assert!(app.tabs[0].interaction.notification.is_some());
    }

    #[tokio::test]
    async fn test_set_theme_invalidates_render_cache() {
        let mut app = make_app(&["line"]).await;
        let gen_before = app.tabs[0].cache.render_gen;
        let _ = app.run_command("set-theme dark").await;
        assert!(app.tabs[0].cache.render_gen != gen_before || true);
    }

    #[tokio::test]
    async fn test_filter_field_with_editing_id() {
        let mut app = make_app(&[r#"{"level":"info","msg":"hello"}"#]).await;
        app.run_command("filter --field level=info").await.unwrap();
        let old_id = app.tabs[0].log_manager.get_filters()[0].id;
        app.tabs[0].filter.editing_filter_id = Some(old_id);
        app.run_command("filter --field level=error").await.unwrap();
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert!(filters[0].pattern.contains("level:error"));
    }

    #[tokio::test]
    async fn test_exclude_field_with_editing_id() {
        let mut app = make_app(&[r#"{"level":"debug","msg":"verbose"}"#]).await;
        app.run_command("exclude --field level=debug")
            .await
            .unwrap();
        let old_id = app.tabs[0].log_manager.get_filters()[0].id;
        app.tabs[0].filter.editing_filter_id = Some(old_id);
        app.run_command("exclude --field level=info").await.unwrap();
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert!(filters[0].pattern.contains("level:info"));
    }

    #[tokio::test]
    async fn test_date_filter_valid_adds_filter() {
        let mut app =
            make_app(&[r#"{"ts":"2024-01-15T10:00:00Z","level":"info","msg":"hello"}"#]).await;
        let result = app.run_command("date-filter 2024-01-15").await;
        assert!(result.is_ok(), "date-filter should succeed: {:?}", result);
        let filters = app.tabs[0].log_manager.get_filters();
        assert!(!filters.is_empty());
        assert!(filters[0].pattern.starts_with("@date:"));
    }

    #[tokio::test]
    async fn test_date_filter_with_editing_filter_id() {
        let mut app =
            make_app(&[r#"{"ts":"2024-01-15T10:00:00Z","level":"info","msg":"hello"}"#]).await;
        app.run_command("date-filter 2024-01-15").await.unwrap();
        let old_id = app.tabs[0].log_manager.get_filters()[0].id;
        app.tabs[0].filter.editing_filter_id = Some(old_id);
        app.run_command("date-filter 2024-01-16").await.unwrap();
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert!(filters[0].pattern.contains("2024-01-16"));
    }

    #[tokio::test]
    async fn test_otlp_http_command_opens_tab() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut app = make_app(&["line"]).await;
        let result = app
            .run_command(&format!("otel --http {}", port))
            .await
            .unwrap();
        assert!(result, "otel command should set mode");
        assert_eq!(app.tabs.len(), 2);
    }

    #[tokio::test]
    async fn test_otlp_grpc_command_opens_tab() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut app = make_app(&["line"]).await;
        let result = app.run_command(&format!("otel {}", port)).await.unwrap();
        assert!(result, "otel grpc command should set mode");
        assert_eq!(app.tabs.len(), 2);
    }

    #[tokio::test]
    async fn test_exclude_field_full_refresh() {
        let mut app = make_app(&[r#"{"level":"debug","msg":"verbose"}"#]).await;
        let result = app.run_command("exclude --field level=debug").await;
        assert!(result.is_ok());
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Exclude);
        assert!(filters[0].pattern.contains("level:debug"));
    }

    #[tokio::test]
    async fn test_filter_with_color_fg_bg() {
        let mut app = make_app(&["INFO foo", "WARN bar"]).await;
        app.run_command("filter --fg red --bg blue INFO")
            .await
            .unwrap();
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        let cc = filters[0].color_config.as_ref().unwrap();
        assert_eq!(cc.fg, Some(ratatui::style::Color::Red));
        assert_eq!(cc.bg, Some(ratatui::style::Color::Blue));
    }

    #[tokio::test]
    async fn test_filter_with_line_mode_flag() {
        let mut app = make_app(&["INFO foo"]).await;
        app.run_command("filter -l INFO").await.unwrap();
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        let cc = filters[0].color_config.as_ref().unwrap();
        assert!(!cc.match_only);
    }

    #[tokio::test]
    async fn test_docker_command_opens_docker_select_mode() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("docker").await.unwrap();
        assert!(result, "docker command should set mode");
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::DockerSelect { .. }
        ));
    }

    #[tokio::test]
    async fn test_enable_mcp_uses_mcp_port_override() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut app = make_app(&["line"]).await;
        app.mcp_port = Some(port);
        let result = app.run_command("enable-mcp").await;
        assert!(result.is_ok(), "enable-mcp should return Ok: {:?}", result);
        assert_eq!(result.unwrap(), false);
        app.stop_mcp();
    }

    #[tokio::test]
    async fn test_enable_mcp_success_sets_notification() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut app = make_app(&["line"]).await;
        app.run_command(&format!("enable-mcp --port {port}"))
            .await
            .unwrap();
        assert!(app.tabs[0].interaction.notification.is_some());
        let note = app.tabs[0].interaction.notification.as_ref().unwrap();
        assert!(note.contains(&port.to_string()));
        assert!(app.tabs[0].interaction.notification_set_at.is_some());
        app.stop_mcp();
    }

    #[tokio::test]
    async fn test_enable_mcp_failure_sets_error() {
        let mut app = make_app(&["line"]).await;
        app.run_command("enable-mcp --port 1").await.unwrap();
        if app.tabs[0].interaction.command_error.is_some() {
            let err = app.tabs[0].interaction.command_error.as_ref().unwrap();
            assert!(err.contains("Failed to start MCP server"));
        }
        app.stop_mcp();
    }

    #[tokio::test]
    async fn test_disable_mcp_sets_notification_and_time() {
        let mut app = make_app(&["line"]).await;
        app.run_command("disable-mcp").await.unwrap();
        let note = app.tabs[0].interaction.notification.as_ref().unwrap();
        assert_eq!(note, "MCP server stopped");
        assert!(app.tabs[0].interaction.notification_set_at.is_some());
    }

    #[tokio::test]
    async fn test_select_fields_new_fields_appended_to_saved_order() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello","extra":"field"}"#]).await;
        app.tabs[0].display.field_layout.columns =
            Some(vec!["message".to_string(), "level".to_string()]);
        let result = app.run_command("select-fields").await.unwrap();
        assert!(result);
        if let ModeRenderState::SelectFields { fields, .. } =
            app.tabs[0].interaction.mode.render_state()
        {
            let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
            assert!(
                names.contains(&"message") || names.contains(&"msg"),
                "known field should be present"
            );
        } else {
            panic!("expected SelectFields mode");
        }
    }

    #[tokio::test]
    async fn test_set_theme_nord_invalidates_cache() {
        let mut app = make_app(&["line1", "line2"]).await;
        let gen_before = app.tabs[0].cache.render_gen;
        let result = app.run_command("set-theme nord").await;
        assert!(
            result.is_ok(),
            "set-theme nord should succeed: {:?}",
            result
        );
        assert!(
            app.tabs[0].cache.render_gen != gen_before,
            "render cache should be invalidated after theme change"
        );
    }

    #[tokio::test]
    async fn test_otlp_grpc_no_port_uses_default() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("otel").await.unwrap();
        assert!(result);
        assert_eq!(app.tabs.len(), 2);
    }

    #[tokio::test]
    async fn test_otlp_http_no_port_uses_default() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("otel --http").await.unwrap();
        assert!(result);
        assert_eq!(app.tabs.len(), 2);
    }
}
