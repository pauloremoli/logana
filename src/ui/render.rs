use ratatui::{
    Frame,
    prelude::*,
    widgets::{Block, Paragraph},
};
use std::sync::Arc;

use crate::mode::app_mode::ModeRenderState;

use super::App;
use super::field_layout::count_wrapped_lines;
use super::widgets::{
    CommandBar, CompletionSource, InputBar, LogPanel, ModeBar, Sidebar, TabBar, TabBarEntry,
    command_help_lines, file_display_name, prepare_log_panel, resolve_completions,
};

type DltSelectData = Option<(
    Vec<crate::config::DltDevice>,
    usize,
    Option<String>,
    Option<crate::mode::dlt_select_mode::AddDeviceRenderState>,
)>;

type ValueColorsData = Option<(
    Vec<crate::mode::value_colors_mode::ValueColorGroup>,
    String,
    usize,
    &'static str,
)>;

type ExportFooterData = Option<(String, Vec<(String, Vec<String>)>, usize, usize, usize)>;

type DefaultFiltersData = Option<(
    Vec<crate::mode::default_filters_mode::DefaultFilterRow>,
    String,
    usize,
    Option<crate::mode::default_filters_mode::PathEditState>,
)>;

type ArchivePickerData = Option<(
    Vec<crate::mode::archive_picker_mode::ArchiveRow>,
    usize,
    String,
    String,
    bool,
)>;

type FileSwitcherData = Option<(Vec<(usize, String)>, usize, usize, String)>;

type ThemePickerData = Option<(Vec<String>, usize, String)>;

struct UiRenderState {
    has_input_bar: bool,
    command_input: Option<(String, usize)>,
    completion_index: Option<usize>,
    completion_query: Option<String>,
    search_input: Option<(String, usize, bool, bool)>,
    is_confirm_restore: bool,
    session_files: Option<Arc<Vec<String>>>,
    selected_filter_idx: usize,
    /// Live typeahead query narrowing the sidebar; empty when not searching.
    filter_search: String,
    /// True while filter search is capturing input — see `ModeRenderState::FilterManagement`.
    filter_searching: bool,
    /// Name of the group selected in `GroupManagement`, or `None` outside it.
    selected_group: Option<String>,
    visual_anchor: Option<usize>,
    visual_char_selection: Option<(usize, usize)>,
    comment_popup: Option<(Vec<String>, usize, usize, usize, bool)>,
    help_state: Option<(usize, String)>,
    select_fields_state: Option<(Vec<(String, bool)>, usize)>,
    merge_select_state: Option<(Vec<(String, bool)>, usize)>,
    archive_picker_state: ArchivePickerData,
    docker_select: Option<(
        Vec<crate::mode::docker_select_mode::DockerContainer>,
        usize,
        Option<String>,
    )>,
    dlt_select: DltSelectData,
    value_colors_state: ValueColorsData,
    export_footer: ExportFooterData,
    default_filters_state: DefaultFiltersData,
    file_switcher_state: FileSwitcherData,
    theme_picker_state: ThemePickerData,
    notification: Option<String>,
    has_notification: bool,
    has_warnings: bool,
    warnings_height: u16,
    show_mode_bar: bool,
    status_line: Line<'static>,
    keybindings: std::sync::Arc<crate::config::Keybindings>,
}

fn compute_inner_width(total_width: u16, show_borders: bool) -> usize {
    let border_width = if show_borders { 2 } else { 1 };
    (total_width as usize).saturating_sub(border_width)
}

fn compute_status_height(status_line: &Line, inner_width: usize, show_borders: bool) -> u16 {
    let status_text: String = status_line
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    let content_lines = count_wrapped_lines(&status_text, inner_width);
    if show_borders {
        (content_lines + 1).clamp(2, 5) as u16
    } else {
        content_lines.clamp(1, 4) as u16
    }
}

/// The format name to show in the tab bar for a merged tab: its sources'
/// shared format name, when every source that detected one detected the
/// *same* format (the common case — e.g. merging two journalctl files).
/// `None` when sources disagree or none detected anything, same as a plain
/// tab with no detected format — "unknown format" would be misleading here
/// (detection didn't fail, there just isn't one single answer to show).
fn merged_format_name(merged: &crate::ui::MergedState) -> Option<String> {
    merged.uniform_parser().map(|p| p.name().to_string())
}

/// Caps how many startup warnings (and how many terminal rows total, once
/// a multi-line one — e.g. a `regex::Error`'s caret-pointer diagram under
/// an invalid schema pattern — is split into one row per line) the
/// warnings banner will show, so one badly-formatted message can't push
/// the log panel off-screen.
const MAX_WARNINGS_SHOWN: usize = 10;
const MAX_WARNING_ROWS: usize = 20;

/// Flattens `warnings` into one entry per terminal row — each warning may
/// itself span multiple lines (e.g. a regex compile error), which a plain
/// one-`Line`-per-warning render would show as a single garbled row full of
/// literal newline characters instead of separate lines. Each entry is
/// `(is_first_line_of_its_warning, text)`; the first line of a warning is
/// the one that should get the "Warning: " label.
fn warning_rows(warnings: &[String]) -> Vec<(bool, &str)> {
    let mut rows = Vec::new();
    for w in warnings.iter().take(MAX_WARNINGS_SHOWN) {
        for (i, line) in w.lines().enumerate() {
            if rows.len() >= MAX_WARNING_ROWS {
                return rows;
            }
            rows.push((i == 0, line));
        }
    }
    rows
}

impl App {
    pub(super) fn ui(&mut self, frame: &mut Frame) {
        let size = frame.area();
        frame.render_widget(Block::default().bg(self.theme.root_bg), size);

        let show_tab_bar = !(self.tabs.is_empty()
            || self.pending_archive.is_some()
                && self.tabs.len() == 1
                && self.tabs[0].file_reader.line_count() == 0
                && self.tabs[0].load_state.is_none());
        let render_state = self.tabs[self.active_tab].interaction.mode.render_state();
        let show_borders = self.tabs[self.active_tab].display.show_borders;
        let mode_name = if !self.display.show_mode_bar {
            Some(render_state.mode_name())
        } else {
            None
        };

        let state = self.extract_ui_render_state(&render_state);

        let inner_width = compute_inner_width(size.width, show_borders);
        let status_height = compute_status_height(&state.status_line, inner_width, show_borders);
        let hint_height = self.compute_hint_height(
            &state.command_input,
            state.completion_query.as_deref(),
            inner_width,
            state.completion_index,
        );

        let (constraints, notification_chunk_idx, warnings_chunk_idx) =
            self.build_layout_constraints(&state, show_tab_bar, hint_height, status_height);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(size);

        let mut chunk_idx = 0;

        if show_tab_bar {
            self.input.tab_bar_area = Some(chunks[chunk_idx]);
            self.render_tab_bar_widget(frame, chunks[chunk_idx], show_borders, mode_name, &state);
            chunk_idx += 1;
        } else {
            self.input.tab_bar_area = None;
        }

        let main_chunk = chunks[chunk_idx];
        chunk_idx += 1;

        let (logs_area, sidebar_area) =
            self.compute_main_areas(main_chunk, show_tab_bar, show_borders);
        self.input.log_panel_area = logs_area;
        self.input.sidebar_area = sidebar_area;
        self.render_log_panel(frame, logs_area, show_tab_bar, mode_name, &state);
        if let Some(sa) = sidebar_area {
            let is_filter_mode = matches!(
                render_state,
                ModeRenderState::FilterManagement { .. } | ModeRenderState::FilterEdit
            );
            let is_group_mode = matches!(render_state, ModeRenderState::GroupManagement { .. });
            let show_groups = self.tabs[self.active_tab].display.show_groups_panel;
            self.render_sidebar(
                frame,
                sa,
                show_borders,
                state.selected_filter_idx,
                is_filter_mode,
                &state.filter_search,
                state.filter_searching,
                state.selected_group.as_deref(),
                is_group_mode,
                show_groups,
            );
        }

        self.render_command_bar_widget(frame, &chunks, chunk_idx, &state);
        self.render_input_bar_widget(frame, &chunks, chunk_idx, &state);

        self.render_notification(frame, &chunks, notification_chunk_idx, &state);
        self.render_warnings(frame, &chunks, warnings_chunk_idx);
        self.render_mode_bar_widget(frame, &chunks, show_borders, &state);

        let frame_area = frame.area();
        self.render_overlay_popups(frame, frame_area, &state);
    }

