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
                    let filter_id = filter.id;
                    self.tabs[self.active_tab]
                        .log_manager
                        .set_color_config(filter_id, fg.as_deref(), bg.as_deref(), m)
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
