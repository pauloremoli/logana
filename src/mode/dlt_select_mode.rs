use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::{DltDevice, Keybindings},
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::normal_mode::NormalMode,
    theme::Theme,
    ui::{KeyResult, TabState},
};

#[derive(Debug, Clone)]
pub struct AddDeviceState {
    pub fields: [String; 3], // name, host, port
    pub active_field: usize, // 0=name, 1=host, 2=port
    pub cursor: usize,
}

impl AddDeviceState {
    fn new() -> Self {
        Self {
            fields: [String::new(), String::new(), "3490".to_string()],
            active_field: 0,
            cursor: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AddDeviceRenderState {
    pub fields: [String; 3],
    pub active_field: usize,
    pub cursor: usize,
}

#[derive(Debug)]
pub struct DltSelectMode {
    pub devices: Vec<DltDevice>,
    pub selected: usize,
    pub error: Option<String>,
    pub adding: Option<AddDeviceState>,
}

impl DltSelectMode {
    pub fn new(devices: Vec<DltDevice>) -> Self {
        Self {
            devices,
            selected: 0,
            error: None,
            adding: None,
        }
    }

    fn total_entries(&self) -> usize {
        self.devices.len() + 1 // +1 for "Add new device..."
    }

    fn handle_list_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.keybindings;

        if kb.navigation.scroll_down.matches(key, modifiers) {
            let max = self.total_entries().saturating_sub(1);
            self.selected = (self.selected + 1).min(max);
        } else if kb.navigation.scroll_up.matches(key, modifiers) {
            self.selected = self.selected.saturating_sub(1);
        } else if kb.dlt_select.confirm.matches(key, modifiers) {
            if self.selected < self.devices.len() {
                let dev = &self.devices[self.selected];
                let host = dev.host.clone();
                let port = dev.port;
                let name = dev.name.clone();
                return (
                    Box::new(NormalMode::default()),
                    KeyResult::DltAttach(host, port, name),
                );
            } else {
                self.adding = Some(AddDeviceState::new());
            }
        } else if kb.dlt_select.delete.matches(key, modifiers) {
            if self.selected < self.devices.len() {
                let name = self.devices[self.selected].name.clone();
                self.devices.remove(self.selected);
                if self.selected >= self.total_entries() && self.selected > 0 {
                    self.selected -= 1;
                }
                if let Err(e) = DltDevice::remove(&name) {
                    self.error = Some(e);
                }
            }
        } else if kb.dlt_select.cancel.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        } else {
            return (self, KeyResult::Ignored);
        }
        (self, KeyResult::Handled)
    }

    fn handle_add_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.keybindings;
        let adding = self.adding.as_mut().unwrap();

        if kb.dlt_select.confirm.matches(key, modifiers) {
            let name = adding.fields[0].trim().to_string();
            let host = adding.fields[1].trim().to_string();
            let port_str = adding.fields[2].trim().to_string();

            if name.is_empty() || host.is_empty() {
                self.error = Some("Name and host are required".to_string());
                return (self, KeyResult::Handled);
            }

            let port: u16 = match port_str.parse() {
                Ok(p) => p,
                Err(_) => {
                    self.error = Some("Port must be a number (0-65535)".to_string());
                    return (self, KeyResult::Handled);
                }
            };

            let device = DltDevice {
                name: name.clone(),
                host,
                port,
            };
            if let Err(e) = DltDevice::save(&device) {
                self.error = Some(e);
                return (self, KeyResult::Handled);
            }
            self.devices.push(device);
            self.selected = self.devices.len() - 1;
            self.adding = None;
            self.error = None;
        } else if kb.dlt_select.cancel.matches(key, modifiers) {
            self.adding = None;
            self.error = None;
        } else if kb.dlt_select.next_field.matches(key, modifiers) {
            adding.active_field = (adding.active_field + 1) % 3;
            adding.cursor = adding.fields[adding.active_field].len();
        } else if kb.dlt_select.prev_field.matches(key, modifiers) {
            adding.active_field = (adding.active_field + 2) % 3;
            adding.cursor = adding.fields[adding.active_field].len();
        } else if matches!(key, KeyCode::Backspace) {
            let field = &mut adding.fields[adding.active_field];
            if adding.cursor > 0 {
                let byte_pos = field
                    .char_indices()
                    .nth(adding.cursor - 1)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let end_pos = field
                    .char_indices()
                    .nth(adding.cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(field.len());
                field.replace_range(byte_pos..end_pos, "");
                adding.cursor -= 1;
            }
        } else if matches!(key, KeyCode::Left) {
            adding.cursor = adding.cursor.saturating_sub(1);
        } else if matches!(key, KeyCode::Right) {
            let max = adding.fields[adding.active_field].chars().count();
            adding.cursor = (adding.cursor + 1).min(max);
        } else if let KeyCode::Char(c) = key
            && !modifiers.contains(KeyModifiers::CONTROL)
            && !modifiers.contains(KeyModifiers::ALT)
        {
            let field = &mut adding.fields[adding.active_field];
            let byte_pos = field
                .char_indices()
                .nth(adding.cursor)
                .map(|(i, _)| i)
                .unwrap_or(field.len());
            field.insert(byte_pos, c);
            adding.cursor += 1;
        }

        (self, KeyResult::Handled)
    }
}

#[async_trait]
impl Mode for DltSelectMode {
    async fn handle_key(
        self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if self.adding.is_some() {
            self.handle_add_key(tab, key, modifiers)
        } else {
            self.handle_list_key(tab, key, modifiers)
        }
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        if self.adding.is_some() {
            let mut spans: Vec<Span<'static>> = vec![Span::styled(
                "[DLT ADD]  ",
                Style::default()
                    .fg(theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )];
            status_entry(
                &mut spans,
                kb.dlt_select.next_field.display(),
                "next field",
                theme,
            );
            status_entry(&mut spans, kb.dlt_select.confirm.display(), "save", theme);
            status_entry(&mut spans, kb.dlt_select.cancel.display(), "cancel", theme);
            Line::from(spans)
        } else {
            let mut spans: Vec<Span<'static>> = vec![Span::styled(
                "[DLT]  ",
                Style::default()
                    .fg(theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            )];
            spans.push(Span::styled("<", Style::default().fg(theme.text)));
            spans.push(Span::styled(
                kb.navigation.scroll_up.display(),
                Style::default()
                    .fg(theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled("/", Style::default().fg(theme.text)));
            spans.push(Span::styled(
                kb.navigation.scroll_down.display(),
                Style::default()
                    .fg(theme.text_highlight_fg)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                "> navigate  ",
                Style::default().fg(theme.text),
            ));
            status_entry(
                &mut spans,
                kb.dlt_select.confirm.display(),
                "connect",
                theme,
            );
            status_entry(&mut spans, kb.dlt_select.delete.display(), "delete", theme);
            status_entry(&mut spans, kb.dlt_select.cancel.display(), "cancel", theme);
            Line::from(spans)
        }
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::DltSelect {
            devices: self.devices.clone(),
            selected: self.selected,
            error: self.error.clone(),
            adding: self.adding.as_ref().map(|a| AddDeviceRenderState {
                fields: a.fields.clone(),
                active_field: a.active_field,
                cursor: a.cursor,
            }),
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

    fn sample_devices() -> Vec<DltDevice> {
        vec![
            DltDevice {
                name: "ecu1".to_string(),
                host: "192.168.1.10".to_string(),
                port: 3490,
            },
            DltDevice {
                name: "ecu2".to_string(),
                host: "192.168.1.20".to_string(),
                port: 3491,
            },
        ]
    }

    async fn press(
        mode: DltSelectMode,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, code, KeyModifiers::NONE)
            .await
    }

    async fn press_shift(
        mode: Box<dyn Mode>,
        tab: &mut TabState,
        code: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        mode.handle_key(tab, code, KeyModifiers::NONE).await
    }

    fn extract_dlt_state(
        state: ModeRenderState,
    ) -> (
        Vec<DltDevice>,
        usize,
        Option<String>,
        Option<AddDeviceRenderState>,
    ) {
        match state {
            ModeRenderState::DltSelect {
                devices,
                selected,
                error,
                adding,
            } => (devices, selected, error, adding),
            other => panic!("expected DltSelect, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_j_moves_cursor_down() {
        let mut tab = make_tab().await;
        let mode = DltSelectMode::new(sample_devices());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        let (_, selected, _, _) = extract_dlt_state(mode2.render_state());
        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn test_k_moves_cursor_up() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(sample_devices());
        mode.selected = 2; // "Add new device..." entry
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        let (_, selected, _, _) = extract_dlt_state(mode2.render_state());
        assert_eq!(selected, 1);
    }

    #[tokio::test]
    async fn test_k_at_zero_stays() {
        let mut tab = make_tab().await;
        let mode = DltSelectMode::new(sample_devices());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('k')).await;
        let (_, selected, _, _) = extract_dlt_state(mode2.render_state());
        assert_eq!(selected, 0);
    }

    #[tokio::test]
    async fn test_j_at_end_stays() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(sample_devices());
        mode.selected = 2; // last entry ("Add new device...")
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('j')).await;
        let (_, selected, _, _) = extract_dlt_state(mode2.render_state());
        assert_eq!(selected, 2);
    }

