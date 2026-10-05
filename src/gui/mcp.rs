use crate::gui::{app::App, state::StatusMessage};
use crate::mcp::{McpCommand, McpServerHandle, McpSnapshot};
use gpui_kit::gpui::Context;

type SnapshotKey = (
    crate::shared::TabId,
    u64,
    Vec<usize>,
    Vec<crate::db::Comment>,
);

#[derive(Default)]
pub(super) struct Server {
    generation: u64,
    handle: Option<McpServerHandle>,
    receiver: Option<tokio::sync::mpsc::Receiver<McpCommand>>,
    snapshot: std::sync::Arc<tokio::sync::RwLock<McpSnapshot>>,
    key: Option<(
        crate::shared::TabId,
        u64,
        Vec<usize>,
        Vec<crate::db::Comment>,
    )>,
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.cancel.cancel();
        }
    }
}
fn snapshot(state: &crate::gui::state::GuiState) -> McpSnapshot {
    state
        .active_tab()
        .map(|tab| McpSnapshot {
            marked_lines: crate::mcp::build_marked_lines(&tab.file_reader, &tab.mark_manager),
            annotations: crate::mcp::build_annotations(&tab.comment_manager),
        })
        .unwrap_or_default()
}
fn apply_command(state: &mut crate::gui::state::GuiState, command: McpCommand) {
    if let McpCommand::ServerError(error) = command {
        state.notify(StatusMessage::Error(error));
        return;
    }
    let Some(tab) = state.active_tab_mut() else {
        return;
    };
    match command {
        McpCommand::ToggleMark(line) if line < tab.file_reader.line_count() => {
            tab.toggle_mark(line)
        }
        McpCommand::AddAnnotation { text, line_indices } => {
            let indices = line_indices
                .into_iter()
                .filter(|&i| i < tab.file_reader.line_count())
                .collect::<Vec<_>>();
            tab.mark_lines(&indices);
            tab.comment_manager.add(text, indices);
        }
        McpCommand::RemoveAnnotation(index) => tab.comment_manager.remove(index),
        _ => {}
    }
}
impl App {
    pub(super) fn configure_mcp(&mut self, port: Option<u16>, cx: &mut Context<Self>) {
        self.mcp.generation = self.mcp.generation.wrapping_add(1);
        let generation = self.mcp.generation;
        if let Some(handle) = self.mcp.handle.take() {
            handle.cancel.cancel();
        }
        self.mcp.receiver = None;
        self.mcp.key = None;
        let Some(port) = port else {
            self.state
                .notify(StatusMessage::Info("MCP server stopped.".into()));
            return;
        };
        let shared = self.mcp.snapshot.clone();
        let initial = snapshot(&self.state);
        cx.spawn(async move |this, cx| {
            let result = crate::gui::runtime::handle()
                .spawn(async move {
                    let (sender, receiver) = tokio::sync::mpsc::channel(64);
                    *shared.write().await = initial;
                    crate::mcp::start_mcp_server(port, shared, sender)
                        .await
                        .map(|handle| (handle, receiver))
                })
                .await;
            this.update_in(cx, |app, _, cx| {
                match result {
                    Ok(Ok((handle, receiver))) if app.mcp.generation == generation => {
                        app.mcp.handle = Some(handle);
                        app.mcp.receiver = Some(receiver);
                        app.state.notify(StatusMessage::Info(format!(
                            "MCP server listening on port {port}."
                        )));
                    }
                    Ok(Ok((handle, _))) => handle.cancel.cancel(),
                    Ok(Err(error)) => app.state.notify(StatusMessage::Error(error.to_string())),
                    Err(error) => app.state.notify(StatusMessage::Error(error.to_string())),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn poll_mcp(&mut self, cx: &mut Context<Self>) {
        if self.mcp.handle.is_none() {
            return;
        }
        let handle = crate::gui::runtime::handle();
        let _guard = handle.enter();
        let mut commands = Vec::new();
        if let Some(receiver) = &mut self.mcp.receiver {
            while let Ok(command) = receiver.try_recv() {
                commands.push(command);
            }
        }
        for command in commands {
            let failed = matches!(command, McpCommand::ServerError(_));
            apply_command(&mut self.state, command);
            if failed {
                if let Some(handle) = self.mcp.handle.take() {
                    handle.cancel.cancel();
                }
                self.mcp.receiver = None;
            }
            cx.notify();
        }
        let key = self.state.active_tab().map(|tab| {
            (
                tab.id,
                tab.cache.parse_gen,
                tab.mark_manager.get_indices(),
                tab.comment_manager.get().to_vec(),
            )
        });
        if key != self.mcp.key {
            self.mcp.key = key;
            let value = snapshot(&self.state);
            let shared = self.mcp.snapshot.clone();
            crate::gui::runtime::handle().spawn(async move {
                *shared.write().await = value;
            });
        }
    }
}
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn mcp_annotations_use_valid_source_rows_and_mark_them() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let manager = crate::db::LogManager::new(db.clone(), None).await;
        let mut state = crate::gui::state::GuiState::new(db);
        state.tabs.push(crate::shared::TabState::new(
            crate::ingestion::FileReader::from_bytes(b"a\nb\n".to_vec()),
            manager,
            "test".into(),
        ));
        super::apply_command(
            &mut state,
            crate::mcp::McpCommand::AddAnnotation {
                text: "note".into(),
                line_indices: vec![1, 99],
            },
        );
        assert_eq!(state.tabs[0].mark_manager.get_indices(), vec![1]);
        assert_eq!(super::snapshot(&state).annotations[0].line_indices, vec![1]);
        super::apply_command(&mut state, crate::mcp::McpCommand::RemoveAnnotation(0));
        assert!(super::snapshot(&state).annotations.is_empty());
    }
}
