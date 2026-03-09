//! Character-level visual mode, entered with `v` from NormalMode.
//!
//! On entry the cursor moves freely with vim-style motions (h/l, w/b/e,
//! W/B/E, 0/^/$, f/F/t/T/;/,). Pressing `v` anchors the selection at the
//! current cursor position. Actions (i/o///y) work at any time: without an
//! anchor they operate on the single char under the cursor.

use async_trait::async_trait;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::{
    config::Keybindings,
    mode::app_mode::{Mode, ModeRenderState, status_entry},
    mode::command_mode::CommandMode,
    mode::normal_mode::NormalMode,
    mode::search_mode::SearchMode,
    theme::Theme,
    ui::{KeyResult, TabState, field_layout::apply_field_layout},
};

// ---------------------------------------------------------------------------
// PendingMotion
// ---------------------------------------------------------------------------

/// A character-search motion waiting for the target character.
#[derive(Debug, Clone)]
pub enum PendingMotion {
    FindForward,
    FindBackward,
    TillForward,
    TillBackward,
}

// ---------------------------------------------------------------------------
// VisualMode
// ---------------------------------------------------------------------------

/// Character-level visual mode.
///
/// On entry the cursor is placed at col 0 and moves freely.
/// Pressing `v` anchors the selection at the current cursor position;
/// subsequent motion keys extend it. Actions (filter/search/yank) work at
/// any time: without an anchor they operate on the single char under the cursor.
#[derive(Debug)]
pub struct VisualMode {
    /// Column where the selection was anchored (None = cursor-only, no active selection).
    pub anchor_col: Option<usize>,
    /// Column (char index) of the moving cursor.
    pub cursor_col: usize,
    /// Snapshot of the displayed line text captured on entry.
    pub line_text: String,
    /// A pending `f/F/t/T` motion waiting for a target character.
    pub pending_motion: Option<PendingMotion>,
    /// Last `f/F/t/T` motion for `;`/`,` repeat.
    pub last_char_motion: Option<(PendingMotion, char)>,
}

impl VisualMode {
    pub fn new(line_text: String) -> Self {
        Self {
            anchor_col: None,
            cursor_col: 0,
            line_text,
            pending_motion: None,
            last_char_motion: None,
        }
    }

    /// Refreshes `line_text` from the current scroll position, clamps `cursor_col`
    /// to the new line length, and resets selection state.
    /// Called after any motion that changes the active line.
    fn on_line_change(&mut self, tab: &TabState) {
        self.line_text = display_line_text(tab);
        let n = self.line_text.chars().count();
        self.cursor_col = if n == 0 {
            0
        } else {
            self.cursor_col.min(n - 1)
        };
        self.anchor_col = None;
        self.pending_motion = None;
    }

    /// Returns the currently selected text.
    /// Without an anchor, returns the single character under the cursor.
    fn selected_text(&self) -> String {
        let (lo, hi) = self.selection_range();
        self.line_text.chars().skip(lo).take(hi - lo + 1).collect()
    }

    /// Returns (lo, hi) char indices of the active selection (inclusive).
    /// Falls back to (cursor, cursor) when no anchor is set.
    pub fn selection_range(&self) -> (usize, usize) {
        let anchor = self.anchor_col.unwrap_or(self.cursor_col);
        let lo = anchor.min(self.cursor_col);
        let hi = anchor.max(self.cursor_col);
        (lo, hi)
    }
}

