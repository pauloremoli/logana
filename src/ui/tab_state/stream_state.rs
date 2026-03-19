use super::{FileWatchState, StreamRetryState};

#[derive(Default)]
pub struct StreamState {
    pub watch: Option<FileWatchState>,
    pub tail_mode: bool,
    pub paused: bool,
    pub retry: Option<StreamRetryState>,
}
