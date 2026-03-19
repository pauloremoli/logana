use ratatui::{
    Frame,
    prelude::*,
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::mode::app_mode::ModeRenderState;

use super::field_layout::count_wrapped_lines;
use super::widgets::{
    CommandBar, CompletionSource, InputBar, LogPanel, Sidebar, TabBar, TabBarEntry,
    file_display_name, prepare_log_panel, resolve_completions,
};
use super::{App, LoadContext};

impl App {
    pub(super) fn ui(&mut self, frame: &mut Frame) {
        let size = frame.area();
        frame.render_widget(Block::default().bg(self.theme.root_bg), size);

        let show_tab_bar = !self.tabs.is_empty();

        // Extract mode-derived state up front via a single render_state() call,
        // avoiding holding a borrow over the rest of rendering.
        let render_state = self.tabs[self.active_tab].interaction.mode.render_state();

        let persistent_pattern: Option<String> = if matches!(render_state, ModeRenderState::Normal)
        {
            self.tabs[self.active_tab]
                .search
                .query
                .get_pattern()
                .map(|p| p.to_string())
        } else {
            None
        };
        let has_input_bar = matches!(
            render_state,
            ModeRenderState::Command { .. } | ModeRenderState::Search { .. }
        ) || persistent_pattern.is_some();
        let command_input: Option<(String, usize)> = match &render_state {
            ModeRenderState::Command { input, cursor, .. } => Some((input.clone(), *cursor)),
            _ => None,
        };
        let completion_index: Option<usize> = match &render_state {
            ModeRenderState::Command {
                completion_index, ..
            } => *completion_index,
            _ => None,
        };
        // When a completion session is active, suggestions are computed from the original query
        // (what the user typed before Tab cycling), not from the currently displayed completion.
        let completion_query: Option<String> = match &render_state {
            ModeRenderState::Command {
                completion_query, ..
            } => completion_query.clone(),
            _ => None,
        };
        // (query, forward, is_active): is_active=true while typing (shows cursor + count),
        // false when persistent after execution (shows "match X / N").
        let search_input: Option<(String, bool, bool)> = match &render_state {
            ModeRenderState::Search { query, forward } => Some((query.clone(), *forward, true)),
            _ => persistent_pattern.map(|p| (p, true, false)),
        };
        let is_confirm_restore = matches!(render_state, ModeRenderState::ConfirmRestore);
        let session_files: Option<Vec<String>> = match &render_state {
            ModeRenderState::ConfirmRestoreSession { files } => Some(files.clone()),
            _ => None,
        };
        let selected_filter_idx = match &render_state {
            ModeRenderState::FilterManagement { selected_index } => *selected_index,
            // When CommandMode is entered from the filter menu (set-color, filter-edit),
            // filter_context holds the originating filter index — use it so the sidebar
            // keeps the correct filter highlighted throughout the command.
            _ => self.tabs[self.active_tab]
                .filter
                .filter_context
                .unwrap_or(0),
        };
        let keybindings = self.tabs[self.active_tab].interaction.keybindings.clone();
        let status_line = self.tabs[self.active_tab]
            .interaction
            .mode
            .mode_bar_content(&keybindings, &self.theme);
        let show_mode_bar = self.show_mode_bar;
        let has_warnings = !self.startup_warnings.is_empty();
        let warnings_height = self.startup_warnings.len().min(10) as u16;
        let visual_anchor: Option<usize> = match &render_state {
            ModeRenderState::VisualLine { anchor } => Some(*anchor),
            _ => None,
        };
        let visual_char_selection: Option<(usize, usize)> = match &render_state {
            ModeRenderState::Visual {
                anchor_col,
                cursor_col,
                ..
            } => {
                let anchor = anchor_col.unwrap_or(*cursor_col);
                let lo = anchor.min(*cursor_col);
                let hi = anchor.max(*cursor_col);
                Some((lo, hi))
            }
            _ => None,
        };
        let comment_popup: Option<(Vec<String>, usize, usize, usize)> = match &render_state {
            ModeRenderState::Comment {
                lines,
                cursor_row,
                cursor_col,
                line_count,
            } => Some((lines.clone(), *cursor_row, *cursor_col, *line_count)),
            _ => None,
        };
        let help_state: Option<(usize, String)> = match &render_state {
            ModeRenderState::KeybindingsHelp { scroll, search } => Some((*scroll, search.clone())),
            _ => None,
        };
        let select_fields_state: Option<(Vec<(String, bool)>, usize)> = match &render_state {
            ModeRenderState::SelectFields { fields, selected } => Some((fields.clone(), *selected)),
            _ => None,
        };
        let docker_select: Option<(Vec<crate::types::DockerContainer>, usize, Option<String>)> =
            match &render_state {
                ModeRenderState::DockerSelect {
                    containers,
                    selected,
                    error,
                } => Some((containers.clone(), *selected, error.clone())),
                _ => None,
            };
        #[allow(clippy::type_complexity)]
        let dlt_select: Option<(
            Vec<crate::config::DltDevice>,
            usize,
            Option<String>,
            Option<crate::mode::dlt_select_mode::AddDeviceRenderState>,
        )> = match &render_state {
            ModeRenderState::DltSelect {
                devices,
                selected,
                error,
                adding,
            } => Some((devices.clone(), *selected, error.clone(), adding.clone())),
            _ => None,
        };
        let value_colors_state: Option<(
            Vec<crate::mode::value_colors_mode::ValueColorGroup>,
            String,
            usize,
            &'static str,
        )> = match &render_state {
            ModeRenderState::ValueColors {
                groups,
                search,
                selected,
            } => Some((groups.clone(), search.clone(), *selected, "Value Colors")),
            ModeRenderState::LevelColors {
                groups,
                search,
                selected,
            } => Some((groups.clone(), search.clone(), *selected, "Level Colors")),
            _ => None,
        };
        let confirm_open_dir: Option<(String, Vec<String>)> = match &render_state {
            ModeRenderState::ConfirmOpenDir { dir, files } => Some((dir.clone(), files.clone())),
            _ => None,
        };

        let show_borders = self.tabs[self.active_tab].display.show_borders;
        // Auto-dismiss the notification after 10 seconds.
        if let Some(set_at) = self.tabs[self.active_tab].interaction.notification_set_at
            && set_at.elapsed() > std::time::Duration::from_secs(10)
        {
            self.tabs[self.active_tab].clear_notification();
        }
        let notification = self.tabs[self.active_tab].interaction.notification.clone();
        // Show the notification row when there is a message and the command bar
        // is not already open (where it would appear in the hint area instead).
        let has_notification = notification.is_some() && !has_input_bar;

        // Compute how many rows the mode bar needs so wrapped text is fully visible.
        // When borders are on they consume 1 col on each side (2 total); when off we
        // still reserve 1 col on the left for visual padding.
        let border_width = if show_borders { 2 } else { 1 };
        let inner_width = (size.width as usize).saturating_sub(border_width);
        let status_text: String = status_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        let content_lines = count_wrapped_lines(&status_text, inner_width);
        let status_height = if show_borders {
            (content_lines + 1).clamp(2, 5) as u16
        } else {
            content_lines.clamp(1, 4) as u16
        };

        let mut constraints = vec![];
        if show_tab_bar {
            constraints.push(Constraint::Length(1)); // Tab bar
        }
        constraints.push(Constraint::Min(1)); // Main content
        if has_input_bar {
            constraints.push(Constraint::Length(1)); // input line
            let hint_height = self.compute_hint_height(
                &command_input,
                completion_query.as_deref(),
                inner_width,
                completion_index,
            );
            constraints.push(Constraint::Length(hint_height)); // hint line(s)
        }
        let notification_chunk_idx = if has_notification {
            let idx = constraints.len();
            constraints.push(Constraint::Length(1)); // notification bar
            Some(idx)
        } else {
            None
        };
        let warnings_chunk_idx = if has_warnings {
            let idx = constraints.len();
            constraints.push(Constraint::Length(warnings_height)); // warnings bar
            Some(idx)
        } else {
            None
        };
        if show_mode_bar {
            constraints.push(Constraint::Length(status_height)); // command list
        }
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(size);

        let mut chunk_idx = 0;

        let mode_name_for_title = if !show_mode_bar {
            Some(render_state.mode_name())
        } else {
            None
        };

        if show_tab_bar {
            let tab_bar_area = chunks[chunk_idx];
            chunk_idx += 1;

            let loading_info: Option<(usize, usize)> = self.file_load_state.as_ref().map(|s| {
                let pct = (*s.progress_rx.borrow() * 100.0) as usize;
                let tab_idx = match &s.on_complete {
                    LoadContext::ReplaceInitialTab => 0,
                    LoadContext::ReplaceTab { tab_idx } => *tab_idx,
                    LoadContext::SessionRestoreTab { tab_idx, .. } => *tab_idx,
                };
                (tab_idx, pct)
            });
            let filtering_tabs: Vec<(usize, usize)> = self
                .tabs
                .iter()
                .enumerate()
                .filter_map(|(i, t)| {
                    t.filter.handle.as_ref().map(|h| {
                        let pct = (h.displayed_progress * 100.0) as usize;
                        (i, pct)
                    })
                })
                .collect();
            let tab_entries: Vec<TabBarEntry<'_>> = self
                .tabs
                .iter()
                .map(|t| {
                    let format_name = if t.display.raw_mode {
                        None
                    } else {
                        t.display.format.as_ref().map(|p| p.name().to_string())
                    };
                    TabBarEntry {
                        title: &t.title,
                        format_name,
                        num_visible: t.filter.visible_indices.len(),
                        tail_mode: t.stream.tail_mode,
                        raw_mode: t.display.raw_mode,
                        paused: t.stream.paused,
                        retry_attempt: t
                            .stream
                            .retry
                            .as_ref()
                            .filter(|r| !r.connected)
                            .map(|r| r.attempt),
                        has_lines: t.file_reader.line_count() > 0,
                    }
                })
                .collect();
            frame.render_widget(
                TabBar {
                    tabs: tab_entries,
                    active_tab: self.active_tab,
                    loading_info,
                    filtering_tabs,
                    show_borders,
                    mode_name: mode_name_for_title,
                    theme: &self.theme,
                },
                tab_bar_area,
            );
        }

        let main_chunk = chunks[chunk_idx];
        chunk_idx += 1;

        let tab = &self.tabs[self.active_tab];

        let sidebar_width = tab.display.sidebar_width;
        let (logs_area, sidebar_area) = if tab.display.show_sidebar {
            if show_borders {
                let horizontal = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(1), Constraint::Length(sidebar_width)])
                    .split(main_chunk);
                let raw_sidebar = horizontal[1];
                let sidebar = if show_tab_bar {
                    Rect {
                        y: raw_sidebar.y.saturating_sub(1),
                        height: raw_sidebar.height + 1,
                        ..raw_sidebar
                    }
                } else {
                    raw_sidebar
                };
                (horizontal[0], Some(sidebar))
            } else {
                let horizontal = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Min(1),
                        Constraint::Length(1),
                        Constraint::Length(sidebar_width),
                    ])
                    .split(main_chunk);
                (horizontal[0], Some(horizontal[2]))
            }
        } else {
            (main_chunk, None)
        };

        let log_panel_data = prepare_log_panel(
            &mut self.tabs[self.active_tab],
            logs_area,
            visual_anchor,
            visual_char_selection,
            mode_name_for_title,
            show_tab_bar,
            has_input_bar,
            &self.theme,
        );
        frame.render_widget(
            LogPanel {
                data: &log_panel_data,
            },
            logs_area,
        );

        if let Some(sidebar_area) = sidebar_area {
            let tab = &self.tabs[self.active_tab];
            let filters = tab.log_manager.get_filters();
            let match_counts = tab.filter.match_counts.clone();
            let filter_progress: Option<usize> = tab
                .filter
                .handle
                .as_ref()
                .map(|h| (h.displayed_progress * 100.0) as usize);
            frame.render_widget(
                Sidebar {
                    filters,
                    match_counts: &match_counts,
                    selected_filter_idx,
                    filter_enabled: tab.filter.enabled,
                    show_marks_only: tab.filter.show_marks_only,
                    filter_progress,
                    show_borders,
                    theme: &self.theme,
                },
                sidebar_area,
            );
        }

        if let Some((input_text, cursor_pos)) = command_input {
            let query_text = completion_query.as_deref().unwrap_or(input_text.as_str());
            let completion = resolve_completions(
                &mut self.tabs[self.active_tab],
                query_text,
                completion_index,
            );
            let input_area = chunks[chunk_idx];
            let hint_area = chunks[chunk_idx + 1];
            let cmd_bar = CommandBar {
                input_text: &input_text,
                cursor_pos,
                completion,
                theme: &self.theme,
            };
            if let Some((cx, cy)) = cmd_bar.cursor_position(input_area) {
                frame.set_cursor_position((cx, cy));
            }
            let combined = Rect {
                height: input_area.height + hint_area.height,
                ..input_area
            };
            frame.render_widget(cmd_bar, combined);
        }

        if let Some((input_str, forward, is_active)) = search_input {
            let input_area = chunks[chunk_idx];
            let hint_area = chunks[chunk_idx + 1];
            let total = self.tabs[self.active_tab]
                .search
                .query
                .get_total_match_count();
            let current_occurrence = self.tabs[self.active_tab]
                .search
                .query
                .get_current_occurrence_number();
            let progress = self.tabs[self.active_tab]
                .search
                .handle
                .as_ref()
                .map(|h| progress_bar_str(*h.progress_rx.borrow()));
            let bar = InputBar {
                query: &input_str,
                forward,
                is_active,
                total_matches: total,
                current_occurrence,
                progress,
                theme: &self.theme,
            };
            if let Some((cx, cy)) = bar.cursor_position(input_area) {
                frame.set_cursor_position((cx, cy));
            }
            let combined = Rect {
                height: input_area.height + hint_area.height,
                ..input_area
            };
            frame.render_widget(bar, combined);
        }

        if let Some(idx) = notification_chunk_idx
            && let Some(msg) = &notification
        {
            let notification_area = chunks[idx];
            frame.render_widget(
                Paragraph::new(msg.as_str()).style(
                    Style::default()
                        .fg(self.theme.warning_fg)
                        .bg(self.theme.root_bg),
                ),
                notification_area,
            );
        }

        if let Some(idx) = warnings_chunk_idx {
            let warnings_area = chunks[idx];
            let lines: Vec<Line> = self
                .startup_warnings
                .iter()
                .take(10)
                .map(|w| {
                    Line::from(vec![
                        Span::styled("Warning: ", Style::default().fg(Color::Red)),
                        Span::raw(w.clone()),
                    ])
                })
                .collect();
            frame.render_widget(
                Paragraph::new(lines).style(Style::default().bg(self.theme.root_bg)),
                warnings_area,
            );
        }

        if show_mode_bar {
            let status_block = if show_borders {
                Block::default()
                    .borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
                    .border_style(Style::default().fg(self.theme.border))
            } else {
                Block::default()
                    .borders(Borders::NONE)
                    .padding(Padding::new(1, 0, 0, 0))
            };
            let command_list = Paragraph::new(status_line)
                .block(status_block)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(self.theme.root_bg));
            if let Some(&status_area) = chunks.last() {
                frame.render_widget(command_list, status_area);
            }
        }

        let frame_area = frame.area();

        if is_confirm_restore {
            frame.render_widget(
                super::widgets::ConfirmRestoreModal {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                },
                frame_area,
            );
        }

        if let Some(files) = session_files {
            frame.render_widget(
                super::widgets::ConfirmRestoreSessionModal {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    files: &files,
                },
                frame_area,
            );
        }

        if let Some((_dir, files)) = confirm_open_dir {
            frame.render_widget(
                super::widgets::ConfirmOpenDirModal {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    files: &files,
                },
                frame_area,
            );
        }

        if let Some((lines, cursor_row, cursor_col, line_count)) = comment_popup {
            let popup = super::widgets::CommentPopup {
                theme: &self.theme,
                keybindings: &self.tabs[self.active_tab].interaction.keybindings,
                lines: &lines,
                cursor_row,
                cursor_col,
                line_count,
            };
            if let Some((cx, cy)) = popup.cursor_position(frame_area) {
                frame.set_cursor_position((cx, cy));
            }
            frame.render_widget(popup, frame_area);
        }

        if let Some((fields, selected)) = select_fields_state {
            frame.render_widget(
                super::widgets::SelectFieldsPopup {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    fields: &fields,
                    selected,
                },
                frame_area,
            );
        }

        if let Some((containers, selected, error)) = docker_select {
            frame.render_widget(
                super::widgets::DockerSelectPopup {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    containers: &containers,
                    selected,
                    error: error.as_deref(),
                },
                frame_area,
            );
        }

        if let Some((devices, selected, error, adding)) = dlt_select {
            frame.render_widget(
                super::widgets::DltSelectPopup {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    devices: &devices,
                    selected,
                    error: error.as_deref(),
                    adding: adding.as_ref(),
                },
                frame_area,
            );
        }

        if let Some((groups, search, selected, title)) = value_colors_state {
            frame.render_widget(
                super::widgets::ValueColorsPopup {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    groups: &groups,
                    search: &search,
                    selected,
                    title,
                },
                frame_area,
            );
        }

        if let Some((scroll, search)) = help_state {
            frame.render_widget(
                super::widgets::KeybindingsHelpPopup {
                    theme: &self.theme,
                    keybindings: &keybindings,
                    scroll,
                    search: &search,
                },
                frame_area,
            );
        }
    }

    fn compute_hint_height(
        &mut self,
        command_input: &Option<(String, usize)>,
        completion_query: Option<&str>,
        width: usize,
        completion_index: Option<usize>,
    ) -> u16 {
        let text = match command_input {
            Some((input_text, _)) => {
                let query_text = completion_query.unwrap_or(input_text.as_str());
                let tab = &mut self.tabs[self.active_tab];
                match resolve_completions(tab, query_text, completion_index) {
                    CompletionSource::Error(e) => e,
                    CompletionSource::Items(items) => items.join("  "),
                    CompletionSource::ColorItems(items) => items
                        .iter()
                        .map(|n| format!(" {} ", n))
                        .collect::<Vec<_>>()
                        .join(" "),
                    CompletionSource::FileItems(items) => items
                        .iter()
                        .map(|c| file_display_name(c))
                        .collect::<Vec<_>>()
                        .join("  "),
                    CompletionSource::CommandHelp(help) => help,
                }
            }
            None => String::new(),
        };
        if text.is_empty() {
            return 1;
        }
        (count_wrapped_lines(&text, width) as u16).clamp(1, 3)
    }
}

