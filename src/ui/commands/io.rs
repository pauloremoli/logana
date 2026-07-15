use crate::commands::auto_complete::expand_tilde;
use crate::ui::App;
use std::io::{BufWriter, Write};

impl App {
    pub(super) fn cmd_export_marked(&mut self, path: String) -> Result<bool, String> {
        if !path.is_empty() {
            let expanded = expand_tilde(&path);
            let tab = &self.tabs[self.active_tab];
            if let Some(src) = tab.log_manager.source_file()
                && crate::headless::same_file(src, std::path::Path::new(&expanded))
            {
                return Err(format!(
                    "Output path '{}' is the same as the input file",
                    expanded
                ));
            }
            let marked_lines = tab.mark_manager.get_lines(&tab.file_reader);
            let file = std::fs::File::create(&expanded)
                .map_err(|e| format!("Failed to write '{}': {}", expanded, e))?;
            let mut writer = BufWriter::with_capacity(8 * 1024 * 1024, file);
            for line in marked_lines {
                writer
                    .write_all(line)
                    .and_then(|_| writer.write_all(b"\n"))
                    .map_err(|e| format!("Failed to write '{}': {}", expanded, e))?;
            }
            writer
                .flush()
                .map_err(|e| format!("Failed to write '{}': {}", expanded, e))?;
        }
        Ok(false)
    }

    pub(super) async fn cmd_save(&mut self, path: String) -> Result<bool, String> {
        if path.is_empty() {
            return Err("Path is required".to_string());
        }
        let expanded = expand_tilde(&path);
        let tab = &self.tabs[self.active_tab];
        if let Some(src) = tab.log_manager.source_file()
            && crate::headless::same_file(src, std::path::Path::new(&expanded))
        {
            return Err(format!(
                "Output path '{}' is the same as the input file",
                expanded
            ));
        }
        let file = std::fs::File::create(&expanded)
            .map_err(|e| format!("Failed to write '{}': {}", expanded, e))?;
        let mut writer = BufWriter::with_capacity(8 * 1024 * 1024, file);
        for file_idx in tab.filter.visible_indices.iter() {
            writer
                .write_all(tab.file_reader.get_line(file_idx))
                .and_then(|_| writer.write_all(b"\n"))
                .map_err(|e| format!("Failed to write '{}': {}", expanded, e))?;
        }
        writer
            .flush()
            .map_err(|e| format!("Failed to write '{}': {}", expanded, e))?;

        // A temp-backed tab (an extracted archive file, or a picker-triggered
        // merge — see `TabState::is_temp_backed`) has nothing permanent
        // behind it; now that its content has been saved somewhere real,
        // that's its new home. Re-point the tab at it instead of leaving it
        // tied to a temp file the user can no longer see or find again.
        if self.tabs[self.active_tab].is_temp_backed() {
            self.switch_tab_to_saved_file(self.active_tab, expanded)
                .await;
        }
        Ok(false)
    }

    /// Re-points a temp-backed tab at the file it was just saved to: drops
    /// the temp copies (`archive_temp`/`merge_source_temps`/`merged_temp`)
    /// and, for a picker-triggered merge, the multi-source `merged` state
    /// too (the saved file is a single flat file now, not several sources to
    /// track), then reloads the tab's content from the saved path — same as
    /// opening it fresh, including live tail-watching for future growth.
    async fn switch_tab_to_saved_file(&mut self, tab_idx: usize, path: String) {
        let abs_path = std::fs::canonicalize(&path)
            .ok()
            .and_then(|c| c.to_str().map(|s| s.to_string()))
            .unwrap_or(path);
        let title = std::path::Path::new(&abs_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&abs_path)
            .to_string();

        self.tabs[tab_idx].title = title;
        self.tabs[tab_idx].archive_temp = None;
        self.tabs[tab_idx].merge_source_temps = Vec::new();
        self.tabs[tab_idx].merged_temp = None;
        self.tabs[tab_idx].merged = None;
        self.tabs[tab_idx].log_manager =
            crate::db::LogManager::new(self.db.clone(), Some(abs_path.clone())).await;

        self.begin_file_load(
            abs_path,
            crate::ui::LoadContext::ReplaceTab { tab_idx },
            None,
            false,
        )
        .await;
    }

