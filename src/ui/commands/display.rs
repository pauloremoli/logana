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
        tab.display.field_layout.columns = None;
        tab.invalidate_parse_cache();
    }

    pub(super) fn cmd_select_fields(&mut self) -> Result<bool, String> {
        let tab = &mut self.tabs[self.active_tab];
        let all_names = tab.collect_field_names();
        if all_names.is_empty() {
            return Err("No structured fields found in visible lines".to_string());
        }
        let default_order = all_names.clone();
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
            default_order,
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
                let custom = crate::config::custom_schemas()
                    .iter()
                    .find(|s| s.name == schema_name)
                    .cloned();
                let parser: std::sync::Arc<dyn crate::parser::LogFormatParser> = match custom {
                    Some(cfg) => std::sync::Arc::new(
                        crate::parser::CustomParser::from_config(&cfg)
                            .map_err(|e| format!("invalid schema '{schema_name}': {e}"))?,
                    ),
                    None => crate::parser::find_builtin_parser(&schema_name)
                        .map(std::sync::Arc::from)
                        .ok_or_else(|| format!("no schema named '{schema_name}'"))?,
                };
                let tab = &mut self.tabs[self.active_tab];
                tab.display.format = Some(parser);
                tab.invalidate_parse_cache();
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
    use crate::theme::Theme;
    use std::sync::Arc;

    async fn make_app() -> App {
        let db = Arc::new(Database::in_memory().await.unwrap());
        let fr = FileReader::from_bytes(b"line\n".to_vec());
        let lm = LogManager::new(db, None).await;
        App::builder(lm, fr, Theme::default(), Arc::new(Keybindings::default()))
            .build()
            .await
    }

    #[tokio::test]
    async fn test_cmd_schema_no_arg_shows_active_schema_name() {
        let mut app = make_app().await;
        let result = app.cmd_schema(None).await;
        assert_eq!(result, Err("active schema: none".to_string()));
    }

    #[tokio::test]
    async fn test_cmd_schema_unknown_name_returns_error() {
        let mut app = make_app().await;
        let result = app.cmd_schema(Some("nonexistent".to_string())).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("nonexistent"));
    }

    #[tokio::test]
    async fn test_cmd_schema_builtin_name_switches_format() {
        let mut app = make_app().await;
        let result = app.cmd_schema(Some("syslog".to_string())).await;
        assert_eq!(result, Ok(false));
        let tab = &app.tabs[app.active_tab];
        assert_eq!(
            tab.display.format.as_deref().map(|f| f.name()),
            Some("syslog")
        );
    }
}
