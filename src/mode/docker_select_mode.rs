use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::Keybindings,
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::normal_mode::NormalMode,
    theme::Theme,
    types::DockerContainer,
    ui::{KeyResult, TabState},
};

#[derive(Debug)]
pub struct DockerSelectMode {
    pub containers: Vec<DockerContainer>,
    pub selected: usize,
    pub error: Option<String>,
}

impl DockerSelectMode {
    pub fn new(containers: Vec<DockerContainer>) -> Self {
        DockerSelectMode {
            containers,
            selected: 0,
            error: None,
        }
    }

    pub fn with_error(error: String) -> Self {
        DockerSelectMode {
            containers: Vec::new(),
            selected: 0,
            error: Some(error),
        }
    }
}

#[async_trait]
impl Mode for DockerSelectMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.keybindings.docker_select;
        if kb.navigate_down.matches(key, modifiers) {
            if !self.containers.is_empty() {
                self.selected = (self.selected + 1).min(self.containers.len() - 1);
            }
        } else if kb.navigate_up.matches(key, modifiers) {
            self.selected = self.selected.saturating_sub(1);
        } else if kb.confirm.matches(key, modifiers) {
            if let Some(c) = self.containers.get(self.selected) {
                let id = c.id.clone();
                let name = c.name.clone();
                return (Box::new(NormalMode), KeyResult::DockerAttach(id, name));
            }
            return (Box::new(NormalMode), KeyResult::Handled);
        } else if kb.cancel.matches(key, modifiers) {
            return (Box::new(NormalMode), KeyResult::Handled);
        }
        (self, KeyResult::Handled)
    }

    fn status_line(&self) -> &str {
        "[DOCKER] <j/k> navigate  <Enter> attach  <Esc> cancel"
    }

    fn dynamic_status_line(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[DOCKER]  ",
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(Modifier::BOLD),
        )];
        // Navigate up/down
        spans.push(Span::styled("<", Style::default().fg(theme.border)));
        spans.push(Span::styled(
            kb.docker_select.navigate_up.display(),
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("/", Style::default().fg(theme.border)));
        spans.push(Span::styled(
            kb.docker_select.navigate_down.display(),
            Style::default()
                .fg(theme.text_highlight)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            "> navigate  ",
            Style::default().fg(theme.text),
        ));
        status_entry(
            &mut spans,
            kb.docker_select.confirm.display(),
            "attach",
            theme,
        );
        status_entry(
            &mut spans,
            kb.docker_select.cancel.display(),
            "cancel",
            theme,
        );
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::DockerSelect {
            containers: self.containers.clone(),
            selected: self.selected,
            error: self.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::mode::app_mode::ModeRenderState;
    use std::sync::Arc;

    async fn make_tab() -> TabState {
        let file_reader = FileReader::from_bytes(b"line1\nline2\n".to_vec());
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    fn sample_containers() -> Vec<DockerContainer> {
        vec![
            DockerContainer {
                id: "abc123".to_string(),
                name: "web-app".to_string(),
                image: "nginx:latest".to_string(),
                status: "Up 2 hours".to_string(),
            },
            DockerContainer {
                id: "def456".to_string(),
                name: "db-server".to_string(),
                image: "postgres:15".to_string(),
                status: "Up 3 hours".to_string(),
            },
            DockerContainer {
                id: "ghi789".to_string(),
                name: "cache".to_string(),
                image: "redis:7".to_string(),
                status: "Up 1 hour".to_string(),
            },
        ]
    }

    async fn press(
        mode: DockerSelectMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    #[tokio::test]
    async fn test_j_moves_cursor_down() {
        let mut tab = make_tab().await;
        let mode = DockerSelectMode::new(sample_containers());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        match mode2.render_state() {
            ModeRenderState::DockerSelect { selected, .. } => assert_eq!(selected, 1),
            other => panic!("expected DockerSelect, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_k_moves_cursor_up() {
        let mut tab = make_tab().await;
        let mut mode = DockerSelectMode::new(sample_containers());
        mode.selected = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        match mode2.render_state() {
            ModeRenderState::DockerSelect { selected, .. } => assert_eq!(selected, 1),
            other => panic!("expected DockerSelect, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_k_at_zero_stays() {
        let mut tab = make_tab().await;
        let mode = DockerSelectMode::new(sample_containers());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        match mode2.render_state() {
            ModeRenderState::DockerSelect { selected, .. } => assert_eq!(selected, 0),
            other => panic!("expected DockerSelect, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_j_at_end_stays() {
        let mut tab = make_tab().await;
        let mut mode = DockerSelectMode::new(sample_containers());
        mode.selected = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        match mode2.render_state() {
            ModeRenderState::DockerSelect { selected, .. } => assert_eq!(selected, 2),
            other => panic!("expected DockerSelect, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_down_arrow_moves_cursor() {
        let mut tab = make_tab().await;
        let mode = DockerSelectMode::new(sample_containers());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Down).await;
        match mode2.render_state() {
            ModeRenderState::DockerSelect { selected, .. } => assert_eq!(selected, 1),
            other => panic!("expected DockerSelect, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_up_arrow_moves_cursor() {
        let mut tab = make_tab().await;
        let mut mode = DockerSelectMode::new(sample_containers());
        mode.selected = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Up).await;
        match mode2.render_state() {
            ModeRenderState::DockerSelect { selected, .. } => assert_eq!(selected, 1),
            other => panic!("expected DockerSelect, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_enter_returns_docker_attach() {
        let mut tab = make_tab().await;
        let mode = DockerSelectMode::new(sample_containers());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(
            result,
            KeyResult::DockerAttach(ref id, ref name) if id == "abc123" && name == "web-app"
        ));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::DockerSelect { .. }
        )); // NormalMode
    }

    #[tokio::test]
    async fn test_enter_with_selection() {
        let mut tab = make_tab().await;
        let mut mode = DockerSelectMode::new(sample_containers());
        mode.selected = 1;
        let (_, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(
            result,
            KeyResult::DockerAttach(ref id, ref name) if id == "def456" && name == "db-server"
        ));
    }

    #[tokio::test]
    async fn test_enter_empty_list() {
        let mut tab = make_tab().await;
        let mode = DockerSelectMode::new(vec![]);
        let (mode2, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::DockerSelect { .. }
        ));
    }

    #[tokio::test]
    async fn test_esc_cancels() {
        let mut tab = make_tab().await;
        let mode = DockerSelectMode::new(sample_containers());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::DockerSelect { .. }
        ));
    }

    #[tokio::test]
    async fn test_status_line() {
        let mode = DockerSelectMode::new(sample_containers());
        assert!(mode.status_line().contains("[DOCKER]"));
    }

    #[tokio::test]
    async fn test_error_mode_shows_error() {
        let mode = DockerSelectMode::with_error("Docker not found".to_string());
        match mode.render_state() {
            ModeRenderState::DockerSelect { error, .. } => {
                assert_eq!(error.as_deref(), Some("Docker not found"));
            }
            other => panic!("expected DockerSelect, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_error_mode_esc_cancels() {
        let mut tab = make_tab().await;
        let mode = DockerSelectMode::with_error("Docker not found".to_string());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::DockerSelect { .. }
        ));
    }

    #[tokio::test]
    async fn test_j_on_empty_list() {
        let mut tab = make_tab().await;
        let mode = DockerSelectMode::new(vec![]);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        match mode2.render_state() {
            ModeRenderState::DockerSelect {
                containers,
                selected,
                ..
            } => {
                assert!(containers.is_empty());
                assert_eq!(selected, 0);
            }
            other => panic!("expected DockerSelect, got {:?}", other),
        }
    }
}