    fn extract_ui_render_state(&mut self, render_state: &ModeRenderState) -> UiRenderState {
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
        let command_input: Option<(String, usize)> = match render_state {
            ModeRenderState::Command { input, cursor, .. } => Some((input.clone(), *cursor)),
            _ => None,
        };
        let completion_index: Option<usize> = match render_state {
            ModeRenderState::Command {
                completion_index, ..
            } => *completion_index,
            _ => None,
        };
        let completion_query: Option<String> = match render_state {
            ModeRenderState::Command {
                completion_query, ..
            } => completion_query.clone(),
            _ => None,
        };
        let search_input: Option<(String, usize, bool, bool)> = match render_state {
            ModeRenderState::Search {
                query,
                cursor,
                forward,
            } => Some((query.clone(), *cursor, *forward, true)),
            _ => persistent_pattern.map(|p| {
                let len = p.len();
                (p, len, true, false)
            }),
        };
        let is_confirm_restore = matches!(render_state, ModeRenderState::ConfirmRestore);
        let session_files: Option<Arc<Vec<String>>> = match render_state {
            ModeRenderState::ConfirmRestoreSession { files } => Some(Arc::clone(files)),
            _ => None,
        };
        let selected_filter_idx = match render_state {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                self.tabs[self.active_tab].filter.last_selected_filter = *selected_index;
                *selected_index
            }
            _ => self.tabs[self.active_tab].filter.last_selected_filter,
        };
        let filter_search = match render_state {
            ModeRenderState::FilterManagement { search, .. } => search.clone(),
            _ => String::new(),
        };
        let filter_searching = matches!(
            render_state,
            ModeRenderState::FilterManagement {
                searching: true,
                ..
            }
        );
        // Unlike `selected_filter_idx`, this is intentionally not "sticky" —
        // the Groups section shows no highlight once you leave
        // `GroupManagement`, rather than remembering the last selection the
        // way the filter list does via `last_selected_filter`.
        let selected_group = match render_state {
            ModeRenderState::GroupManagement { selected_group } => Some(selected_group.clone()),
            _ => None,
        };
        let keybindings = self.tabs[self.active_tab].interaction.keybindings.clone();
        let status_line = self.tabs[self.active_tab]
            .interaction
            .mode
            .mode_bar_content(&keybindings, &self.theme);
        let show_mode_bar = self.display.show_mode_bar;
        let has_warnings = !self.session.startup_warnings.is_empty();
        let warnings_height = warning_rows(&self.session.startup_warnings).len() as u16;
        let visual_anchor: Option<usize> = match render_state {
            ModeRenderState::VisualLine { anchor } => Some(*anchor),
            _ => None,
        };
        let visual_char_selection: Option<(usize, usize)> = match render_state {
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
        let comment_popup: Option<(Vec<String>, usize, usize, usize, bool)> = match render_state {
            ModeRenderState::Comment {
                lines,
                cursor_row,
                cursor_col,
                line_count,
                is_editing,
            } => Some((
                lines.clone(),
                *cursor_row,
                *cursor_col,
                *line_count,
                *is_editing,
            )),
            _ => None,
        };
        let help_state: Option<(usize, String)> = match render_state {
            ModeRenderState::KeybindingsHelp { scroll, search } => Some((*scroll, search.clone())),
            _ => None,
        };
        let select_fields_state: Option<(Vec<(String, bool)>, usize)> = match render_state {
            ModeRenderState::SelectFields { fields, selected } => Some((fields.clone(), *selected)),
            _ => None,
        };
        let merge_select_state: Option<(Vec<(String, bool)>, usize)> = match render_state {
            ModeRenderState::MergeSelect { tabs, selected } => Some((tabs.clone(), *selected)),
            _ => None,
        };
        let archive_picker_state: ArchivePickerData = match render_state {
            ModeRenderState::ArchivePicker {
                rows,
                selected,
                source_path,
                search,
                searching,
            } => Some((
                rows.clone(),
                *selected,
                source_path.clone(),
                search.clone(),
                *searching,
            )),
            _ => None,
        };
        let docker_select: Option<(
            Vec<crate::mode::docker_select_mode::DockerContainer>,
            usize,
            Option<String>,
        )> = match render_state {
            ModeRenderState::DockerSelect {
                containers,
                selected,
                error,
            } => Some((containers.clone(), *selected, error.clone())),
            _ => None,
        };
        let dlt_select: DltSelectData = match render_state {
            ModeRenderState::DltSelect {
                devices,
                selected,
                error,
                adding,
            } => Some((devices.clone(), *selected, error.clone(), adding.clone())),
            _ => None,
        };
        let value_colors_state: ValueColorsData = match render_state {
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
        let export_footer: ExportFooterData = match render_state {
            ModeRenderState::ExportFooter {
                path,
                fields,
                active_idx,
                cursor_row,
                cursor_col,
                ..
            } => Some((
                path.clone(),
                fields.clone(),
                *active_idx,
                *cursor_row,
                *cursor_col,
            )),
            _ => None,
        };
        let default_filters_state: DefaultFiltersData = match render_state {
            ModeRenderState::DefaultFilters {
                rows,
                search,
                selected,
                editing,
            } => Some((rows.clone(), search.clone(), *selected, editing.clone())),
            _ => None,
        };
        let file_switcher_state: FileSwitcherData = match render_state {
            ModeRenderState::FileSwitcher {
                entries,
                active_tab,
                selected,
                search,
            } => Some((entries.clone(), *active_tab, *selected, search.clone())),
            _ => None,
        };
        let theme_picker_state: ThemePickerData = match render_state {
            ModeRenderState::ThemePicker {
                entries,
                selected,
                search,
            } => Some((entries.clone(), *selected, search.clone())),
            _ => None,
        };

        if self.decompression_message.is_none()
            && let Some(set_at) = self.tabs[self.active_tab].interaction.notification_set_at
            && set_at.elapsed() > std::time::Duration::from_secs(10)
        {
            self.tabs[self.active_tab].clear_notification();
        }
        let notification = self
            .decompression_message
            .clone()
            .or_else(|| self.tabs[self.active_tab].interaction.notification.clone());
        let has_notification = notification.is_some() && !has_input_bar;

        UiRenderState {
            has_input_bar,
            command_input,
            completion_index,
            completion_query,
            search_input,
            is_confirm_restore,
            session_files,
            selected_filter_idx,
            filter_search,
            filter_searching,
            selected_group,
            visual_anchor,
            visual_char_selection,
            comment_popup,
            help_state,
            select_fields_state,
            merge_select_state,
            archive_picker_state,
            docker_select,
            dlt_select,
            value_colors_state,
            export_footer,
            default_filters_state,
            file_switcher_state,
            theme_picker_state,
            notification,
            has_notification,
            has_warnings,
            warnings_height,
            show_mode_bar,
            status_line,
            keybindings,
        }
    }

    fn build_layout_constraints(
        &mut self,
        state: &UiRenderState,
        show_tab_bar: bool,
        hint_height: u16,
        status_height: u16,
    ) -> (Vec<Constraint>, Option<usize>, Option<usize>) {
        let mut constraints = vec![];
        if show_tab_bar {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Min(1));
        if state.has_input_bar {
            constraints.push(Constraint::Length(1));
            constraints.push(Constraint::Length(hint_height));
        }
        let notification_chunk_idx = if state.has_notification {
            let idx = constraints.len();
            constraints.push(Constraint::Length(1));
            Some(idx)
        } else {
            None
        };
        let warnings_chunk_idx = if state.has_warnings {
            let idx = constraints.len();
            constraints.push(Constraint::Length(state.warnings_height));
            Some(idx)
        } else {
            None
        };
        if state.show_mode_bar {
            constraints.push(Constraint::Length(status_height));
        }
        (constraints, notification_chunk_idx, warnings_chunk_idx)
    }

    fn compute_main_areas(
        &self,
        main_chunk: Rect,
        show_tab_bar: bool,
        show_borders: bool,
    ) -> (Rect, Option<Rect>) {
        let tab = &self.tabs[self.active_tab];
        let sidebar_width = tab.display.sidebar_width;
        let sidebar_left = tab.display.sidebar_side.is_left();

        if !tab.display.show_sidebar {
            return (main_chunk, None);
        }

        if show_borders {
            let constraints = if sidebar_left {
                [Constraint::Length(sidebar_width), Constraint::Min(1)]
            } else {
                [Constraint::Min(1), Constraint::Length(sidebar_width)]
            };
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(constraints)
                .split(main_chunk);
            let (logs_idx, sidebar_idx) = if sidebar_left { (1, 0) } else { (0, 1) };
            let raw_sidebar = horizontal[sidebar_idx];
            let sidebar = if show_tab_bar {
                Rect {
                    y: raw_sidebar.y.saturating_sub(1),
                    height: raw_sidebar.height + 1,
                    ..raw_sidebar
                }
            } else {
                raw_sidebar
            };
            (horizontal[logs_idx], Some(sidebar))
        } else {
            let constraints = if sidebar_left {
                [
                    Constraint::Length(sidebar_width),
                    Constraint::Length(1),
                    Constraint::Min(1),
                ]
            } else {
                [
                    Constraint::Min(1),
                    Constraint::Length(1),
                    Constraint::Length(sidebar_width),
                ]
            };
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(constraints)
                .split(main_chunk);
            let (logs_idx, sidebar_idx) = if sidebar_left { (2, 0) } else { (0, 2) };
            (horizontal[logs_idx], Some(horizontal[sidebar_idx]))
        }
    }

    /// Builds the `TabBar` widget used for both rendering and mouse
    /// hit-testing, so a click always maps to the same layout that was
    /// actually drawn.
    fn build_tab_bar<'a>(&'a self, show_borders: bool, mode_name: Option<&'a str>) -> TabBar<'a> {
        let loading_info: Vec<(usize, usize)> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(i, tab)| {
                tab.load_state.as_ref().map(|ls| {
                    let pct = (*ls.progress_rx.borrow() * 100.0) as usize;
                    (i, pct)
                })
            })
            .collect();
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
                } else if let Some(merged) = &t.merged {
                    merged_format_name(merged)
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
                    is_temp: t.is_temp_backed(),
                }
            })
            .collect();
        TabBar {
            tabs: tab_entries,
            active_tab: self.active_tab,
            loading_info,
            filtering_tabs,
            show_borders,
            mode_name,
            theme: &self.theme,
        }
    }

    fn render_tab_bar_widget(
        &self,
        frame: &mut Frame,
        area: Rect,
        show_borders: bool,
        mode_name: Option<&str>,
        state: &UiRenderState,
    ) {
        frame.render_widget(self.build_tab_bar(show_borders, mode_name), area);
        let _ = state;
    }

    /// Maps a mouse click to a tab index, mirroring exactly what
    /// `render_tab_bar_widget` last drew (same entries, offset, and widths).
    pub(super) fn hit_test_tab_bar(&self, col: u16, row: u16) -> Option<usize> {
        let area = self.input.tab_bar_area?;
        let show_borders = self.tabs[self.active_tab].display.show_borders;
        let mode_name = if !self.display.show_mode_bar {
            Some(
                self.tabs[self.active_tab]
                    .interaction
                    .mode
                    .render_state()
                    .mode_name(),
            )
        } else {
            None
        };
        self.build_tab_bar(show_borders, mode_name)
            .hit_test(area, col, row)
    }

    fn render_log_panel(
        &mut self,
        frame: &mut Frame,
        logs_area: Rect,
        show_tab_bar: bool,
        mode_name: Option<&str>,
        state: &UiRenderState,
    ) {
        let log_panel_data = prepare_log_panel(
            &mut self.tabs[self.active_tab],
            logs_area,
            state.visual_anchor,
            state.visual_char_selection,
            mode_name,
            show_tab_bar,
            state.has_input_bar,
            &self.theme,
        );
        frame.render_widget(
            LogPanel {
                data: &log_panel_data,
            },
            logs_area,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn render_sidebar(
        &mut self,
        frame: &mut Frame,
        sidebar_area: Rect,
        show_borders: bool,
        selected_filter_idx: usize,
        is_filter_mode: bool,
        search: &str,
        searching: bool,
        selected_group: Option<&str>,
        is_group_mode: bool,
        show_groups: bool,
    ) {
        let tab = &mut self.tabs[self.active_tab];
        let all_filters = tab.log_manager.get_filters();
        let group_names = tab.log_manager.group_names();
        let all_match_counts = &tab.filter.match_counts;
        // Narrowing to only the entries matching `search` (everything, when not
        // searching) is expressed uniformly here rather than branching, since
        // `narrowed_filter_indices` already returns every index in order for an
        // empty query.
        let narrowed =
            super::widgets::sidebar::narrowed_filter_indices(all_filters, all_match_counts, search);
        let filters: Vec<crate::filters::FilterDef> =
            narrowed.iter().map(|&i| all_filters[i].clone()).collect();
        let match_counts: Vec<usize> = narrowed
            .iter()
            .map(|&i| all_match_counts.get(i).copied().unwrap_or(0))
            .collect();
        let filters = filters.as_slice();
        let filter_progress: Option<usize> = tab
            .filter
            .handle
            .as_ref()
            .map(|h| (h.displayed_progress * 100.0) as usize);
        let (content_w, content_h) =
            super::widgets::sidebar::sidebar_inner_dims(sidebar_area, show_borders);
        let group_count = if show_groups { group_names.len() } else { 0 };
        let (filters_h, _groups_h) =
            super::widgets::sidebar::split_sidebar_heights(content_h, group_count);
        tab.filter.sidebar_visible_height = filters_h;
        let scroll_offset = super::widgets::sidebar::compute_scroll_offset(
            filters,
            selected_filter_idx,
            &match_counts,
            content_w,
            filters_h,
            tab.filter.sidebar_scroll,
        );
        tab.filter.sidebar_scroll = scroll_offset;
        frame.render_widget(
            Sidebar {
                filters,
                groups: tab.log_manager.get_group_styles(),
                all_filters,
                group_names: &group_names,
                match_counts: &match_counts,
                selected_filter_idx,
                selected_group,
                filter_enabled: tab.filter.enabled,
                show_marks_only: tab.filter.show_marks_only,
                filter_progress,
                show_borders,
                is_filter_mode,
                is_group_mode,
                show_groups,
                scroll_offset,
                highlight_mode: tab.filter.highlight_mode,
                search,
                searching,
                theme: &self.theme,
            },
            sidebar_area,
        );
    }

    fn render_command_bar_widget(
        &mut self,
        frame: &mut Frame,
        chunks: &[Rect],
        chunk_idx: usize,
        state: &UiRenderState,
    ) {
        if let Some((input_text, cursor_pos)) = state.command_input.clone() {
            let query_text = state
                .completion_query
                .as_deref()
                .unwrap_or(input_text.as_str());
            let completion = resolve_completions(
                &mut self.tabs[self.active_tab],
                query_text,
                state.completion_index,
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
    }

    fn render_input_bar_widget(
        &mut self,
        frame: &mut Frame,
        chunks: &[Rect],
        chunk_idx: usize,
        state: &UiRenderState,
    ) {
        if let Some((input_str, cursor_pos, forward, is_active)) = state.search_input.clone() {
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
                cursor_pos,
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
    }

    fn render_notification(
        &self,
        frame: &mut Frame,
        chunks: &[Rect],
        notification_chunk_idx: Option<usize>,
        state: &UiRenderState,
    ) {
        if let Some(idx) = notification_chunk_idx
            && let Some(msg) = &state.notification
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
    }

    fn render_warnings(
        &self,
        frame: &mut Frame,
        chunks: &[Rect],
        warnings_chunk_idx: Option<usize>,
    ) {
        if let Some(idx) = warnings_chunk_idx {
            let warnings_area = chunks[idx];
            let lines: Vec<Line> = warning_rows(&self.session.startup_warnings)
                .into_iter()
                .map(|(is_first, text)| {
                    if is_first {
                        Line::from(vec![
                            Span::styled("Warning: ", Style::default().fg(Color::Red)),
                            Span::raw(text.to_string()),
                        ])
                    } else {
                        // Indented to align under the text that follows
                        // "Warning: " (9 characters) on the first line.
                        Line::from(Span::raw(format!("         {text}")))
                    }
                })
                .collect();
            frame.render_widget(
                Paragraph::new(lines).style(Style::default().bg(self.theme.root_bg)),
                warnings_area,
            );
        }
    }

    fn render_mode_bar_widget(
        &self,
        frame: &mut Frame,
        chunks: &[Rect],
        show_borders: bool,
        state: &UiRenderState,
    ) {
        if state.show_mode_bar
            && let Some(&status_area) = chunks.last()
        {
            frame.render_widget(
                ModeBar {
                    content: state.status_line.clone(),
                    show_borders,
                    theme: &self.theme,
                },
                status_area,
            );
        }
    }

    fn render_overlay_popups(
        &mut self,
        frame: &mut Frame,
        frame_area: Rect,
        state: &UiRenderState,
    ) {
        if state.is_confirm_restore {
            frame.render_widget(
                super::widgets::ConfirmRestoreModal {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                },
                frame_area,
            );
        }

        if let Some(files) = &state.session_files {
            frame.render_widget(
                super::widgets::ConfirmRestoreSessionModal {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    files,
                },
                frame_area,
            );
        }

        if let Some((lines, cursor_row, cursor_col, line_count, is_editing)) = &state.comment_popup
        {
            let popup = super::widgets::CommentPopup {
                theme: &self.theme,
                keybindings: &self.tabs[self.active_tab].interaction.keybindings,
                lines,
                cursor_row: *cursor_row,
                cursor_col: *cursor_col,
                line_count: *line_count,
                is_editing: *is_editing,
            };
            if let Some((cx, cy)) = popup.cursor_position(frame_area) {
                frame.set_cursor_position((cx, cy));
            }
            frame.render_widget(popup, frame_area);
        }

        if let Some((fields, selected)) = &state.select_fields_state {
            frame.render_widget(
                super::widgets::SelectFieldsPopup {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    fields,
                    selected: *selected,
                },
                frame_area,
            );
        }

        if let Some((tabs, selected)) = &state.merge_select_state {
            frame.render_widget(
                super::widgets::MergeSelectPopup {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    tabs,
                    selected: *selected,
                },
                frame_area,
            );
        }

        if let Some((rows, selected, source_path, search, searching)) = &state.archive_picker_state
        {
            self.tabs[self.active_tab]
                .interaction
                .archive_picker_visible_height =
                super::widgets::archive_picker_popup::popup_content_height(
                    frame_area.height,
                    rows.len(),
                    *searching,
                );
            frame.render_widget(
                super::widgets::ArchivePickerPopup {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    rows,
                    selected: *selected,
                    source_path,
                    search,
                    searching: *searching,
                },
                frame_area,
            );
        }

        if let Some((containers, selected, error)) = &state.docker_select {
            frame.render_widget(
                super::widgets::DockerSelectPopup {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    containers,
                    selected: *selected,
                    error: error.as_deref(),
                },
                frame_area,
            );
        }

        if let Some((devices, selected, error, adding)) = &state.dlt_select {
            frame.render_widget(
                super::widgets::DltSelectPopup {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    devices,
                    selected: *selected,
                    error: error.as_deref(),
                    adding: adding.as_ref(),
                },
                frame_area,
            );
        }

        if let Some((groups, search, selected, title)) = &state.value_colors_state {
            frame.render_widget(
                super::widgets::ValueColorsPopup {
                    theme: &self.theme,
                    keybindings: &self.keybindings,
                    groups,
                    search,
                    selected: *selected,
                    title,
                },
                frame_area,
            );
        }

        if let Some((rows, search, selected, editing)) = &state.default_filters_state {
            frame.render_widget(
                super::widgets::DefaultFiltersPopup {
                    theme: &self.theme,
                    rows,
                    search,
                    selected: *selected,
                    editing: editing.as_ref(),
                },
                frame_area,
            );
        }

        if let Some((entries, active_tab, selected, search)) = &state.file_switcher_state {
            frame.render_widget(
                super::widgets::FileSwitcherPopup {
                    theme: &self.theme,
                    entries,
                    active_tab: *active_tab,
                    selected: *selected,
                    search,
                },
                frame_area,
            );
        }

        if let Some((entries, selected, search)) = &state.theme_picker_state {
            frame.render_widget(
                super::widgets::ThemePickerPopup {
                    theme: &self.theme,
                    entries,
                    selected: *selected,
                    search,
                },
                frame_area,
            );
        }

        if let Some((scroll, search)) = &state.help_state {
            frame.render_widget(
                super::widgets::KeybindingsHelpPopup {
                    theme: &self.theme,
                    keybindings: &state.keybindings,
                    scroll: *scroll,
                    search,
                },
                frame_area,
            );
        }

        if let Some((path, fields, active_idx, cursor_row, cursor_col)) = &state.export_footer {
            let popup = super::widgets::ExportFooterPopup {
                theme: &self.theme,
                keybindings: &self.tabs[self.active_tab].interaction.keybindings,
                path,
                fields,
                active_idx: *active_idx,
                cursor_row: *cursor_row,
                cursor_col: *cursor_col,
            };
            if let Some((cx, cy)) = popup.cursor_position(frame_area) {
                frame.set_cursor_position((cx, cy));
            }
            frame.render_widget(popup, frame_area);
        }
    }

    fn compute_hint_height(
        &mut self,
        command_input: &Option<(String, usize)>,
        completion_query: Option<&str>,
        width: usize,
        completion_index: Option<usize>,
    ) -> u16 {
        let Some((input_text, _)) = command_input else {
            return 1;
        };
        let query_text = completion_query.unwrap_or(input_text.as_str());
        let tab = &mut self.tabs[self.active_tab];
        let text = match resolve_completions(tab, query_text, completion_index) {
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
            CompletionSource::SchemaItems(items) => items
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join("  "),
            CompletionSource::CommandHelp(cmd) => {
                // Structured (usage / description / examples), not a single
                // wrapped paragraph — size by summing each section's own
                // wrapped height instead of `count_wrapped_lines` on one
                // flattened string. Built via the same `command_help_lines`
                // the widget itself renders, so the two can't drift apart.
                let lines = command_help_lines(cmd, &self.theme);
                let total: usize = lines
                    .iter()
                    .map(|line| {
                        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                        count_wrapped_lines(&plain, width).max(1)
                    })
                    .sum();
                return (total as u16).clamp(1, 15);
            }
        };
        if text.is_empty() {
            return 1;
        }
        (count_wrapped_lines(&text, width) as u16).clamp(1, 3)
    }
}

pub(crate) fn progress_bar_str(progress: f64) -> (String, usize) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
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
        App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
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
    async fn test_click_on_tab_bar_switches_active_tab() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].display.show_borders = false;
        let data: Vec<u8> = b"tab2\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut t = super::super::TabState::new(fr, lm, "tab2".to_string());
        t.interaction.keybindings = app.keybindings.clone();
        t.display.show_borders = false;
        app.tabs.push(t);
        assert_eq!(app.active_tab, 0);

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let area = app.input.tab_bar_area.expect("tab bar area should be set");

        let col = (area.x..area.x + area.width)
            .find(|&c| app.hit_test_tab_bar(c, area.y) == Some(1))
            .expect("second tab should be clickable somewhere in the bar");

        app.handle_left_click(col, area.y).await;
        assert_eq!(app.active_tab, 1);
    }

    #[tokio::test]
    async fn test_click_below_tab_bar_does_not_change_active_tab() {
        let mut app = make_app(&["line"]).await;
        let data: Vec<u8> = b"tab2\n".to_vec();
        let fr = FileReader::from_bytes(data);
        let lm = LogManager::new(app.db.clone(), None).await;
        let mut t = super::super::TabState::new(fr, lm, "tab2".to_string());
        t.interaction.keybindings = app.keybindings.clone();
        app.tabs.push(t);

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let area = app.input.tab_bar_area.expect("tab bar area should be set");

        app.handle_left_click(area.x, area.y + area.height).await;
        assert_eq!(app.active_tab, 0);
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
            cursor: 4,
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
            cursor: 4,
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
            cursor: 0,
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
                crate::filters::FilterType::Include,
                crate::filters::FilterOptions::default().line_mode(),
            )
            .await;
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "ERROR".to_string(),
                crate::filters::FilterType::Include,
                crate::filters::FilterOptions::default().line_mode(),
            )
            .await;
        app.tabs[0].refresh_visible();
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode::new(0));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_persists_sidebar_visible_height_for_page_motions() {
        let mut app = make_app(&["INFO something"]).await;
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "INFO".to_string(),
                crate::filters::FilterType::Include,
                crate::filters::FilterOptions::default().line_mode(),
            )
            .await;
        app.tabs[0].refresh_visible();
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode::new(0));
        assert_eq!(app.tabs[0].filter.sidebar_visible_height, 0);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        assert!(
            app.tabs[0].filter.sidebar_visible_height > 0,
            "sidebar_visible_height should be persisted from the sidebar's rendered content height"
        );
    }

    #[tokio::test]
    async fn test_ui_persists_archive_picker_visible_height_for_page_motions() {
        use crate::ingestion::{ArchiveNode, ArchiveTree, NodeKind};
        use crate::mode::archive_picker_mode::ArchivePickerMode;

        let mut app = make_app(&[]).await;
        let nodes: Vec<ArchiveNode> = (0..30)
            .map(|i| ArchiveNode {
                id: i,
                parent: None,
                name: format!("file{i}.log"),
                full_path: format!("file{i}.log"),
                depth: 0,
                kind: NodeKind::File,
                selected: false,
                merge_marked: false,
                cached_bytes: None,
                collapsed: false,
                disk_path: false,
            })
            .collect();
        let roots = (0..30).collect();
        let tree = ArchiveTree { nodes, roots };
        app.tabs[0].interaction.mode =
            Box::new(ArchivePickerMode::new(tree, "archive.zip".to_string()));
        assert_eq!(app.tabs[0].interaction.archive_picker_visible_height, 0);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        assert!(
            app.tabs[0].interaction.archive_picker_visible_height > 0,
            "archive_picker_visible_height should be persisted from the popup's rendered content height"
        );
    }

    #[tokio::test]
    async fn test_ui_search_narrows_sidebar_to_matching_filters() {
        let mut app = make_app(&["INFO something"]).await;
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "errno".to_string(),
                crate::filters::FilterType::Include,
                crate::filters::FilterOptions::default(),
            )
            .await;
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "timeout".to_string(),
                crate::filters::FilterType::Exclude,
                crate::filters::FilterOptions::default(),
            )
            .await;
        app.tabs[0].refresh_visible();
        let mut mode = FilterManagementMode::new(0);
        mode.searching = true;
        mode.search = "err".to_string();
        app.tabs[0].interaction.mode = Box::new(mode);

        let mut terminal = make_terminal();
        let buf = terminal.draw(|f| app.ui(f)).unwrap().buffer.clone();
        let rendered: String = (0..buf.area.height).map(|y| row_content(&buf, y)).collect();

        assert!(
            rendered.contains("errno"),
            "matching filter should still be shown while searching"
        );
        assert!(
            !rendered.contains("timeout"),
            "non-matching filter must be hidden from the sidebar while searching: {rendered}"
        );
    }

    #[tokio::test]
    async fn test_ui_search_query_shown_in_sidebar_title() {
        let mut app = make_app(&["INFO something"]).await;
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "errno".to_string(),
                crate::filters::FilterType::Include,
                crate::filters::FilterOptions::default(),
            )
            .await;
        app.tabs[0].refresh_visible();
        let mut mode = FilterManagementMode::new(0);
        mode.searching = true;
        mode.search = "err".to_string();
        app.tabs[0].interaction.mode = Box::new(mode);

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let filters_row = (0..buf.area.height)
            .map(|y| row_content(&buf, y))
            .find(|row| row.contains("Filters"))
            .expect("a row containing 'Filters' should be rendered");
        assert!(
            filters_row.contains("/err"),
            "sidebar title should show the active search query: {filters_row:?}"
        );
    }

    #[tokio::test]
    async fn test_ui_search_marker_shown_immediately_on_empty_query() {
        // Right after pressing '/', before any character is typed, the title
        // must already signal that search mode is active — otherwise there's
        // no visible clue the app is now waiting for search text.
        let mut app = make_app(&["INFO something"]).await;
        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "errno".to_string(),
                crate::filters::FilterType::Include,
                crate::filters::FilterOptions::default(),
            )
            .await;
        app.tabs[0].refresh_visible();
        let mut mode = FilterManagementMode::new(0);
        mode.searching = true;
        mode.search = String::new();
        app.tabs[0].interaction.mode = Box::new(mode);

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let filters_row = (0..buf.area.height)
            .map(|y| row_content(&buf, y))
            .find(|row| row.contains("Filters"))
            .expect("a row containing 'Filters' should be rendered");
        assert!(
            filters_row.contains("type to search"),
            "sidebar title must show a placeholder even with an empty query: {filters_row:?}"
        );
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
        app.tabs[0].mark_manager.toggle(0);
        app.tabs[0].mark_manager.toggle(2);
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
            .comment_manager
            .add("test comment".to_string(), vec![0, 1]);
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
        app.tabs[0].mark_manager.toggle(1);
        app.tabs[0].filter.show_marks_only = true;
        app.tabs[0].refresh_visible();
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_ui_confirm_restore_session() {
        let mut app = make_app(&[]).await;
        app.tabs[0].interaction.mode = Box::new(ConfirmRestoreSessionMode {
            files: std::sync::Arc::new(vec!["file.log".to_string()]),
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
        app.tabs[0].load_state = Some(super::super::FileLoadState {
            path: "/tmp/test.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 1000,
            on_complete: super::super::LoadContext::ReplaceInitialTab,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

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
            scan_highlight_mode: false,
            scan_group_fingerprint: vec![],
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
            scan_highlight_mode: false,
            scan_group_fingerprint: vec![],
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
            scan_highlight_mode: false,
            scan_group_fingerprint: vec![],
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
            scan_highlight_mode: false,
            scan_group_fingerprint: vec![],
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

    #[test]
    fn test_find_token_offset_simple() {
        assert_eq!(find_token_offset("abc def ghi", "abc"), Some(0));
        assert_eq!(find_token_offset("abc def ghi", "def"), Some(4));
        assert_eq!(find_token_offset("abc def ghi", "ghi"), Some(8));
    }

    #[test]
    fn test_find_token_offset_short_value_not_matched_inside_longer_token() {
        let rendered = "2024-01-15T10:00:00Z INFO  systemd myhost 1 daemon Started.";
        let pos = find_token_offset(rendered, "1").unwrap();
        let token = &rendered[pos..pos + 1];
        assert_eq!(token, "1");
        assert_eq!(rendered.as_bytes()[pos - 1], b' ');
        assert_eq!(rendered.as_bytes()[pos + 1], b' ');
    }

    #[test]
    fn test_find_token_offset_systemd_pid1_in_syslog_rfc5424() {
        let rendered =
            "2024-01-15T10:30:00.000000+01:00 INFO  systemd myhost 1 local3 Started network.";
        let pid_pos = find_token_offset(rendered, "1").unwrap();
        assert_eq!(&rendered[pid_pos..pid_pos + 1], "1");
        assert!(pid_pos > 0 && rendered.as_bytes()[pid_pos - 1] == b' ');
        assert!(pid_pos + 1 < rendered.len() && rendered.as_bytes()[pid_pos + 1] == b' ');
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
        assert_eq!(find_token_offset("hello world", "lo"), None);
    }

    #[test]
    fn test_find_token_offset_single_token_haystack() {
        assert_eq!(find_token_offset("only", "only"), Some(0));
    }

    #[test]
    fn test_find_token_offset_bsd_timestamp_with_spaces() {
        let rendered = "Mar  8 10:30:00 INFO  systemd";
        assert_eq!(find_token_offset(rendered, "Mar  8 10:30:00"), Some(0));
    }

    fn stable_hash(s: &str) -> usize {
        s.bytes().fold(5381usize, |acc, b| {
            acc.wrapping_mul(33).wrapping_add(b as usize)
        })
    }

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
        app.display.show_mode_bar = false;
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

        let terminal = make_terminal();
        (app, terminal)
    }

    #[tokio::test]
    async fn test_search_progress_bar_shown_in_hint_area() {
        let (mut app, mut terminal) = make_app_with_search(Some(0.5)).await;
        terminal.draw(|f| app.ui(f)).unwrap();

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

    #[tokio::test]
    async fn test_ui_viewport_fills_backward_after_filter_toggle() {
        let lines: Vec<String> = (0..50).map(|i| format!("line {i}")).collect();
        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let mut app = make_app(&line_refs).await;

        app.tabs[0].scroll.scroll_offset = 49;
        app.tabs[0].scroll.viewport_offset = 49;

        app.execute_command_str("include-filter line [012][0-9]$".to_string())
            .await;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        let vp = app.tabs[0].scroll.viewport_offset;
        let visible = app.tabs[0].filter.visible_indices.len();
        let visible_height = 23;
        assert!(
            vp + visible_height >= visible,
            "viewport_offset {vp} leaves blank rows: {visible} visible lines, height {visible_height}"
        );
    }

    #[tokio::test]
    async fn test_visible_height_reduced_when_input_bar_visible() {
        let lines: Vec<String> = (0..10).map(|i| format!("line {i}")).collect();
        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();

        let mut app_no_bar = make_app(&line_refs).await;
        app_no_bar.display.show_mode_bar = false;
        app_no_bar.tabs[0].display.show_mode_bar = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app_no_bar.ui(f)).unwrap();
        let height_without_bar = app_no_bar.tabs[0].scroll.visible_height;

        let mut app_with_bar = make_app(&line_refs).await;
        app_with_bar.display.show_mode_bar = false;
        app_with_bar.tabs[0].display.show_mode_bar = false;
        app_with_bar.tabs[0].interaction.mode = Box::new(SearchMode {
            input: String::new(),
            cursor: 0,
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

    async fn make_two_tab_app() -> App {
        let mut app = make_app(&["line 0", "line 1"]).await;
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
        app.display.show_mode_bar = false;
        app.tabs[0].display.show_mode_bar = false;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

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
        app.display.show_mode_bar = true;

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
        app.display.show_mode_bar = false;
        app.tabs[0].display.show_mode_bar = false;
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode::new(0));

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
        app.display.show_mode_bar = false;
        app.tabs[0].display.show_mode_bar = false;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let tab_row = row_content(&buf, 0);
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
        app.display.show_mode_bar = false;
        app.tabs[0].display.show_mode_bar = false;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

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

        let tab_bar_row = row_content(&buf, 0);
        assert!(
            tab_bar_row.contains('┌') && tab_bar_row.contains('┐'),
            "tab bar row should contain border corners, got: {:?}",
            tab_bar_row,
        );

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
        app.tabs[1].display.format =
            crate::parser::detect_format(&[br#"{"level":"INFO","msg":"hi"}"#])
                .map(std::sync::Arc::from);

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
        app.tabs[0].display.format =
            crate::parser::detect_format(&[br#"{"level":"INFO","msg":"hi"}"#])
                .map(std::sync::Arc::from);
        app.tabs[1].display.show_borders = true;
        app.tabs[1].display.format =
            crate::parser::detect_format(&[br#"{"level":"INFO","msg":"hi"}"#])
                .map(std::sync::Arc::from);

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

    #[test]
    fn test_merged_format_name_shared_when_all_sources_agree() {
        let a: Option<Arc<dyn crate::parser::LogFormatParser>> =
            Some(Arc::new(crate::parser::JournalctlParser::default()));
        let b: Option<Arc<dyn crate::parser::LogFormatParser>> =
            Some(Arc::new(crate::parser::JournalctlParser::default()));
        let merged = crate::ui::MergedState {
            source_tab_indices: vec![],
            source_parsers: vec![a, b],
            source_labels: vec![],
            source_line_counts: vec![],
            label_col_width: 0,
            stopped: true,
            building: None,
        };
        assert_eq!(merged_format_name(&merged), Some("journalctl".to_string()));
    }

    #[test]
    fn test_merged_format_name_none_when_sources_disagree() {
        let a: Option<Arc<dyn crate::parser::LogFormatParser>> =
            Some(Arc::new(crate::parser::JournalctlParser::default()));
        let b: Option<Arc<dyn crate::parser::LogFormatParser>> =
            Some(Arc::new(crate::parser::SyslogParser::default()));
        let merged = crate::ui::MergedState {
            source_tab_indices: vec![],
            source_parsers: vec![a, b],
            source_labels: vec![],
            source_line_counts: vec![],
            label_col_width: 0,
            stopped: true,
            building: None,
        };
        assert_eq!(merged_format_name(&merged), None);
    }

    #[test]
    fn test_merged_format_name_none_when_no_source_detected_a_format() {
        let merged = crate::ui::MergedState {
            source_tab_indices: vec![],
            source_parsers: vec![None, None],
            source_labels: vec![],
            source_line_counts: vec![],
            label_col_width: 0,
            stopped: true,
            building: None,
        };
        assert_eq!(merged_format_name(&merged), None);
    }

    #[test]
    fn test_merged_format_name_ignores_sources_with_no_detected_format() {
        // A source that didn't detect anything shouldn't block agreement
        // between the sources that did.
        let a: Option<Arc<dyn crate::parser::LogFormatParser>> =
            Some(Arc::new(crate::parser::JournalctlParser::default()));
        let merged = crate::ui::MergedState {
            source_tab_indices: vec![],
            source_parsers: vec![a, None],
            source_labels: vec![],
            source_line_counts: vec![],
            label_col_width: 0,
            stopped: true,
            building: None,
        };
        assert_eq!(merged_format_name(&merged), Some("journalctl".to_string()));
    }

    #[tokio::test]
    async fn test_merged_tab_shows_shared_format_name_in_tab_bar() {
        let mut app = make_app(&["2024-02-22T10:15:30+0000 hostA sshd[1]: hi"]).await;
        app.tabs[0].display.format = crate::parser::detect_format(&[
            b"2024-02-22T10:15:30+0000 hostA sshd[1]: hi".as_slice(),
        ])
        .map(Arc::from);
        app.open_merge_tab(vec![0]).await;
        app.tabs.last_mut().unwrap().display.show_borders = true;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let tab_row = row_content(&buf, 0);
        assert!(
            !tab_row.contains("[unknown format]"),
            "merged tab whose sole source has a detected format must not show \
             the misleading [unknown format], got: {:?}",
            tab_row,
        );
    }

    #[tokio::test]
    async fn test_logs_title_shows_filename_when_no_tab_bar() {
        let data: Vec<u8> = "line A\nline B\n".as_bytes().to_vec();
        let file_reader = FileReader::from_bytes(data);
        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, Some("/tmp/uniquename.log".to_string())).await;
        let mut app = App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
        .await;
        app.tabs[0].title = "uniquename.log".to_string();

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

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

        let row0 = row_content(&buf, 0);
        assert!(
            row0.contains("Filters"),
            "sidebar title should appear on row 0 (same as tab bar), got: {:?}",
            row0,
        );
        let row1 = row_content(&buf, 1);
        assert!(
            !row1.contains("Filters"),
            "sidebar title must not appear on row 1 (would mean misaligned), got: {:?}",
            row1,
        );
    }

    #[tokio::test]
    async fn test_sidebar_does_not_rescroll_when_selection_stays_in_view() {
        use crate::ui::widgets::sidebar::sidebar_inner_dims;

        let mut app = make_app(&["line 0", "line 1"]).await;
        app.tabs[0].display.show_borders = false;
        for i in 0..30 {
            app.execute_command_str(format!("filter pattern_{i}")).await;
        }
        let mut terminal = make_terminal();

        // Render once to learn the sidebar's actual on-screen dimensions,
        // then pick a selection near the bottom that forces a scroll.
        terminal.draw(|f| app.ui(f)).unwrap();
        let sidebar_area = app.input.sidebar_area.expect("sidebar should be visible");
        let (_content_w, content_h) =
            sidebar_inner_dims(sidebar_area, app.tabs[0].display.show_borders);
        let bottom = 29usize;
        let middle = bottom - content_h / 2;
        let top = bottom.saturating_sub(content_h * 2);

        app.tabs[0].interaction.mode = Box::new(FilterManagementMode::new(bottom));
        terminal.draw(|f| app.ui(f)).unwrap();
        let scroll_after_bottom = app.tabs[0].filter.sidebar_scroll;
        assert!(
            scroll_after_bottom > 0,
            "selecting the last filter should have scrolled the sidebar down"
        );

        // Moving the selection to a filter still inside the visible window
        // must not change the scroll offset.
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode::new(middle));
        terminal.draw(|f| app.ui(f)).unwrap();
        assert_eq!(
            app.tabs[0].filter.sidebar_scroll, scroll_after_bottom,
            "selection already visible must not trigger a rescroll"
        );

        // Moving the selection far outside the visible window must scroll.
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode::new(top));
        terminal.draw(|f| app.ui(f)).unwrap();
        assert_eq!(app.tabs[0].filter.sidebar_scroll, top);
    }

    #[tokio::test]
    async fn test_leaving_and_reentering_filter_mode_restores_selection_and_scroll() {
        use crate::mode::app_mode::ModeRenderState;
        use crate::mode::normal_mode::NormalMode;

        let mut app = make_app(&["line 0", "line 1"]).await;
        app.tabs[0].display.show_borders = false;
        for i in 0..30 {
            app.execute_command_str(format!("filter pattern_{i}")).await;
        }
        let mut terminal = make_terminal();

        // Select a filter far enough down to force the sidebar to scroll.
        app.tabs[0].interaction.mode = Box::new(FilterManagementMode::new(25));
        terminal.draw(|f| app.ui(f)).unwrap();
        let scroll_in_filter_mode = app.tabs[0].filter.sidebar_scroll;
        assert!(scroll_in_filter_mode > 0);

        // Leaving filter mode (Esc) must not reset the remembered position;
        // the sidebar keeps showing the same scroll while browsing.
        app.handle_key_event(crossterm::event::KeyCode::Esc).await;
        assert!(matches!(
            app.tabs[0].interaction.mode.render_state(),
            ModeRenderState::Normal
        ));
        terminal.draw(|f| app.ui(f)).unwrap();
        assert_eq!(app.tabs[0].filter.sidebar_scroll, scroll_in_filter_mode);

        // Re-entering filter mode restores the exact same selection.
        app.tabs[0].interaction.mode = Box::new(NormalMode::default());
        app.handle_key_event(crossterm::event::KeyCode::Char('f'))
            .await;
        match app.tabs[0].interaction.mode.render_state() {
            ModeRenderState::FilterManagement { selected_index, .. } => {
                assert_eq!(selected_index, 25);
            }
            other => panic!("expected FilterManagement, got {other:?}"),
        }
        terminal.draw(|f| app.ui(f)).unwrap();
        assert_eq!(app.tabs[0].filter.sidebar_scroll, scroll_in_filter_mode);
    }

    #[tokio::test]
    async fn test_inactive_tab_uses_inactive_tab_fg_color() {
        let mut app = make_two_tab_app().await;
        let expected_fg = app.theme.inactive_tab_fg;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

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
        use crate::filters::FilterType;

        let mut app = make_app(&["log GET /api"]).await;
        let get_color = app.theme.value_colors.http_get;

        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "log".to_string(),
                FilterType::Include,
                crate::filters::FilterOptions::default()
                    .fg("[255,0,0]")
                    .line_mode(),
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
        use crate::filters::FilterType;

        let mut app = make_app(&["log GET /api"]).await;
        let get_color = app.theme.value_colors.http_get;

        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "log".to_string(),
                FilterType::Include,
                crate::filters::FilterOptions::default().fg("[255,0,0]"),
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
        use crate::filters::FilterType;

        let mut app = make_app(&["log from 5.120.204.67 done"]).await;
        let ip_color = app.theme.value_colors.ip_address;

        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "5.120.204.67".to_string(),
                FilterType::Include,
                crate::filters::FilterOptions::default()
                    .fg("Black")
                    .bg("Salmon"),
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
        use crate::filters::FilterType;

        let mut app = make_app(&["log from 5.120.204.67 done"]).await;
        let ip_color = app.theme.value_colors.ip_address;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "5.120.204.67".to_string(),
                FilterType::Include,
                crate::filters::FilterOptions::default()
                    .fg("Black")
                    .bg("Salmon"),
            )
            .await;
        app.tabs[0].begin_filter_refresh();

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
        use crate::filters::FilterType;

        let mut app = make_app(&["log from 5.120.204.67 done"]).await;
        let ip_color = app.theme.value_colors.ip_address;

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "5.120.204.67".to_string(),
                FilterType::Include,
                crate::filters::FilterOptions::default()
                    .fg("Black")
                    .bg("Salmon"),
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
        use crate::filters::FilterType;

        let json_line = r#"{"level":"info","msg":"request from 5.120.204.67 done"}"#;
        let mut app = make_app(&[json_line]).await;
        let ip_color = app.theme.value_colors.ip_address;

        app.tabs[0]
            .log_manager
            .add_filter_with_color(
                "5.120.204.67".to_string(),
                FilterType::Include,
                crate::filters::FilterOptions::default()
                    .fg("Black")
                    .bg("Salmon"),
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

    #[tokio::test]
    async fn test_startup_warnings_shown_above_mode_bar() {
        let mut app = make_app(&["line 0"]).await;
        app.display.show_mode_bar = true;
        app.session.startup_warnings = vec![
            "keybinding conflict: j".to_string(),
            "keybinding conflict: k".to_string(),
        ];

        let mut terminal = make_terminal();
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

    #[tokio::test]
    async fn test_startup_warnings_capped_at_10_rows() {
        let mut app = make_app(&["line 0"]).await;
        app.session.startup_warnings = (0..15).map(|i| format!("conflict {i}")).collect();

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = (0..buf.area.height)
            .map(|y| row_content(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(content.contains("conflict 0"));
    }

    #[test]
    fn test_warning_rows_splits_multiline_warning_into_separate_rows() {
        let warnings = vec![
            "invalid custom schema 'x': regex parse error:\n    (abc\n    ^\nerror: unclosed group"
                .to_string(),
        ];
        let rows = warning_rows(&warnings);
        assert_eq!(
            rows,
            vec![
                (true, "invalid custom schema 'x': regex parse error:"),
                (false, "    (abc"),
                (false, "    ^"),
                (false, "error: unclosed group"),
            ]
        );
    }

    #[test]
    fn test_warning_rows_caps_total_rows_across_warnings() {
        let warnings: Vec<String> = (0..5)
            .map(|i| format!("line a {i}\nline b {i}\nline c {i}\nline d {i}\nline e {i}"))
            .collect();
        let rows = warning_rows(&warnings);
        assert_eq!(rows.len(), MAX_WARNING_ROWS);
    }

    #[tokio::test]
    async fn test_startup_warnings_multiline_message_renders_as_separate_rows() {
        let mut app = make_app(&["line 0"]).await;
        app.session.startup_warnings = vec![
            "invalid custom schema 'x': regex parse error:\n    ^\nerror: unclosed group"
                .to_string(),
        ];

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = (0..buf.area.height)
            .map(|y| row_content(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            content.contains("Warning: invalid custom schema 'x': regex parse error:"),
            "first line should carry the Warning label, got:\n{content}"
        );
        assert!(
            content.contains("error: unclosed group"),
            "continuation lines of a multi-line warning must still be visible, got:\n{content}"
        );
    }

    #[tokio::test]
    async fn test_cursor_line_has_bold_and_underlined() {
        let mut app = make_app(&["INFO first line", "ERROR second line"]).await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let cursor_row = 1u16;
        let has_bold = (0..buf.area.width).any(|x| {
            buf.cell((x, cursor_row))
                .is_some_and(|c| c.modifier.contains(Modifier::BOLD))
        });
        let has_underlined = (0..buf.area.width).any(|x| {
            buf.cell((x, cursor_row))
                .is_some_and(|c| c.modifier.contains(Modifier::UNDERLINED))
        });
        assert!(has_bold, "cursor line should have BOLD modifier");
        assert!(
            has_underlined,
            "cursor line should have UNDERLINED modifier"
        );
    }

    #[tokio::test]
    async fn test_cursor_line_has_no_cursor_bg() {
        let mut app = make_app(&["INFO first line", "ERROR second line"]).await;
        let cursor_bg = app.theme.cursor_bg;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let cursor_row = 1u16;
        let has_cursor_bg = (0..buf.area.width)
            .any(|x| buf.cell((x, cursor_row)).is_some_and(|c| c.bg == cursor_bg));
        assert!(!has_cursor_bg, "cursor line should not use cursor_bg color");
    }

    #[tokio::test]
    async fn test_startup_warnings_cleared_on_keypress() {
        let mut app = make_app(&["line 0"]).await;
        app.session.startup_warnings = vec!["conflict".to_string()];
        app.handle_key_event(crossterm::event::KeyCode::Esc).await;
        assert!(
            app.session.startup_warnings.is_empty(),
            "startup_warnings should be cleared after a keypress"
        );
    }
}
