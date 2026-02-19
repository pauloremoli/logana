use crossterm::event::{KeyCode, KeyModifiers};

use crate::{
    mode::{app_mode::Mode, normal_mode::NormalMode},
    ui::{KeyResult, TabState},
};

#[derive(Debug)]
pub struct SearchMode {
    pub input: String,
    pub forward: bool,
}

impl Mode for SearchMode {
    fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        _modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        if matches!(key, KeyCode::Tab | KeyCode::BackTab) {
            return (self, KeyResult::Ignored);
        }
        match key {
            KeyCode::Enter => {
                let visible = tab.visible_indices.clone();
                let _ = tab.search.search(&self.input, &visible, &tab.file_reader);
                let result = if self.forward {
                    tab.search.next_match()
                } else {
                    tab.search.previous_match()
                };
                if let Some(r) = result {
                    let line_idx = r.line_idx;
                    tab.scroll_to_line_idx(line_idx);
                }
                (Box::new(NormalMode), KeyResult::Handled)
            }
            KeyCode::Esc => {
                self.input.clear();
                (Box::new(NormalMode), KeyResult::Handled)
            }
            KeyCode::Backspace => {
                self.input.pop();
                (self, KeyResult::Handled)
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                (self, KeyResult::Handled)
            }
            _ => (self, KeyResult::Handled),
        }
    }

    fn status_line(&self) -> &str {
        "[SEARCH] Esc => cancel | Enter => search"
    }

    fn search_state(&self) -> Option<(&str, bool)> {
        Some((&self.input, self.forward))
    }

    fn needs_input_bar(&self) -> bool {
        true
    }
}
