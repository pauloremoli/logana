use crate::commands::Commands;
use crate::db::LogManager;
use std::path::PathBuf;

/// What `update()` wants the gpui glue layer (`app.rs`) to actually do —
/// async work or an imperative widget action — without `update()` itself
/// depending on any gpui type. Mirrors what iced's `Task<Message>` did for
/// the discarded prototype, but as a plain, directly-testable enum.
#[derive(Debug)]
pub enum Effect {
    None,
    Quit,
    OpenFileDialog,
    LoadFile(PathBuf),
    ToggleFilter {
        tab_idx: usize,
        log_manager: LogManager,
        id: usize,
    },
    ToggleGroup {
        tab_idx: usize,
        log_manager: LogManager,
        name: String,
    },
    RemoveGroup {
        tab_idx: usize,
        log_manager: LogManager,
        name: String,
    },
    RemoveFilter {
        tab_idx: usize,
        log_manager: LogManager,
        id: usize,
    },
    ExecuteCommand {
        tab_idx: usize,
        log_manager: LogManager,
        command: Commands,
        /// The active tab's theme background, for `--auto`/`--fga` color
        /// generation (`execute_command`/`resolve_colors`) — resolved here
        /// rather than inside `execute_command` so that function stays a
        /// plain, gpui-free `(LogManager, Commands) -> Result<..>` step.
        theme_bg: (u8, u8, u8),
        /// `tab.filter.filter_context` — which filter `:set-color` (the
        /// only command needing it so far) targets. Populated for free by
        /// `FilterManagementMode::handle_key` whenever a filter is
        /// selected there, same as the TUI.
        filter_context: Option<usize>,
    },
}
