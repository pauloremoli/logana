use clap::Parser;
use std::collections::HashSet;

use crate::auto_complete::shell_split;
use crate::mode::command_mode::{CommandLine, Commands};
use crate::theme::Theme;
use crate::types::FilterType;

use super::App;

impl App {
    /// Returns `Ok(true)` when the command sets the mode itself (e.g. select-fields
    /// opens a popup), so `execute_command_str` should not override it.
    pub(super) async fn run_command(&mut self, input: &str) -> Result<bool, String> {
        let args = CommandLine::try_parse_from(shell_split(input))
            .map_err(|e| format!("Invalid command: {}", e))?;

        match args.command {
            Some(Commands::Filter { pattern, fg, bg, m }) => {
                if let Some(old_id) = self.tabs[self.active_tab].editing_filter_id.take() {
                    self.tabs[self.active_tab]
                        .log_manager
                        .remove_filter(old_id)
                        .await;
                }
                self.tabs[self.active_tab]
                    .log_manager
                    .add_filter_with_color(
                        pattern,
                        FilterType::Include,
                        fg.as_deref(),
                        bg.as_deref(),
                        m,
                    )
                    .await;
                self.tabs[self.active_tab].scroll_offset = 0;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::Exclude { pattern }) => {
                if let Some(old_id) = self.tabs[self.active_tab].editing_filter_id.take() {
                    self.tabs[self.active_tab]
                        .log_manager
                        .remove_filter(old_id)
                        .await;
                }
                self.tabs[self.active_tab]
                    .log_manager
                    .add_filter_with_color(pattern, FilterType::Exclude, None, None, false)
                    .await;
                self.tabs[self.active_tab].scroll_offset = 0;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::SetColor { fg, bg, m }) => {
                let selected_filter_index = self.tabs[self.active_tab].filter_context.unwrap_or(0);
                let filters = self.tabs[self.active_tab].log_manager.get_filters();
                if let Some(filter) = filters.get(selected_filter_index)
                    && filter.filter_type == FilterType::Include
                {
                    // When -m is not explicitly passed, preserve the filter's
                    // existing match_only setting instead of resetting it.
                    let match_only = if m {
                        true
                    } else {
                        filter
                            .color_config
                            .as_ref()
                            .map(|cc| cc.match_only)
                            .unwrap_or(false)
                    };
                    let filter_id = filter.id;
                    self.tabs[self.active_tab]
                        .log_manager
                        .set_color_config(filter_id, fg.as_deref(), bg.as_deref(), match_only)
                        .await;
                    self.tabs[self.active_tab].refresh_visible();
                }
            }
            Some(Commands::ExportMarked { path }) => {
                if !path.is_empty() {
                    let tab = &self.tabs[self.active_tab];
                    let marked_lines = tab.log_manager.get_marked_lines(&tab.file_reader);
                    let mut content: Vec<u8> = Vec::new();
                    for line in marked_lines {
                        content.extend_from_slice(line);
                        content.push(b'\n');
                    }
                    let _ = std::fs::write(path, content);
                }
            }
            Some(Commands::SaveFilters { path }) => {
                if !path.is_empty() {
                    self.tabs[self.active_tab]
                        .log_manager
                        .save_filters(&path)
                        .map_err(|e| format!("Failed to save filters: {}", e))?;
                }
            }
            Some(Commands::LoadFilters { path }) => {
                if !path.is_empty() {
                    self.tabs[self.active_tab]
                        .log_manager
                        .load_filters(&path)
                        .await
                        .map_err(|e| format!("Failed to load filters: {}", e))?;
                    self.tabs[self.active_tab].refresh_visible();
                }
            }
            Some(Commands::Wrap) => {
                self.tabs[self.active_tab].wrap = !self.tabs[self.active_tab].wrap;
            }
            Some(Commands::LineNumbers) => {
                self.tabs[self.active_tab].show_line_numbers =
                    !self.tabs[self.active_tab].show_line_numbers;
            }
            Some(Commands::LevelColors) => {
                self.tabs[self.active_tab].level_colors = !self.tabs[self.active_tab].level_colors;
            }
            Some(Commands::SetTheme { theme_name }) => {
                let theme_filename = format!("{}.json", theme_name.to_lowercase());
                self.theme = Theme::from_file(&theme_filename)
                    .map_err(|e| format!("Failed to load theme '{}': {}", theme_name, e))?;
            }
            Some(Commands::Open { path }) => {
                self.open_file(&path).await?;
            }
            Some(Commands::CloseTab) => {
                if self.tabs.len() <= 1 {
                    return Err("Cannot close last tab. Use 'q' to quit.".to_string());
                }
                self.tabs.remove(self.active_tab);
                if self.active_tab >= self.tabs.len() {
                    self.active_tab = self.tabs.len() - 1;
                }
            }
            Some(Commands::ClearFilters) => {
                self.tabs[self.active_tab].log_manager.clear_filters().await;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::DisableFilters) => {
                self.tabs[self.active_tab]
                    .log_manager
                    .disable_all_filters()
                    .await;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::EnableFilters) => {
                self.tabs[self.active_tab]
                    .log_manager
                    .enable_all_filters()
                    .await;
                self.tabs[self.active_tab].refresh_visible();
            }
            Some(Commands::Filtering) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.filtering_enabled = !tab.filtering_enabled;
                tab.refresh_visible();
            }
            Some(Commands::HideField { field }) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.hidden_fields.insert(field);
            }
            Some(Commands::ShowField { field }) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.hidden_fields.remove(&field);
            }
            Some(Commands::ShowAllFields) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.hidden_fields.clear();
            }
            Some(Commands::SelectFields) => {
                let tab = &mut self.tabs[self.active_tab];
                let all_names = tab.collect_field_names();
                if all_names.is_empty() {
                    return Err("No structured fields found in visible lines".to_string());
                }
                let enabled_cols = &tab.field_layout.columns;
                let saved_order = &tab.field_layout.columns_order;
                // Restore the previous full ordering (enabled + disabled) if
                // available, then append any newly-discovered fields.
                let fields: Vec<(String, bool)> = match saved_order {
                    Some(order) => {
                        let enabled: HashSet<&String> = enabled_cols
                            .as_ref()
                            .map(|v| v.iter().collect())
                            .unwrap_or_default();
                        let mut ordered: Vec<(String, bool)> = order
                            .iter()
                            .filter(|n| all_names.contains(n))
                            .map(|n| (n.clone(), enabled.contains(n)))
                            .collect();
                        // Append fields not yet in the saved order.
                        for name in &all_names {
                            if !order.contains(name) {
                                ordered.push((name.clone(), false));
                            }
                        }
                        ordered
                    }
                    None => all_names.into_iter().map(|n| (n, true)).collect(),
                };
                let original = tab.field_layout.clone();
                tab.mode = Box::new(crate::mode::select_fields_mode::SelectFieldsMode::new(
                    fields, original,
                ));
                return Ok(true);
            }
            Some(Commands::Docker) => {
                let output = std::process::Command::new("docker")
                    .args([
                        "ps",
                        "--format",
                        "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}",
                    ])
                    .output();
                match output {
                    Ok(out) if out.status.success() => {
                        let text = String::from_utf8_lossy(&out.stdout);
                        let containers: Vec<crate::types::DockerContainer> = text
                            .lines()
                            .filter(|l| !l.is_empty())
                            .filter_map(|line| {
                                let parts: Vec<&str> = line.splitn(4, '\t').collect();
                                if parts.len() == 4 {
                                    Some(crate::types::DockerContainer {
                                        id: parts[0].to_string(),
                                        name: parts[1].to_string(),
                                        image: parts[2].to_string(),
                                        status: parts[3].to_string(),
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if containers.is_empty() {
                            self.tabs[self.active_tab].mode = Box::new(
                                crate::mode::docker_select_mode::DockerSelectMode::with_error(
                                    "No running containers found".to_string(),
                                ),
                            );
                        } else {
                            self.tabs[self.active_tab].mode = Box::new(
                                crate::mode::docker_select_mode::DockerSelectMode::new(containers),
                            );
                        }
                        return Ok(true);
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        self.tabs[self.active_tab].mode = Box::new(
                            crate::mode::docker_select_mode::DockerSelectMode::with_error(
                                if stderr.is_empty() {
                                    "docker ps failed".to_string()
                                } else {
                                    stderr
                                },
                            ),
                        );
                        return Ok(true);
                    }
                    Err(e) => {
                        self.tabs[self.active_tab].mode = Box::new(
                            crate::mode::docker_select_mode::DockerSelectMode::with_error(format!(
                                "Failed to run docker: {}",
                                e
                            )),
                        );
                        return Ok(true);
                    }
                }
            }
            Some(Commands::ValueColors) => {
                use crate::mode::value_colors_mode::{ValueColorEntry, ValueColorGroup as VCGroup};
                let disabled = &self.theme.value_colors.disabled;
                let groups: Vec<VCGroup> = self
                    .theme
                    .value_colors
                    .grouped_categories()
                    .into_iter()
                    .map(|g| VCGroup {
                        label: g.label.to_string(),
                        children: g
                            .children
                            .into_iter()
                            .map(|(key, label, color)| ValueColorEntry {
                                key: key.to_string(),
                                label: label.to_string(),
                                color,
                                enabled: !disabled.contains(key),
                            })
                            .collect(),
                    })
                    .collect();
                let original_disabled = disabled.clone();
                self.tabs[self.active_tab].mode = Box::new(
                    crate::mode::value_colors_mode::ValueColorsMode::new(groups, original_disabled),
                );
                return Ok(true);
            }
            None => {}
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::mode::app_mode::ModeRenderState;
    use crate::theme::Theme;
    use crate::types::FilterType;
    use crate::ui::app::App;
    use std::sync::Arc;

    async fn make_app(lines: &[&str]) -> App {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .await
    }

    #[tokio::test]
    async fn test_line_numbers_toggle() {
        let mut app = make_app(&["line1", "line2"]).await;
        assert!(app.tab().show_line_numbers);
        app.run_command("line-numbers").await.unwrap();
        assert!(!app.tab().show_line_numbers);
        app.run_command("line-numbers").await.unwrap();
        assert!(app.tab().show_line_numbers);
    }

    #[tokio::test]
    async fn test_level_colors_toggle() {
        let mut app = make_app(&["line1"]).await;
        assert!(app.tab().level_colors);
        app.run_command("level-colors").await.unwrap();
        assert!(!app.tab().level_colors);
        app.run_command("level-colors").await.unwrap();
        assert!(app.tab().level_colors);
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
        tab2.keybindings = app.keybindings.clone();
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
    async fn test_select_fields_json_opens_mode() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#]).await;
        let result = app.run_command("select-fields").await.unwrap();
        assert!(result, "select-fields should return true (mode was set)");
        assert!(matches!(
            app.tabs[0].mode.render_state(),
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
        // Set a saved order on the field_layout
        app.tabs[0].field_layout.columns = Some(vec!["msg".to_string()]);
        app.tabs[0].field_layout.columns_order = Some(vec!["msg".to_string(), "level".to_string()]);
        let result = app.run_command("select-fields").await.unwrap();
        assert!(result);
        if let ModeRenderState::SelectFields { fields, .. } = app.tabs[0].mode.render_state() {
            // "msg" should be first and enabled, "level" second and disabled
            assert_eq!(fields[0].0, "msg");
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
            app.tabs[0].mode.render_state(),
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
        app.tabs[0].filter_context = Some(0);
        // set-color on an exclude filter should be a no-op (no crash)
        let result = app.run_command("set-color --fg red").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_color_with_match_only_flag() {
        let mut app = make_app(&["INFO a", "WARN b"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        app.tabs[0].filter_context = Some(0);
        app.run_command("set-color --fg green -m").await.unwrap();
        let cc = app.tabs[0].log_manager.get_filters()[0]
            .color_config
            .as_ref()
            .unwrap();
        assert!(cc.match_only);
        assert_eq!(cc.fg, Some(ratatui::style::Color::Green));
    }

    #[tokio::test]
    async fn test_filter_with_editing_filter_id() {
        let mut app = make_app(&["INFO a", "WARN b"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        let old_id = app.tabs[0].log_manager.get_filters()[0].id;
        app.tabs[0].editing_filter_id = Some(old_id);

        // Adding a new filter while editing should remove the old one
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
        app.tabs[0].editing_filter_id = Some(old_id);

        app.run_command("exclude WARN").await.unwrap();
        let filters = app.tabs[0].log_manager.get_filters();
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].filter_type, FilterType::Exclude);
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
    async fn test_wrap_toggle() {
        let mut app = make_app(&["line"]).await;
        assert!(app.tab().wrap);
        app.run_command("wrap").await.unwrap();
        assert!(!app.tab().wrap);
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
            t.keybindings = app.keybindings.clone();
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
        assert!(app.tab().filtering_enabled);
        app.run_command("filtering").await.unwrap();
        assert!(!app.tab().filtering_enabled);
        app.run_command("filtering").await.unwrap();
        assert!(app.tab().filtering_enabled);
    }

    #[tokio::test]
    async fn test_hide_field() {
        let mut app = make_app(&["line"]).await;
        app.run_command("hide-field level").await.unwrap();
        assert!(app.tabs[0].hidden_fields.contains("level"));
    }

    #[tokio::test]
    async fn test_show_field() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].hidden_fields.insert("level".to_string());
        app.run_command("show-field level").await.unwrap();
        assert!(!app.tabs[0].hidden_fields.contains("level"));
    }

    #[tokio::test]
    async fn test_show_all_fields() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].hidden_fields.insert("level".to_string());
        app.tabs[0].hidden_fields.insert("msg".to_string());
        app.run_command("show-all-fields").await.unwrap();
        assert!(app.tabs[0].hidden_fields.is_empty());
    }

    #[tokio::test]
    async fn test_open_nonexistent_file() {
        let mut app = make_app(&["line"]).await;
        let result = app.run_command("open /nonexistent/path/xyz.log").await;
        assert!(result.is_err());
    }
}
