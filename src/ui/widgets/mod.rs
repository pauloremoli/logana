pub mod command_bar;
pub mod comment_popup;
pub mod confirm_modal;
pub mod input_bar;
pub mod keybindings_help_popup;
pub mod log_panel;
pub mod select_fields_popup;
pub mod sidebar;
pub mod source_select_popup;
pub mod tab_bar;
pub mod value_colors_popup;

pub use command_bar::{CommandBar, CompletionSource, file_display_name, resolve_completions};
pub use comment_popup::CommentPopup;
pub use confirm_modal::{ConfirmOpenDirModal, ConfirmRestoreModal, ConfirmRestoreSessionModal};
pub use input_bar::InputBar;
pub use keybindings_help_popup::KeybindingsHelpPopup;
pub use log_panel::{LogPanel, prepare_log_panel};
pub use select_fields_popup::SelectFieldsPopup;
pub use sidebar::Sidebar;
pub use source_select_popup::{DltSelectPopup, DockerSelectPopup};
pub use tab_bar::{TabBar, TabBarEntry};
pub use value_colors_popup::ValueColorsPopup;

pub(super) fn popup_entry(
    spans: &mut Vec<ratatui::prelude::Span<'static>>,
    key: String,
    label: &str,
    key_style: ratatui::prelude::Style,
    txt_style: ratatui::prelude::Style,
    br_style: ratatui::prelude::Style,
) {
    spans.push(ratatui::prelude::Span::styled("<", br_style));
    spans.push(ratatui::prelude::Span::styled(key, key_style));
    spans.push(ratatui::prelude::Span::styled(
        format!("> {}  ", label),
        txt_style,
    ));
}
