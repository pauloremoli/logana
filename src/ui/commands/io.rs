use crate::auto_complete::expand_tilde;
use crate::ui::App;
use crate::utils::filesystem::list_dir_files;
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

    pub(super) fn cmd_save(&mut self, path: String) -> Result<bool, String> {
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
        Ok(false)
    }

    pub(super) fn cmd_export(&mut self, path: String, template: String) -> Result<bool, String> {
        if path.is_empty() {
            return Err("Path is required".to_string());
        }
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
        };
        let output = crate::commands::render_export(&tpl, &data);
        let file = std::fs::File::create(&path).map_err(|e| format!("Failed to write: {}", e))?;
        let mut writer = BufWriter::new(file);
        writer
            .write_all(output.as_bytes())
            .and_then(|_| writer.flush())
            .map_err(|e| format!("Failed to write: {}", e))?;
        Ok(false)
    }

    pub(super) fn cmd_save_filters(&mut self, path: String) -> Result<bool, String> {
        if !path.is_empty() {
            self.tabs[self.active_tab]
                .log_manager
                .save_filters(&path)
                .map_err(|e| format!("Failed to save filters: {}", e))?;
        }
        Ok(false)
    }

    pub(super) async fn cmd_load_filters(&mut self, path: String) -> Result<bool, String> {
        if !path.is_empty() {
            self.tabs[self.active_tab]
                .log_manager
                .load_filters(&path)
                .await
                .map_err(|e| format!("Failed to load filters: {}", e))?;
            self.tabs[self.active_tab].begin_filter_refresh();
        }
        Ok(false)
    }

    pub(super) async fn cmd_open(&mut self, path: String) -> Result<bool, String> {
        let path = expand_tilde(&path);
        if std::path::Path::new(&path).is_dir() {
            let files = list_dir_files(&path);
            if files.is_empty() {
                return Err(format!("'{}' contains no files.", path));
            }
            self.tabs[self.active_tab].interaction.mode =
                Box::new(crate::mode::app_mode::ConfirmOpenDirMode {
                    dir: path,
                    files: std::sync::Arc::new(files),
                });
            return Ok(true);
        }
        if crate::ingestion::detect_archive_type(&path).is_some() {
            self.begin_archive_extraction(&path).await;
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
