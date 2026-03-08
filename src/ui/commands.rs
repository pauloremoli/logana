//! Command handler — implements all 30+ TUI commands.
//!
//! [`run_command`] is the single dispatch point called by
//! [`crate::ui::app::App::execute_command_str`]. Commands include: `filter`,
//! `exclude`, `set-color`, `export`, `wrap`, `open`, `docker`, `date-filter`,
//! `fields`, `select-fields`, `value-colors`, `tail`, and more.

use clap::Parser;
use std::collections::HashSet;

use crate::auto_complete::{expand_tilde, shell_split};
use crate::mode::command_mode::{CommandLine, Commands};
use crate::theme::Theme;
use crate::types::FilterType;

use super::App;

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
            }) => {
                // Check eligibility for incremental include BEFORE mutating state.
                // Safe when: not editing, filtering enabled, not marks-only, and no
                // pre-existing enabled include filters (current visible == all minus excludes).
                let can_incremental = self.tabs[self.active_tab].editing_filter_id.is_none() && {
                    let tab = &self.tabs[self.active_tab];
                    tab.filtering_enabled
                        && !tab.show_marks_only
                        && !tab.log_manager.get_filters().iter().any(|f| {
                            f.enabled
                                && f.filter_type == FilterType::Include
                                && !f.pattern.starts_with(crate::date_filter::DATE_PREFIX)
                        })
                };
                if let Some(old_id) = self.tabs[self.active_tab].editing_filter_id.take() {
                    self.tabs[self.active_tab]
                        .log_manager
                        .remove_filter(old_id)
                        .await;
                }
                self.tabs[self.active_tab]
                    .log_manager
                    .add_filter_with_color(
                        pattern.clone(),
                        FilterType::Include,
                        fg.as_deref(),
                        bg.as_deref(),
                        !line_mode,
                    )
                    .await;
                self.tabs[self.active_tab].scroll_offset = 0;
                // Incremental include — only re-check visible lines instead of
                // scanning the entire file again via refresh_visible/compute_visible.
                if can_incremental {
                    self.tabs[self.active_tab].apply_incremental_include(&pattern);
                } else {
                    self.tabs[self.active_tab].begin_filter_refresh();
                }
            }
            Some(Commands::Exclude { pattern }) => {
                if let Some(old_id) = self.tabs[self.active_tab].editing_filter_id.take() {
                    self.tabs[self.active_tab]
                        .log_manager
                        .remove_filter(old_id)
                        .await;
                    // Editing a filter means removing then re-adding; do a full refresh.
                    self.tabs[self.active_tab]
                        .log_manager
                        .add_filter_with_color(pattern, FilterType::Exclude, None, None, true)
                        .await;
                    self.tabs[self.active_tab].scroll_offset = 0;
                    self.tabs[self.active_tab].begin_filter_refresh();
                } else {
                    self.tabs[self.active_tab]
                        .log_manager
                        .add_filter_with_color(
                            pattern.clone(),
                            FilterType::Exclude,
                            None,
                            None,
                            true,
                        )
                        .await;
                    self.tabs[self.active_tab].scroll_offset = 0;
                    // Incremental exclude — only re-check visible lines instead of
                    // scanning the entire file again via refresh_visible/compute_visible.
                    self.tabs[self.active_tab].apply_incremental_exclude(&pattern);
                }
            }
            Some(Commands::SetColor { fg, bg, line_mode }) => {
                let selected_filter_index = self.tabs[self.active_tab].filter_context.unwrap_or(0);
                let filters = self.tabs[self.active_tab].log_manager.get_filters();
                if let Some(filter) = filters.get(selected_filter_index)
                    && filter.filter_type == FilterType::Include
                {
                    // When -l is not explicitly passed, preserve the filter's
                    // existing match_only setting instead of resetting it.
                    let match_only = if line_mode {
                        false
                    } else {
                        filter
                            .color_config
                            .as_ref()
                            .map(|cc| cc.match_only)
                            .unwrap_or(true)
                    };
                    let filter_id = filter.id;
                    self.tabs[self.active_tab]
                        .log_manager
                        .set_color_config(filter_id, fg.as_deref(), bg.as_deref(), match_only)
                        .await;
                    self.tabs[self.active_tab].begin_filter_refresh();
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
            Some(Commands::Export { path, template }) => {
                if path.is_empty() {
                    return Err("Path is required".to_string());
                }
                let tpl = crate::export::load_template(&template).map_err(|e| e.to_string())?;
                let tab = &self.tabs[self.active_tab];
                let data = crate::export::ExportData {
                    filename: tab.log_manager.source_file().unwrap_or("stdin"),
                    comments: tab.log_manager.get_comments(),
                    marked_indices: tab.log_manager.get_marked_indices(),
                    file_reader: &tab.file_reader,
                    parser: if tab.raw_mode {
                        None
                    } else {
                        tab.detected_format.as_deref()
                    },
                    field_layout: &tab.field_layout,
                    hidden_fields: &tab.hidden_fields,
                    show_keys: tab.show_keys,
                };
                let output = crate::export::render_export(&tpl, &data);
                std::fs::write(&path, output).map_err(|e| format!("Failed to write: {}", e))?;
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
                    self.tabs[self.active_tab].begin_filter_refresh();
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
                use crate::mode::value_colors_mode::{
                    ValueColorEntry, ValueColorGroup as VCGroup, ValueColorsMode,
                };
                let disabled = &self.tabs[self.active_tab].level_colors_disabled;
                let levels: Vec<(&str, &str, ratatui::style::Color)> = vec![
                    ("trace", "TRACE", self.theme.trace_fg),
                    ("debug", "DEBUG", self.theme.debug_fg),
                    ("info", "INFO", self.theme.info_fg),
                    ("notice", "NOTICE", self.theme.notice_fg),
                    ("warning", "WARNING", self.theme.warning_fg),
                    ("error", "ERROR", self.theme.error_fg),
                    ("fatal", "FATAL", self.theme.fatal_fg),
                ];
                let groups = vec![VCGroup {
                    label: "Log levels".to_string(),
                    children: levels
                        .into_iter()
                        .map(|(key, label, color)| ValueColorEntry {
                            key: key.to_string(),
                            label: label.to_string(),
                            color,
                            enabled: !disabled.contains(key),
                        })
                        .collect(),
                }];
                let original_disabled = disabled.clone();
                self.tabs[self.active_tab].mode =
                    Box::new(ValueColorsMode::new_level_colors(groups, original_disabled));
                return Ok(true);
            }
            Some(Commands::SetTheme { theme_name }) => {
                let theme_filename = format!("{}.json", theme_name.to_lowercase());
                self.theme = Theme::from_file(&theme_filename)
                    .map_err(|e| format!("Failed to load theme '{}': {}", theme_name, e))?;
                for tab in &mut self.tabs {
                    tab.render_cache_gen = tab.render_cache_gen.wrapping_add(1);
                    tab.render_line_cache.clear();
                }
            }
            Some(Commands::Open { path }) => {
                let path = expand_tilde(&path);
                if std::path::Path::new(&path).is_dir() {
                    let files = crate::ui::list_dir_files(&path);
                    if files.is_empty() {
                        return Err(format!("'{}' contains no files.", path));
                    }
                    self.tabs[self.active_tab].mode =
                        Box::new(crate::mode::app_mode::ConfirmOpenDirMode { dir: path, files });
                    return Ok(true);
                }
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
                self.tabs[self.active_tab].begin_filter_refresh();
            }
            Some(Commands::DisableFilters) => {
                self.tabs[self.active_tab]
                    .log_manager
                    .disable_all_filters()
                    .await;
                self.tabs[self.active_tab].begin_filter_refresh();
            }
            Some(Commands::EnableFilters) => {
                self.tabs[self.active_tab]
                    .log_manager
                    .enable_all_filters()
                    .await;
                self.tabs[self.active_tab].begin_filter_refresh();
            }
            Some(Commands::Filtering) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.filtering_enabled = !tab.filtering_enabled;
                tab.begin_filter_refresh();
            }
            Some(Commands::HideField { field }) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.hidden_fields.insert(field);
                tab.invalidate_parse_cache();
            }
            Some(Commands::ShowField { field }) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.hidden_fields.remove(&field);
                tab.invalidate_parse_cache();
            }
            Some(Commands::ShowAllFields) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.hidden_fields.clear();
                tab.invalidate_parse_cache();
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
                let process_representative = self.theme.process_colors.first().copied();
                let groups: Vec<VCGroup> = self
                    .theme
                    .value_colors
                    .grouped_categories(process_representative)
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
            Some(Commands::DateFilter {
                expr,
                fg,
                bg,
                line_mode,
            }) => {
                let tab = &self.tabs[self.active_tab];
                if tab.detected_format.is_none() {
                    return Err(
                        "No log format detected — date filter requires structured timestamps"
                            .to_string(),
                    );
                }
                let expression = expr.join(" ");
                // Validate the expression parses before storing.
                crate::date_filter::parse_date_filter(&expression)
                    .map_err(|e| format!("Invalid date filter: {}", e))?;
                let pattern = format!("{}{}", crate::date_filter::DATE_PREFIX, expression);
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
                        !line_mode,
                    )
                    .await;
                self.tabs[self.active_tab].begin_filter_refresh();
            }
            Some(Commands::Tail) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.tail_mode = !tab.tail_mode;
                if tab.tail_mode {
                    let new_count = tab.visible_indices.len();
                    tab.scroll_offset = new_count.saturating_sub(1);
                }
            }
            Some(Commands::ShowKeys) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.show_keys = true;
                tab.invalidate_parse_cache();
            }
            Some(Commands::HideKeys) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.show_keys = false;
                tab.invalidate_parse_cache();
            }
            Some(Commands::Raw) => {
                let tab = &mut self.tabs[self.active_tab];
                tab.raw_mode = !tab.raw_mode;
                tab.invalidate_parse_cache();
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
    use crate::ui::VisibleLines;
    use crate::ui::app::App;
    use std::sync::Arc;

    async fn await_filter_computations(app: &mut App) {
        for tab in &mut app.tabs {
            if let Some(h) = tab.filter_handle.take() {
                if let Ok(visible) = h.result_rx.await {
                    tab.visible_indices = VisibleLines::Filtered(visible);
                    if tab.visible_indices.is_empty() {
                        tab.scroll_offset = 0;
                    } else {
                        tab.scroll_offset = tab.scroll_offset.min(tab.visible_indices.len() - 1);
                    }
                }
            }
        }
    }

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

    // ── tail ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_tail_command_toggles_on() {
        let mut app = make_app(&["line1", "line2", "line3"]).await;
        assert!(!app.tab().tail_mode);
        app.run_command("tail").await.unwrap();
        assert!(app.tab().tail_mode);
    }

    #[tokio::test]
    async fn test_tail_command_toggles_off() {
        let mut app = make_app(&["line1", "line2"]).await;
        app.run_command("tail").await.unwrap();
        assert!(app.tab().tail_mode);
        app.run_command("tail").await.unwrap();
        assert!(!app.tab().tail_mode);
    }

    #[tokio::test]
    async fn test_tail_on_jumps_to_last_line() {
        let mut app = make_app(&["l1", "l2", "l3", "l4", "l5"]).await;
        app.tabs[0].scroll_offset = 0;
        app.run_command("tail").await.unwrap();
        // Enabling tail should immediately jump to the last visible line.
        assert_eq!(app.tab().scroll_offset, 4);
    }

    #[tokio::test]
    async fn test_tail_off_does_not_change_scroll() {
        let mut app = make_app(&["l1", "l2", "l3", "l4", "l5"]).await;
        // Enable then disable tail; the disabling should not move the cursor.
        app.run_command("tail").await.unwrap();
        assert_eq!(app.tab().scroll_offset, 4);
        app.tabs[0].scroll_offset = 2;
        app.run_command("tail").await.unwrap();
        assert!(!app.tab().tail_mode);
        assert_eq!(app.tab().scroll_offset, 2);
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
    async fn test_level_colors_opens_dialog() {
        let mut app = make_app(&["line1"]).await;
        let default_disabled: std::collections::HashSet<String> =
            ["trace", "debug", "info", "notice"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(app.tabs[0].level_colors_disabled, default_disabled);
        let result = app.run_command("level-colors").await.unwrap();
        assert!(result);
        assert!(matches!(
            app.tabs[0].mode.render_state(),
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
    async fn test_set_color_with_line_flag() {
        let mut app = make_app(&["INFO a", "WARN b"]).await;
        app.execute_command_str("filter INFO".to_string()).await;
        app.tabs[0].filter_context = Some(0);
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
        assert_eq!(app.tab().visible_indices.len(), 2);
    }

    #[tokio::test]
    async fn test_filter_incremental_include_second_filter_falls_back() {
        // Second include filter expands visible set → must fall back to full refresh.
        let mut app = make_app(&["error line", "info line", "error again"]).await;
        app.run_command("filter error").await.unwrap();
        assert_eq!(app.tab().visible_indices.len(), 2);
        // Adding a second include filter expands the visible set back.
        app.run_command("filter info").await.unwrap();
        await_filter_computations(&mut app).await;
        assert_eq!(app.tab().visible_indices.len(), 3);
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
    async fn test_show_keys_command() {
        let mut app = make_app(&["line"]).await;
        assert!(!app.tab().show_keys);
        app.run_command("show-keys").await.unwrap();
        assert!(app.tab().show_keys);
    }

    #[tokio::test]
    async fn test_hide_keys_command() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].show_keys = true;
        app.run_command("hide-keys").await.unwrap();
        assert!(!app.tab().show_keys);
    }

    #[tokio::test]
    async fn test_raw_toggle() {
        let mut app = make_app(&["line"]).await;
        assert!(!app.tab().raw_mode);
        app.run_command("raw").await.unwrap();
        assert!(app.tab().raw_mode);
        app.run_command("raw").await.unwrap();
        assert!(!app.tab().raw_mode);
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
            app.tabs[0].mode.render_state(),
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

    // ── export ─────────────────────────────────────────────────────────

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
        assert_eq!(app.tab().scroll_offset, 2);
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
        assert_eq!(app.tab().scroll_offset, 2);
    }

    #[tokio::test]
    async fn test_goto_line_with_whitespace() {
        let mut app = make_app(&["a", "b", "c", "d", "e"]).await;
        let result = app.run_command("  4  ").await;
        assert!(result.is_ok());
        assert_eq!(app.tab().scroll_offset, 3);
    }
}