    pub(super) fn cmd_export(&mut self, path: String, template: String) -> Result<bool, String> {
        if path.is_empty() {
            return Err("Path is required".to_string());
        }
        let path = expand_tilde(&path);
        let tab = &self.tabs[self.active_tab];
        if let Some(src) = tab.log_manager.source_file()
            && crate::headless::same_file(src, std::path::Path::new(&path))
        {
            return Err(format!(
                "Output path '{}' is the same as the input file",
                path
            ));
        }
        let tpl = crate::commands::load_template(&template).map_err(|e| e.to_string())?;
        let fields = crate::commands::extract_user_fields(&tpl);
        if !fields.is_empty() {
            self.tab_mut().interaction.mode = Box::new(
                crate::mode::export_footer_mode::ExportFooterMode::new(path, template, fields),
            );
            return Ok(true);
        }
        self.write_export(&path, &template, &[])
    }

    pub(crate) fn cmd_export_with_footer(
        &mut self,
        path: String,
        template_name: String,
        footer_fields: Vec<(String, String)>,
    ) {
        let refs: Vec<(&str, &str)> = footer_fields
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if let Err(e) = self.write_export(&path, &template_name, &refs) {
            self.tab_mut().interaction.command_error = Some(e);
        }
    }

    fn write_export(
        &mut self,
        path: &str,
        template: &str,
        footer_fields: &[(&str, &str)],
    ) -> Result<bool, String> {
        let tpl = crate::commands::load_template(template).map_err(|e| e.to_string())?;
        let tab = &self.tabs[self.active_tab];
        let data = crate::commands::ExportData {
            filename: tab.log_manager.source_file().unwrap_or("stdin"),
            comments: tab.comment_manager.get(),
            marked_indices: tab.mark_manager.get_indices(),
            file_reader: &tab.file_reader,
            parser: if tab.display.raw_mode {
                None
            } else {
                tab.display.format.as_deref()
            },
            field_layout: &tab.display.field_layout,
            hidden_fields: &tab.display.hidden_fields,
            show_keys: tab.display.show_keys,
            footer_fields,
        };
        let output = crate::commands::render_export(&tpl, &data);
        let file = std::fs::File::create(path)
            .map_err(|e| format!("Failed to write '{}': {}", path, e))?;
        let mut writer = BufWriter::new(file);
        writer
            .write_all(output.as_bytes())
            .and_then(|_| writer.flush())
            .map_err(|e| format!("Failed to write '{}': {}", path, e))?;
        Ok(false)
    }

    pub(super) fn cmd_save_filters(&mut self, path: String) -> Result<bool, String> {
        if !path.is_empty() {
            let expanded = expand_tilde(&path);
            self.tabs[self.active_tab]
                .log_manager
                .save_filters(&expanded)
                .map_err(|e| format!("Failed to save filters to '{}': {}", expanded, e))?;
        }
        Ok(false)
    }

    pub(super) async fn cmd_load_filters(&mut self, path: String) -> Result<bool, String> {
        if !path.is_empty() {
            let expanded = expand_tilde(&path);
            self.tabs[self.active_tab]
                .log_manager
                .load_filters(&expanded)
                .await
                .map_err(|e| format!("Failed to load filters from '{}': {}", expanded, e))?;
            self.tabs[self.active_tab].begin_filter_refresh();
        }
        Ok(false)
    }

    pub(super) async fn cmd_open(&mut self, path: String) -> Result<bool, String> {
        let path = expand_tilde(&path);
        if std::path::Path::new(&path).is_dir() {
            let tree = crate::ingestion::list_directory_tree(&path)?;
            self.tabs[self.active_tab].interaction.mode = Box::new(
                crate::mode::archive_picker_mode::ArchivePickerMode::new(tree, path),
            );
            return Ok(true);
        }
        if crate::ingestion::detect_archive_type(&path).is_some() {
            self.begin_archive_listing(&path).await;
            return Ok(true);
        }
        self.open_file(&path).await?;
        Ok(false)
    }

    pub(super) fn cmd_close_tab(&mut self) -> Result<bool, String> {
        if self.tabs.len() <= 1 {
            return Err("Cannot close last tab. Use 'q' to quit.".to_string());
        }
        self.tabs.remove(self.active_tab);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        Ok(false)
    }
}
