use crate::gui::{app::App, message::Message};
use gpui_kit::gpui::Context;

#[derive(Debug)]
pub enum Source {
    Dlt { host: String, port: u16 },
    Otel { port: u16, grpc: bool },
    Run { program: String, args: Vec<String> },
}
impl Source {
    async fn open(
        self,
        db: std::sync::Arc<crate::db::Database>,
    ) -> Result<Box<crate::shared::TabState>, String> {
        let (label, connect, no_retry) = match self {
            Self::Dlt { host, port } => (
                format!("dlt://{host}:{port}"),
                crate::shared::dlt_connect_fn(host, port),
                false,
            ),
            Self::Otel { port, grpc } => {
                if grpc {
                    (
                        format!("otlp-grpc://{port}"),
                        crate::shared::otlp_grpc_connect_fn(port),
                        false,
                    )
                } else {
                    (
                        format!("otlp://{port}"),
                        crate::shared::otlp_connect_fn(port),
                        false,
                    )
                }
            }
            Self::Run { program, args } => {
                let label = format!(
                    "run:{}",
                    std::iter::once(program.as_str())
                        .chain(args.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                (label, crate::shared::run_connect_fn(program, args), true)
            }
        };
        let connection = connect().await?;
        let watch = crate::shared::watch_state_from_connection(connection);
        let reader = crate::ingestion::FileReader::new(
            watch.reader_path.to_str().ok_or("Invalid source path")?,
        )
        .map_err(|e| e.to_string())?;
        let manager = crate::db::LogManager::new(db, Some(label.clone())).await;
        let mut tab = crate::shared::TabState::new(reader, manager, label);
        tab.detect_and_apply_format();
        tab.stream.watch = Some(watch);
        tab.stream.tail_mode = true;
        tab.stream.no_retry = no_retry;
        Ok(Box::new(tab))
    }
}
impl App {
    pub(super) fn connect_source(&self, source: Source, cx: &mut Context<Self>) {
        let db = self.state.db.clone();
        cx.spawn(async move |this, cx| {
            let result = crate::gui::runtime::handle()
                .spawn(source.open(db))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            this.update_in(cx, |app, window, cx| {
                app.dispatch(Message::SourceLoaded(result), window, cx)
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn subprocess_output_is_a_watched_gui_source() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let mut tab = Source::Run {
            program: "printf".into(),
            args: vec!["hello α\\n".into()],
        }
        .open(db)
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let watch = tab.stream.watch.as_ref().unwrap();
        tab.file_reader =
            crate::ingestion::FileReader::new(watch.reader_path.to_str().unwrap()).unwrap();
        assert_eq!(tab.file_reader.get_line(0), "hello α".as_bytes());
        assert!(tab.stream.no_retry);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod lifecycle_tests {
    #[tokio::test]
    async fn dropping_the_watch_stops_the_child_process() {
        let (mut rx, file) = crate::ingestion::FileReader::spawn_process_stream(
            "sh",
            &["-c", "echo $$; exec sleep 60"],
            false,
        )
        .await
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), rx.changed())
            .await
            .unwrap()
            .unwrap();
        let pid = std::fs::read_to_string(file.path())
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        drop(rx);
        for _ in 0..50 {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("child process was not stopped");
    }
}

#[cfg(test)]
mod otel_tests {
    #[tokio::test]
    async fn receiver_editor_validates_port_and_protocol() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let mut state = crate::gui::state::GuiState::new(db);
        crate::gui::actions::open(&mut state, crate::gui::actions::Action::Otel, None, None);
        state.editor.as_mut().unwrap().values[0] = "0".into();
        assert!(matches!(
            crate::gui::actions::apply(&mut state),
            crate::gui::effect::Effect::None
        ));
        assert!(state.editor.as_ref().unwrap().error.is_some());
        state.editor.as_mut().unwrap().values[0] = "4317".into();
        state.editor.as_mut().unwrap().values[1] = "grpc".into();
        assert!(matches!(
            crate::gui::actions::apply(&mut state),
            crate::gui::effect::Effect::ConnectSource(super::Source::Otel {
                port: 4317,
                grpc: true
            })
        ));
    }
}
