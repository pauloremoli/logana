use crate::gui::{app::App, message::Message};
use gpui_kit::gpui::Context;

#[derive(Debug)]
pub enum Source {
    Stdin,
    Docker { container: String, name: String },
    Dlt { host: String, port: u16 },
    NamedDlt(crate::config::DltDevice),
    Otel { port: u16, grpc: bool },
    Run { program: String, args: Vec<String> },
}
impl Source {
    async fn open(
        self,
        db: std::sync::Arc<crate::db::Database>,
    ) -> Result<Box<crate::shared::TabState>, String> {
        let title = match &self {
            Self::NamedDlt(device) => Some(device.name.clone()),
            _ => None,
        };
        let (label, connect, no_retry) = match self {
            Self::Stdin => {
                let connect: crate::shared::ConnectFn = std::sync::Arc::new(|| {
                    Box::pin(async { Ok(crate::ingestion::FileReader::stream_stdin().await) })
                });
                ("stdin".into(), connect, true)
            }
            Self::Docker { container, name } => (
                format!("docker://{name}"),
                crate::shared::docker_connect_fn(container),
                false,
            ),
            Self::Dlt { host, port } => (
                format!("dlt://{host}:{port}"),
                crate::shared::dlt_connect_fn(host, port),
                false,
            ),
            Self::NamedDlt(device) => {
                let port = device.port.unwrap_or(crate::config::DEFAULT_DLT_PORT);
                (
                    format!("dlt://{}:{port}", device.host),
                    crate::shared::dlt_connect_fn(device.host, port),
                    false,
                )
            }
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
        if let Some(title) = title {
            tab.title = title;
        }
        tab.detect_and_apply_format();
        tab.stream.watch = Some(watch);
        tab.stream.tail_mode = true;
        tab.stream.no_retry = no_retry;
        Ok(Box::new(tab))
    }
}
impl App {
    pub(super) fn connect_source(&self, source: Source, cx: &mut Context<Self>) {
        self.connect_source_with_args(source, None, cx);
    }
    pub(super) fn connect_source_with_args(
        &self,
        source: Source,
        args: Option<crate::cli::Args>,
        cx: &mut Context<Self>,
    ) {
        let db = self.state.db.clone();
        let background = crate::gui::theme::GuiPalette::from_theme(&self.state.theme).background;
        cx.spawn(async move |this, cx| {
            let result = crate::gui::runtime::handle()
                .spawn(async move {
                    let mut tab = source.open(db).await?;
                    if let Some(args) = args {
                        tab.log_manager =
                            crate::gui::startup::apply_filters(tab.log_manager, args, background)
                                .await?;
                    }
                    Ok::<_, String>(tab)
                })
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
    #[tokio::test]
    async fn dlt_editor_carries_device_name_into_connection() {
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let mut state = crate::gui::state::GuiState::new(db);
        crate::gui::actions::open(&mut state, crate::gui::actions::Action::Dlt, None, None);
        state.editor.as_mut().unwrap().values[3] = "mydevice".into();
        let effect = crate::gui::actions::apply(&mut state);
        let crate::gui::effect::Effect::ConnectSource(Source::NamedDlt(device)) = effect else {
            panic!("expected a named DLT connection");
        };
        assert_eq!(device.name, "mydevice");
    }
    #[tokio::test]
    async fn named_dlt_tab_uses_name_and_keeps_endpoint_identity() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let tab = Source::NamedDlt(crate::config::DltDevice {
            name: "mydevice".into(),
            host: "127.0.0.1".into(),
            port: Some(port),
        })
        .open(db)
        .await
        .unwrap();
        assert_eq!(tab.title, "mydevice");
        assert_eq!(
            tab.log_manager.source_file(),
            Some(format!("dlt://127.0.0.1:{port}").as_str())
        );
        assert!(tab.stream.watch.is_some());
    }

    #[tokio::test]
    async fn saving_named_dlt_device_validates_and_roundtrips_configuration() {
        use crate::gui::{
            actions::{self, Action},
            effect::Effect,
            message::Message,
            update::update,
        };
        let db = std::sync::Arc::new(crate::db::Database::in_memory().await.unwrap());
        let mut state = crate::gui::state::GuiState::new(db);
        actions::open(&mut state, Action::Dlt, None, None);
        state.editor.as_mut().unwrap().members.clear();
        state.editor.as_mut().unwrap().conditions.clear();
        assert!(matches!(
            update(&mut state, Message::SaveDltDevice),
            Effect::None
        ));
        assert!(
            state
                .editor
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("name")
        );
        state.editor.as_mut().unwrap().values[3] = " mydevice ".into();
        state.editor.as_mut().unwrap().values[1] = "0".into();
        assert!(matches!(
            update(&mut state, Message::SaveDltDevice),
            Effect::None
        ));
        state.editor.as_mut().unwrap().values[1] = "3490".into();
        let Effect::SaveDltDevice { epoch, device } = update(&mut state, Message::SaveDltDevice)
        else {
            panic!("expected device save");
        };
        assert_eq!(device.name, "mydevice");
        assert!(matches!(
            update(&mut state, Message::SaveDltDevice),
            Effect::None
        ));
        let file = tempfile::NamedTempFile::new().unwrap();
        crate::config::DltDevice::save_to(&device, file.path()).unwrap();
        let config: crate::config::Config =
            serde_json::from_slice(&std::fs::read(file.path()).unwrap()).unwrap();
        assert_eq!(config.dlt_devices, vec![device.clone()]);
        update(
            &mut state,
            Message::DltDeviceSaved(epoch, device.clone(), Ok(())),
        );
        assert!(!state.editor.as_ref().unwrap().dlt_saving);
        assert_eq!(state.editor.as_ref().unwrap().members[0].1, "mydevice");
        assert!(matches!(
            update(&mut state, Message::SaveDltDevice),
            Effect::None
        ));
        assert!(
            state
                .editor
                .as_ref()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("already")
        );
        state.editor.as_mut().unwrap().values[3] = "anotherdevice".into();
        let Effect::SaveDltDevice { epoch, device } = update(&mut state, Message::SaveDltDevice)
        else {
            panic!("expected device save");
        };
        update(
            &mut state,
            Message::DltDeviceSaved(epoch, device, Err("Write failed".into())),
        );
        assert_eq!(
            state.editor.as_ref().unwrap().error.as_deref(),
            Some("Write failed")
        );
        assert!(!state.editor.as_ref().unwrap().dlt_saving);
    }
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

fn parse_containers(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let parts = line.splitn(4, '\t').collect::<Vec<_>>();
            (parts.len() == 4).then(|| {
                (
                    parts[0].into(),
                    format!("{} · {} · {}", parts[1], parts[2], parts[3]),
                )
            })
        })
        .collect()
}
impl App {
    pub(super) fn list_docker(&self, epoch: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = crate::gui::runtime::handle()
                .spawn(async {
                    let output = tokio::process::Command::new("docker")
                        .args([
                            "ps",
                            "--format",
                            "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}",
                        ])
                        .output()
                        .await
                        .map_err(|e| e.to_string())?;
                    if !output.status.success() {
                        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
                    }
                    Ok(parse_containers(&String::from_utf8_lossy(&output.stdout)))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            this.update_in(cx, |app, window, cx| {
                app.dispatch(Message::DockerListed(epoch, result), window, cx)
            })
            .ok();
        })
        .detach();
    }
}
#[cfg(test)]
mod docker_tests {
    #[test]
    fn container_listing_keeps_ids_and_rejects_incomplete_rows() {
        assert_eq!(
            super::parse_containers("abc\tservice\timage:v1\tUp 2 hours\ninvalid\n"),
            vec![("abc".into(), "service · image:v1 · Up 2 hours".into())]
        );
    }
}
