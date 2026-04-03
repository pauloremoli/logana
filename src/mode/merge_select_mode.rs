use crate::{
    config::Keybindings,
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::normal_mode::NormalMode,
    theme::Theme,
    ui::{KeyResult, TabState},
};
use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug)]
pub struct MergeSelectMode {
    /// Tab title + selected toggle (for display).
    pub tabs: Vec<(String, bool)>,
    /// Actual `App::tabs` index for each entry in `tabs`.
    pub tab_indices: Vec<usize>,
    /// Cursor position in the list.
    pub selected: usize,
}

impl MergeSelectMode {
    pub fn new(tabs: Vec<(String, bool)>, tab_indices: Vec<usize>) -> Self {
        MergeSelectMode {
            tabs,
            tab_indices,
            selected: 0,
        }
    }
}

#[async_trait]
impl Mode for MergeSelectMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = &tab.interaction.keybindings;

        if kb.select_fields.apply.matches(key, modifiers) {
            let selected: Vec<usize> = self
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, (_, on))| *on)
                .map(|(i, _)| self.tab_indices[i])
                .collect();
            if selected.len() < 2 {
                tab.interaction.command_error = Some("Select at least 2 tabs to merge".to_string());
                return (self, KeyResult::Handled);
            }
            return (
                Box::new(NormalMode::default()),
                KeyResult::OpenMergedView {
                    source_tab_indices: selected,
                },
            );
        }

        if kb.select_fields.cancel.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        }

        if kb.navigation.scroll_down.matches(key, modifiers) {
            if !self.tabs.is_empty() {
                self.selected = (self.selected + 1).min(self.tabs.len() - 1);
            }
        } else if kb.navigation.scroll_up.matches(key, modifiers) {
            self.selected = self.selected.saturating_sub(1);
        } else if kb.select_fields.toggle.matches(key, modifiers) {
            if let Some(t) = self.tabs.get_mut(self.selected) {
                t.1 = !t.1;
            }
        } else if kb.select_fields.all.matches(key, modifiers) {
            for t in &mut self.tabs {
                t.1 = true;
            }
        } else if kb.select_fields.none.matches(key, modifiers) {
            for t in &mut self.tabs {
                t.1 = false;
            }
        }

        (self, KeyResult::Ignored)
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            "[MERGE SELECT]  ",
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(
            &mut spans,
            kb.select_fields.toggle.display(),
            "toggle",
            theme,
        );
        status_entry(&mut spans, kb.select_fields.apply.display(), "merge", theme);
        status_entry(
            &mut spans,
            kb.select_fields.cancel.display(),
            "cancel",
            theme,
        );
        status_entry(&mut spans, kb.select_fields.all.display(), "all", theme);
        status_entry(&mut spans, kb.select_fields.none.display(), "none", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::MergeSelect {
            tabs: self.tabs.clone(),
            selected: self.selected,
        }
    }
}