// ---------------------------------------------------------------------------
// Mode impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Mode for VisualMode {
    async fn handle_key(
        mut self: Box<Self>,
        tab: &mut TabState,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> (Box<dyn Mode>, KeyResult) {
        let kb = tab.keybindings.clone();

        if let Some(pending) = self.pending_motion.take() {
            if let KeyCode::Char(c) = key {
                self.cursor_col = apply_char_motion(&self.line_text, self.cursor_col, &pending, c);
                self.last_char_motion = Some((pending, c));
            }
            // Non-char key cancels the pending motion; cursor stays put.
            return (self, KeyResult::Handled);
        }

        let char_count = self.line_text.chars().count();

        // Clear the gg-chord flag for any key that isn't the go-to-top chord.
        if !kb.normal.go_to_top_chord.matches(key, modifiers) {
            tab.g_key_pressed = false;
        }

        if kb.visual.move_left.matches(key, modifiers) {
            self.cursor_col = char_left(self.cursor_col);
        } else if kb.visual.move_right.matches(key, modifiers) {
            self.cursor_col = char_right(&self.line_text, self.cursor_col);
        } else if kb.visual.word_forward.matches(key, modifiers) {
            self.cursor_col = word_forward(&self.line_text, self.cursor_col);
        } else if kb.visual.word_backward.matches(key, modifiers) {
            self.cursor_col = word_backward(&self.line_text, self.cursor_col);
        } else if kb.visual.word_end.matches(key, modifiers) {
            self.cursor_col = word_end(&self.line_text, self.cursor_col);
        } else if kb.visual.word_forward_big.matches(key, modifiers) {
            self.cursor_col = word_forward_big(&self.line_text, self.cursor_col);
        } else if kb.visual.word_backward_big.matches(key, modifiers) {
            self.cursor_col = word_backward_big(&self.line_text, self.cursor_col);
        } else if kb.visual.word_end_big.matches(key, modifiers) {
            self.cursor_col = word_end_big(&self.line_text, self.cursor_col);
        } else if kb.visual.start_of_line.matches(key, modifiers) {
            self.cursor_col = 0;
        } else if kb.visual.first_nonblank.matches(key, modifiers) {
            self.cursor_col = first_nonblank(&self.line_text);
        } else if kb.visual.end_of_line.matches(key, modifiers) {
            self.cursor_col = char_count.saturating_sub(1);
        } else if kb.visual.find_forward.matches(key, modifiers) {
            self.pending_motion = Some(PendingMotion::FindForward);
        } else if kb.visual.find_backward.matches(key, modifiers) {
            self.pending_motion = Some(PendingMotion::FindBackward);
        } else if kb.visual.till_forward.matches(key, modifiers) {
            self.pending_motion = Some(PendingMotion::TillForward);
        } else if kb.visual.till_backward.matches(key, modifiers) {
            self.pending_motion = Some(PendingMotion::TillBackward);
        } else if kb.visual.repeat_motion.matches(key, modifiers) {
            if let Some((motion, c)) = self.last_char_motion.clone() {
                self.cursor_col = apply_char_motion(&self.line_text, self.cursor_col, &motion, c);
            }
        } else if kb.visual.repeat_motion_rev.matches(key, modifiers) {
            if let Some((motion, c)) = self.last_char_motion.clone() {
                let reversed = reverse_motion(&motion);
                self.cursor_col = apply_char_motion(&self.line_text, self.cursor_col, &reversed, c);
            }
        } else if kb.visual.start_selection.matches(key, modifiers) {
            self.anchor_col = Some(self.cursor_col);
        } else if kb.visual.filter_include.matches(key, modifiers) {
            let selected = quote_for_command(&regex::escape(&self.selected_text()));
            let input = format!("filter {}", selected);
            let cursor = input.len();
            let history = tab.command_history.clone();
            return (
                Box::new(CommandMode::with_history(input, cursor, history)),
                KeyResult::Handled,
            );
        } else if kb.visual.filter_exclude.matches(key, modifiers) {
            let selected = quote_for_command(&regex::escape(&self.selected_text()));
            let input = format!("exclude {}", selected);
            let cursor = input.len();
            let history = tab.command_history.clone();
            return (
                Box::new(CommandMode::with_history(input, cursor, history)),
                KeyResult::Handled,
            );
        } else if kb.visual.search.matches(key, modifiers) {
            let selected = regex::escape(&self.selected_text());
            return (
                Box::new(SearchMode {
                    input: selected,
                    forward: true,
                }),
                KeyResult::Handled,
            );
        } else if kb.visual.yank.matches(key, modifiers) {
            let selected = self.selected_text();
            return (
                Box::new(NormalMode::default()),
                KeyResult::CopyToClipboard(selected),
            );
        } else if kb.visual.exit.matches(key, modifiers) {
            return (Box::new(NormalMode::default()), KeyResult::Handled);
        } else if kb.navigation.scroll_down.matches(key, modifiers) {
            tab.scroll_offset = tab.scroll_offset.saturating_add(1);
            self.on_line_change(tab);
        } else if kb.navigation.scroll_up.matches(key, modifiers) {
            tab.scroll_offset = tab.scroll_offset.saturating_sub(1);
            self.on_line_change(tab);
        } else if kb.navigation.half_page_down.matches(key, modifiers) {
            let half = (tab.visible_height / 2).max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(half);
            self.on_line_change(tab);
        } else if kb.navigation.half_page_up.matches(key, modifiers) {
            let half = (tab.visible_height / 2).max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(half);
            self.on_line_change(tab);
        } else if kb.navigation.page_down.matches(key, modifiers) {
            let page = tab.visible_height.max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_add(page);
            self.on_line_change(tab);
        } else if kb.navigation.page_up.matches(key, modifiers) {
            let page = tab.visible_height.max(1);
            tab.scroll_offset = tab.scroll_offset.saturating_sub(page);
            self.on_line_change(tab);
        } else if kb.normal.go_to_bottom.matches(key, modifiers) {
            let n = tab.visible_indices.len();
            if n > 0 {
                tab.scroll_offset = n - 1;
            }
            self.on_line_change(tab);
        } else if kb.normal.go_to_top_chord.matches(key, modifiers) {
            if tab.g_key_pressed {
                tab.scroll_offset = 0;
                tab.g_key_pressed = false;
                self.on_line_change(tab);
            } else {
                tab.g_key_pressed = true;
            }
        }

        (self, KeyResult::Handled)
    }

    fn mode_bar_content(&self, kb: &Keybindings, theme: &Theme) -> Line<'static> {
        if self.pending_motion.is_some() {
            return Line::from(vec![
                Span::styled(
                    "[VISUAL-CHAR]  ",
                    Style::default()
                        .fg(theme.text_highlight_fg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "pending — type a character",
                    Style::default().fg(theme.text),
                ),
            ]);
        }

        let label = if self.anchor_col.is_some() {
            "[VISUAL-CHAR SELECT]  "
        } else {
            "[VISUAL-CHAR]  "
        };
        let mut spans: Vec<Span<'static>> = vec![Span::styled(
            label,
            Style::default()
                .fg(theme.text_highlight_fg)
                .add_modifier(Modifier::BOLD),
        )];
        status_entry(
            &mut spans,
            kb.navigation.scroll_down.display(),
            "line↓",
            theme,
        );
        status_entry(
            &mut spans,
            kb.navigation.scroll_up.display(),
            "line↑",
            theme,
        );
        status_entry(&mut spans, kb.visual.move_left.display(), "char←", theme);
        status_entry(&mut spans, kb.visual.move_right.display(), "char→", theme);
        status_entry(&mut spans, kb.visual.word_forward.display(), "word→", theme);
        status_entry(&mut spans, kb.visual.word_end.display(), "end→", theme);
        status_entry(
            &mut spans,
            kb.visual.word_backward.display(),
            "word←",
            theme,
        );
        status_entry(
            &mut spans,
            kb.visual.start_of_line.display(),
            "line-start",
            theme,
        );
        status_entry(
            &mut spans,
            kb.visual.end_of_line.display(),
            "line-end",
            theme,
        );
        status_entry(&mut spans, kb.visual.find_forward.display(), "find→", theme);
        status_entry(
            &mut spans,
            kb.visual.start_selection.display(),
            "select",
            theme,
        );
        status_entry(
            &mut spans,
            kb.visual.filter_include.display(),
            "filter",
            theme,
        );
        status_entry(
            &mut spans,
            kb.visual.filter_exclude.display(),
            "exclude",
            theme,
        );
        status_entry(&mut spans, kb.visual.search.display(), "search", theme);
        status_entry(&mut spans, kb.visual.yank.display(), "yank", theme);
        status_entry(&mut spans, kb.visual.exit.display(), "cancel", theme);
        Line::from(spans)
    }

    fn render_state(&self) -> ModeRenderState {
        ModeRenderState::Visual {
            anchor_col: self.anchor_col,
            cursor_col: self.cursor_col,
            pending_motion: self.pending_motion.is_some(),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helper — also used by normal_mode to capture the line text on entry
// ---------------------------------------------------------------------------

/// Wraps `pattern` in double quotes when it contains whitespace, escaping
/// any embedded `"` as `\"` so that `shell_split` reconstructs the original
/// pattern correctly.
pub(crate) fn quote_for_command(pattern: &str) -> String {
    if pattern.chars().any(char::is_whitespace) {
        let escaped = pattern.replace('"', "\\\"");
        format!("\"{}\"", escaped)
    } else {
        pattern.to_string()
    }
}

/// Returns the displayed text for the current scroll line using parsed
/// field-layout (if available) or raw bytes as fallback.
pub(crate) fn display_line_text(tab: &TabState) -> String {
    if let Some(idx) = tab.visible_indices.get_opt(tab.scroll_offset) {
        let bytes = tab.file_reader.get_line(idx);
        if !tab.raw_mode
            && let Some(parser) = tab.detected_format.as_ref()
            && let Some(parts) = parser.parse_line(bytes)
        {
            return apply_field_layout(
                &parts,
                &tab.field_layout,
                &tab.hidden_fields,
                tab.show_keys,
            )
            .join(" ");
        }
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// Motion helpers
// ---------------------------------------------------------------------------

fn apply_char_motion(text: &str, col: usize, motion: &PendingMotion, c: char) -> usize {
    match motion {
        PendingMotion::FindForward => find_char_forward(text, col, c),
        PendingMotion::FindBackward => find_char_backward(text, col, c),
        PendingMotion::TillForward => till_char_forward(text, col, c),
        PendingMotion::TillBackward => till_char_backward(text, col, c),
    }
}

fn reverse_motion(motion: &PendingMotion) -> PendingMotion {
    match motion {
        PendingMotion::FindForward => PendingMotion::FindBackward,
        PendingMotion::FindBackward => PendingMotion::FindForward,
        PendingMotion::TillForward => PendingMotion::TillBackward,
        PendingMotion::TillBackward => PendingMotion::TillForward,
    }
}

pub fn char_left(col: usize) -> usize {
    col.saturating_sub(1)
}

pub fn char_right(text: &str, col: usize) -> usize {
    let n = text.chars().count();
    if n == 0 { 0 } else { (col + 1).min(n - 1) }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn word_forward(text: &str, col: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if col >= n {
        return n.saturating_sub(1);
    }
    let mut pos = col;
    if is_word_char(chars[pos]) {
        while pos < n && is_word_char(chars[pos]) {
            pos += 1;
        }
    } else if !chars[pos].is_whitespace() {
        while pos < n && !is_word_char(chars[pos]) && !chars[pos].is_whitespace() {
            pos += 1;
        }
    }
    while pos < n && chars[pos].is_whitespace() {
        pos += 1;
    }
    pos.min(n.saturating_sub(1))
}

pub fn word_backward(text: &str, col: usize) -> usize {
    if col == 0 {
        return 0;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut pos = col - 1;
    while pos > 0 && chars[pos].is_whitespace() {
        pos -= 1;
    }
    if chars[pos].is_whitespace() {
        return 0;
    }
    if is_word_char(chars[pos]) {
        while pos > 0 && is_word_char(chars[pos - 1]) {
            pos -= 1;
        }
    } else {
        while pos > 0 && !is_word_char(chars[pos - 1]) && !chars[pos - 1].is_whitespace() {
            pos -= 1;
        }
    }
    pos
}

pub fn word_end(text: &str, col: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return 0;
    }
    let mut pos = col;
    // Step right if already at the end of a word/punct group.
    if pos + 1 < n {
        let at_word_end = is_word_char(chars[pos]) && !is_word_char(chars[pos + 1]);
        let at_punct_end = !is_word_char(chars[pos])
            && !chars[pos].is_whitespace()
            && (is_word_char(chars[pos + 1]) || chars[pos + 1].is_whitespace());
        if at_word_end || at_punct_end {
            pos += 1;
        }
    } else {
        return n - 1;
    }
    while pos < n && chars[pos].is_whitespace() {
        pos += 1;
    }
    if pos >= n {
        return n - 1;
    }
    if is_word_char(chars[pos]) {
        while pos + 1 < n && is_word_char(chars[pos + 1]) {
            pos += 1;
        }
    } else {
        while pos + 1 < n && !is_word_char(chars[pos + 1]) && !chars[pos + 1].is_whitespace() {
            pos += 1;
        }
    }
    pos
}

pub fn word_forward_big(text: &str, col: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if col >= n {
        return n.saturating_sub(1);
    }
    let mut pos = col;
    while pos < n && !chars[pos].is_whitespace() {
        pos += 1;
    }
    while pos < n && chars[pos].is_whitespace() {
        pos += 1;
    }
    pos.min(n.saturating_sub(1))
}

pub fn word_backward_big(text: &str, col: usize) -> usize {
    if col == 0 {
        return 0;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut pos = col - 1;
    while pos > 0 && chars[pos].is_whitespace() {
        pos -= 1;
    }
    if chars[pos].is_whitespace() {
        return 0;
    }
    while pos > 0 && !chars[pos - 1].is_whitespace() {
        pos -= 1;
    }
    pos
}

pub fn word_end_big(text: &str, col: usize) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return 0;
    }
    let mut pos = col;
    if pos + 1 < n && !chars[pos].is_whitespace() && chars[pos + 1].is_whitespace() {
        pos += 1;
    } else if pos + 1 >= n {
        return n - 1;
    }
    while pos < n && chars[pos].is_whitespace() {
        pos += 1;
    }
    if pos >= n {
        return n - 1;
    }
    while pos + 1 < n && !chars[pos + 1].is_whitespace() {
        pos += 1;
    }
    pos
}

pub fn first_nonblank(text: &str) -> usize {
    text.chars().position(|c| !c.is_whitespace()).unwrap_or(0)
}

pub fn find_char_forward(text: &str, col: usize, target: char) -> usize {
    text.chars()
        .enumerate()
        .skip(col + 1)
        .find(|&(_, c)| c == target)
        .map(|(i, _)| i)
        .unwrap_or(col)
}

pub fn find_char_backward(text: &str, col: usize, target: char) -> usize {
    if col == 0 {
        return 0;
    }
    let chars: Vec<char> = text.chars().collect();
    for i in (0..col).rev() {
        if chars[i] == target {
            return i;
        }
    }
    col // not found, stay
}

pub fn till_char_forward(text: &str, col: usize, target: char) -> usize {
    let found = find_char_forward(text, col, target);
    if found > col {
        found.saturating_sub(1)
    } else {
        col
    }
}

pub fn till_char_backward(text: &str, col: usize, target: char) -> usize {
    let found = find_char_backward(text, col, target);
    if found < col {
        (found + 1).min(col)
    } else {
        col
    }
}

/// Overlay REVERSED modifier on chars [lo, hi] (inclusive) of a rendered Line.
/// Used by the render layer to show the visual-char selection.
pub fn apply_char_selection(line: Line<'static>, lo: usize, hi: usize) -> Line<'static> {
    let base_style = line.style;
    let mut new_spans: Vec<Span<'static>> = Vec::new();
    let mut char_pos = 0usize;

    for span in line.spans {
        let content = span.content.as_ref().to_owned();
        let style = span.style;
        let span_chars: Vec<char> = content.chars().collect();
        let span_len = span_chars.len();
        let span_end = char_pos + span_len;

        // Compute overlap with [lo, hi] within this span.
        let sel_start = lo.saturating_sub(char_pos).min(span_len);
        let sel_end = (hi + 1).saturating_sub(char_pos).min(span_len);
        let has_overlap = sel_start < sel_end && char_pos <= hi && span_end > lo;

        if !has_overlap {
            new_spans.push(Span::styled(content, style));
        } else {
            if sel_start > 0 {
                let s: String = span_chars[..sel_start].iter().collect();
                new_spans.push(Span::styled(s, style));
            }
            let s: String = span_chars[sel_start..sel_end].iter().collect();
            new_spans.push(Span::styled(s, style.add_modifier(Modifier::REVERSED)));
            if sel_end < span_len {
                let s: String = span_chars[sel_end..].iter().collect();
                new_spans.push(Span::styled(s, style));
            }
        }

        char_pos = span_end;
    }

    Line::from(new_spans).style(base_style)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::mode::app_mode::ModeRenderState;
    use crate::ui::TabState;
    use std::sync::Arc;

    async fn make_tab(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        TabState::new(file_reader, log_manager, "test".to_string())
    }

    fn make_mode(text: &str) -> VisualMode {
        VisualMode::new(text.to_string())
    }

    async fn press(
        mode: VisualMode,
        tab: &mut TabState,
        key: KeyCode,
    ) -> (Box<dyn Mode>, KeyResult) {
        Box::new(mode)
            .handle_key(tab, key, KeyModifiers::NONE)
            .await
    }

    fn cursor_col(mode: &dyn Mode) -> usize {
        match mode.render_state() {
            ModeRenderState::Visual { cursor_col, .. } => cursor_col,
            other => panic!("expected Visual, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_h_moves_left() {
        let mut tab = make_tab(&["hello"]).await;
        let mut mode = make_mode("hello");
        mode.cursor_col = 3;
        let (m, _) = press(mode, &mut tab, KeyCode::Char('h')).await;
        assert_eq!(cursor_col(m.as_ref()), 2);
    }

    #[tokio::test]
    async fn test_h_saturates_at_zero() {
        let mut tab = make_tab(&["hello"]).await;
        let mode = make_mode("hello");
        let (m, _) = press(mode, &mut tab, KeyCode::Char('h')).await;
        assert_eq!(cursor_col(m.as_ref()), 0);
    }

    #[tokio::test]
    async fn test_l_moves_right() {
        let mut tab = make_tab(&["hello"]).await;
        let mode = make_mode("hello");
        let (m, _) = press(mode, &mut tab, KeyCode::Char('l')).await;
        assert_eq!(cursor_col(m.as_ref()), 1);
    }

    #[tokio::test]
    async fn test_l_saturates_at_end() {
        let mut tab = make_tab(&["hi"]).await;
        let mut mode = make_mode("hi");
        mode.cursor_col = 1;
        let (m, _) = press(mode, &mut tab, KeyCode::Char('l')).await;
        assert_eq!(cursor_col(m.as_ref()), 1);
    }

    #[tokio::test]
    async fn test_zero_moves_to_start() {
        let mut tab = make_tab(&["hello"]).await;
        let mut mode = make_mode("hello");
        mode.cursor_col = 4;
        let (m, _) = press(mode, &mut tab, KeyCode::Char('0')).await;
        assert_eq!(cursor_col(m.as_ref()), 0);
    }

    #[tokio::test]
    async fn test_caret_moves_to_first_nonblank() {
        let mut tab = make_tab(&["  hello"]).await;
        let mut mode = make_mode("  hello");
        mode.cursor_col = 6;
        let (m, _) = press(mode, &mut tab, KeyCode::Char('^')).await;
        assert_eq!(cursor_col(m.as_ref()), 2);
    }

    #[tokio::test]
    async fn test_dollar_moves_to_end() {
        let mut tab = make_tab(&["hello"]).await;
        let mode = make_mode("hello");
        let (m, _) = press(mode, &mut tab, KeyCode::Char('$')).await;
        assert_eq!(cursor_col(m.as_ref()), 4);
    }

    #[tokio::test]
    async fn test_w_moves_to_next_word() {
        let mut tab = make_tab(&["foo bar"]).await;
        let mode = make_mode("foo bar");
        let (m, _) = press(mode, &mut tab, KeyCode::Char('w')).await;
        assert_eq!(cursor_col(m.as_ref()), 4);
    }

    #[tokio::test]
    async fn test_b_moves_to_word_start() {
        let mut tab = make_tab(&["foo bar"]).await;
        let mut mode = make_mode("foo bar");
        mode.cursor_col = 4;
        let (m, _) = press(mode, &mut tab, KeyCode::Char('b')).await;
        assert_eq!(cursor_col(m.as_ref()), 0);
    }

    #[tokio::test]
    async fn test_e_moves_to_word_end() {
        let mut tab = make_tab(&["foo bar"]).await;
        let mode = make_mode("foo bar");
        let (m, _) = press(mode, &mut tab, KeyCode::Char('e')).await;
        assert_eq!(cursor_col(m.as_ref()), 2);
    }

    #[tokio::test]
    async fn test_big_w_skips_whitespace_delimited() {
        let mut tab = make_tab(&["foo.bar baz"]).await;
        let mode = make_mode("foo.bar baz");
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('W'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(m.as_ref()), 8);
    }

    #[tokio::test]
    async fn test_big_b_skips_whitespace_delimited_backward() {
        let mut tab = make_tab(&["foo bar.baz"]).await;
        let mut mode = make_mode("foo bar.baz");
        mode.cursor_col = 10;
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('B'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(m.as_ref()), 4);
    }

    #[tokio::test]
    async fn test_big_e_skips_whitespace_delimited_end() {
        let mut tab = make_tab(&["foo bar.baz"]).await;
        let mut mode = make_mode("foo bar.baz");
        mode.cursor_col = 3; // space after "foo"
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('E'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(m.as_ref()), 10);
    }

    #[tokio::test]
    async fn test_f_enters_pending_then_finds_char() {
        let mut tab = make_tab(&["hello world"]).await;
        let mode = make_mode("hello world");
        // 'f' → pending
        let (mb, _) = press(mode, &mut tab, KeyCode::Char('f')).await;
        assert!(matches!(
            mb.render_state(),
            ModeRenderState::Visual {
                pending_motion: true,
                ..
            }
        ));
        // 'o' → find first 'o' after col 0 → index 4
        let (mc, _) = mb
            .handle_key(&mut tab, KeyCode::Char('o'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(mc.as_ref()), 4);
    }

    #[tokio::test]
    async fn test_f_no_match_stays_put() {
        let mut tab = make_tab(&["hello"]).await;
        let mode = make_mode("hello");
        let (mb, _) = press(mode, &mut tab, KeyCode::Char('f')).await;
        let (mc, _) = mb
            .handle_key(&mut tab, KeyCode::Char('z'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(mc.as_ref()), 0);
    }

    #[tokio::test]
    async fn test_big_f_finds_prev_char() {
        let mut tab = make_tab(&["hello world"]).await;
        let mut mode = make_mode("hello world");
        mode.cursor_col = 10;
        let (mb, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE)
            .await;
        // 'o' backward from 10 → 'o' in "world" at index 7
        let (mc, _) = mb
            .handle_key(&mut tab, KeyCode::Char('o'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(mc.as_ref()), 7);
    }

    #[tokio::test]
    async fn test_t_stops_one_before() {
        let mut tab = make_tab(&["hello"]).await;
        let mode = make_mode("hello");
        let (mb, _) = press(mode, &mut tab, KeyCode::Char('t')).await;
        // 'l' found at 2, stop at 1
        let (mc, _) = mb
            .handle_key(&mut tab, KeyCode::Char('l'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(mc.as_ref()), 1);
    }

    #[tokio::test]
    async fn test_big_t_stops_one_after() {
        let mut tab = make_tab(&["hello"]).await;
        let mut mode = make_mode("hello");
        mode.cursor_col = 4;
        let (mb, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('T'), KeyModifiers::NONE)
            .await;
        // 'e' found at 1, stop at 2
        let (mc, _) = mb
            .handle_key(&mut tab, KeyCode::Char('e'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(mc.as_ref()), 2);
    }

    #[tokio::test]
    async fn test_semicolon_repeats_last_motion() {
        let mut tab = make_tab(&["aXbXcX"]).await;
        let mode = make_mode("aXbXcX");
        // f X → col=1
        let (mb, _) = press(mode, &mut tab, KeyCode::Char('f')).await;
        let (mb, _) = mb
            .handle_key(&mut tab, KeyCode::Char('X'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(mb.as_ref()), 1);
        // ; → col=3
        let (mb, _) = mb
            .handle_key(&mut tab, KeyCode::Char(';'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(mb.as_ref()), 3);
        // ; again → col=5
        let (mb, _) = mb
            .handle_key(&mut tab, KeyCode::Char(';'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(mb.as_ref()), 5);
    }

    #[tokio::test]
    async fn test_comma_reverses_motion() {
        let mut tab = make_tab(&["aXbXcX"]).await;
        let mut mode = make_mode("aXbXcX");
        mode.cursor_col = 5;
        // F X → col=3
        let (mb, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('F'), KeyModifiers::NONE)
            .await;
        let (mb, _) = mb
            .handle_key(&mut tab, KeyCode::Char('X'), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(mb.as_ref()), 3);
        // , → reverse F = f X → from 3 forward → col=5
        let (mb, _) = mb
            .handle_key(&mut tab, KeyCode::Char(','), KeyModifiers::NONE)
            .await;
        assert_eq!(cursor_col(mb.as_ref()), 5);
    }

    #[tokio::test]
    async fn test_pending_cancelled_by_non_char_key() {
        let mut tab = make_tab(&["hello"]).await;
        let mode = make_mode("hello");
        let (mb, _) = press(mode, &mut tab, KeyCode::Char('f')).await;
        assert!(matches!(
            mb.render_state(),
            ModeRenderState::Visual {
                pending_motion: true,
                ..
            }
        ));
        // Left arrow cancels pending, cursor stays at 0
        let (mb2, _) = mb
            .handle_key(&mut tab, KeyCode::Left, KeyModifiers::NONE)
            .await;
        assert!(matches!(
            mb2.render_state(),
            ModeRenderState::Visual {
                pending_motion: false,
                ..
            }
        ));
        assert_eq!(cursor_col(mb2.as_ref()), 0);
    }

    #[tokio::test]
    async fn test_i_enters_command_mode_with_filter_prefix() {
        let mut tab = make_tab(&["hello world"]).await;
        let mut mode = make_mode("hello world");
        mode.anchor_col = Some(0);
        mode.cursor_col = 4; // selected = "hello"
        let (m, _) = press(mode, &mut tab, KeyCode::Char('i')).await;
        match m.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("filter "), "got: {input}");
                assert!(input.contains("hello"));
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_i_escapes_regex_metacharacters_in_filter() {
        let mut tab = make_tab(&["192.168.1.1 GET /api"]).await;
        let mut mode = make_mode("192.168.1.1 GET /api");
        mode.anchor_col = Some(0);
        mode.cursor_col = 10; // selected = "192.168.1.1"
        let (m, _) = press(mode, &mut tab, KeyCode::Char('i')).await;
        match m.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, r"filter 192\.168\.1\.1", "got: {input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_o_enters_command_mode_with_exclude_prefix() {
        let mut tab = make_tab(&["hello world"]).await;
        let mut mode = make_mode("hello world");
        mode.anchor_col = Some(0);
        mode.cursor_col = 4; // selected = "hello"
        let (m, _) = press(mode, &mut tab, KeyCode::Char('o')).await;
        match m.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert!(input.starts_with("exclude "), "got: {input}");
                assert!(input.contains("hello"));
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_o_escapes_regex_metacharacters_in_exclude() {
        let mut tab = make_tab(&["error: connection(reset)"]).await;
        let mut mode = make_mode("error: connection(reset)");
        mode.anchor_col = Some(7);
        mode.cursor_col = 23; // selected = "connection(reset)"
        let (m, _) = press(mode, &mut tab, KeyCode::Char('o')).await;
        match m.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, r"exclude connection\(reset\)", "got: {input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_filter_with_spaces_wraps_in_quotes() {
        let mut tab = make_tab(&["hello world foo"]).await;
        let mut mode = make_mode("hello world foo");
        mode.anchor_col = Some(0);
        mode.cursor_col = 10; // selected = "hello world"
        let (m, _) = press(mode, &mut tab, KeyCode::Char('i')).await;
        match m.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, r#"filter "hello world""#, "got: {input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_exclude_with_spaces_wraps_in_quotes() {
        let mut tab = make_tab(&["error hello world"]).await;
        let mut mode = make_mode("error hello world");
        mode.anchor_col = Some(6);
        mode.cursor_col = 16; // selected = "hello world"
        let (m, _) = press(mode, &mut tab, KeyCode::Char('o')).await;
        match m.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, r#"exclude "hello world""#, "got: {input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_filter_with_embedded_quote_escaped() {
        let mut tab = make_tab(&[r#"say "hi" now"#]).await;
        let mut mode = make_mode(r#"say "hi" now"#);
        mode.anchor_col = Some(0);
        mode.cursor_col = 11; // selected = r#"say "hi" now"#
        let (m, _) = press(mode, &mut tab, KeyCode::Char('i')).await;
        match m.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, r#"filter "say \"hi\" now""#, "got: {input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_filter_no_spaces_no_quotes() {
        let mut tab = make_tab(&["error"]).await;
        let mut mode = make_mode("error");
        mode.anchor_col = Some(0);
        mode.cursor_col = 4;
        let (m, _) = press(mode, &mut tab, KeyCode::Char('i')).await;
        match m.render_state() {
            ModeRenderState::Command { input, .. } => {
                assert_eq!(input, "filter error", "got: {input}");
            }
            other => panic!("expected Command, got {:?}", other),
        }
    }

    // ── quote_for_command unit tests ─────────────────────────────────────────

    #[test]
    fn test_quote_for_command_no_spaces() {
        assert_eq!(quote_for_command("hello"), "hello");
        assert_eq!(quote_for_command(r"192\.168\.1\.1"), r"192\.168\.1\.1");
    }

    #[test]
    fn test_quote_for_command_with_spaces() {
        assert_eq!(quote_for_command("hello world"), r#""hello world""#);
    }

    #[test]
    fn test_quote_for_command_embedded_quote() {
        assert_eq!(quote_for_command(r#"say "hi" now"#), r#""say \"hi\" now""#);
    }

    #[test]
    fn test_quote_for_command_empty() {
        assert_eq!(quote_for_command(""), "");
    }

    #[tokio::test]
    async fn test_slash_enters_search_mode_with_selected_text() {
        let mut tab = make_tab(&["hello world"]).await;
        let mut mode = make_mode("hello world");
        mode.anchor_col = Some(0);
        mode.cursor_col = 4; // selected = "hello"
        let (m, _) = press(mode, &mut tab, KeyCode::Char('/')).await;
        match m.render_state() {
            ModeRenderState::Search { query, forward } => {
                assert!(query.contains("hello"), "got: {query}");
                assert!(forward);
            }
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_slash_escapes_regex_metacharacters_in_search() {
        let mut tab = make_tab(&["GET /api/v1?foo=bar"]).await;
        let mut mode = make_mode("GET /api/v1?foo=bar");
        mode.anchor_col = Some(4);
        mode.cursor_col = 18; // selected = "/api/v1?foo=bar"
        let (m, _) = press(mode, &mut tab, KeyCode::Char('/')).await;
        match m.render_state() {
            ModeRenderState::Search { query, forward } => {
                assert_eq!(query, r"/api/v1\?foo=bar", "got: {query}");
                assert!(forward);
            }
            other => panic!("expected Search, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_y_copies_selected_text_to_clipboard() {
        let mut tab = make_tab(&["hello world"]).await;
        let mut mode = make_mode("hello world");
        mode.anchor_col = Some(0);
        mode.cursor_col = 4;
        let (m, result) = press(mode, &mut tab, KeyCode::Char('y')).await;
        assert!(matches!(m.render_state(), ModeRenderState::Normal));
        match result {
            KeyResult::CopyToClipboard(text) => assert_eq!(text, "hello"),
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_y_anchor_after_cursor_still_selects_correctly() {
        let mut tab = make_tab(&["hello"]).await;
        let mut mode = make_mode("hello");
        mode.anchor_col = Some(4);
        mode.cursor_col = 1;
        let (_, result) = press(mode, &mut tab, KeyCode::Char('y')).await;
        match result {
            KeyResult::CopyToClipboard(text) => assert_eq!(text, "ello"),
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_v_anchors_selection_at_cursor() {
        let mut tab = make_tab(&["hello"]).await;
        let mut mode = make_mode("hello");
        mode.cursor_col = 2;
        assert!(mode.anchor_col.is_none());
        let (m, _) = press(mode, &mut tab, KeyCode::Char('v')).await;
        match m.render_state() {
            ModeRenderState::Visual {
                anchor_col,
                cursor_col,
                ..
            } => {
                assert_eq!(anchor_col, Some(2));
                assert_eq!(cursor_col, 2);
            }
            other => panic!("expected Visual, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_y_without_anchor_copies_single_char_under_cursor() {
        let mut tab = make_tab(&["hello"]).await;
        let mut mode = make_mode("hello");
        mode.cursor_col = 1; // 'e'
        let (_, result) = press(mode, &mut tab, KeyCode::Char('y')).await;
        match result {
            KeyResult::CopyToClipboard(text) => assert_eq!(text, "e"),
            other => panic!("expected CopyToClipboard, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_esc_returns_normal_mode() {
        let mut tab = make_tab(&["hello"]).await;
        let mode = make_mode("hello");
        let (m, result) = press(mode, &mut tab, KeyCode::Esc).await;
        assert!(matches!(result, KeyResult::Handled));
        assert!(matches!(m.render_state(), ModeRenderState::Normal));
    }

    #[test]
    fn test_mode_bar_content_contains_filter_search_yank() {
        let mode = make_mode("hello");
        let content = mode.mode_bar_content(&Keybindings::default(), &Theme::default());
        let text: String = content.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("filter"), "missing filter in: {text}");
        assert!(text.contains("search"), "missing search in: {text}");
        assert!(text.contains("yank"), "missing yank in: {text}");
    }

    #[test]
    fn test_mode_bar_pending_shows_pending_message() {
        let mut mode = make_mode("hello");
        mode.pending_motion = Some(PendingMotion::FindForward);
        let content = mode.mode_bar_content(&Keybindings::default(), &Theme::default());
        let text: String = content.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("pending"), "missing pending in: {text}");
    }

    #[test]
    fn test_word_forward_from_start() {
        assert_eq!(word_forward("foo bar", 0), 4);
    }

    #[test]
    fn test_word_forward_at_end() {
        assert_eq!(word_forward("foo", 2), 2);
    }

    #[test]
    fn test_word_backward_from_second_word() {
        assert_eq!(word_backward("foo bar", 4), 0);
    }

    #[test]
    fn test_word_backward_at_start() {
        assert_eq!(word_backward("foo", 0), 0);
    }

    #[test]
    fn test_word_end_from_start() {
        assert_eq!(word_end("foo bar", 0), 2);
    }

    #[test]
    fn test_first_nonblank_with_leading_spaces() {
        assert_eq!(first_nonblank("  hello"), 2);
    }

    #[test]
    fn test_first_nonblank_no_leading_spaces() {
        assert_eq!(first_nonblank("hello"), 0);
    }

    #[test]
    fn test_first_nonblank_all_spaces() {
        assert_eq!(first_nonblank("   "), 0);
    }

    #[test]
    fn test_find_char_forward_finds_first() {
        assert_eq!(find_char_forward("hello", 0, 'l'), 2);
    }

    #[test]
    fn test_find_char_forward_finds_second() {
        assert_eq!(find_char_forward("hello", 2, 'l'), 3);
    }

    #[test]
    fn test_find_char_forward_not_found() {
        assert_eq!(find_char_forward("hello", 4, 'z'), 4);
    }

    #[test]
    fn test_find_char_backward_finds_last() {
        assert_eq!(find_char_backward("hello", 4, 'l'), 3);
    }

    #[test]
    fn test_find_char_backward_finds_prev() {
        assert_eq!(find_char_backward("hello", 3, 'l'), 2);
    }

    #[test]
    fn test_find_char_backward_not_found() {
        assert_eq!(find_char_backward("hello", 1, 'z'), 1);
    }

    #[test]
    fn test_till_char_forward_stops_before() {
        assert_eq!(till_char_forward("hello", 0, 'l'), 1); // 'l' at 2, stop at 1
    }

    #[test]
    fn test_till_char_forward_not_found() {
        assert_eq!(till_char_forward("hello", 0, 'z'), 0);
    }

    #[test]
    fn test_till_char_backward_stops_after() {
        assert_eq!(till_char_backward("hello", 4, 'e'), 2); // 'e' at 1, one after = 2
    }

    #[test]
    fn test_char_right_empty_string() {
        assert_eq!(char_right("", 0), 0);
    }

    #[test]
    fn test_word_forward_big() {
        assert_eq!(word_forward_big("foo.bar baz", 0), 8);
    }

    #[test]
    fn test_word_backward_big() {
        assert_eq!(word_backward_big("foo bar.baz", 10), 4);
    }

    #[test]
    fn test_apply_char_selection_single_span() {
        let line = Line::from("hello world");
        let result = apply_char_selection(line, 0, 4);
        let text: String = result.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "hello world");
        // First span should have REVERSED
        assert!(result.spans[0].style.add_modifier == Modifier::REVERSED);
        // Suffix should not have REVERSED
        assert!(
            !result.spans[1]
                .style
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn test_apply_char_selection_full_span() {
        let line = Line::from("hi");
        let result = apply_char_selection(line, 0, 1);
        assert_eq!(result.spans.len(), 1);
        assert!(
            result.spans[0]
                .style
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn test_apply_char_selection_out_of_range_noop() {
        let line = Line::from("hello");
        let result = apply_char_selection(line, 10, 20);
        // No selection overlap: single span, no REVERSED
        assert_eq!(result.spans.len(), 1);
        assert!(
            !result.spans[0]
                .style
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    // ── Line navigation ──────────────────────────────────────────────────────

    async fn make_multi_tab(lines: &[&str]) -> TabState {
        let data = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut tab = TabState::new(file_reader, log_manager, "test".to_string());
        tab.refresh_visible();
        tab
    }

    #[tokio::test]
    async fn test_j_scrolls_down_and_updates_line_text() {
        let mut tab = make_multi_tab(&["line0", "line1", "line2"]).await;
        tab.scroll_offset = 0;
        let mode = VisualMode::new("line0".to_string());
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 1);
        match m.render_state() {
            ModeRenderState::Visual {
                anchor_col,
                cursor_col,
                ..
            } => {
                assert_eq!(anchor_col, None);
                assert_eq!(cursor_col, 0);
            }
            other => panic!("expected Visual, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_k_scrolls_up_and_updates_line_text() {
        let mut tab = make_multi_tab(&["line0", "line1", "line2"]).await;
        tab.scroll_offset = 2;
        let mode = VisualMode::new("line2".to_string());
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('k'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 1);
        assert!(matches!(m.render_state(), ModeRenderState::Visual { .. }));
    }

    #[tokio::test]
    async fn test_j_clamps_cursor_col_to_new_line_length() {
        let mut tab = make_multi_tab(&["long line here", "hi"]).await;
        tab.scroll_offset = 0;
        let mut mode = VisualMode::new("long line here".to_string());
        mode.cursor_col = 10;
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 1);
        // "hi" has 2 chars; col 10 clamps to 1
        assert_eq!(cursor_col(m.as_ref()), 1);
    }

    #[tokio::test]
    async fn test_j_resets_anchor() {
        let mut tab = make_multi_tab(&["line0", "line1"]).await;
        tab.scroll_offset = 0;
        let mut mode = VisualMode::new("line0".to_string());
        mode.anchor_col = Some(2);
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('j'), KeyModifiers::NONE)
            .await;
        match m.render_state() {
            ModeRenderState::Visual { anchor_col, .. } => assert_eq!(anchor_col, None),
            other => panic!("expected Visual, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_k_at_top_stays_at_zero() {
        let mut tab = make_multi_tab(&["only"]).await;
        tab.scroll_offset = 0;
        let mode = VisualMode::new("only".to_string());
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('k'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 0);
        assert!(matches!(m.render_state(), ModeRenderState::Visual { .. }));
    }

    #[tokio::test]
    async fn test_capital_g_goes_to_last_line() {
        let mut tab = make_multi_tab(&["a", "b", "c"]).await;
        tab.scroll_offset = 0;
        let mode = VisualMode::new("a".to_string());
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('G'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 2);
        assert!(matches!(m.render_state(), ModeRenderState::Visual { .. }));
    }

    #[tokio::test]
    async fn test_gg_chord_goes_to_first_line() {
        let mut tab = make_multi_tab(&["a", "b", "c"]).await;
        tab.scroll_offset = 2;
        let mode = VisualMode::new("c".to_string());
        // first 'g' sets the flag
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 2); // not moved yet
        assert!(tab.g_key_pressed);
        // second 'g' jumps to top
        let (m2, _) = m
            .handle_key(&mut tab, KeyCode::Char('g'), KeyModifiers::NONE)
            .await;
        assert_eq!(tab.scroll_offset, 0);
        assert!(!tab.g_key_pressed);
        assert!(matches!(m2.render_state(), ModeRenderState::Visual { .. }));
    }

    #[tokio::test]
    async fn test_non_g_key_clears_g_flag() {
        let mut tab = make_multi_tab(&["a", "b"]).await;
        tab.scroll_offset = 1;
        tab.g_key_pressed = true;
        let mode = VisualMode::new("b".to_string());
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('h'), KeyModifiers::NONE)
            .await;
        assert!(!tab.g_key_pressed);
        assert!(matches!(m.render_state(), ModeRenderState::Visual { .. }));
    }

    #[tokio::test]
    async fn test_ctrl_d_half_page_down() {
        let mut tab = make_multi_tab(&["a", "b", "c", "d", "e"]).await;
        tab.scroll_offset = 0;
        tab.visible_height = 4;
        let mode = VisualMode::new("a".to_string());
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('d'), KeyModifiers::CONTROL)
            .await;
        assert_eq!(tab.scroll_offset, 2); // half of 4 = 2
        assert!(matches!(m.render_state(), ModeRenderState::Visual { .. }));
    }

    #[tokio::test]
    async fn test_ctrl_u_half_page_up() {
        let mut tab = make_multi_tab(&["a", "b", "c", "d", "e"]).await;
        tab.scroll_offset = 4;
        tab.visible_height = 4;
        let mode = VisualMode::new("e".to_string());
        let (m, _) = Box::new(mode)
            .handle_key(&mut tab, KeyCode::Char('u'), KeyModifiers::CONTROL)
            .await;
        assert_eq!(tab.scroll_offset, 2);
        assert!(matches!(m.render_state(), ModeRenderState::Visual { .. }));
    }

    #[tokio::test]
    async fn test_mode_bar_contains_line_nav_hints() {
        let mode = make_mode("hello");
        let content = mode.mode_bar_content(&Keybindings::default(), &Theme::default());
        let text: String = content.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("line↓"), "missing line↓ in: {text}");
        assert!(text.contains("line↑"), "missing line↑ in: {text}");
    }
}
