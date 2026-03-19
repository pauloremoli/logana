pub mod comment_popup;
pub mod confirm_modal;
pub mod keybindings_help_popup;
pub mod select_fields_popup;
pub mod source_select_popup;
pub mod value_colors_popup;

pub use comment_popup::CommentPopup;
pub use confirm_modal::{ConfirmOpenDirModal, ConfirmRestoreModal, ConfirmRestoreSessionModal};
pub use keybindings_help_popup::KeybindingsHelpPopup;
pub use select_fields_popup::SelectFieldsPopup;
pub use source_select_popup::{DltSelectPopup, DockerSelectPopup};
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
