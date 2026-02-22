use std::collections::{HashMap, HashSet};

use ratatui::{
    Frame,
    prelude::*,
    style::Modifier,
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
};

use crate::auto_complete::{
    complete_color, complete_file_path, extract_color_partial, find_command_completions,
    find_matching_command,
};
use crate::filters::{SEARCH_STYLE_ID, render_line};
use crate::theme::complete_theme;
use crate::types::{FilterType, LogLevel};
use crate::value_colors::colorize_known_values;

use super::field_layout::{apply_field_layout, count_wrapped_lines, line_row_count};
use super::{App, LoadContext};

impl App {
    pub(super) fn ui(&mut self, frame: &mut Frame) {
        let size = frame.size();
        frame.render_widget(Block::default().bg(self.theme.root_bg), size);

        let has_multiple_tabs = self.tabs.len() > 1;

        // Extract mode-derived state up front to avoid holding a borrow over the rest of rendering
        let has_input_bar = self.tabs[self.active_tab].mode.needs_input_bar();
        let command_input: Option<(String, usize)> = self.tabs[self.active_tab]
            .mode
            .command_state()
            .map(|(s, c)| (s.to_string(), c));
        let completion_index: Option<usize> = self.tabs[self.active_tab].mode.completion_index();
        let search_input: Option<(String, bool)> = self.tabs[self.active_tab]
            .mode
            .search_state()
            .map(|(s, f)| (s.to_string(), f));
        let is_confirm_restore = self.tabs[self.active_tab]
            .mode
            .confirm_restore_context()
            .is_some();
        let session_files: Option<Vec<String>> = self.tabs[self.active_tab]
            .mode
            .confirm_restore_session_files()
            .map(|f| f.to_vec());
        let selected_filter_idx = self.tabs[self.active_tab]
            .mode
            .selected_filter_index()
            .unwrap_or(0);
        let keybindings = self.tabs[self.active_tab].keybindings.clone();
        let status_line = self.tabs[self.active_tab]
            .mode
            .dynamic_status_line(&keybindings, &self.theme);
        let visual_anchor: Option<usize> =
            self.tabs[self.active_tab].mode.visual_selection_anchor();
        let comment_popup: Option<(Vec<String>, usize, usize, usize)> =
            self.tabs[self.active_tab].mode.comment_popup();
        let help_state: Option<(usize, String)> = self.tabs[self.active_tab]
            .mode
            .keybindings_help_scroll()
            .map(|scroll| {
                let search = self.tabs[self.active_tab]
                    .mode
                    .keybindings_help_search()
                    .unwrap_or("")
                    .to_string();
                (scroll, search)
            });
        let select_fields_state: Option<(Vec<(String, bool)>, usize)> = self.tabs[self.active_tab]
            .mode
            .select_fields_state()
            .map(|(fields, sel)| (fields.to_vec(), sel));
        let docker_select: Option<(Vec<crate::types::DockerContainer>, usize, Option<String>)> =
            self.tabs[self.active_tab]
                .mode
                .docker_select_state()
                .map(|(c, sel, err)| (c.to_vec(), sel, err.map(|s| s.to_string())));
        let value_colors_state: Option<(
            Vec<crate::mode::value_colors_mode::ValueColorGroup>,
            String,
            usize,
        )> = self.tabs[self.active_tab]
            .mode
            .value_colors_state()
            .map(|(groups, search, sel)| (groups.to_vec(), search.to_string(), sel));

        if is_confirm_restore {
            self.render_confirm_restore_modal(frame);
            return;
        }

        // Compute how many rows the status bar needs so wrapped text is fully visible.
        let inner_width = (size.width as usize).saturating_sub(2); // minus 2 for L/R borders
        let status_text: String = status_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        let content_lines = count_wrapped_lines(&status_text, inner_width);
        let status_height = (content_lines + 2).clamp(3, 6) as u16; // +2 for borders

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
        constraints.push(Constraint::Length(status_height)); // command list
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
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(1), Constraint::Length(30)])
                .split(main_chunk);
            (horizontal[0], Some(horizontal[1]))
        } else {
            (main_chunk, None)
        };

        self.render_logs_panel(frame, logs_area, visual_anchor);

        self.render_side_bar(frame, selected_filter_idx, sidebar_area);

        self.render_command_bar(frame, command_input, completion_index, &chunks, chunk_idx);

        self.render_input_bar(frame, search_input, &chunks, chunk_idx);

        let command_list = Paragraph::new(status_line)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border)),
            )
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(self.theme.root_bg));
        frame.render_widget(command_list, *chunks.last().unwrap());

        // Session restore modal renders on top of the full TUI so stdin content
        // is visible behind it.
        if let Some(files) = session_files {
            self.render_confirm_restore_session_modal(frame, &files);
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

        let visible_height = (logs_area.height as usize).saturating_sub(2);
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
        let inner_width = (logs_area.width as usize).saturating_sub(2 + ln_prefix_width);

        let wrap = self.tabs[self.active_tab].wrap;

        // Clamp scroll_offset
        if num_visible == 0 {
            self.tabs[self.active_tab].scroll_offset = 0;
        } else if self.tabs[self.active_tab].scroll_offset >= num_visible {
            self.tabs[self.active_tab].scroll_offset = num_visible - 1;
        }

        let scroll_offset = self.tabs[self.active_tab].scroll_offset;
        let viewport_offset = self.tabs[self.active_tab].viewport_offset;

        let new_viewport = if scroll_offset < viewport_offset {
            scroll_offset
        } else if wrap && inner_width > 0 && num_visible > 0 {
            let rows_used: usize = (viewport_offset..=scroll_offset)
                .map(|i| {
                    let li = self.tabs[self.active_tab].visible_indices[i];
                    line_row_count(
                        self.tabs[self.active_tab].file_reader.get_line(li),
                        inner_width,
                    )
                })
                .sum();
            if rows_used > visible_height {
                let mut rows = 0usize;
                let mut new_vp = scroll_offset + 1;
                loop {
                    if new_vp == 0 {
                        break;
                    }
                    new_vp -= 1;
                    let li = self.tabs[self.active_tab].visible_indices[new_vp];
                    let h = line_row_count(
                        self.tabs[self.active_tab].file_reader.get_line(li),
                        inner_width,
                    );
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

        self.tabs[self.active_tab].viewport_offset = new_viewport;
        let start = new_viewport;

        let end = if wrap && inner_width > 0 {
            let mut rows = 0usize;
            let mut e = start;
            while e < num_visible {
                let li = self.tabs[self.active_tab].visible_indices[e];
                let h = line_row_count(
                    self.tabs[self.active_tab].file_reader.get_line(li),
                    inner_width,
                );
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

        let (filter_manager, mut styles) = self.tabs[self.active_tab]
            .log_manager
            .build_filter_manager();
        let search_style = Style::default()
            .fg(Color::Black)
            .bg(self.theme.text_highlight);
        styles.resize(256, Style::default());
        styles[255] = search_style;

        let search_results = self.tabs[self.active_tab].search.get_results();
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
        // Clone the hidden-field set so the closure doesn't borrow `self` while iterating.
        let hidden_fields = self.tabs[self.active_tab].hidden_fields.clone();
        let field_layout = self.tabs[self.active_tab].field_layout.clone();
        // Pre-compute visual selection range (indices into visible_indices space).
        let visual_range: Option<(usize, usize)> = visual_anchor.map(|anchor| {
            let lo = anchor.min(current_scroll);
            let hi = anchor.max(current_scroll);
            (lo, hi)
        });
        // Visual selection highlight colour (same as border bg, distinct from cursor).
        let visual_style = Style::default().fg(theme.text).bg(Color::Rgb(68, 71, 90));

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
            .fg(theme.text_highlight)
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
                        LogLevel::Error => base_style = base_style.fg(theme.error_fg),
                        LogLevel::Warning => base_style = base_style.fg(theme.warning_fg),
                        _ => {}
                    }
                }
                if is_marked {
                    base_style = base_style.bg(Color::Rgb(70, 60, 15));
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

                        if cols.is_empty() {
                            // All fields hidden — fall back to raw bytes with filter +
                            // search highlighting (raw-byte positions are correct here).
                            let mut collector = filter_manager.evaluate_line(line_bytes);
                            if let Some(sr) = search_map.get(&line_idx) {
                                collector.with_priority(1000);
                                for &(s, e) in &sr.matches {
                                    collector.push(s, e, SEARCH_STYLE_ID);
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
                                for m in regex.find_iter(&rendered) {
                                    collector.push(m.start(), m.end(), SEARCH_STYLE_ID);
                                }
                            }
                            render_line(&collector, &styles)
                        }
                    });

                let mut line = if let Some(structured_line) = structured_line {
                    structured_line
                } else {
                    let mut collector = filter_manager.evaluate_line(line_bytes);
                    if let Some(sr) = search_map.get(&line_idx) {
                        collector.with_priority(1000);
                        for &(s, e) in &sr.matches {
                            collector.push(s, e, SEARCH_STYLE_ID);
                        }
                    }
                    render_line(&collector, &styles)
                };
                line = colorize_known_values(line, &theme.value_colors);
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
                        (ch, theme.text_highlight)
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

        let logs_title = format!(
            "{} ({})",
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
            num_visible
        );

        let mut paragraph = Paragraph::new(log_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border))
                    .title(logs_title)
                    .title_style(Style::default().fg(self.theme.border_title)),
            )
            .scroll((0, self.tabs[self.active_tab].horizontal_scroll as u16));

        if self.tabs[self.active_tab].wrap {
            paragraph = paragraph.wrap(Wrap { trim: false });
        }

        frame.render_widget(paragraph, logs_area);

        if num_visible > 0 {
            let mut scrollbar_state = ScrollbarState::new(num_visible)
                .position(start)
                .viewport_content_length(end.saturating_sub(start));
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
        search_input: Option<(String, bool)>,
        chunks: &std::rc::Rc<[Rect]>,
        chunk_idx: usize,
    ) {
        if let Some((input_str, forward)) = search_input {
            let prefix = if forward { "/" } else { "?" };
            let search_line = Paragraph::new(format!("{}{}", prefix, input_str))
                .style(Style::default().fg(self.theme.text).bg(self.theme.border))
                .wrap(Wrap { trim: false });
            let input_area = chunks[chunk_idx];
            frame.render_widget(search_line, input_area);
            let cursor_x = input_area.x + 1 + input_str.len() as u16;
            if cursor_x < input_area.x + input_area.width {
                frame.set_cursor(cursor_x, input_area.y);
            }

            let hint_area = chunks[chunk_idx + 1];
            let match_count = self.tabs[self.active_tab].search.get_results().len();
            let hint_text = if !input_str.is_empty() {
                format!("  {} matches", match_count)
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

        let area = frame.size();
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
                    .bg(self.theme.text_highlight),
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
                if self.tabs[self.active_tab].command_error.is_some() {
                    let err = self.tabs[self.active_tab].command_error.as_ref().unwrap();
                    err.clone()
                } else if let Some(partial) = extract_color_partial(input_text) {
                    let completions = complete_color(partial);
                    completions
                        .iter()
                        .map(|n| format!(" {} ", n))
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    let file_commands = ["open", "load-filters", "save-filters", "export-marked"];
                    let trimmed = input_text.trim();
                    let file_cmd = file_commands
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
                .style(Style::default().fg(self.theme.text).bg(self.theme.border))
                .wrap(Wrap { trim: false });
            let input_area = chunks[chunk_idx];
            frame.render_widget(command_line, input_area);
            let cursor_x = input_area.x + 1 + cursor_pos as u16;
            if cursor_x < input_area.x + input_area.width {
                frame.set_cursor(cursor_x, input_area.y);
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
                let file_commands = ["open", "load-filters", "save-filters", "export-marked"];
                let trimmed_input = input_text.trim();
                let file_cmd = file_commands
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
            let filters = self.tabs[self.active_tab].log_manager.get_filters();
            let filters_text: Vec<Line> = filters
                .iter()
                .enumerate()
                .map(|(i, filter)| {
                    let status = if filter.enabled { "[x]" } else { "[ ]" };
                    let selected_prefix = if i == selected_filter_idx { ">" } else { " " };
                    let filter_type_str = match filter.filter_type {
                        FilterType::Include => "In",
                        FilterType::Exclude => "Out",
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
                        selected_prefix, status, filter_type_str, filter.pattern
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
            let sidebar = Paragraph::new(filters_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.border))
                    .title(sidebar_title)
                    .title_style(Style::default().fg(self.theme.border_title)),
            );
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
                            .bg(self.theme.text_highlight)
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
