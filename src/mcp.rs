use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, serde,
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use tokio::sync::{RwLock, mpsc};
use tokio_util::sync::CancellationToken;

use crate::db::LogManager;
use crate::ingestion::FileReader;
use crate::types::Comment;

#[derive(Default)]
pub struct McpSnapshot {
    pub marked_lines: Vec<(usize, String)>,
    pub annotations: Vec<Comment>,
}

pub enum McpCommand {
    ToggleMark(usize),
    AddAnnotation {
        text: String,
        line_indices: Vec<usize>,
    },
    RemoveAnnotation(usize),
}

pub struct McpServerHandle {
    pub cancel: CancellationToken,
    pub port: u16,
}

pub fn build_marked_lines(reader: &FileReader, log_manager: &LogManager) -> Vec<(usize, String)> {
    log_manager
        .get_marked_indices()
        .into_iter()
        .filter(|&i| i < reader.line_count())
        .map(|i| {
            let text = String::from_utf8_lossy(reader.get_line(i)).into_owned();
            (i + 1, text)
        })
        .collect()
}

pub fn build_annotations(log_manager: &LogManager) -> Vec<Comment> {
    log_manager.get_comments().to_vec()
}

pub fn format_marks_resource(snapshot: &McpSnapshot) -> String {
    snapshot
        .marked_lines
        .iter()
        .map(|(n, t)| format!("{n}: {t}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_annotations_resource(snapshot: &McpSnapshot) -> String {
    snapshot
        .annotations
        .iter()
        .map(|a| {
            let lines = a
                .line_indices
                .iter()
                .map(|&i| i.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!("Lines: {lines}\n{}\n---", a.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct ToggleMarkParams {
    /// 1-based line number to toggle mark on
    line_index: usize,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct AddAnnotationParams {
    /// Annotation text
    text: String,
    /// 1-based line numbers this annotation applies to
    line_indices: Vec<usize>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
struct RemoveAnnotationParams {
    /// 0-based annotation index
    index: usize,
}

#[derive(Clone)]
struct LoganaServer {
    snapshot: Arc<RwLock<McpSnapshot>>,
    cmd_tx: mpsc::Sender<McpCommand>,
    tool_router: ToolRouter<Self>,
}

impl LoganaServer {
    fn new(snapshot: Arc<RwLock<McpSnapshot>>, cmd_tx: mpsc::Sender<McpCommand>) -> Self {
        Self {
            snapshot,
            cmd_tx,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl LoganaServer {
    #[tool(description = "Toggle a mark on a log line (1-based line number)")]
    async fn toggle_mark(
        &self,
        Parameters(ToggleMarkParams { line_index }): Parameters<ToggleMarkParams>,
    ) -> String {
        let _ = self.cmd_tx.send(McpCommand::ToggleMark(line_index)).await;
        format!("Mark toggled on line {line_index}")
    }

    #[tool(description = "Add an annotation to log lines (1-based line numbers)")]
    async fn add_annotation(
        &self,
        Parameters(AddAnnotationParams { text, line_indices }): Parameters<AddAnnotationParams>,
    ) -> String {
        let _ = self
            .cmd_tx
            .send(McpCommand::AddAnnotation { text, line_indices })
            .await;
        "Annotation added".to_string()
    }

    #[tool(description = "Remove an annotation by its 0-based index")]
    async fn remove_annotation(
        &self,
        Parameters(RemoveAnnotationParams { index }): Parameters<RemoveAnnotationParams>,
    ) -> String {
        let _ = self.cmd_tx.send(McpCommand::RemoveAnnotation(index)).await;
        format!("Annotation {index} removed")
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LoganaServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_resources()
                .enable_tools()
                .build(),
        )
        .with_server_info(Implementation::new("logana", env!("CARGO_PKG_VERSION")))
        .with_instructions("Logana MCP server: read marks and annotations")
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(vec![
            Resource::new(RawResource::new("logana://marks", "Marked Lines"), None),
            Resource::new(
                RawResource::new("logana://annotations", "Annotations"),
                None,
            ),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let snapshot = self.snapshot.read().await;
        let text = match request.uri.as_str() {
            "logana://marks" => format_marks_resource(&snapshot),
            "logana://annotations" => format_annotations_resource(&snapshot),
            _ => {
                return Err(ErrorData::invalid_params(
                    format!("Unknown resource URI: {}", request.uri),
                    None,
                ));
            }
        };
        Ok(ReadResourceResult::new(vec![ResourceContents::text(
            text,
            request.uri,
        )]))
    }
}

pub async fn start_mcp_server(
    port: u16,
    snapshot: Arc<RwLock<McpSnapshot>>,
    cmd_tx: mpsc::Sender<McpCommand>,
) -> std::io::Result<McpServerHandle> {
    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let listener = tokio::net::TcpListener::bind(addr).await?;

    let cancel = CancellationToken::new();
    let child_token = cancel.child_token();

    tokio::spawn(async move {
        let server = LoganaServer::new(snapshot, cmd_tx);
        let mut config = StreamableHttpServerConfig::default();
        config.cancellation_token = child_token.clone();

        let service: StreamableHttpService<LoganaServer, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(server.clone()),
                Arc::new(LocalSessionManager::default()),
                config,
            );

        let router = Router::new().nest_service("/mcp", service);

        if let Err(e) = axum::serve(listener, router)
            .with_graceful_shutdown(async move { child_token.cancelled_owned().await })
            .await
        {
            eprintln!("MCP server error: {e}");
        }
    });

    Ok(McpServerHandle { cancel, port })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::{RwLock, mpsc};

    use crate::db::Database;
    use crate::db::LogManager;
    use crate::types::Comment;

    async fn make_reader_and_manager(lines: &[&str]) -> (FileReader, LogManager) {
        let bytes = lines.join("\n").into_bytes();
        let reader = FileReader::from_bytes(bytes);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let lm = LogManager::new(db, None).await;
        (reader, lm)
    }

    fn make_server() -> (LoganaServer, mpsc::Receiver<McpCommand>) {
        let snapshot = Arc::new(RwLock::new(McpSnapshot::default()));
        let (tx, rx) = mpsc::channel(16);
        let server = LoganaServer::new(snapshot, tx);
        (server, rx)
    }

    // ── build_marked_lines ───────────────────────────────────────────

    #[tokio::test]
    async fn test_build_marked_lines_empty() {
        let (reader, lm) = make_reader_and_manager(&["a", "b", "c"]).await;
        let result = build_marked_lines(&reader, &lm);
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_build_marked_lines_with_marks() {
        let (reader, mut lm) = make_reader_and_manager(&["a", "b", "c"]).await;
        lm.toggle_mark(0);
        lm.toggle_mark(2);
        let result = build_marked_lines(&reader, &lm);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], (1, "a".to_string()));
        assert_eq!(result[1], (3, "c".to_string()));
    }

    // ── build_annotations ────────────────────────────────────────────

    #[tokio::test]
    async fn test_build_annotations_empty() {
        let (_, lm) = make_reader_and_manager(&["a"]).await;
        let result = build_annotations(&lm);
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_build_annotations_with_comments() {
        let (_, mut lm) = make_reader_and_manager(&["a", "b"]).await;
        lm.add_comment("note".to_string(), vec![0, 1]);
        let result = build_annotations(&lm);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "note");
        assert_eq!(result[0].line_indices, vec![0, 1]);
    }

    // ── format helpers ───────────────────────────────────────────────

    #[test]
    fn test_read_resource_marks_uri() {
        let snap = McpSnapshot {
            marked_lines: vec![(2, "marked".to_string())],
            annotations: vec![],
        };
        let text = format_marks_resource(&snap);
        assert!(text.contains("2: marked"));
    }

    #[test]
    fn test_read_resource_annotations_uri() {
        let snap = McpSnapshot {
            marked_lines: vec![],
            annotations: vec![Comment {
                text: "my note".to_string(),
                line_indices: vec![1, 2],
            }],
        };
        let text = format_annotations_resource(&snap);
        assert!(text.contains("Lines: 1,2"));
        assert!(text.contains("my note"));
    }

    #[test]
    fn test_list_tools_returns_three() {
        let snap = McpSnapshot::default();
        let snapshot = Arc::new(RwLock::new(snap));
        let (tx, _rx) = mpsc::channel(1);
        let server = LoganaServer::new(snapshot, tx);
        let tools = server.tool_router.list_all();
        assert_eq!(tools.len(), 3);
    }

    // ── tool command dispatch ────────────────────────────────────────

    #[tokio::test]
    async fn test_toggle_mark_sends_command() {
        let (server, mut rx) = make_server();
        server
            .toggle_mark(Parameters(ToggleMarkParams { line_index: 5 }))
            .await;
        match rx.try_recv().unwrap() {
            McpCommand::ToggleMark(idx) => assert_eq!(idx, 5),
            _ => panic!("expected ToggleMark"),
        }
    }

    #[tokio::test]
    async fn test_add_annotation_sends_command() {
        let (server, mut rx) = make_server();
        server
            .add_annotation(Parameters(AddAnnotationParams {
                text: "note".to_string(),
                line_indices: vec![1, 2],
            }))
            .await;
        match rx.try_recv().unwrap() {
            McpCommand::AddAnnotation { text, line_indices } => {
                assert_eq!(text, "note");
                assert_eq!(line_indices, vec![1, 2]);
            }
            _ => panic!("expected AddAnnotation"),
        }
    }

    #[tokio::test]
    async fn test_remove_annotation_sends_command() {
        let (server, mut rx) = make_server();
        server
            .remove_annotation(Parameters(RemoveAnnotationParams { index: 3 }))
            .await;
        match rx.try_recv().unwrap() {
            McpCommand::RemoveAnnotation(idx) => assert_eq!(idx, 3),
            _ => panic!("expected RemoveAnnotation"),
        }
    }

    // ── start_mcp_server ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_start_mcp_server_binds_successfully() {
        let snapshot = Arc::new(RwLock::new(McpSnapshot::default()));
        let (tx, _rx) = mpsc::channel(1);
        let result = start_mcp_server(0, snapshot, tx).await;
        assert!(result.is_ok());
        let handle = result.unwrap();
        assert!(handle.port == 0);
        handle.cancel.cancel();
    }

    #[tokio::test]
    async fn test_start_mcp_server_fails_on_used_port() {
        let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let snapshot = Arc::new(RwLock::new(McpSnapshot::default()));
        let (tx, _rx) = mpsc::channel(1);
        let result = start_mcp_server(port, snapshot, tx).await;
        assert!(result.is_err());
    }
}
