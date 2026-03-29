use super::{FileWatchState, StreamRetryState};

#[derive(Default)]
pub struct StreamState {
    pub watch: Option<FileWatchState>,
    pub tail_mode: bool,
    pub paused: bool,
    pub retry: Option<StreamRetryState>,
    /// When `true`, a closed watch sender is treated as normal process exit
    /// rather than a lost connection.  The output stays visible (like a
    /// static file) and no reconnect is attempted.
    pub no_retry: bool,
}
