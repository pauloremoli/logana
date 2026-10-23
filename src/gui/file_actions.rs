//! Native dialogs capture the source tab identity before yielding.
use crate::commands::Commands;
use crate::gui::{actions::Action, app::App, message::Message};
use gpui_kit::gpui::{Context, PathPromptOptions};

impl App {
    pub(super) fn prompt_filter_file(
        &self,
        action: Action,
        tab_id: crate::shared::TabId,
        cx: &mut Context<Self>,
    ) {
        let save = action == Action::SaveFilters;
        let save_prompt = save.then(|| {
            cx.prompt_for_new_path(
                &std::env::current_dir().unwrap_or_default(),
                Some("filters.json"),
            )
        });
        let open_prompt = (!save).then(|| {
            cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: None,
            })
        });
        cx.spawn(async move |this, cx| {
            let result = if let Some(prompt) = save_prompt {
                prompt
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
            } else {
                open_prompt
                    .unwrap()
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r.map_err(|e| e.to_string()))
                    .map(|paths| paths.and_then(|mut p| p.pop()))
            };
            this.update_in(cx, |app, window, cx| {
                let path = match result {
                    Ok(Some(path)) => path.to_string_lossy().into_owned(),
                    Ok(None) => return,
                    Err(error) => {
                        app.state
                            .notify(crate::gui::state::StatusMessage::Error(error));
                        cx.notify();
                        return;
                    }
                };
                let Some(index) = app.state.tabs.iter().position(|t| t.id == tab_id) else {
                    return;
                };
                let previous = app.state.active_tab;
                app.state.active_tab = index;
                let command = match action {
                    Action::SaveFilters => Commands::SaveFilters { path },
                    Action::LoadFilters => Commands::LoadFilters {
                        path,
                        append: false,
                    },
                    Action::AppendFilters => Commands::LoadFilters { path, append: true },
                    Action::ImportFilters => Commands::ImportFilters { path, append: true },
                    _ => unreachable!(),
                };
                app.dispatch(
                    Message::GuiCommand(command, "Filter file".into()),
                    window,
                    cx,
                );
                app.state.active_tab = previous;
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_filter_file_commands_round_trip_and_merge() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let mut manager = crate::db::LogManager::new(db, None).await;
        manager
            .add_filter_with_color(
                "error".into(),
                crate::filters::FilterType::Include,
                Default::default(),
            )
            .await;
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_string_lossy().into_owned();
        manager = crate::gui::update::execute_command(
            manager,
            Commands::SaveFilters { path: path.clone() },
            (0, 0, 0),
            None,
        )
        .await
        .unwrap();
        manager.clear_filters().await;
        manager = crate::gui::update::execute_command(
            manager,
            Commands::LoadFilters {
                path: path.clone(),
                append: false,
            },
            (0, 0, 0),
            None,
        )
        .await
        .unwrap();
        assert_eq!(manager.get_filters()[0].pattern, "error");
        manager = crate::gui::update::execute_command(
            manager,
            Commands::LoadFilters { path, append: true },
            (0, 0, 0),
            None,
        )
        .await
        .unwrap();
        assert_eq!(manager.get_filters().len(), 1);
    }
}
