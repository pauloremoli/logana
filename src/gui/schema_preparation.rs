use crate::gui::{app::App, effect::Effect, message::Message};
use crate::ingestion::{FileReader, format_detect::DetectedFormat};
use gpui_kit::gpui::Context;

pub struct PreparedSchema {
    pub reader: FileReader,
    pub metadata: Result<DetectedFormat, String>,
}

pub(super) fn prepare(reader: FileReader, name: &str) -> PreparedSchema {
    let metadata = (|| {
        let format = if name == "none" {
            None
        } else {
            Some(crate::parser::resolve_parser(name)?)
        };
        let (continuation_map, year_map) =
            crate::ingestion::format_detect::derive_format_structures(&reader, format.as_deref());
        Ok(DetectedFormat {
            format,
            continuation_map,
            year_map,
        })
    })();
    PreparedSchema { reader, metadata }
}

impl App {
    pub(super) fn prepare_schema(
        &mut self,
        request_id: u64,
        tab_id: crate::shared::TabId,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.state.tabs.iter().find(|tab| tab.id == tab_id) else {
            self.state.notify(crate::gui::state::StatusMessage::Error(
                "The source tab was closed.".into(),
            ));
            return;
        };
        let reader = tab.file_reader.clone();
        let schema = name.clone();
        let worker = cx
            .background_executor()
            .spawn(async move { prepare(reader, &schema) });
        cx.spawn(async move |this, cx| {
            let prepared = worker.await;
            this.update_in(cx, |app, window, cx| {
                app.dispatch(
                    Message::SchemaPrepared(request_id, tab_id, name, Box::new(prepared)),
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }
}

pub(super) fn install(
    state: &mut crate::gui::state::GuiState,
    request_id: u64,
    tab_id: crate::shared::TabId,
    name: String,
    prepared: PreparedSchema,
) -> Effect {
    use crate::gui::palette::ExecutionState;
    if !state.palette.outcome.as_ref().is_some_and(|outcome| {
        outcome.request_id == request_id && outcome.state == ExecutionState::Running
    }) {
        return Effect::None;
    }
    let Some(tab) = state.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
        super::update::finish_palette(
            state,
            request_id,
            ExecutionState::Error,
            "The source tab was closed.".into(),
        );
        return Effect::None;
    };
    if !tab.file_reader.same_snapshot(&prepared.reader) {
        return Effect::PrepareSchema {
            request_id,
            tab_id,
            name,
        };
    }
    match prepared.metadata {
        Ok(metadata) => {
            tab.display.format = metadata.format;
            tab.continuation_map = metadata.continuation_map;
            tab.year_map = metadata.year_map;
            tab.filter.overridden_groups.clear();
            if name != "none" {
                tab.display.hidden_fields.clear();
            }
            tab.invalidate_parse_cache();
            tab.begin_filter_refresh_background();
            super::update::finish_palette(
                state,
                request_id,
                ExecutionState::Success,
                format!("Schema changed to {name}."),
            );
        }
        Err(error) => {
            if let Some(editor) = &mut state.editor {
                editor.error = Some(error.clone());
            }
            super::update::finish_palette(state, request_id, ExecutionState::Error, error);
        }
    }
    Effect::None
}