    #[tokio::test]
    async fn test_enter_on_device_returns_dlt_attach() {
        let mut tab = make_tab().await;
        let mode = DltSelectMode::new(sample_devices());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(
            result,
            KeyResult::DltAttach(ref host, port, ref name)
                if host == "192.168.1.10" && port == 3490 && name == "ecu1"
        ));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::DltSelect { .. }
        ));
    }

    #[tokio::test]
    async fn test_enter_on_add_transitions_to_add_view() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(sample_devices());
        mode.selected = 2; // "Add new device..."
        let (mode2, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(result, KeyResult::Handled));
        let (_, _, _, adding) = extract_dlt_state(mode2.render_state());
        assert!(adding.is_some());
    }

    #[tokio::test]
    async fn test_esc_cancels() {
        let mut tab = make_tab().await;
        let mode = DltSelectMode::new(sample_devices());
        let (mode2, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(!matches!(
            mode2.render_state(),
            ModeRenderState::DltSelect { .. }
        ));
    }

    #[tokio::test]
    async fn test_d_deletes_device() {
        let mut tab = make_tab().await;
        let mode = DltSelectMode::new(sample_devices());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('d')).await;
        let (devices, _, _, _) = extract_dlt_state(mode2.render_state());
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "ecu2");
    }

    #[tokio::test]
    async fn test_d_on_add_entry_does_nothing() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(sample_devices());
        mode.selected = 2;
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('d')).await;
        let (devices, _, _, _) = extract_dlt_state(mode2.render_state());
        assert_eq!(devices.len(), 2);
    }

    #[tokio::test]
    async fn test_empty_list_shows_add_entry() {
        let mut tab = make_tab().await;
        let mode = DltSelectMode::new(vec![]);
        let (_, selected, _, _) = extract_dlt_state(mode.render_state());
        assert_eq!(selected, 0);
        assert_eq!(mode.total_entries(), 1);
        // Enter on empty list opens add view
        let (mode2, _) = press(mode, &mut tab, KeyCode::Enter).await;
        let (_, _, _, adding) = extract_dlt_state(mode2.render_state());
        assert!(adding.is_some());
    }

    #[tokio::test]
    async fn test_add_view_tab_cycles_fields() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(vec![]);
        mode.adding = Some(AddDeviceState::new());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Tab).await;
        let (_, _, _, adding) = extract_dlt_state(mode2.render_state());
        assert_eq!(adding.as_ref().unwrap().active_field, 1);
    }

    #[tokio::test]
    async fn test_add_view_backtab_cycles_fields() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(vec![]);
        mode.adding = Some(AddDeviceState::new());
        let (mode2, _) = press_shift(Box::new(mode), &mut tab, KeyCode::BackTab).await;
        let (_, _, _, adding) = extract_dlt_state(mode2.render_state());
        assert_eq!(adding.as_ref().unwrap().active_field, 2);
    }

    #[tokio::test]
    async fn test_add_view_char_input() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(vec![]);
        mode.adding = Some(AddDeviceState::new());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Char('t')).await;
        let (_, _, _, adding) = extract_dlt_state(mode2.render_state());
        assert_eq!(adding.as_ref().unwrap().fields[0], "t");
    }

    #[tokio::test]
    async fn test_add_view_backspace() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(vec![]);
        let mut add_state = AddDeviceState::new();
        add_state.fields[0] = "test".to_string();
        add_state.cursor = 4;
        mode.adding = Some(add_state);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Backspace).await;
        let (_, _, _, adding) = extract_dlt_state(mode2.render_state());
        assert_eq!(adding.as_ref().unwrap().fields[0], "tes");
    }

    #[tokio::test]
    async fn test_add_view_esc_returns_to_list() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(sample_devices());
        mode.adding = Some(AddDeviceState::new());
        let (mode2, _) = press(mode, &mut tab, KeyCode::Esc).await;
        let (_, _, _, adding) = extract_dlt_state(mode2.render_state());
        assert!(adding.is_none());
    }

    #[tokio::test]
    async fn test_add_view_enter_empty_name_shows_error() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(vec![]);
        let mut add_state = AddDeviceState::new();
        add_state.fields[1] = "localhost".to_string();
        mode.adding = Some(add_state);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Enter).await;
        let (_, _, error, adding) = extract_dlt_state(mode2.render_state());
        assert!(error.is_some());
        assert!(adding.is_some()); // still in add view
    }

    #[tokio::test]
    async fn test_add_view_enter_invalid_port_shows_error() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(vec![]);
        let mut add_state = AddDeviceState::new();
        add_state.fields[0] = "test".to_string();
        add_state.fields[1] = "localhost".to_string();
        add_state.fields[2] = "notanumber".to_string();
        mode.adding = Some(add_state);
        let (mode2, _) = press(mode, &mut tab, KeyCode::Enter).await;
        let (_, _, error, _) = extract_dlt_state(mode2.render_state());
        assert!(error.is_some());
    }

    #[tokio::test]
    async fn test_enter_with_selection() {
        let mut tab = make_tab().await;
        let mut mode = DltSelectMode::new(sample_devices());
        mode.selected = 1;
        let (_, result) = press(mode, &mut tab, KeyCode::Enter).await;
        assert!(matches!(
            result,
            KeyResult::DltAttach(ref host, port, ref name)
                if host == "192.168.1.20" && port == 3491 && name == "ecu2"
        ));
    }

    #[tokio::test]
    async fn test_unrecognized_key_returns_ignored() {
        let mut tab = make_tab().await;
        let mode = DltSelectMode::new(sample_devices());
        let (_, result) = press(mode, &mut tab, KeyCode::F(2)).await;
        assert!(matches!(result, KeyResult::Ignored));
    }

    #[tokio::test]
    async fn test_render_state_variant() {
        let mode = DltSelectMode::new(sample_devices());
        assert!(matches!(
            mode.render_state(),
            ModeRenderState::DltSelect { .. }
        ));
    }

    #[tokio::test]
    async fn test_mode_name() {
        let state = ModeRenderState::DltSelect {
            devices: vec![],
            selected: 0,
            error: None,
            adding: None,
        };
        assert_eq!(state.mode_name(), "DLT");
    }
}
