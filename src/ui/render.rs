//! Main rendering pipeline: log panel, tab bar, sidebar, and command bar.
//!
//! [`App::ui`] is called every frame. Wrap-aware viewport math uses
//! [`line_row_count`] to keep the selected line on-screen. Each visible line
//! is parsed by the detected format parser, evaluated through the filter
//! pipeline, and post-processed by value-based coloring.

use std::collections::{HashMap, HashSet};

use ratatui::{
    Frame,
    prelude::*,
    style::Modifier,
    widgets::{
        Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

use crate::auto_complete::{
    complete_color, complete_file_path, extract_color_partial, find_command_completions,
};
use crate::commands::{FILE_PATH_COMMANDS, find_matching_command};
use crate::filters::{CURRENT_SEARCH_STYLE_ID, SEARCH_STYLE_ID, render_line};
use crate::theme::complete_theme;
use crate::types::{FilterType, LogLevel};
use crate::value_colors::colorize_known_values;

use crate::mode::app_mode::ModeRenderState;

use super::field_layout::{apply_field_layout, count_wrapped_lines, effective_row_count};
use super::{App, LoadContext};

impl App {
    pub(super) fn ui(&mut self, frame: &mut Frame) {
        let size = frame.area();
        frame.render_widget(Block::default().bg(self.theme.root_bg), size);

        let has_multiple_tabs = self.tabs.len() > 1;

        // Extract mode-derived state up front via a single render_state() call,
        // avoiding holding a borrow over the rest of rendering.
        let render_state = self.tabs[self.active_tab].mode.render_state();

        let persistent_pattern: Option<String> =
            if matches!(render_state, ModeRenderState::Normal) {
                self.tabs[self.active_tab]
                    .search
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
            _ => 0,
        };
        let keybindings = self.tabs[self.active_tab].keybindings.clone();
        let status_line = self.tabs[self.active_tab]
            .mode
            .mode_bar_content(&keybindings, &self.theme);
        let visual_anchor: Option<usize> = match &render_state {
            ModeRenderState::VisualLine { anchor } => Some(*anchor),
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
        let value_colors_state: Option<(
            Vec<crate::mode::value_colors_mode::ValueColorGroup>,
            String,
            usize,
        )> = match &render_state {
            ModeRenderState::ValueColors {
                groups,
                search,
                selected,
            } => Some((groups.clone(), search.clone(), *selected)),
            _ => None,
        };
        let confirm_open_dir: Option<(String, Vec<String>)> = match &render_state {
            ModeRenderState::ConfirmOpenDir { dir, files } => Some((dir.clone(), files.clone())),
            _ => None,
        };

        let show_mode_bar = self.tabs[self.active_tab].show_mode_bar;
        let show_borders = self.tabs[self.active_tab].show_borders;

        if is_confirm_restore {
            self.render_confirm_restore_modal(frame);
            return;
        }

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
            (content_lines + 2).clamp(3, 6) as u16
        } else {
            content_lines.clamp(1, 4) as u16
        };

        let mut constraints = vec![];
        if has_multiple_tabs {
            constraints.push(Constraint::Length(1)); // Tab bar
        }
        constraints.push(Constraint::Min(1)); // Main content
        if has_input_bar {
            constraints.push(Constraint::Length(1)); // input line
            let hint_height =
                self.compute_hint_height(&command_input, inner_width, completion_index);
            constraints.push(Constraint::Length(hint_height)); // hint line(s)
        }
        if show_mode_bar {
            if !show_borders {
                constraints.push(Constraint::Length(1)); // visual gap above mode bar
            }
            constraints.push(Constraint::Length(status_height)); // command list
        }
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(size);

        let mut chunk_idx = 0;

        self.render_tab_bar(frame, has_multiple_tabs, &chunks, &mut chunk_idx);

        let main_chunk = chunks[chunk_idx];
        chunk_idx += 1;

        let tab = &self.tabs[self.active_tab];

        let (logs_area, sidebar_area) = if tab.show_sidebar {
            if show_borders {
                let horizontal = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(1), Constraint::Length(30)])
                    .split(main_chunk);
                (horizontal[0], Some(horizontal[1]))
            } else {
                // Add a 1-column gap between logs and sidebar when borders are off.
                let horizontal = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Min(1),
                        Constraint::Length(1),
                        Constraint::Length(30),
                    ])
                    .split(main_chunk);
                (horizontal[0], Some(horizontal[2]))
            }
        } else {
            (main_chunk, None)
        };

        self.render_logs_panel(frame, logs_area, visual_anchor);

        self.render_side_bar(frame, selected_filter_idx, sidebar_area);

        self.render_command_bar(frame, command_input, completion_index, &chunks, chunk_idx);

        self.render_input_bar(frame, search_input, &chunks, chunk_idx);

        if show_mode_bar {
            let status_block = if show_borders {
                Block::default()
                    .borders(Borders::ALL)
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

        // Session restore modal renders on top of the full TUI so stdin content
        // is visible behind it.
        if let Some(files) = session_files {
            self.render_confirm_restore_session_modal(frame, &files);
        }

        // Open-directory confirmation popup.
        if let Some((dir, files)) = confirm_open_dir {
            self.render_confirm_open_dir_modal(frame, &dir, &files);
        }

        // Comment popup renders over everything except the loading bar.
        if let Some((lines, cursor_row, cursor_col, line_count)) = comment_popup {
            self.render_comment_popup(frame, &lines, cursor_row, cursor_col, line_count);
        }

        // Select-fields popup renders over everything except the loading bar.
        if let Some((fields, selected)) = select_fields_state {
            self.render_select_fields_popup(frame, &fields, selected);
        }

        // Docker container selection popup.
        if let Some((containers, selected, error)) = docker_select {
            self.render_docker_select_popup(frame, &containers, selected, error.as_deref());
        }

        // Value colors toggle popup.
        if let Some((groups, search, selected)) = value_colors_state {
            self.render_value_colors_popup(frame, &groups, &search, selected);
        }

        // Keybindings help popup renders over everything except the loading bar.
        if let Some((scroll, search)) = help_state {
            self.render_keybindings_help_popup(frame, &keybindings, scroll, &search);
        }

        // Loading status bar renders last, on top of everything.
        self.render_loading_status_bar(frame);
    }

    fn render_logs_panel(
        &mut self,
        frame: &mut Frame<'_>,
        logs_area: Rect,
        visual_anchor: Option<usize>,
    ) {
        let num_visible = self.tabs[self.active_tab].visible_indices.len();
        let show_borders = self.tabs[self.active_tab].show_borders;

        // When borders are on they consume 1 row/col on each side (2 total).
        // When borders are off we still reserve 1 col on the left for visual padding.
        // The block title always occupies 1 row (ratatui Block::inner subtracts 1 for
        // has_title_at_position(Top) even when Borders::NONE), so vertical cost is 1.
        let vertical_border = if show_borders { 2 } else { 1 };
        let horizontal_shrink = if show_borders { 2 } else { 1 };
        let visible_height = (logs_area.height as usize).saturating_sub(vertical_border);
        self.tabs[self.active_tab].visible_height = visible_height;

        let show_line_numbers = self.tabs[self.active_tab].show_line_numbers;
        let total_lines = self.tabs[self.active_tab].file_reader.line_count();
        let line_number_width = if show_line_numbers {
            total_lines.max(1).to_string().len()
        } else {
            0
        };

        let ln_prefix_width = if show_line_numbers {
            // "{number}{annot_marker}{space}" = line_number_width + 1 (marker) + 1 (space)
            line_number_width + 2
        } else {
            0
        };
        let inner_width =
            (logs_area.width as usize).saturating_sub(horizontal_shrink + ln_prefix_width);

        let wrap = self.tabs[self.active_tab].wrap;

        // Clone early so both the viewport row-count closure and the flat_map
        // rendering closure can use them without re-borrowing `self`.
        let hidden_fields = self.tabs[self.active_tab].hidden_fields.clone();
        let field_layout = self.tabs[self.active_tab].field_layout.clone();

        // Clamp scroll_offset
        if num_visible == 0 {
            self.tabs[self.active_tab].scroll_offset = 0;
        } else if self.tabs[self.active_tab].scroll_offset >= num_visible {
            self.tabs[self.active_tab].scroll_offset = num_visible - 1;
        }

        let scroll_offset = self.tabs[self.active_tab].scroll_offset;
        let viewport_offset = self.tabs[self.active_tab].viewport_offset;

        // Compute new_viewport and end in a scoped block so the shared borrow of
        // `self.tabs[active_tab]` (for detected_format) is released before the
        // mutable write to viewport_offset below.
        let (new_viewport, end) = {
            let tab = &self.tabs[self.active_tab];
            let parser = tab.detected_format.as_deref();
            // In wrap mode, use the structured-rendering width when a format is
            // detected: raw JSON/tracing bytes can be 3-5× wider than the rendered
            // columns, causing the viewport to show far fewer lines than it should.
            let row_count = |li: usize| -> usize {
                effective_row_count(
                    tab.file_reader.get_line(li),
                    inner_width,
                    parser,
                    &field_layout,
                    &hidden_fields,
                )
            };

            let new_viewport = if scroll_offset < viewport_offset {
                scroll_offset
            } else if wrap && inner_width > 0 && num_visible > 0 {
                let rows_used: usize = (viewport_offset..=scroll_offset)
                    .map(|i| row_count(tab.visible_indices[i]))
                    .sum();
                if rows_used > visible_height {
                    let mut rows = 0usize;
                    let mut new_vp = scroll_offset + 1;
                    loop {
                        if new_vp == 0 {
                            break;
                        }
                        new_vp -= 1;
                        let h = row_count(tab.visible_indices[new_vp]);
                        if rows + h > visible_height {
                            new_vp += 1;
                            break;
                        }
                        rows += h;
                        if new_vp == 0 {
                            break;
                        }
                    }
                    new_vp.min(scroll_offset)
                } else {
                    viewport_offset
                }
            } else if visible_height > 0 && scroll_offset >= viewport_offset + visible_height {
                scroll_offset - visible_height + 1
            } else {
                viewport_offset
            };

            let start = new_viewport;
            let end = if wrap && inner_width > 0 {
                let mut rows = 0usize;
                let mut e = start;
                while e < num_visible {
                    let h = row_count(tab.visible_indices[e]);
                    if rows + h > visible_height {
                        break;
                    }
                    rows += h;
                    e += 1;
                }
                if e == start && start < num_visible {
                    e = start + 1;
                }
                e
            } else {
                (start + visible_height).min(num_visible)
            };

            (new_viewport, end)
        };

        self.tabs[self.active_tab].viewport_offset = new_viewport;
        let start = new_viewport;

        let (filter_manager, mut styles) = self.tabs[self.active_tab]
            .log_manager
            .build_filter_manager();
        let search_style = Style::default()
            .fg(self.theme.search_fg)
            .bg(self.theme.text_highlight_fg);
        let current_search_style = Style::default()
            .fg(self.theme.text_highlight_fg)
            .bg(self.theme.search_fg);
        styles.resize(256, Style::default());
        styles[255] = search_style;
        styles[254] = current_search_style;

        let search_results = self.tabs[self.active_tab].search.get_results();
        // Pre-compute which line holds the current occurrence and which index within it.
        let current_search_info: Option<(usize, usize)> = if search_results.is_empty() {
            None
        } else {
            let ri = self.tabs[self.active_tab].search.get_current_match_index();
            Some((
                search_results[ri].line_idx,
                self.tabs[self.active_tab].search.get_current_occurrence_index(),
            ))
        };
        let search_map: HashMap<usize, &crate::types::SearchResult> =
            search_results.iter().map(|r| (r.line_idx, r)).collect();
        // Clone the compiled regex once so the JSON render path can re-match against
        // the rendered string (raw-byte positions from search_map don't map there).
        let search_regex = self.tabs[self.active_tab]
            .search
            .get_compiled_pattern()
            .cloned();

        let theme = &self.theme;
        let level_colors = self.tabs[self.active_tab].level_colors;
        let current_scroll = self.tabs[self.active_tab].scroll_offset;
        // Pre-compute visual selection range (indices into visible_indices space).
        let visual_range: Option<(usize, usize)> = visual_anchor.map(|anchor| {
            let lo = anchor.min(current_scroll);
            let hi = anchor.max(current_scroll);
            (lo, hi)
        });
        // Visual selection highlight colour (same as border bg, distinct from cursor).
        let visual_style = Style::default()
            .fg(theme.visual_select_fg)
            .bg(theme.visual_select_bg);

        // Clone comment data before borrowing visible_indices for iteration.
        let comments_for_render: Vec<(Vec<usize>, String)> = self.tabs[self.active_tab]
            .log_manager
            .get_comments()
            .iter()
            .map(|a| (a.line_indices.clone(), a.text.clone()))
            .collect();

        // Two maps built in one pass over comments × visible window:
        //   banner_at:         abs_vis_idx → cmt_idx  (where a banner header is injected)
        //   vis_comment_map: abs_vis_idx → cmt_idx  (every visible commented line)
        // The latter drives the tree characters (│ / └) on log lines.
        let mut banner_at: HashMap<usize, usize> = HashMap::new();
        let mut vis_comment_map: HashMap<usize, usize> = HashMap::new();
        for (cmt_idx, (line_indices, _)) in comments_for_render.iter().enumerate() {
            let ann_set: HashSet<usize> = line_indices.iter().cloned().collect();
            let mut first_for_ann: Option<usize> = None;
            for abs_vi in start..end {
                let li = self.tabs[self.active_tab].visible_indices[abs_vi];
                if ann_set.contains(&li) {
                    // First comment wins when a line belongs to multiple groups.
                    vis_comment_map.entry(abs_vi).or_insert(cmt_idx);
                    if first_for_ann.is_none() {
                        first_for_ann = Some(abs_vi);
                        banner_at.insert(abs_vi, cmt_idx);
                    }
                }
            }
        }

        // Comment banner styles.
        let banner_prefix_style = Style::default()
            .fg(theme.text_highlight_fg)
            .add_modifier(Modifier::BOLD);
        let banner_text_style = Style::default().fg(theme.text);

        let log_lines: Vec<Line> = self.tabs[self.active_tab].visible_indices[start..end]
            .iter()
            .enumerate()
            .flat_map(|(vis_idx, &line_idx)| {
                let abs_vis_idx = start + vis_idx;
                let line_bytes = self.tabs[self.active_tab].file_reader.get_line(line_idx);
                let is_current = abs_vis_idx == current_scroll;
                let is_marked = self.tabs[self.active_tab].log_manager.is_marked(line_idx);
                let is_visual_selected = visual_range
                    .map(|(lo, hi)| abs_vis_idx >= lo && abs_vis_idx <= hi)
                    .unwrap_or(false);

                let mut base_style = Style::default().fg(theme.text);
                if level_colors {
                    match LogLevel::detect_from_bytes(line_bytes) {
                        LogLevel::Trace => base_style = base_style.fg(theme.trace_fg),
                        LogLevel::Debug => base_style = base_style.fg(theme.debug_fg),
                        LogLevel::Notice => base_style = base_style.fg(theme.notice_fg),
                        LogLevel::Warning => base_style = base_style.fg(theme.warning_fg),
                        LogLevel::Error => base_style = base_style.fg(theme.error_fg),
                        LogLevel::Fatal => base_style = base_style.fg(theme.fatal_fg),
                        _ => {}
                    }
                }
                if is_marked {
                    base_style = base_style.fg(theme.mark_fg).bg(theme.mark_bg);
                }
                if is_visual_selected {
                    base_style = visual_style;
                }

                let render_style = if is_current {
                    Style::default().fg(theme.cursor_fg).bg(theme.border)
                } else {
                    base_style
                };

                // For structured lines, render columns and run filter evaluation
                // against the rendered string so match-only highlights apply correctly.
                //   timestamp  level  target  span_name: k=v, k=v  extra=val  message
                // Known-field values are shown without their key names. Unknown fields
                // and span context are rendered as key=value before the message.
                // Filter visibility decisions still use the raw bytes (unaffected).
                let structured_line: Option<Line<'static>> = self.tabs[self.active_tab]
                    .detected_format
                    .as_ref()
                    .and_then(|parser| parser.parse_line(line_bytes))
                    .map(|parts| {
                        let cols = apply_field_layout(&parts, &field_layout, &hidden_fields);

                        // Determine which occurrence index (if any) is current for this line.
                        let current_occ = current_search_info
                            .and_then(|(cl, co)| if cl == line_idx { Some(co) } else { None });

                        if cols.is_empty() {
                            // All fields hidden — fall back to raw bytes with filter +
                            // search highlighting (raw-byte positions are correct here).
                            let mut collector = filter_manager.evaluate_line(line_bytes);
                            if let Some(sr) = search_map.get(&line_idx) {
                                collector.with_priority(1000);
                                for (i, &(s, e)) in sr.matches.iter().enumerate() {
                                    let sid = if current_occ == Some(i) {
                                        CURRENT_SEARCH_STYLE_ID
                                    } else {
                                        SEARCH_STYLE_ID
                                    };
                                    collector.push(s, e, sid);
                                }
                            }
                            render_line(&collector, &styles)
                        } else {
                            // Evaluate filters AND search against the rendered string so
                            // all spans land at the correct visible positions.
                            let rendered = cols.join(" ");
                            let mut collector = filter_manager.evaluate_line(rendered.as_bytes());
                            if let Some(ref regex) = search_regex {
                                collector.with_priority(1000);
                                for (i, m) in regex.find_iter(&rendered).enumerate() {
                                    let sid = if current_occ == Some(i) {
                                        CURRENT_SEARCH_STYLE_ID
                                    } else {
                                        SEARCH_STYLE_ID
                                    };
                                    collector.push(m.start(), m.end(), sid);
                                }
                            }
                            render_line(&collector, &styles)
                        }
                    });

                // Determine which occurrence index (if any) is current for this line.
                let current_occ = current_search_info
                    .and_then(|(cl, co)| if cl == line_idx { Some(co) } else { None });

                let mut line = if let Some(structured_line) = structured_line {
                    structured_line
                } else {
                    let mut collector = filter_manager.evaluate_line(line_bytes);
                    if let Some(sr) = search_map.get(&line_idx) {
                        collector.with_priority(1000);
                        for (i, &(s, e)) in sr.matches.iter().enumerate() {
                            let sid = if current_occ == Some(i) {
                                CURRENT_SEARCH_STYLE_ID
                            } else {
                                SEARCH_STYLE_ID
                            };
                            collector.push(s, e, sid);
                        }
                    }
                    render_line(&collector, &styles)
                };
                line = colorize_known_values(line, &theme.value_colors);
                // Use line-level base style so per-span highlights (search, filters) are
                // preserved on the cursor line. Spans with explicit fg/bg override the base.
                line = line.style(render_style);

                if show_line_numbers {
                    let line_num = line_idx + 1;
                    // Tree character: │ for mid-group lines, └ for the last line of a group,
                    // space for non-commented lines.
                    let (tree_char, ln_fg) = if let Some(&cmt_idx) =
                        vis_comment_map.get(&abs_vis_idx)
                    {
                        let next_same = vis_comment_map.get(&(abs_vis_idx + 1)) == Some(&cmt_idx);
                        let ch = if next_same { "│" } else { "└" };
                        (ch, theme.text_highlight_fg)
                    } else {
                        (" ", theme.border)
                    };
                    // Format: {tree_char}{line_num right-aligned}{space}
                    // Total width = 1 + line_number_width + 1 = ln_prefix_width ✓
                    let line_num_str = format!(
                        "{}{:>width$} ",
                        tree_char,
                        line_num,
                        width = line_number_width
                    );
                    let line_num_style = Style::default().fg(ln_fg).add_modifier(Modifier::DIM);
                    let mut all_spans = vec![Span::styled(line_num_str, line_num_style)];
                    // Extra indent padding for lines nested under a comment banner.
                    if vis_comment_map.contains_key(&abs_vis_idx) {
                        all_spans.push(Span::raw("  "));
                    }
                    all_spans.extend(line.spans);
                    line = Line::from(all_spans).style(render_style);
                }

                // Optionally prepend a comment banner before the first commented line in view.
                // Tree-prefix strings are ln_prefix_width wide so comment text aligns with
                // log content:  "├" + "─"*(w-2) + " "  and  "│" + " "*(w-2) + " "
                let mut result: Vec<Line> = Vec::new();
                if let Some(&cmt_idx) = banner_at.get(&abs_vis_idx) {
                    let (_, text) = &comments_for_render[cmt_idx];
                    let (first_prefix, cont_prefix) = if show_line_numbers && ln_prefix_width >= 2 {
                        (
                            format!("├{} ", "─".repeat(ln_prefix_width - 2)),
                            format!("│{} ", " ".repeat(ln_prefix_width - 2)),
                        )
                    } else {
                        ("├── ".to_string(), "│   ".to_string())
                    };
                    for (i, text_line) in text.lines().enumerate() {
                        let (prefix, p_style) = if i == 0 {
                            (first_prefix.clone(), banner_prefix_style)
                        } else {
                            (cont_prefix.clone(), banner_text_style)
                        };
                        let spans = vec![
                            Span::styled(prefix, p_style),
                            Span::styled(text_line.to_string(), banner_text_style),
                        ];
                        result.push(Line::from(spans).style(banner_text_style));
                    }
                }
                result.push(line);
                result
            })
            .collect();

        let tail_mode = self.tabs[self.active_tab].tail_mode;
        let logs_title = format!(
            "{} ({}){}",
            self.tabs[self.active_tab]
                .log_manager
                .source_file()
                .map(|s| {
                    std::path::Path::new(s)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(s)
                        .to_string()
                })
                .unwrap_or(String::from("Logs")),
            num_visible,
            if tail_mode { " [TAIL]" } else { "" }
        );

        let logs_block = if show_borders {
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(self.theme.border))
                .title(logs_title)
                .title_style(Style::default().fg(self.theme.border_title))
        } else {
            Block::default()
                .borders(Borders::NONE)
                .padding(Padding::new(1, 0, 0, 0))
                .title(logs_title)
                .title_style(Style::default().fg(self.theme.border_title))
        };
        let mut paragraph = Paragraph::new(log_lines)
            .block(logs_block)
            .scroll((0, self.tabs[self.active_tab].horizontal_scroll as u16));

        if self.tabs[self.active_tab].wrap {
            paragraph = paragraph.wrap(Wrap { trim: false });
        }

        frame.render_widget(paragraph, logs_area);

        if num_visible > 0 {
            // content_length = max_scroll ensures position/content_length == 1.0
            // when at the last entry, so the thumb reaches the bottom of the track.
            let max_scroll = num_visible.saturating_sub(visible_height);
            let mut scrollbar_state = ScrollbarState::new(max_scroll.max(1))
                .position(start);
            frame.render_stateful_widget(
                Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight),
                logs_area,
                &mut scrollbar_state,
            );
        }
    }

    fn render_input_bar(
        &mut self,
        frame: &mut Frame<'_>,
        search_input: Option<(String, bool, bool)>,
        chunks: &std::rc::Rc<[Rect]>,
        chunk_idx: usize,
    ) {
        if let Some((input_str, forward, is_active)) = search_input {
            let prefix = if forward { "/" } else { "?" };
            let search_line = Paragraph::new(format!("{}{}", prefix, input_str))
                .style(Style::default().fg(self.theme.cursor_fg).bg(self.theme.border))
                .wrap(Wrap { trim: false });
            let input_area = chunks[chunk_idx];
            frame.render_widget(search_line, input_area);
            if is_active {
                let cursor_x = input_area.x + 1 + input_str.len() as u16;
                if cursor_x < input_area.x + input_area.width {
                    frame.set_cursor_position((cursor_x, input_area.y));
                }
            }

            let hint_area = chunks[chunk_idx + 1];
            let total = self.tabs[self.active_tab].search.get_total_match_count();
            let hint_text = if !input_str.is_empty() {
                if is_active {
                    format!("  {} matches", total)
                } else if total == 0 {
                    "  no matches".to_string()
                } else {
                    let current = self.tabs[self.active_tab].search.get_current_occurrence_number();
                    format!("  match {} / {}", current, total)
                }
            } else {
                "  Type pattern and press Enter to search".to_string()
            };
            let hint = Paragraph::new(hint_text).style(
                Style::default()
                    .fg(self.theme.border)
                    .bg(self.theme.root_bg),
            );
            frame.render_widget(hint, hint_area);
        }
    }

    fn render_loading_status_bar(&mut self, frame: &mut Frame<'_>) {
        let s = match self.file_load_state.as_ref() {
            Some(s) => s,
            None => return,
        };
        let progress = *s.progress_rx.borrow();
        let subtitle = match &s.on_complete {
            LoadContext::SessionRestoreTab {
                remaining, total, ..
            } => {
                let current = total - remaining.len();
                format!(
                    "({}/{}) {}",
                    current,
                    total,
                    std::path::Path::new(&s.path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&s.path)
                )
            }
            LoadContext::ReplaceInitialTab => std::path::Path::new(&s.path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&s.path)
                .to_string(),
        };

        let bar_width = 20_usize;
        let filled = ((progress * bar_width as f64) as usize).min(bar_width);
        let bar = format!(
            "{}{}",
            "\u{2588}".repeat(filled),
            "\u{2591}".repeat(bar_width - filled),
        );
        let pct = (progress * 100.0) as usize;
        let text = format!(" Loading {}  {} {}% ", subtitle, bar, pct);

        let area = frame.area();
        if area.height == 0 {
            return;
        }
        let bar_rect = ratatui::layout::Rect::new(
            area.x,
            area.y + area.height.saturating_sub(1),
            area.width,
            1,
        );
        frame.render_widget(ratatui::widgets::Clear, bar_rect);
        frame.render_widget(
            Paragraph::new(text).style(
                Style::default()
                    .fg(self.theme.root_bg)
                    .bg(self.theme.text_highlight_fg),
            ),
            bar_rect,
        );
    }

    /// Compute how many rows the command-mode hint area needs (1–3).
    fn compute_hint_height(
        &self,
        command_input: &Option<(String, usize)>,
        width: usize,
        completion_index: Option<usize>,
    ) -> u16 {
        let text = match command_input {
            Some((input_text, _)) => {
                if let Some(err) = &self.tabs[self.active_tab].command_error {
                    err.clone()
                } else if let Some(partial) = extract_color_partial(input_text) {
                    let completions = complete_color(partial);
                    completions
                        .iter()
                        .map(|n| format!(" {} ", n))
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    let trimmed = input_text.trim();
                    let file_cmd = FILE_PATH_COMMANDS
                        .iter()
                        .find(|cmd| trimmed.starts_with(&format!("{} ", cmd)));

                    if let Some(&cmd) = file_cmd {
                        let partial = trimmed[cmd.len()..].trim_start();
                        let completions = complete_file_path(partial);
                        completions
                            .iter()
                            .map(|c| {
                                std::path::Path::new(c)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .map(|n| {
                                        if c.ends_with('/') {
                                            format!("{}/", n.trim_end_matches('/'))
                                        } else {
                                            n.to_string()
                                        }
                                    })
                                    .unwrap_or_else(|| c.clone())
                            })
                            .collect::<Vec<_>>()
                            .join("  ")
                    } else if let Some(partial_raw) =
                        input_text.trim_start().strip_prefix("set-theme ")
                    {
                        let partial = partial_raw.trim_start();
                        complete_theme(partial).join("  ")
                    } else if completion_index.is_none() {
                        if let Some(cmd) = find_matching_command(input_text) {
                            format!("  {} - {}", cmd.usage, cmd.description)
                        } else {
                            find_command_completions(trimmed).join("  ")
                        }
                    } else {
                        find_command_completions(trimmed).join("  ")
                    }
                }
            }
            None => String::new(),
        };
        if text.is_empty() {
            return 1;
        }
        (count_wrapped_lines(&text, width) as u16).clamp(1, 3)
    }

    fn render_command_bar(
        &mut self,
        frame: &mut Frame<'_>,
        command_input: Option<(String, usize)>,
        completion_index: Option<usize>,
        chunks: &std::rc::Rc<[Rect]>,
        chunk_idx: usize,
    ) {
        if let Some((input_text, cursor_pos)) = command_input {
            let input_prefix = ":";
            let command_line = Paragraph::new(format!("{}{}", input_prefix, input_text))
                .style(Style::default().fg(self.theme.cursor_fg).bg(self.theme.border))
                .wrap(Wrap { trim: false });
            let input_area = chunks[chunk_idx];
            frame.render_widget(command_line, input_area);
            let cursor_x = input_area.x + 1 + cursor_pos as u16;
            if cursor_x < input_area.x + input_area.width {
                frame.set_cursor_position((cursor_x, input_area.y));
            }

            let hint_area = chunks[chunk_idx + 1];
            let normal_style = Style::default()
                .fg(self.theme.border)
                .bg(self.theme.root_bg);
            let highlight_style = Style::default()
                .fg(self.theme.root_bg)
                .bg(self.theme.border);

            if let Some(err) = &self.tabs[self.active_tab].command_error {
                let error_paragraph = Paragraph::new(err.as_str())
                    .style(Style::default().fg(Color::Red).bg(self.theme.root_bg))
                    .wrap(Wrap { trim: false });
                frame.render_widget(error_paragraph, hint_area);
            } else if let Some(partial) = extract_color_partial(&input_text) {
                let completions = complete_color(partial);
                if !completions.is_empty() {
                    let hint_spans: Vec<Span> = completions
                        .iter()
                        .enumerate()
                        .flat_map(|(i, name)| {
                            let color = name.parse::<Color>().unwrap_or(Color::White);
                            let style = if completion_index == Some(i) {
                                Style::default().fg(color).bg(self.theme.border)
                            } else {
                                Style::default().fg(color).bg(self.theme.root_bg)
                            };
                            vec![Span::styled(format!(" {} ", name), style), Span::raw(" ")]
                        })
                        .collect();
                    let hint = Paragraph::new(Line::from(hint_spans))
                        .style(Style::default().bg(self.theme.root_bg))
                        .wrap(Wrap { trim: false });
                    frame.render_widget(hint, hint_area);
                }
            } else {
                let trimmed_input = input_text.trim();
                let file_cmd = FILE_PATH_COMMANDS
                    .iter()
                    .find(|cmd| trimmed_input.starts_with(&format!("{} ", cmd)));

                if let Some(&cmd) = file_cmd {
                    let partial = trimmed_input[cmd.len()..].trim_start();
                    let completions = complete_file_path(partial);
                    if !completions.is_empty() {
                        let hint_spans: Vec<Span> = completions
                            .iter()
                            .enumerate()
                            .flat_map(|(i, c)| {
                                let display = std::path::Path::new(c)
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .map(|n| {
                                        if c.ends_with('/') {
                                            format!("{}/", n.trim_end_matches('/'))
                                        } else {
                                            n.to_string()
                                        }
                                    })
                                    .unwrap_or_else(|| c.clone());
                                let style = if completion_index == Some(i) {
                                    highlight_style
                                } else {
                                    normal_style
                                };
                                vec![
                                    Span::styled(format!(" {} ", display), style),
                                    Span::raw(" "),
                                ]
                            })
                            .collect();
                        let hint = Paragraph::new(Line::from(hint_spans))
                            .style(Style::default().bg(self.theme.root_bg))
                            .wrap(Wrap { trim: false });
                        frame.render_widget(hint, hint_area);
                    }
                } else if let Some(partial_raw) = input_text.trim_start().strip_prefix("set-theme ")
                {
                    let partial = partial_raw.trim_start();
                    let completions = complete_theme(partial);
                    if !completions.is_empty() {
                        let hint_spans: Vec<Span> = completions
                            .iter()
                            .enumerate()
                            .flat_map(|(i, name)| {
                                let style = if completion_index == Some(i) {
                                    highlight_style
                                } else {
                                    normal_style
                                };
                                vec![Span::styled(format!(" {} ", name), style), Span::raw(" ")]
                            })
                            .collect();
                        let hint = Paragraph::new(Line::from(hint_spans))
                            .style(Style::default().bg(self.theme.root_bg))
                            .wrap(Wrap { trim: false });
                        frame.render_widget(hint, hint_area);
                    }
                } else if completion_index.is_none() {
                    if let Some(cmd) = find_matching_command(&input_text) {
                        let hint = Paragraph::new(format!("  {} - {}", cmd.usage, cmd.description))
                            .style(normal_style)
                            .wrap(Wrap { trim: false });
                        frame.render_widget(hint, hint_area);
                    } else {
                        self.render_command_completions(
                            frame,
                            &input_text,
                            completion_index,
                            hint_area,
                            normal_style,
                            highlight_style,
                        );
                    }
                } else {
                    self.render_command_completions(
                        frame,
                        &input_text,
                        completion_index,
                        hint_area,
                        normal_style,
                        highlight_style,
                    );
                }
            }
        }
    }

    fn render_command_completions(
        &self,
        frame: &mut Frame<'_>,
        input_text: &str,
        completion_index: Option<usize>,
        hint_area: Rect,
        normal_style: Style,
        highlight_style: Style,
    ) {
        let completions = find_command_completions(input_text.trim());
        if !completions.is_empty() {
            let hint_spans: Vec<Span> = completions
                .iter()
                .enumerate()
                .flat_map(|(i, name)| {
                    let style = if completion_index == Some(i) {
                        highlight_style
                    } else {
                        normal_style
                    };
                    vec![Span::styled(format!(" {} ", name), style), Span::raw(" ")]
                })
                .collect();
            let hint = Paragraph::new(Line::from(hint_spans))
                .style(Style::default().bg(self.theme.root_bg))
                .wrap(Wrap { trim: false });
            frame.render_widget(hint, hint_area);
        }
    }

    fn render_side_bar(
        &mut self,
        frame: &mut Frame<'_>,
        selected_filter_idx: usize,
        sidebar_area: Option<Rect>,
    ) {
        if let Some(sidebar_area) = sidebar_area {
            let show_borders = self.tabs[self.active_tab].show_borders;
            let filters = self.tabs[self.active_tab].log_manager.get_filters();
            let filters_text: Vec<Line> = filters
                .iter()
                .enumerate()
                .map(|(i, filter)| {
                    let status = if filter.enabled { "[x]" } else { "[ ]" };
                    let selected_prefix = if i == selected_filter_idx { ">" } else { " " };
                    let is_date = filter.pattern.starts_with(crate::date_filter::DATE_PREFIX);
                    let filter_type_str = if is_date {
                        "Date"
                    } else {
                        match filter.filter_type {
                            FilterType::Include => "In",
                            FilterType::Exclude => "Out",
                        }
                    };
                    let display_pattern = if is_date {
                        &filter.pattern[crate::date_filter::DATE_PREFIX.len()..]
                    } else {
                        &filter.pattern
                    };
                    let mut style = Style::default().fg(self.theme.text);
                    if let Some(cfg) = &filter.color_config {
                        if let Some(fg) = cfg.fg {
                            style = style.fg(fg);
                        }
                        if let Some(bg) = cfg.bg {
                            style = style.bg(bg);
                        }
                    }
                    Line::from(format!(
                        "{}{} {}: {}",
                        selected_prefix, status, filter_type_str, display_pattern
                    ))
                    .style(style)
                })
                .collect();

            let sidebar_title = if self.tabs[self.active_tab].show_marks_only {
                "Filters [MARKS ONLY]"
            } else if self.tabs[self.active_tab].filtering_enabled {
                "Filters"
            } else {
                "Filters [OFF]"
            };
            let sidebar_block = if show_borders {
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border))
                    .title(sidebar_title)
                    .title_style(Style::default().fg(self.theme.border_title))
            } else {
                Block::default()
                    .borders(Borders::NONE)
                    .padding(Padding::new(1, 0, 0, 0))
                    .title(sidebar_title)
                    .title_style(Style::default().fg(self.theme.border_title))
            };
            let sidebar = Paragraph::new(filters_text).block(sidebar_block);
            frame.render_widget(sidebar, sidebar_area);
        }
    }

    fn render_tab_bar(
        &mut self,
        frame: &mut Frame<'_>,
        has_multiple_tabs: bool,
        chunks: &std::rc::Rc<[Rect]>,
        chunk_idx: &mut usize,
    ) {
        if has_multiple_tabs {
            let tab_bar_area = chunks[*chunk_idx];
            *chunk_idx += 1;

            let tab_spans: Vec<Span> = self
                .tabs
                .iter()
                .enumerate()
                .flat_map(|(i, t)| {
                    let is_active = i == self.active_tab;
                    let label = format!(" {} ", t.title);
                    let style = if is_active {
                        Style::default()
                            .fg(self.theme.text)
                            .bg(self.theme.text_highlight_bg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(self.theme.border)
                            .bg(self.theme.root_bg)
                    };
                    vec![
                        Span::styled(label, style),
                        Span::styled(" ", Style::default().bg(self.theme.root_bg)),
                    ]
                })
                .collect();

            let tab_bar = Paragraph::new(Line::from(tab_spans))
                .style(Style::default().bg(self.theme.root_bg));
            frame.render_widget(tab_bar, tab_bar_area);
        }
    }
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
        )
        .await
    }

    fn make_terminal() -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(80, 24)).unwrap()
    }

    // 1. Basic normal mode rendering with 10 lines
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

    // 2. Sidebar hidden
    #[tokio::test]
    async fn test_ui_no_sidebar() {
        let mut app = make_app(&["line A", "line B", "line C"]).await;
        app.tabs[0].show_sidebar = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 3. Command mode rendering
    #[tokio::test]
    async fn test_ui_command_mode() {
        let mut app = make_app(&["log line"]).await;
        app.tabs[0].mode = Box::new(CommandMode::with_history("filter ".to_string(), 7, vec![]));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 4. Command mode with error
    #[tokio::test]
    async fn test_ui_command_mode_error() {
        let mut app = make_app(&["log line"]).await;
        app.tabs[0].command_error = Some("test error".to_string());
        app.tabs[0].mode = Box::new(CommandMode::with_history("bad-cmd".to_string(), 7, vec![]));
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 5. Command mode with completion index
    #[tokio::test]
    async fn test_ui_command_mode_completion_index() {
        let mut app = make_app(&["log line"]).await;
        app.tabs[0].mode = Box::new(CommandMode {
            input: "fil".to_string(),
            cursor: 3,
            history: vec![],
            history_index: None,
            completion_index: Some(0),
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 6. Search mode forward
    #[tokio::test]
    async fn test_ui_search_mode_forward() {
        let mut app = make_app(&["hello world", "test line"]).await;
        app.tabs[0].mode = Box::new(SearchMode {
            input: "test".to_string(),
            forward: true,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 7. Search mode backward
    #[tokio::test]
    async fn test_ui_search_mode_backward() {
        let mut app = make_app(&["hello world", "test line"]).await;
        app.tabs[0].mode = Box::new(SearchMode {
            input: "test".to_string(),
            forward: false,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 8. Search mode with empty input
    #[tokio::test]
    async fn test_ui_search_mode_empty() {
        let mut app = make_app(&["hello world"]).await;
        app.tabs[0].mode = Box::new(SearchMode {
            input: String::new(),
            forward: true,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 9. Filter management mode with filters
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
        app.tabs[0].mode = Box::new(FilterManagementMode {
            selected_filter_index: 0,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 10. Visual line mode
    #[tokio::test]
    async fn test_ui_visual_line_mode() {
        let mut app = make_app(&["line 0", "line 1", "line 2"]).await;
        app.tabs[0].mode = Box::new(VisualLineMode {
            anchor: 0,
            count: None,
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 11. Lines with marks
    #[tokio::test]
    async fn test_ui_with_marks() {
        let mut app = make_app(&["line 0", "line 1", "line 2", "line 3"]).await;
        app.tabs[0].log_manager.toggle_mark(0);
        app.tabs[0].log_manager.toggle_mark(2);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 12. Level colors enabled (default)
    #[tokio::test]
    async fn test_ui_level_colors() {
        let mut app = make_app(&[
            "INFO something happened",
            "WARN warning message",
            "ERROR error occurred",
        ])
        .await;
        assert!(app.tabs[0].level_colors);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 13. Level colors disabled
    #[tokio::test]
    async fn test_ui_no_level_colors() {
        let mut app = make_app(&[
            "INFO something happened",
            "WARN warning message",
            "ERROR error occurred",
        ])
        .await;
        app.tabs[0].level_colors = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 14. Line numbers shown (default)
    #[tokio::test]
    async fn test_ui_with_line_numbers() {
        let mut app = make_app(&["line A", "line B"]).await;
        assert!(app.tabs[0].show_line_numbers);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 15. Line numbers hidden
    #[tokio::test]
    async fn test_ui_without_line_numbers() {
        let mut app = make_app(&["line A", "line B"]).await;
        app.tabs[0].show_line_numbers = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 16. Lines with comments
    #[tokio::test]
    async fn test_ui_with_comments() {
        let mut app = make_app(&["line 0", "line 1", "line 2"]).await;
        app.tabs[0]
            .log_manager
            .add_comment("test comment".to_string(), vec![0, 1]);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 17. Wrap enabled (default) with long lines
    #[tokio::test]
    async fn test_ui_wrap_enabled() {
        let long_line = "A".repeat(200);
        let mut app = make_app(&[&long_line, "short"]).await;
        assert!(app.tabs[0].wrap);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 18. Wrap disabled
    #[tokio::test]
    async fn test_ui_wrap_disabled() {
        let long_line = "B".repeat(200);
        let mut app = make_app(&[&long_line, "short"]).await;
        app.tabs[0].wrap = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 19. Horizontal scroll with wrap disabled
    #[tokio::test]
    async fn test_ui_horizontal_scroll() {
        let long_line = "C".repeat(200);
        let mut app = make_app(&[&long_line]).await;
        app.tabs[0].wrap = false;
        app.tabs[0].horizontal_scroll = 10;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 20. Empty file
    #[tokio::test]
    async fn test_ui_empty_file() {
        let mut app = make_app(&[]).await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 21. JSON structured format auto-detection
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

    // 22. Structured format with all fields hidden
    #[tokio::test]
    async fn test_ui_structured_all_hidden() {
        let mut app = make_app(&[
            r#"{"level":"INFO","msg":"hello"}"#,
            r#"{"level":"WARN","msg":"world"}"#,
        ])
        .await;
        app.tabs[0].hidden_fields.insert("level".to_string());
        app.tabs[0].hidden_fields.insert("msg".to_string());
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 23. Multiple tabs
    #[tokio::test]
    async fn test_ui_multiple_tabs() {
        let mut app = make_app(&["tab1 line"]).await;
        let data2: Vec<u8> = "second tab line\n".as_bytes().to_vec();
        let file_reader2 = FileReader::from_bytes(data2);
        let log_manager2 = LogManager::new(app.db.clone(), None).await;
        let mut tab2 = super::super::TabState::new(file_reader2, log_manager2, "tab2".to_string());
        tab2.keybindings = app.keybindings.clone();
        app.tabs.push(tab2);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 24. Filtering disabled
    #[tokio::test]
    async fn test_ui_filtering_disabled() {
        let mut app = make_app(&["line 0", "line 1"]).await;
        app.tabs[0].filtering_enabled = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 25. Marks-only mode
    #[tokio::test]
    async fn test_ui_marks_only() {
        let mut app = make_app(&["line 0", "line 1", "line 2"]).await;
        app.tabs[0].log_manager.toggle_mark(1);
        app.tabs[0].show_marks_only = true;
        app.tabs[0].refresh_visible();
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 26. Confirm restore session mode
    #[tokio::test]
    async fn test_ui_confirm_restore_session() {
        let mut app = make_app(&[]).await;
        app.tabs[0].mode = Box::new(ConfirmRestoreSessionMode {
            files: vec!["file.log".to_string()],
        });
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 27. compute_hint_height with None input returns 1
    #[tokio::test]
    async fn test_compute_hint_height_empty() {
        let app = make_app(&["line"]).await;
        let result = app.compute_hint_height(&None, 80, None);
        assert_eq!(result, 1);
    }

    // 28. compute_hint_height with matching command
    #[tokio::test]
    async fn test_compute_hint_height_matching_command() {
        let app = make_app(&["line"]).await;
        let input = Some(("filter".to_string(), 6));
        let result = app.compute_hint_height(&input, 80, None);
        assert!(result >= 1);
    }

    // 29. compute_hint_height with command error
    #[tokio::test]
    async fn test_compute_hint_height_error() {
        let mut app = make_app(&["line"]).await;
        app.tabs[0].command_error = Some("something went wrong".to_string());
        let input = Some(("bad".to_string(), 3));
        let result = app.compute_hint_height(&input, 80, None);
        assert!(result >= 1);
    }

    // 30. Very small terminal
    #[tokio::test]
    async fn test_ui_small_terminal() {
        let mut app = make_app(&["hello", "world"]).await;
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 31. Scroll offset beyond visible lines
    #[tokio::test]
    async fn test_ui_scroll_beyond_visible() {
        let mut app = make_app(&["line 0", "line 1"]).await;
        app.tabs[0].scroll_offset = 999;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 32. Loading status bar with progress
    #[tokio::test]
    async fn test_ui_loading_status_bar() {
        let mut app = make_app(&["placeholder"]).await;
        let (_progress_tx, progress_rx) = tokio::sync::watch::channel(0.5f64);
        let (_result_tx, result_rx) = tokio::sync::oneshot::channel();
        app.file_load_state = Some(super::super::FileLoadState {
            path: "/tmp/test.log".to_string(),
            progress_rx,
            result_rx,
            total_bytes: 1000,
            on_complete: super::super::LoadContext::ReplaceInitialTab,
        });
        let mut terminal = make_terminal();
        // _progress_tx is kept alive until after draw
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    // 33. Filters and search combined
    #[tokio::test]
    async fn test_ui_filters_and_search() {
        let mut app = make_app(&[
            "INFO something happened",
            "ERROR another thing",
            "INFO something else",
        ])
        .await;
        app.execute_command_str("filter INFO".to_string()).await;
        let visible = app.tabs[0].visible_indices.clone();
        let tab = &mut app.tabs[0];
        let texts = tab.collect_display_texts(&visible);
        let _ = tab.search.search("something", &visible, |li| texts.get(&li).cloned());
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }
}