/// djb2-style hash for stable per-process color assignment.
/// Returns `(bar, pct)` for a progress fraction in `0.0..=1.0`.
fn progress_bar_str(progress: f64) -> (String, usize) {
    const BAR_WIDTH: usize = 20;
    let filled = ((progress * BAR_WIDTH as f64) as usize).min(BAR_WIDTH);
    let bar = format!(
        "{}{}",
        "\u{2588}".repeat(filled),
        "\u{2591}".repeat(BAR_WIDTH - filled),
    );
    let pct = (progress * 100.0) as usize;
    (bar, pct)
}

#[allow(dead_code)]
fn stable_hash(s: &str) -> usize {
    s.bytes().fold(5381usize, |acc, b| {
        acc.wrapping_mul(33).wrapping_add(b as usize)
    })
}

#[allow(dead_code)]
fn find_token_offset(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let hb = haystack.as_bytes();
    let nb = needle.len();
    let mut start = 0;
    while start + nb <= hb.len() {
        match haystack[start..].find(needle) {
            None => break,
            Some(rel) => {
                let abs = start + rel;
                let before_ok = abs == 0 || hb[abs - 1] == b' ';
                let after_ok = abs + nb == hb.len() || hb[abs + nb] == b' ';
                if before_ok && after_ok {
                    return Some(abs);
                }
                start = abs + 1;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::file_reader::FileReader;
    use crate::log_manager::LogManager;
    use crate::mode::app_mode::ConfirmRestoreSessionMode;
    use crate::mode::command_mode::CommandMode;
    use crate::mode::filter_mode::FilterManagementMode;
    use crate::mode::search_mode::SearchMode;
    use crate::mode::visual_mode::VisualLineMode;
    use crate::theme::Theme;
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;

    async fn make_app(lines: &[&str]) -> App {
        let data: Vec<u8> = lines.join("\n").into_bytes();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
    }

    fn make_terminal() -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(80, 24)).unwrap()
    }

    #[tokio::test]
    async fn test_ui_normal_mode_basic() {
        let lines: Vec<&str> = (0..10)
            .map(|i| match i {
                0 => "line 0",
                1 => "line 1",
                2 => "line 2",
                3 => "line 3",
                4 => "line 4",
                5 => "line 5",
                6 => "line 6",
                7 => "line 7",
                8 => "line 8",
                _ => "line 9",
            })
            .collect();
        let mut app = make_app(&lines).await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_no_sidebar() {
        let mut app = make_app(&["line A", "line B", "line C"]).await;
        app.tabs[0].display.show_sidebar = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_command_mode() {
        let mut app = make_app(&["log line"]).await;
        app.tabs[0].interaction.mode =
            Box::new(CommandMode::with_history("filter ".to_string(), 7, vec![]));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_command_mode_error() {
        let mut app = make_app(&["log line"]).await;
        app.tabs[0].interaction.command_error = Some("test error".to_string());
        app.tabs[0].interaction.mode =
            Box::new(CommandMode::with_history("bad-cmd".to_string(), 7, vec![]));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_command_mode_completion_index() {
        let mut app = make_app(&["log line"]).await;
        app.tabs[0].interaction.mode = Box::new(CommandMode {
            input: "fil".to_string(),
            cursor: 3,
            history: vec![],
            history_index: None,
            completion_index: Some(0),
            completion_query: None,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_search_mode_forward() {
        let mut app = make_app(&["hello world", "test line"]).await;
        app.tabs[0].interaction.mode = Box::new(SearchMode {
            input: "test".to_string(),
            forward: true,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_search_mode_backward() {
        let mut app = make_app(&["hello world", "test line"]).await;
        app.tabs[0].interaction.mode = Box::new(SearchMode {
            input: "test".to_string(),
            forward: false,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_search_mode_empty() {
        let mut app = make_app(&["hello world"]).await;
        app.tabs[0].interaction.mode = Box::new(SearchMode {
            input: String::new(),
            forward: true,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_filter_management_mode() {
        let mut app = make_app(&["INFO something", "ERROR bad thing"]).await;
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "INFO".to_string(),
                crate::types::FilterType::Include,
                None,
                None,
                false,
            )
            .await;
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
                crate::types::FilterType::Include,
                None,
                None,
                false,
            )
            .await;
        app.tabs[0].refresh_visible();
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode {
            selected_filter_index: 0,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_visual_line_mode() {
        let mut app = make_app(&["line 0", "line 1", "line 2"]).await;
        app.tabs[0].interaction.mode = Box::new(VisualLineMode {
            anchor: 0,
            count: None,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_with_marks() {
        let mut app = make_app(&["line 0", "line 1", "line 2", "line 3"]).await;
        app.tabs[0].log_manager.toggle_mark(0);
        app.tabs[0].log_manager.toggle_mark(2);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_level_colors() {
        let mut app = make_app(&[
            "INFO something happened",
            "WARN warning message",
            "ERROR error occurred",
        ])
        .await;
        let default_disabled: std::collections::HashSet<String> =
            ["trace", "debug", "info", "notice"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(app.tabs[0].display.level_colors_disabled, default_disabled);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_no_level_colors() {
        let mut app = make_app(&[
            "INFO something happened",
            "WARN warning message",
            "ERROR error occurred",
        ])
        .await;
        app.tabs[0].display.level_colors_disabled = [
            "trace", "debug", "info", "notice", "warning", "error", "fatal",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_with_line_numbers() {
        let mut app = make_app(&["line A", "line B"]).await;
        assert!(app.tabs[0].display.show_line_numbers);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_without_line_numbers() {
        let mut app = make_app(&["line A", "line B"]).await;
        app.tabs[0].display.show_line_numbers = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_with_comments() {
        let mut app = make_app(&["line 0", "line 1", "line 2"]).await;
        app.tabs[0]
            .log_manager
            .add_comment("test comment".to_string(), vec![0, 1]);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let content: String = (0..buf.area.height)
            .map(|y| row_content(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            content.contains("──"),
            "comment banner should use ── separator, got:\n{content}"
        );
        assert!(
            !content.contains("├"),
            "banner should not use tree connector ├, got:\n{content}"
        );
        assert!(
            content.contains("test comment"),
            "comment text should appear in output, got:\n{content}"
        );
    }

    #[tokio::test]
    async fn test_ui_wrap_enabled() {
        let long_line = "A".repeat(200);
        let mut app = make_app(&[&long_line, "short"]).await;
        app.tabs[0].display.wrap = true;
        assert!(app.tabs[0].display.wrap);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_wrap_disabled() {
        let long_line = "B".repeat(200);
        let mut app = make_app(&[&long_line, "short"]).await;
        app.tabs[0].display.wrap = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_horizontal_scroll() {
        let long_line = "C".repeat(200);
        let mut app = make_app(&[&long_line]).await;
        app.tabs[0].display.wrap = false;
        app.tabs[0].scroll.horizontal_scroll = 10;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_empty_file() {
        let mut app = make_app(&[]).await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_json_structured() {
        let mut app = make_app(&[
            r#"{"level":"INFO","msg":"hello"}"#,
            r#"{"level":"WARN","msg":"world"}"#,
        ])
        .await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_structured_all_hidden() {
        let mut app = make_app(&[
            r#"{"level":"INFO","msg":"hello"}"#,
            r#"{"level":"WARN","msg":"world"}"#,
        ])
        .await;
        app.tabs[0]
            .display
            .hidden_fields
            .insert("level".to_string());
        app.tabs[0].display.hidden_fields.insert("msg".to_string());
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_multiple_tabs() {
        let mut app = make_app(&["tab1 line"]).await;
        let data2: Vec<u8> = "second tab line\n".as_bytes().to_vec();
        let file_reader2 = FileReader::from_bytes(data2);
        let log_manager2 = LogManager::new(app.db.clone(), None).await;
        let mut tab2 = super::super::TabState::new(file_reader2, log_manager2, "tab2".to_string());
        tab2.interaction.keybindings = app.keybindings.clone();
        app.tabs.push(tab2);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_filtering_disabled() {
        let mut app = make_app(&["line 0", "line 1"]).await;
        app.tabs[0].filter.enabled = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_marks_only() {
        let mut app = make_app(&["line 0", "line 1", "line 2"]).await;
        app.tabs[0].log_manager.toggle_mark(1);
        app.tabs[0].filter.show_marks_only = true;
        app.tabs[0].refresh_visible();
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_confirm_restore_session() {
        let mut app = make_app(&[]).await;
        app.tabs[0].interaction.mode = Box::new(ConfirmRestoreSessionMode {
            files: vec!["file.log".to_string()],
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_compute_hint_height_empty() {
        let mut app = make_app(&["line"]).await;
        let result = app.compute_hint_height(&None, None, 80, None);
        assert_eq!(result, 1);
    }

    #[tokio::test]
    async fn test_compute_hint_height_matching_command() {
        let mut app = make_app(&["line"]).await;
        let input = Some(("filter".to_string(), 6));
        let result = app.compute_hint_height(&input, None, 80, None);
        assert!(result >= 1);
    }

    #[tokio::test]
    async fn test_compute_hint_height_error() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].interaction.command_error = Some("something went wrong".to_string());
        let input = Some(("bad".to_string(), 3));
        let result = app.compute_hint_height(&input, None, 80, None);
        assert!(result >= 1);
    }

    #[tokio::test]
    async fn test_ui_small_terminal() {
        let mut app = make_app(&["hello", "world"]).await;
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_scroll_beyond_visible() {
        let mut app = make_app(&["line 0", "line 1"]).await;
        app.tabs[0].scroll.scroll_offset = 999;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_loading_progress_in_tab_name() {
        let mut app = make_app(&["placeholder"]).await;
        let (_progress_tx, progress_rx) = tokio::sync::watch::channel(0.5f64);
        let (_result_tx, result_rx) = tokio::sync::oneshot::channel();
        app.file_load_state = Some(super::super::FileLoadState {
            path: "/tmp/test.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 1000,
            on_complete: super::super::LoadContext::ReplaceInitialTab,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        let mut terminal = make_terminal();
        // _progress_tx is kept alive until after draw
        terminal.draw(|f| app.ui(f)).unwrap();

        // Tab bar (row 0) should show the tab title with progress percentage.
        let tab_row = row_content(terminal.backend().buffer(), 0);
        assert!(
            tab_row.contains("50%"),
            "tab bar row should contain progress percentage; got: {:?}",
            tab_row,
        );
    }

    #[tokio::test]
    async fn test_ui_filtering_progress_in_sidebar_title() {
        let mut app = make_app(&["line 0", "line 1"]).await;
        let (_result_tx, result_rx) = tokio::sync::mpsc::channel::<super::super::FilterChunk>(4);
        app.tabs[0].filter.handle = Some(super::super::FilterHandle {
            result_rx,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            displayed_progress: 0.42,
            scroll_anchor: None,
            received_first_chunk: false,
            scan_fingerprint: Vec::new(),
            scan_line_count: 0,
            scan_raw_mode: false,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        let buf = terminal.backend().buffer().clone();
        let filters_row = (0..buf.area.height)
            .map(|y| row_content(&buf, y))
            .find(|row| row.contains("Filters"))
            .expect("a row containing 'Filters' should be rendered");
        assert!(
            filters_row.contains("42%"),
            "sidebar title should contain '42%' while filtering; got: {:?}",
            filters_row,
        );
    }

    #[tokio::test]
    async fn test_ui_indexing_shown_in_sidebar_title() {
        let mut app = make_app(&["line 0", "line 1"]).await;
        let (_result_tx, result_rx) = tokio::sync::mpsc::channel::<super::super::FilterChunk>(4);
        app.tabs[0].filter.handle = Some(super::super::FilterHandle {
            result_rx,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            displayed_progress: 1.0,
            scroll_anchor: None,
            received_first_chunk: false,
            scan_fingerprint: Vec::new(),
            scan_line_count: 0,
            scan_raw_mode: false,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        let buf = terminal.backend().buffer().clone();
        let filters_row = (0..buf.area.height)
            .map(|y| row_content(&buf, y))
            .find(|row| row.contains("Filters"))
            .expect("a row containing 'Filters' should be rendered");
        assert!(
            filters_row.contains("Indexing"),
            "sidebar title should show 'Indexing' when progress is 100%; got: {:?}",
            filters_row,
        );
    }

    #[tokio::test]
    async fn test_ui_filtering_progress_in_tab_name() {
        let mut app = make_app(&["line 0", "line 1"]).await;
        let (_result_tx, result_rx) = tokio::sync::mpsc::channel::<super::super::FilterChunk>(4);
        app.tabs[0].filter.handle = Some(super::super::FilterHandle {
            result_rx,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            displayed_progress: 0.42,
            scroll_anchor: None,
            received_first_chunk: false,
            scan_fingerprint: Vec::new(),
            scan_line_count: 0,
            scan_raw_mode: false,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        let tab_row = row_content(terminal.backend().buffer(), 0);
        assert!(
            tab_row.contains("Filtering") && tab_row.contains("42%"),
            "tab bar should contain 'Filtering' and '42%'; got: {:?}",
            tab_row,
        );
    }

    #[tokio::test]
    async fn test_ui_indexing_shown_when_progress_complete() {
        let mut app = make_app(&["line 0", "line 1"]).await;
        let (_result_tx, result_rx) = tokio::sync::mpsc::channel::<super::super::FilterChunk>(4);
        app.tabs[0].filter.handle = Some(super::super::FilterHandle {
            result_rx,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            displayed_progress: 1.0,
            scroll_anchor: None,
            received_first_chunk: false,
            scan_fingerprint: Vec::new(),
            scan_line_count: 0,
            scan_raw_mode: false,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        let tab_row = row_content(terminal.backend().buffer(), 0);
        assert!(
            tab_row.contains("Indexing"),
            "tab bar should contain 'Indexing' when progress is 100%; got: {:?}",
            tab_row,
        );
    }

    #[tokio::test]
    async fn test_ui_filters_and_search() {
        let mut app = make_app(&[
            "INFO something happened",
            "ERROR another thing",
            "INFO something else",
        ])
        .await;
        app.execute_command_str("filter INFO".to_string()).await;
        let visible = app.tabs[0].filter.visible_indices.clone();
        let tab = &mut app.tabs[0];
        let texts = tab.collect_display_texts(visible.iter());
        let _ = tab
            .search
            .query
            .search("something", visible.iter(), |li| texts.get(&li).cloned());
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // -----------------------------------------------------------------------
    // find_token_offset
    // -----------------------------------------------------------------------
    #[test]
    fn test_find_token_offset_simple() {
        // Standalone token at the start
        assert_eq!(find_token_offset("abc def ghi", "abc"), Some(0));
        // Standalone token in the middle
        assert_eq!(find_token_offset("abc def ghi", "def"), Some(4));
        // Standalone token at the end
        assert_eq!(find_token_offset("abc def ghi", "ghi"), Some(8));
    }

    #[test]
    fn test_find_token_offset_short_value_not_matched_inside_longer_token() {
        // "1" appears inside "2024-01-..." but must not be matched there.
        let rendered = "2024-01-15T10:00:00Z INFO  systemd myhost 1 daemon Started.";
        // Should skip the "1" inside the timestamp and find the standalone "1".
        let pos = find_token_offset(rendered, "1").unwrap();
        let token = &rendered[pos..pos + 1];
        assert_eq!(token, "1");
        // The character before must be a space.
        assert_eq!(rendered.as_bytes()[pos - 1], b' ');
        // The character after must be a space.
        assert_eq!(rendered.as_bytes()[pos + 1], b' ');
    }

    #[test]
    fn test_find_token_offset_systemd_pid1_in_syslog_rfc5424() {
        // Reproduces the reported bug: syslog RFC 5424 line with PID=1 (systemd).
        // Before the fix, rendered.find("1") matched the "1" in "2024-01-…".
        let rendered =
            "2024-01-15T10:30:00.000000+01:00 INFO  systemd myhost 1 local3 Started network.";
        let pid_pos = find_token_offset(rendered, "1").unwrap();
        // Must point to the standalone "1" (PID), not into the timestamp.
        assert_eq!(&rendered[pid_pos..pid_pos + 1], "1");
        // The characters around must be spaces.
        assert!(pid_pos > 0 && rendered.as_bytes()[pid_pos - 1] == b' ');
        assert!(pid_pos + 1 < rendered.len() && rendered.as_bytes()[pid_pos + 1] == b' ');
        // The standalone "1" must appear AFTER the timestamp ends.
        let ts_end = "2024-01-15T10:30:00.000000+01:00".len();
        assert!(
            pid_pos > ts_end,
            "pid_pos {pid_pos} should be past timestamp end {ts_end}"
        );
    }

    #[test]
    fn test_find_token_offset_empty_needle() {
        assert_eq!(find_token_offset("hello world", ""), None);
    }

    #[test]
    fn test_find_token_offset_needle_not_present() {
        assert_eq!(find_token_offset("hello world", "xyz"), None);
    }

    #[test]
    fn test_find_token_offset_only_substring_not_token() {
        // "lo" is only a substring of "hello", not a standalone token.
        assert_eq!(find_token_offset("hello world", "lo"), None);
    }

    #[test]
    fn test_find_token_offset_single_token_haystack() {
        // Entire haystack is the needle.
        assert_eq!(find_token_offset("only", "only"), Some(0));
    }

    #[test]
    fn test_find_token_offset_bsd_timestamp_with_spaces() {
        // BSD timestamp "Mar  8 10:30:00" contains internal spaces but is itself
        // a complete token (bounded by start-of-string and a space).
        let rendered = "Mar  8 10:30:00 INFO  systemd";
        assert_eq!(find_token_offset(rendered, "Mar  8 10:30:00"), Some(0));
    }

    // -----------------------------------------------------------------------
    // stable_hash
    // -----------------------------------------------------------------------

    #[test]
    fn test_stable_hash_consistent() {
        assert_eq!(stable_hash("my_service"), stable_hash("my_service"));
        assert_ne!(stable_hash("service_a"), stable_hash("service_b"));
    }

    /// Collect the symbols on a given row of the buffer as a string.
    fn row_content(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        let width = buf.area.width;
        (0..width)
            .map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol()))
            .collect()
    }

    /// Set up an app with a persistent search handle injected at a given progress level.
    async fn make_app_with_search(progress: Option<f64>) -> (App, Terminal<TestBackend>) {
        let mut app = make_app(&["line 0", "line 1"]).await;
        app.show_mode_bar = false;
        app.tabs[0].display.show_mode_bar = false;

        let visible = app.tabs[0].filter.visible_indices.clone();
        let tab = &mut app.tabs[0];
        let texts = tab.collect_display_texts(visible.iter());
        let _ = tab
            .search
            .query
            .search("line", visible.iter(), |li| texts.get(&li).cloned());

        if let Some(p) = progress {
            let (_result_tx, result_rx) = tokio::sync::mpsc::channel(1);
            let (_progress_tx, progress_rx) = tokio::sync::watch::channel(p);
            app.tabs[0].search.handle = Some(super::super::SearchHandle {
                result_rx,
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                progress_rx,
                pattern: "line".to_string(),
                forward: true,
                navigate: false,
            });
        }

        let terminal = make_terminal(); // 80×24
        (app, terminal)
    }

    #[tokio::test]
    async fn test_search_progress_bar_shown_in_hint_area() {
        let (mut app, mut terminal) = make_app_with_search(Some(0.5)).await;
        terminal.draw(|f| app.ui(f)).unwrap();

        // With no mode bar the hint row is at y=23 (rows 22=input, 23=hint).
        let hint_row = row_content(terminal.backend().buffer(), 23);
        assert!(
            hint_row.contains('\u{2588}'),
            "hint row should contain █ when search is in progress; got: {:?}",
            hint_row,
        );
    }

    #[tokio::test]
    async fn test_search_progress_bar_not_shown_without_handle() {
        let (mut app, mut terminal) = make_app_with_search(None).await;
        terminal.draw(|f| app.ui(f)).unwrap();

        let hint_row = row_content(terminal.backend().buffer(), 23);
        assert!(
            !hint_row.contains('\u{2588}'),
            "hint row should not contain █ without an active search handle; got: {:?}",
            hint_row,
        );
    }

    // Before the fix, toggling a filter that reduces num_visible left viewport_offset
    // pointing near the old end, causing the cursor to sit at the top of the viewport
    // with blank rows below even though more visible lines existed above.
    #[tokio::test]
    async fn test_ui_viewport_fills_backward_after_filter_toggle() {
        // 50 lines, terminal height 24 → visible_height = 23 (1 row for title).
        let lines: Vec<String> = (0..50).map(|i| format!("line {i}")).collect();
        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let mut app = make_app(&line_refs).await;

        // Simulate state after scrolling to the end of 50 lines.
        app.tabs[0].scroll.scroll_offset = 49;
        app.tabs[0].scroll.viewport_offset = 49;

        // Add a filter that keeps only lines 0..30 (those containing a single digit
        // or two-digit number < 30).
        app.execute_command_str("include-filter line [012][0-9]$".to_string())
            .await;
        // After the filter, visible = 30 lines; scroll_offset clamped to 29 by render.

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        // viewport_offset must have been pulled back so the full visible_height is used.
        // With 30 visible lines and visible_height=23, the latest valid start is 30-23=7.
        let vp = app.tabs[0].scroll.viewport_offset;
        let visible = app.tabs[0].filter.visible_indices.len();
        let visible_height = 23; // 24-row terminal minus 1 title row (no borders)
        assert!(
            vp + visible_height >= visible,
            "viewport_offset {vp} leaves blank rows: {visible} visible lines, height {visible_height}"
        );
    }

    // When the search or command input bar is visible the viewport must reserve
    // an extra row so the cursor cannot hide behind the bar.
    #[tokio::test]
    async fn test_visible_height_reduced_when_input_bar_visible() {
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();

        // Without input bar.
        let mut app_no_bar = make_app(&line_refs).await;
        app_no_bar.show_mode_bar = false;
        app_no_bar.tabs[0].display.show_mode_bar = false;
        let mut terminal = make_terminal(); // 80×24
        terminal.draw(|f| app_no_bar.ui(f)).unwrap();
        let height_without_bar = app_no_bar.tabs[0].scroll.visible_height;

        // With search input bar active.
        let mut app_with_bar = make_app(&line_refs).await;
        app_with_bar.show_mode_bar = false;
        app_with_bar.tabs[0].display.show_mode_bar = false;
        app_with_bar.tabs[0].interaction.mode = Box::new(SearchMode {
            input: String::new(),
            forward: true,
        });
        let mut terminal2 = make_terminal();
        terminal2.draw(|f| app_with_bar.ui(f)).unwrap();
        let height_with_bar = app_with_bar.tabs[0].scroll.visible_height;

        assert!(
            height_with_bar < height_without_bar,
            "visible_height should be smaller when input bar is visible: \
             without={height_without_bar}, with={height_with_bar}"
        );
    }

    // ── Tab bar mode label ─────────────────────────────────────────────────

    /// Build an app with a second tab so the tab bar is rendered.
    async fn make_two_tab_app() -> App {
        let mut app = make_app(&["line 0", "line 1"]).await;
        // Clone the first tab as a second tab so `has_multiple_tabs` is true.
        let db = app.db.clone();
        let log_manager = LogManager::new(db, None).await;
        let tab2 = crate::ui::TabState::new(
            FileReader::from_bytes(b"other\n".to_vec()),
            log_manager,
            "other".to_string(),
        );
        app.tabs.push(tab2);
        app
    }

    #[tokio::test]
    async fn test_tab_bar_shows_mode_when_mode_bar_hidden() {
        let mut app = make_two_tab_app().await;
        app.show_mode_bar = false;
        app.tabs[0].display.show_mode_bar = false;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        // Row 0 is the tab bar. The active tab (tab 0) should show [NORMAL].
        let tab_row = row_content(&buf, 0);
        assert!(
            tab_row.contains("[NORMAL]"),
            "expected [NORMAL] in tab bar when mode bar is hidden, got: {:?}",
            tab_row
        );
    }

    #[tokio::test]
    async fn test_tab_bar_no_mode_when_mode_bar_visible() {
        let mut app = make_two_tab_app().await;
        app.show_mode_bar = true;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let tab_row = row_content(&buf, 0);
        assert!(
            !tab_row.contains("[NORMAL]"),
            "should not show [NORMAL] in tab bar when mode bar is visible, got: {:?}",
            tab_row
        );
    }

    #[tokio::test]
    async fn test_tab_bar_mode_updates_on_mode_change() {
        use crate::mode::filter_mode::FilterManagementMode;

        let mut app = make_two_tab_app().await;
        app.show_mode_bar = false;
        app.tabs[0].display.show_mode_bar = false;
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode {
            selected_filter_index: 0,
        });

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let tab_row = row_content(&buf, 0);
        assert!(
            tab_row.contains("[FILTER]"),
            "expected [FILTER] in tab bar after mode change, got: {:?}",
            tab_row
        );
    }

    #[tokio::test]
    async fn test_inactive_tab_has_no_mode_prefix() {
        let mut app = make_two_tab_app().await;
        app.show_mode_bar = false;
        app.tabs[0].display.show_mode_bar = false;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let tab_row = row_content(&buf, 0);
        // "other" is the second (inactive) tab title — it must not carry [NORMAL].
        let other_pos = tab_row.find("other").expect("second tab title not found");
        let prefix = &tab_row[..other_pos];
        assert!(
            !prefix.ends_with("[NORMAL] "),
            "inactive tab should not have mode prefix, tab row: {:?}",
            tab_row
        );
    }

    #[tokio::test]
    async fn test_tab_bar_mode_label_uses_highlight_color() {
        let mut app = make_two_tab_app().await;
        let expected_fg = app.theme.text_highlight_fg;
        let expected_bg = app.theme.root_bg;
        app.show_mode_bar = false;
        app.tabs[0].display.show_mode_bar = false;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        // Find the '[' of "[NORMAL]" on row 0 and verify fg matches the mode bar
        // style (text_highlight_fg on root_bg — no tab-highlight background).
        let tab_row = row_content(&buf, 0);
        let bracket_col = tab_row
            .find('[')
            .expect("'[' of mode label not found in tab bar") as u16;
        let cell = buf.cell((bracket_col, 0)).expect("cell out of bounds");
        assert_eq!(
            cell.fg, expected_fg,
            "mode label '[' should use text_highlight_fg, got {:?}",
            cell.fg
        );
        assert_eq!(
            cell.bg, expected_bg,
            "mode label '[' should sit on root_bg (same as mode bar), got {:?}",
            cell.bg
        );
    }

    #[tokio::test]
    async fn test_logs_title_omits_filename_when_tab_bar_visible() {
        let mut app = make_two_tab_app().await;
        app.tabs[0].title = "myfile.log".to_string();
        app.tabs[0].display.show_borders = true;
        app.tabs[1].display.show_borders = true;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        // Row 0 is the tab bar embedded in the top border (┌ … ┐).
        let tab_bar_row = row_content(&buf, 0);
        assert!(
            tab_bar_row.contains('┌') && tab_bar_row.contains('┐'),
            "tab bar row should contain border corners, got: {:?}",
            tab_bar_row,
        );

        // Row 1 is the first content row (│ … │); the panel has no separate
        // title line — status info lives exclusively in the tab bar.
        let content_row = row_content(&buf, 1);
        assert!(
            !content_row.contains("myfile.log"),
            "content row should not repeat filename from tab bar, got: {:?}",
            content_row,
        );
        assert!(
            !content_row.contains("other"),
            "content row should not repeat tab titles, got: {:?}",
            content_row,
        );
    }

    #[tokio::test]
    async fn test_active_tab_shows_count_in_tab_bar() {
        let mut app = make_two_tab_app().await;
        app.tabs[0].title = "myfile.log".to_string();
        app.tabs[0].display.show_borders = true;
        app.tabs[1].display.show_borders = true;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        // Row 0 is the tab bar merged with the top border (┌ … ┐).
        // The active tab should show a line count "(N)" inside the border.
        let tab_row = row_content(&buf, 0);
        assert!(
            tab_row.contains('┌') && tab_row.contains('┐'),
            "tab bar should form the top border of the logs panel, got: {:?}",
            tab_row,
        );
        assert!(
            tab_row.contains('('),
            "active tab in tab bar should contain line count, got: {:?}",
            tab_row,
        );
    }

    #[tokio::test]
    async fn test_active_tab_shows_unknown_format_when_no_parser() {
        let mut app = make_two_tab_app().await;
        app.tabs[0].title = "myfile.log".to_string();
        app.tabs[0].display.show_borders = true;
        app.tabs[1].display.show_borders = true;
        assert!(app.tabs[0].display.format.is_none());

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let tab_row = row_content(&buf, 0);
        assert!(
            tab_row.contains("[unknown format]"),
            "active tab should show [unknown format] when no parser detected, got: {:?}",
            tab_row,
        );
    }

    #[tokio::test]
    async fn test_active_tab_hides_unknown_format_in_raw_mode() {
        let mut app = make_two_tab_app().await;
        app.tabs[0].title = "myfile.log".to_string();
        app.tabs[0].display.show_borders = true;
        app.tabs[0].display.raw_mode = true;
        app.tabs[1].display.show_borders = true;
        app.tabs[1].display.format = Some(std::sync::Arc::from(crate::parser::json::JsonParser {
            schema: &crate::parser::SCHEMA_GENERIC_JSON,
            fields_container: None,
            span_key: None,
            score_weight: 1.0,
        }));

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let tab_row = row_content(&buf, 0);
        assert!(
            !tab_row.contains("[unknown format]"),
            "active tab should not show [unknown format] in raw mode, got: {:?}",
            tab_row,
        );
    }

    #[tokio::test]
    async fn test_active_tab_hides_unknown_format_when_parser_detected() {
        let mut app = make_two_tab_app().await;
        app.tabs[0].title = "myfile.log".to_string();
        app.tabs[0].display.show_borders = true;
        app.tabs[0].display.format = Some(std::sync::Arc::from(crate::parser::json::JsonParser {
            schema: &crate::parser::SCHEMA_GENERIC_JSON,
            fields_container: None,
            span_key: None,
            score_weight: 1.0,
        }));
        app.tabs[1].display.show_borders = true;
        app.tabs[1].display.format = Some(std::sync::Arc::from(crate::parser::json::JsonParser {
            schema: &crate::parser::SCHEMA_GENERIC_JSON,
            fields_container: None,
            span_key: None,
            score_weight: 1.0,
        }));

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let tab_row = row_content(&buf, 0);
        assert!(
            !tab_row.contains("[unknown format]"),
            "active tab should not show [unknown format] when parser is detected, got: {:?}",
            tab_row,
        );
    }

    #[tokio::test]
    async fn test_logs_title_shows_filename_when_no_tab_bar() {
        let data: Vec<u8> = "line A\nline B\n".as_bytes().to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some("/tmp/uniquename.log".to_string())).await;
        let mut app = App::new(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        app.tabs[0].title = "uniquename.log".to_string();

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        // Single tab → no tab bar, row 0 is the top border of the logs panel.
        let border_row = row_content(&buf, 0);
        assert!(
            border_row.contains("uniquename.log"),
            "logs panel title should contain filename when no tab bar, got: {:?}",
            border_row,
        );
    }

    #[tokio::test]
    async fn test_sidebar_title_on_same_row_as_tab_bar() {
        let mut app = make_two_tab_app().await;
        app.tabs[0].display.show_sidebar = true;
        app.tabs[0].display.show_borders = true;
        app.tabs[1].display.show_borders = true;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        // Row 0 is the tab bar merged with the top border.
        // When the sidebar is visible its "Filters" title must appear on that
        // same row — not one row below.
        let row0 = row_content(&buf, 0);
        assert!(
            row0.contains("Filters"),
            "sidebar title should appear on row 0 (same as tab bar), got: {:?}",
            row0,
        );
        // Row 1 must NOT start with the sidebar title (it would if the sidebar
        // top border were misaligned one row down).
        let row1 = row_content(&buf, 1);
        assert!(
            !row1.contains("Filters"),
            "sidebar title must not appear on row 1 (would mean misaligned), got: {:?}",
            row1,
        );
    }

    #[tokio::test]
    async fn test_inactive_tab_uses_inactive_tab_fg_color() {
        let mut app = make_two_tab_app().await;
        let expected_fg = app.theme.inactive_tab_fg;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        // Row 0 is the tab bar. Find the column of the inactive tab title ("other").
        let tab_row = row_content(&buf, 0);
        let other_col = tab_row.find("other").expect("inactive tab title not found") as u16;
        let cell = buf.cell((other_col, 0)).expect("cell out of bounds");
        assert_eq!(
            cell.fg, expected_fg,
            "inactive tab should use inactive_tab_fg color, got {:?}",
            cell.fg
        );
    }

    #[tokio::test]
    async fn test_whole_line_filter_fg_suppresses_value_colors_on_covered_spans() {
        // A whole-line filter (match_only=false) has priority 500, value colors priority 0.
        // The sweep-line compose picks the higher-priority filter fg, so value colors
        // must not be applied to any part of the line.
        use crate::types::FilterType;

        let mut app = make_app(&["log GET /api"]).await;
        let get_color = app.theme.value_colors.http_get;

        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "log".to_string(),
                FilterType::Include,
                Some("[255,0,0]"),
                None,
                false,
            )
            .await;
        app.tabs[0].refresh_visible();

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let content_row = row_content(&buf, 1);
        let get_col = content_row
            .find("GET")
            .expect("GET should appear in content row") as u16;

        let cell = buf.cell((get_col, 1)).expect("cell should exist");
        assert_ne!(
            cell.fg, get_color,
            "GET value color must not be applied when a whole-line filter with fg already covers the span"
        );
    }

    #[tokio::test]
    async fn test_value_colors_applied_without_filter() {
        let mut app = make_app(&["log GET /api"]).await;
        let get_color = app.theme.value_colors.http_get;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let content_row = row_content(&buf, 1);
        let get_col = content_row
            .find("GET")
            .expect("GET should appear in content row") as u16;

        let cell = buf.cell((get_col, 1)).expect("cell should exist");
        assert_eq!(
            cell.fg, get_color,
            "GET value color should be applied when no filter overrides the line"
        );
    }

    #[tokio::test]
    async fn test_value_colors_apply_to_unfiltered_parts_of_filter_colored_line() {
        // A match-only filter colors "log" but leaves "GET" unstyled.
        // Value colors must still apply to the unstyled "GET" span.
        use crate::types::FilterType;

        let mut app = make_app(&["log GET /api"]).await;
        let get_color = app.theme.value_colors.http_get;

        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "log".to_string(),
                FilterType::Include,
                Some("[255,0,0]"),
                None,
                true, // match_only=true: only "log" is colored, "GET" stays unstyled
            )
            .await;
        app.tabs[0].refresh_visible();

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let content_row = row_content(&buf, 1);
        let get_col = content_row
            .find("GET")
            .expect("GET should appear in content row") as u16;

        let cell = buf.cell((get_col, 1)).expect("cell should exist");
        assert_eq!(
            cell.fg, get_color,
            "GET value color must apply to the unstyled part even when another part of the line is filter-colored"
        );
    }

    #[tokio::test]
    async fn test_filter_fg_bg_on_ip_overrides_value_colors() {
        use crate::types::FilterType;

        let mut app = make_app(&["log from 5.120.204.67 done"]).await;
        let ip_color = app.theme.value_colors.ip_address;

        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "5.120.204.67".to_string(),
                FilterType::Include,
                Some("Black"),
                Some("Salmon"),
                true,
            )
            .await;
        app.tabs[0].refresh_visible();

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let content_row = row_content(&buf, 1);
        let ip_col = content_row
            .find("5.120.204.67")
            .expect("IP should appear in content row") as u16;

        let cell = buf.cell((ip_col, 1)).expect("cell should exist");
        assert_ne!(
            cell.fg, ip_color,
            "IP value color must not override filter --fg Black"
        );
    }

    #[tokio::test]
    async fn test_filter_fg_bg_wins_after_initial_value_color_render() {
        use crate::types::FilterType;

        let mut app = make_app(&["log from 5.120.204.67 done"]).await;
        let ip_color = app.theme.value_colors.ip_address;

        // First render WITHOUT filter — populates render cache with value colors.
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        // Add filter the same way the command handler does.
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "5.120.204.67".to_string(),
                FilterType::Include,
                Some("Black"),
                Some("Salmon"),
                true,
            )
            .await;
        app.tabs[0].begin_filter_refresh();

        // Re-render — cache should be invalidated, filter fg must win.
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let content_row = row_content(&buf, 1);
        let ip_col = content_row
            .find("5.120.204.67")
            .expect("IP should appear in content row") as u16;
        let cell = buf.cell((ip_col, 1)).expect("cell should exist");
        assert_ne!(
            cell.fg, ip_color,
            "IP value color must not override filter --fg Black after re-render"
        );
    }

    #[tokio::test]
    async fn test_filter_fg_bg_wins_after_incremental_include() {
        use crate::types::FilterType;

        let mut app = make_app(&["log from 5.120.204.67 done"]).await;
        let ip_color = app.theme.value_colors.ip_address;

        // First render WITHOUT filter — populates render cache with value colors.
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        // Add filter and then go through the incremental include path.
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "5.120.204.67".to_string(),
                FilterType::Include,
                Some("Black"),
                Some("Salmon"),
                true,
            )
            .await;
        app.tabs[0].apply_incremental_include("5.120.204.67");

        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let content_row = row_content(&buf, 1);
        let ip_col = content_row
            .find("5.120.204.67")
            .expect("IP should appear in content row") as u16;
        let cell = buf.cell((ip_col, 1)).expect("cell should exist");
        assert_ne!(
            cell.fg, ip_color,
            "IP value color must not override filter --fg Black after incremental include"
        );
    }

    #[tokio::test]
    async fn test_filter_fg_bg_on_ip_in_structured_log() {
        use crate::types::FilterType;

        let json_line = r#"{"level":"info","msg":"request from 5.120.204.67 done"}"#;
        let mut app = make_app(&[json_line]).await;
        let ip_color = app.theme.value_colors.ip_address;

        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "5.120.204.67".to_string(),
                FilterType::Include,
                Some("Black"),
                Some("Salmon"),
                true,
            )
            .await;
        app.tabs[0].refresh_visible();

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let content: String = (0..buf.area.height)
            .flat_map(|y| {
                let w = buf.area.width;
                let row: String = (0..w)
                    .map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol()))
                    .collect();
                [row, "\n".to_string()]
            })
            .collect();

        if !content.contains("5.120.204.67") {
            return;
        }

        for y in 0..buf.area.height {
            let row = row_content(&buf, y);
            if let Some(ip_pos) = row.find("5.120.204.67") {
                let cell = buf.cell((ip_pos as u16, y)).expect("cell should exist");
                assert_ne!(
                    cell.fg, ip_color,
                    "IP value color must not override filter --fg Black in structured log (row {})",
                    y
                );
                return;
            }
        }
    }

    // When startup_warnings is non-empty a warnings bar must be rendered above
    // the mode bar and display all warning messages.
    #[tokio::test]
    async fn test_startup_warnings_shown_above_mode_bar() {
        let mut app = make_app(&["line 0"]).await;
        app.show_mode_bar = true;
        app.startup_warnings = vec![
            "keybinding conflict: j".to_string(),
            "keybinding conflict: k".to_string(),
        ];

        let mut terminal = make_terminal(); // 80×24
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let content: String = (0..buf.area.height)
            .map(|y| row_content(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            content.contains("keybinding conflict: j"),
            "first warning should appear in the warnings bar, got:\n{content}"
        );
        assert!(
            content.contains("keybinding conflict: k"),
            "second warning should appear in the warnings bar, got:\n{content}"
        );
    }

    // More than 10 warnings must be capped at 10 rows.
    #[tokio::test]
    async fn test_startup_warnings_capped_at_10_rows() {
        let mut app = make_app(&["line 0"]).await;
        app.startup_warnings = (0..15).map(|i| format!("conflict {i}")).collect();

        let mut terminal = make_terminal(); // 80×24
        terminal.draw(|f| app.ui(f)).unwrap();
        // Just verify it renders without panicking and shows the first warning.
        let buf = terminal.backend().buffer().clone();
        let content: String = (0..buf.area.height)
            .map(|y| row_content(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(content.contains("conflict 0"));
    }

    #[tokio::test]
    async fn test_cursor_line_has_bold_and_underlined() {
        let mut app = make_app(&["INFO first line", "ERROR second line"]).await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        // Row 0 is the top border; row 1 is the first content row (cursor at scroll=0).
        let cursor_row = 1u16;
        let has_bold = (0..buf.area.width).any(|x| {
            buf.cell((x, cursor_row))
                .map_or(false, |c| c.modifier.contains(Modifier::BOLD))
        });
        let has_underlined = (0..buf.area.width).any(|x| {
            buf.cell((x, cursor_row))
                .map_or(false, |c| c.modifier.contains(Modifier::UNDERLINED))
        });
        assert!(has_bold, "cursor line should have BOLD modifier");
        assert!(
            has_underlined,
            "cursor line should have UNDERLINED modifier"
        );
    }

    // After a keypress startup_warnings must be cleared.
    #[tokio::test]
    async fn test_startup_warnings_cleared_on_keypress() {
        let mut app = make_app(&["line 0"]).await;
        app.startup_warnings = vec!["conflict".to_string()];
        app.handle_key_event(crossterm::event::KeyCode::Esc).await;
        assert!(
            app.startup_warnings.is_empty(),
            "startup_warnings should be cleared after a keypress"
        );
    }
}
