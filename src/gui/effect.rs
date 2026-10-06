use crate::commands::Commands;
use crate::db::LogManager;
use std::path::PathBuf;

/// What `update()` wants the gpui glue layer (`app.rs`) to actually do —
/// async work or an imperative widget action — without `update()` itself
/// depending on any gpui type. Mirrors what iced's `Task<Message>` did for
/// the discarded prototype, but as a plain, directly-testable enum.
#[derive(Debug)]
pub enum Effect {
    SavePreferences(crate::gui::persistence::Preferences),
    MergeTabs(Vec<crate::shared::TabId>),
    ConfigureMcp(Option<u16>),
    LoadDefaultMappings {
        epoch: u64,
    },
    DefaultMappingDialog {
        epoch: u64,
        format: String,
    },
    SaveDefaultMapping {
        format: String,
        path: String,
    },
    ListDocker {
        epoch: u64,
    },
    ConnectSource(crate::gui::sources::Source),
    ExportDialog {
        tab_id: crate::shared::TabId,
        template: String,
        fields: Vec<(String, String)>,
    },
    SaveLogsDialog {
        tab_id: crate::shared::TabId,
        marked: bool,
    },
    FilterFileDialog {
        action: crate::gui::actions::Action,
        tab_id: crate::shared::TabId,
    },
    ClearGuiDates {
        request_id: u64,
        tab_id: crate::shared::TabId,
        log_manager: LogManager,
    },
    SaveGuiTheme {
        request_id: u64,
        name: String,
        db: std::sync::Arc<crate::db::Database>,
    },
    SaveGuiFilter {
        request_id: u64,
        tab_id: crate::shared::TabId,
        log_manager: LogManager,
        id: Option<usize>,
        pattern: String,
        kind: crate::filters::FilterType,
        options: crate::filters::FilterOptions,
    },
    SaveGuiGroup {
        request_id: u64,
        tab_id: crate::shared::TabId,
        log_manager: LogManager,
        name: String,
        fg: Option<String>,
        bg: Option<String>,
        whole_line: bool,
        members: Vec<(usize, bool)>,
    },
    PersistFilterEdit(FilterEdit),
    ExecutePalette {
        request_id: u64,
        tab_id: crate::shared::TabId,
        log_manager: LogManager,
        command: Commands,
        theme_bg: (u8, u8, u8),
        filter_context: Option<usize>,
    },
    None,
    Quit,
    SaveKeybindings(crate::config::Keybindings),
    CopyText(String),
    OpenFileDialog,
    OpenDirectoryDialog,
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
        request: Option<(u64, crate::shared::TabId)>,
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

/// Ordered persistence operations never replace the live UI's filter snapshot.
#[derive(Debug)]
pub enum FilterEdit {
    Toggle {
        db: std::sync::Arc<crate::db::Database>,
        id: usize,
        enabled: bool,
    },
    Group {
        db: std::sync::Arc<crate::db::Database>,
        source: String,
        name: String,
        enabled: bool,
    },
    Reorder {
        db: std::sync::Arc<crate::db::Database>,
        order: Vec<usize>,
    },
}

impl FilterEdit {
    pub async fn persist(self) -> anyhow::Result<()> {
        use crate::db::{FilterStore, GroupStore};
        match self {
            Self::Toggle { db, id, enabled } => db.set_filter_enabled(id as i64, enabled).await?,
            Self::Group {
                db,
                source,
                name,
                enabled,
            } => {
                db.set_filters_enabled_by_group(&source, &name, enabled)
                    .await?;
                db.set_group_enabled(&source, &name, enabled).await?;
            }
            Self::Reorder { db, order } => db.set_filter_order(&order).await?,
        }
        Ok(())
    }
}
