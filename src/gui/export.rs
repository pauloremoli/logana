use crate::gui::{app::App, state::StatusMessage};
use gpui_kit::gpui::Context;
use std::path::PathBuf;

impl App {
    pub(super) fn prompt_export(
        &self,
        tab_id: crate::shared::TabId,
        template: String,
        fields: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.state.tabs.iter().find(|t| t.id == tab_id) else {
            return;
        };
        let template = match crate::commands::load_template(&template) {
            Ok(t) => t,
            Err(_) => return,
        };
        let refs = fields
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>();
        let output = crate::commands::render_export(
            &template,
            &crate::commands::ExportData {
                filename: tab.log_manager.source_file().unwrap_or("stream"),
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
                footer_fields: &refs,
            },
        );
        let source = tab.log_manager.source_file().map(PathBuf::from);
        let prompt = cx.prompt_for_new_path(
            &std::env::current_dir().unwrap_or_default(),
            Some("analysis.md"),
        );
        cx.spawn(async move |this, cx| {
            let result: anyhow::Result<Option<PathBuf>> = async {
                let Some(path) = prompt.await?? else {
                    return Ok(None);
                };
                crate::gui::runtime::handle()
                    .spawn_blocking(move || {
                        anyhow::ensure!(
                            !source.is_some_and(|source| crate::gui::save_logs::same_file(
                                &source, &path
                            )),
                            "Output is the same file as the input source"
                        );
                        std::fs::write(&path, output)?;
                        Ok(Some(path))
                    })
                    .await?
            }
            .await;
            this.update_in(cx, |app, _, cx| {
                match result {
                    Ok(Some(path)) => app
                        .state
                        .notify(StatusMessage::Info(format!("Exported {}", path.display()))),
                    Ok(None) => return,
                    Err(error) => app.state.notify(StatusMessage::Error(format!(
                        "Cannot export analysis: {error}"
                    ))),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use crate::gui::{
        actions::{self, Action},
        state::GuiState,
    };
    #[tokio::test]
    async fn export_draft_has_all_template_fields_and_preserves_text() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let manager = crate::db::LogManager::new(db.clone(), None).await;
        let mut state = GuiState::new(db);
        state.tabs.push(crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"test\n".to_vec()),
            manager,
            "test".into(),
        ));
        actions::open(&mut state, Action::Export, None, None);
        let editor = state.editor.as_mut().unwrap();
        assert_eq!(editor.values.len(), 6 + editor.fields.len());
        assert!(editor.fields.iter().any(|(key, _)| key == "conclusion"));
        for value in &mut editor.values[6..] {
            *value = "α\nβ".into();
        }
        let effect = actions::apply(&mut state);
        let crate::gui::effect::Effect::ExportDialog { fields, .. } = effect else {
            panic!("export dialog expected")
        };
        assert!(fields.iter().all(|(_, value)| value == "α\nβ"));
    }
}
