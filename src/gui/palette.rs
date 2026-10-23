//! GUI launcher state and latest action outcome.
use crate::shared::TabId;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    Running,
    Success,
    Information,
    Error,
}
#[derive(Debug, Clone)]
pub struct Outcome {
    pub request_id: u64,
    pub invocation: String,
    pub source: Option<TabId>,
    pub source_label: Option<String>,
    pub state: ExecutionState,
    pub output: String,
    pub validation: bool,
}
#[derive(Default)]
pub struct Palette {
    pub open: bool,
    pub input: String,
    pub collapsed: bool,
    pub selected: usize,
    pub outcome: Option<Outcome>,
    pub generation: u64,
    pub focus_epoch: u64,
    pub next_request: u64,
}
impl Palette {
    pub fn visible(&self) -> bool {
        self.open
    }
    pub fn pending(&self) -> bool {
        self.outcome
            .as_ref()
            .is_some_and(|o| o.state == ExecutionState::Running)
    }
    pub fn edit(&mut self, input: String) {
        self.input = input;
        self.selected = 0;
        self.collapsed = false;
        self.generation = self.generation.wrapping_add(1);
    }
}
