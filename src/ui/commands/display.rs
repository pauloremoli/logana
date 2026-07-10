use crate::db::{AppSettingsStore, SettingsKey};
use crate::theme::Theme;
use crate::ui::App;
use crate::ui::SidebarSide;

impl App {
    pub(super) async fn cmd_wrap(&mut self) {
        self.display.wrap = !self.display.wrap;
        for tab in &mut self.tabs {
            tab.display.wrap = self.display.wrap;
        }
        let _ = self
            .db
            .save_app_setting(
                SettingsKey::Wrap,
                if self.display.wrap { "true" } else { "false" },
            )
            .await;
    }

    pub(super) async fn cmd_line_numbers(&mut self) {
        self.display.show_line_numbers = !self.display.show_line_numbers;
        for tab in &mut self.tabs {
            tab.display.show_line_numbers = self.display.show_line_numbers;
        }
        let _ = self
            .db
            .save_app_setting(
                SettingsKey::ShowLineNumbers,
                if self.display.show_line_numbers {
                    "true"
                } else {
                    "false"
                },
            )
            .await;
    }

    pub(super) fn cmd_level_colors(&mut self) -> Result<bool, String> {
        use crate::mode::value_colors_mode::{
            ValueColorEntry, ValueColorGroup as VCGroup, ValueColorsMode,
        };
        let disabled = &self.tabs[self.active_tab].display.level_colors_disabled;
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
        self.tabs[self.active_tab].interaction.mode =
            Box::new(ValueColorsMode::new_level_colors(groups, original_disabled));
        Ok(true)
    }

    pub(super) async fn cmd_set_theme(&mut self, theme_name: String) -> Result<bool, String> {
        let theme_filename = format!("{}.json", theme_name.to_lowercase());
        self.theme = Theme::from_file(&theme_filename)
            .map_err(|e| format!("Failed to load theme '{}': {}", theme_name, e))?;
        for tab in &mut self.tabs {
            tab.cache.render_gen = tab.cache.render_gen.wrapping_add(1);
            tab.cache.render_line.clear();
        }
        let _ = self
            .db
            .save_app_setting(SettingsKey::Theme, &theme_name)
            .await;
        Ok(false)
    }

    pub(super) async fn cmd_sidebar_position(&mut self, side: SidebarSide) -> Result<bool, String> {
        self.display.sidebar_side = side;
        for tab in &mut self.tabs {
            tab.display.sidebar_side = side;
        }
        let _ = self
            .db
            .save_app_setting(
                SettingsKey::SidebarLeft,
                if side.is_left() { "true" } else { "false" },
            )
            .await;
        Ok(false)
    }

    pub(super) fn cmd_hide_field(&mut self, field: String) -> Result<bool, String> {
        let resolved = super::resolve_hide_field_arg(&mut self.tabs[self.active_tab], &field)?;
        let tab = &mut self.tabs[self.active_tab];
        tab.display.hidden_fields.insert(resolved);
        tab.invalidate_parse_cache();
        Ok(false)
    }

    pub(super) fn cmd_show_field(&mut self, field: String) {
        let tab = &mut self.tabs[self.active_tab];
        tab.display.hidden_fields.remove(&field);
        tab.invalidate_parse_cache();
    }

    pub(super) fn cmd_show_all_fields(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.display.hidden_fields.clear();
        tab.invalidate_parse_cache();
    }

    pub(super) fn cmd_select_fields(&mut self) -> Result<bool, String> {
        let tab = &mut self.tabs[self.active_tab];
        let all_names = tab.collect_field_names();
        if all_names.is_empty() {
            return Err("No structured fields found in visible lines".to_string());
        }
        let saved_order = &tab.display.field_layout.columns;
        let fields: Vec<(String, bool)> = match saved_order {
            Some(order) => {
                let mut ordered: Vec<(String, bool)> = order
                    .iter()
                    .filter(|n| all_names.contains(n))
                    .map(|n| (n.clone(), !tab.display.hidden_fields.contains(n.as_str())))
                    .collect();
                for name in &all_names {
                    if !order.contains(name) {
                        ordered.push((
                            name.clone(),
                            !tab.display.hidden_fields.contains(name.as_str()),
                        ));
                    }
                }
                ordered
            }
            None => all_names
                .into_iter()
                .map(|n| {
                    let enabled = !tab.display.hidden_fields.contains(n.as_str());
                    (n, enabled)
                })
                .collect(),
        };
        let original_layout = tab.display.field_layout.clone();
        let original_hidden_fields = tab.display.hidden_fields.clone();
        tab.interaction.mode = Box::new(crate::mode::select_fields_mode::SelectFieldsMode::new(
            fields,
            original_layout,
            original_hidden_fields,
        ));
        Ok(true)
    }

    pub(super) fn cmd_merge(&mut self) -> Result<bool, String> {
        self.handle_open_merge_select();
        Ok(true)
    }

    pub(super) fn cmd_show_keys(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.display.show_keys = true;
        tab.invalidate_parse_cache();
    }

    pub(super) fn cmd_hide_keys(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.display.show_keys = false;
        tab.invalidate_parse_cache();
    }

    pub(super) fn cmd_raw(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.display.raw_mode = !tab.display.raw_mode;
        tab.invalidate_parse_cache();
    }

    pub(super) async fn cmd_schema(&mut self, name: Option<String>) -> Result<bool, String> {
        let tab = &self.tabs[self.active_tab];
        match name {
            None => {
                let schema_name = tab
                    .display
                    .format
                    .as_deref()
                    .map(|f| f.name().to_string())
                    .unwrap_or_else(|| "none".to_string());
                return Err(format!("active schema: {schema_name}"));
            }
            Some(schema_name) => {
                let schema = crate::config::custom_schemas()
                    .iter()
                    .find(|s| s.name == schema_name)
                    .cloned();
                let cfg =
                    schema.ok_or_else(|| format!("no custom schema named '{schema_name}'"))?;
                let parser = crate::parser::CustomParser::from_config(&cfg)
                    .map_err(|e| format!("invalid schema '{schema_name}': {e}"))?;
                let tab = &mut self.tabs[self.active_tab];
                tab.display.format = Some(std::sync::Arc::new(parser));
                tab.invalidate_parse_cache();
            }
        }
        Ok(false)
    }
}
