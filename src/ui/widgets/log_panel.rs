use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ratatui::{
    prelude::*,
    style::Modifier,
    widgets::{
        Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

use crate::filters::{CURRENT_SEARCH_STYLE_ID, MatchCollector, SEARCH_STYLE_ID, render_line};
use crate::parser::LogLevel;
use crate::search::SearchResult;
use crate::theme::Theme;
use crate::ui::field_layout::{apply_field_layout, effective_row_count};
use crate::ui::{CachedParsedLine, TabState, VisibleLines};
use crate::value_colors::{
    VALUE_STYLE_HTTP_DELETE, VALUE_STYLE_HTTP_GET, VALUE_STYLE_HTTP_OTHER, VALUE_STYLE_HTTP_PATCH,
    VALUE_STYLE_HTTP_POST, VALUE_STYLE_HTTP_PUT, VALUE_STYLE_IP, VALUE_STYLE_STATUS_2XX,
    VALUE_STYLE_STATUS_3XX, VALUE_STYLE_STATUS_4XX, VALUE_STYLE_STATUS_5XX, VALUE_STYLE_UUID,
    collect_value_color_spans,
};

fn prepend_line_number(
    line: Line<'static>,
    line_idx: usize,
    line_number_width: usize,
    is_annotated: bool,
    comment_fg: Color,
    line_number_fg: Color,
    render_style: Style,
) -> Line<'static> {
    let line_num = line_idx + 1;
    let line_num_str = format!("{:>width$} ", line_num, width = line_number_width);
    let bar_span = if is_annotated {
        Span::styled("\u{2502}", Style::default().fg(comment_fg))
    } else {
        Span::styled(" ", Style::default().fg(line_number_fg))
    };
    let num_span = Span::styled(line_num_str, Style::default().fg(line_number_fg));
    let mut all_spans = vec![bar_span, num_span];
    all_spans.extend(line.spans);
    Line::from(all_spans).style(render_style)
}

fn prepare_comment_maps(
    comments: &[(Vec<usize>, String)],
    visible_indices: &VisibleLines,
    start: usize,
    end: usize,
) -> (HashMap<usize, usize>, HashMap<usize, usize>) {
    let mut line_cmt_map: HashMap<usize, usize> = HashMap::new();
    for (cmt_idx, (line_indices, _)) in comments.iter().enumerate() {
        for &li in line_indices {
            line_cmt_map.entry(li).or_insert(cmt_idx);
        }
    }
    let mut banner_at: HashMap<usize, usize> = HashMap::new();
    let mut vis_comment_map: HashMap<usize, usize> = HashMap::new();
    let mut seen_cmts: HashSet<usize> = HashSet::new();
    for abs_vi in start..end {
        let li = visible_indices.get(abs_vi);
        if let Some(&cmt_idx) = line_cmt_map.get(&li) {
            vis_comment_map.insert(abs_vi, cmt_idx);
            if seen_cmts.insert(cmt_idx) {
                banner_at.insert(abs_vi, cmt_idx);
            }
        }
    }
    (banner_at, vis_comment_map)
}

fn build_style_table(tab: &TabState, theme: &Theme) -> (u8, [Style; 256]) {
    let mut styles: Vec<Style> = if tab.filter.enabled {
        tab.filter.text_styles.clone()
    } else {
        Vec::new()
    };
    let process_style_start = styles.len() as u8;
    for &color in &theme.process_colors {
        styles.push(Style::default().fg(color));
    }
    styles.resize(256, Style::default());

    let search_style = Style::default()
        .fg(theme.search_fg)
        .bg(theme.text_highlight_fg);
    let current_search_style = Style::default()
        .fg(theme.text_highlight_fg)
        .bg(theme.search_fg);

    styles[255] = search_style;
    styles[254] = current_search_style;
    styles[VALUE_STYLE_HTTP_GET as usize] = Style::default().fg(theme.value_colors.http_get);
    styles[VALUE_STYLE_HTTP_POST as usize] = Style::default().fg(theme.value_colors.http_post);
    styles[VALUE_STYLE_HTTP_PUT as usize] = Style::default().fg(theme.value_colors.http_put);
    styles[VALUE_STYLE_HTTP_DELETE as usize] = Style::default().fg(theme.value_colors.http_delete);
    styles[VALUE_STYLE_HTTP_PATCH as usize] = Style::default().fg(theme.value_colors.http_patch);
    styles[VALUE_STYLE_HTTP_OTHER as usize] = Style::default().fg(theme.value_colors.http_other);
    styles[VALUE_STYLE_STATUS_2XX as usize] = Style::default().fg(theme.value_colors.status_2xx);
    styles[VALUE_STYLE_STATUS_3XX as usize] = Style::default().fg(theme.value_colors.status_3xx);
    styles[VALUE_STYLE_STATUS_4XX as usize] = Style::default().fg(theme.value_colors.status_4xx);
    styles[VALUE_STYLE_STATUS_5XX as usize] = Style::default().fg(theme.value_colors.status_5xx);
    styles[VALUE_STYLE_IP as usize] = Style::default().fg(theme.value_colors.ip_address);
    styles[VALUE_STYLE_UUID as usize] = Style::default().fg(theme.value_colors.uuid);

    let mut arr = [Style::default(); 256];
    arr.copy_from_slice(&styles);
    (process_style_start, arr)
}

fn build_comment_banner_lines(
    text: &str,
    inner_width: usize,
    total_width: usize,
    banner_dash_style: Style,
    banner_text_style: Style,
    banner_cont_style: Style,
) -> Vec<Line<'static>> {
    let _ = inner_width;
    let mut lines = Vec::new();
    for (i, text_line) in text.lines().enumerate() {
        if i == 0 {
            let left = " \u{2500}\u{2500} ";
            let text_len = text_line.chars().count();
            let used = left.len() + text_len + 1;
            let right_dashes = "\u{2500}".repeat(total_width.saturating_sub(used).max(1));
            lines.push(Line::from(vec![
                Span::styled(left, banner_dash_style),
                Span::styled(text_line.to_string(), banner_text_style),
                Span::styled(format!(" {right_dashes}"), banner_dash_style),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled("    ", banner_cont_style),
                Span::styled(text_line.to_string(), banner_cont_style),
            ]));
        }
    }
    lines
}

#[allow(clippy::too_many_arguments)]
fn compute_viewport(
    tab: &TabState,
    scroll_offset: usize,
    viewport_offset: usize,
    num_visible: usize,
    visible_height: usize,
    wrap: bool,
    inner_width: usize,
    hidden_fields: &HashSet<String>,
    field_layout: &crate::ui::FieldLayout,
    show_keys: bool,
    raw_mode: bool,
) -> (usize, usize) {
    let parser = if raw_mode {
        None
    } else {
        tab.display.format.as_deref()
    };
    let row_count = |li: usize| -> usize {
        effective_row_count(
            tab.file_reader.get_line(li),
            inner_width,
            parser,
            field_layout,
            hidden_fields,
            show_keys,
        )
    };

    let new_viewport = if scroll_offset < viewport_offset {
        scroll_offset
    } else if wrap && inner_width > 0 && num_visible > 0 {
        let gap = scroll_offset.saturating_sub(viewport_offset);
        let overflowed = gap > visible_height || {
            let rows_used: usize = (viewport_offset..=scroll_offset)
                .map(|i| row_count(tab.filter.visible_indices.get(i)))
                .sum();
            rows_used > visible_height
        };
        if overflowed {
            let mut rows = 0usize;
            let mut new_vp = scroll_offset + 1;
            loop {
                if new_vp == 0 {
                    break;
                }
                new_vp -= 1;
                let h = row_count(tab.filter.visible_indices.get(new_vp));
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
            let h = row_count(tab.filter.visible_indices.get(e));
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

    if end == num_visible && num_visible > 0 {
        let filled_start = if wrap && inner_width > 0 {
            let mut rows = 0usize;
            let mut s = num_visible;
            loop {
                if s == 0 {
                    break;
                }
                s -= 1;
                let h = row_count(tab.filter.visible_indices.get(s));
                if rows + h > visible_height {
                    s += 1;
                    break;
                }
                rows += h;
                if s == 0 {
                    break;
                }
            }
            s
        } else {
            num_visible.saturating_sub(visible_height)
        };
        if filled_start < new_viewport {
            let adj_end = if wrap && inner_width > 0 {
                let mut rows = 0usize;
                let mut e = filled_start;
                while e < num_visible {
                    let h = row_count(tab.filter.visible_indices.get(e));
                    if rows + h > visible_height {
                        break;
                    }
                    rows += h;
                    e += 1;
                }
                if e == filled_start && filled_start < num_visible {
                    e += 1;
                }
                e
            } else {
                num_visible
            };
            (filled_start, adj_end)
        } else {
            (new_viewport, end)
        }
    } else {
        (new_viewport, end)
    }
}

fn populate_parse_cache(
    tab: &mut TabState,
    start: usize,
    end: usize,
    raw_mode: bool,
    hidden_fields: &HashSet<String>,
    field_layout: &crate::ui::FieldLayout,
    show_keys: bool,
) {
    let cache_gen = tab.cache.parse_gen;
    let mut new_entries: Vec<(usize, CachedParsedLine)> = Vec::new();
    {
        if !raw_mode && let Some(parser) = tab.display.format.as_deref() {
            for vi in start..end {
                let line_idx = tab.filter.visible_indices.get(vi);
                if tab
                    .cache
                    .parse
                    .get(&line_idx)
                    .map(|(g, _)| *g == cache_gen)
                    .unwrap_or(false)
                {
                    continue;
                }
                let line_bytes = tab.file_reader.get_line(line_idx);
                if let Some(parts) = parser.parse_line(line_bytes) {
                    let year_override =
                        tab.year_map.as_deref().map(|ym| ym.year_for_line(line_idx));
                    let cols = apply_field_layout(
                        &parts,
                        field_layout,
                        hidden_fields,
                        show_keys,
                        year_override,
                    );
                    let all_cols_hidden = cols.is_empty();
                    let level = parts.level.map(|s| s.to_string());
                    let timestamp = parts.timestamp.map(|s| s.to_string());
                    let target = parts.target.map(|s| s.to_string());
                    let pid = parts
                        .extra_fields
                        .iter()
                        .find(|(_, k, _)| *k == "pid")
                        .map(|(_, _, v)| v.to_string());
                    let rendered = if all_cols_hidden {
                        String::new()
                    } else {
                        let cap: usize = cols.iter().map(|c| c.len()).sum::<usize>() + cols.len();
                        let mut buf = String::with_capacity(cap);
                        for (i, col) in cols.iter().enumerate() {
                            if i > 0 {
                                buf.push(' ');
                            }
                            buf.push_str(col);
                        }
                        buf
                    };
                    let target_offset = target
                        .as_deref()
                        .filter(|t| !t.is_empty())
                        .and_then(|t| find_token_offset(&rendered, t));
                    let pid_offset = pid
                        .as_deref()
                        .filter(|p| !p.is_empty())
                        .and_then(|p| find_token_offset(&rendered, p));
                    let timestamp_offset = timestamp
                        .as_deref()
                        .filter(|ts| !ts.is_empty())
                        .and_then(|ts| rendered.find(ts));
                    new_entries.push((
                        line_idx,
                        CachedParsedLine {
                            rendered,
                            level,
                            timestamp,
                            target,
                            pid,
                            all_cols_hidden,
                            target_offset,
                            pid_offset,
                            timestamp_offset,
                        },
                    ));
                }
            }
        }
    }
    for (line_idx, entry) in new_entries {
        tab.cache.parse.insert(line_idx, (cache_gen, entry));
    }
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

pub struct LogPanelData {
    pub log_lines: Vec<Line<'static>>,
    pub num_visible: usize,
    pub visible_height: usize,
    pub start: usize,
    pub horizontal_scroll: usize,
    pub logs_title: String,
    pub show_borders: bool,
    pub show_tab_bar: bool,
    pub wrap: bool,
    pub theme_border: Color,
    pub theme_border_title: Color,
    pub extraction_progress: Option<f64>,
    pub archive_name: String,
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_log_panel(
    tab: &mut TabState,
    logs_area: Rect,
    visual_anchor: Option<usize>,
    visual_char_selection: Option<(usize, usize)>,
    mode_name: Option<&str>,
    show_tab_bar: bool,
    has_input_bar: bool,
    theme: &Theme,
) -> LogPanelData {
    let num_visible = tab.filter.visible_indices.len();
    let show_borders = tab.display.show_borders;

    let vertical_border = if show_borders {
        if show_tab_bar { 1 } else { 2 }
    } else {
        1
    };
    let horizontal_shrink = if show_borders { 2 } else { 1 };
    let visible_height = (logs_area.height as usize)
        .saturating_sub(vertical_border)
        .saturating_sub(usize::from(has_input_bar));
    tab.scroll.visible_height = visible_height;

    let show_line_numbers = tab.display.show_line_numbers;
    let total_lines = tab.file_reader.line_count();
    let line_number_width = if show_line_numbers {
        total_lines.max(1).to_string().len()
    } else {
        0
    };
    let ln_prefix_width = if show_line_numbers {
        line_number_width + 2
    } else {
        0
    };
    let inner_width =
        (logs_area.width as usize).saturating_sub(horizontal_shrink + ln_prefix_width);
    tab.scroll.visible_width = inner_width;

    let wrap = tab.display.wrap;
    let hidden_fields = tab.display.hidden_fields.clone();
    let field_layout = tab.display.field_layout.clone();
    let show_keys = tab.display.show_keys;
    let raw_mode = tab.display.raw_mode;

    if num_visible == 0 {
        tab.scroll.scroll_offset = 0;
        tab.scroll.viewport_offset = 0;
    } else {
        if tab.scroll.scroll_offset >= num_visible {
            tab.scroll.scroll_offset = num_visible - 1;
        }
        if tab.scroll.viewport_offset >= num_visible {
            tab.scroll.viewport_offset = num_visible.saturating_sub(visible_height);
        }
    }

    let scroll_offset = tab.scroll.scroll_offset;
    let viewport_offset = tab.scroll.viewport_offset;

    let (new_viewport, end) = compute_viewport(
        tab,
        scroll_offset,
        viewport_offset,
        num_visible,
        visible_height,
        wrap,
        inner_width,
        &hidden_fields,
        &field_layout,
        show_keys,
        raw_mode,
    );

    tab.scroll.viewport_offset = new_viewport;
    let start = new_viewport;

    #[cfg(unix)]
    if start < end && !tab.filter.visible_indices.is_empty() {
        let first = tab.filter.visible_indices.get(start);
        let last = tab.filter.visible_indices.get((end - 1).max(start));
        tab.file_reader.advise_viewport(first, last);
    }

    populate_parse_cache(
        tab,
        start,
        end,
        raw_mode,
        &hidden_fields,
        &field_layout,
        show_keys,
    );

    let filter_manager_arc = tab.filter.manager.clone();
    let filter_manager = &*filter_manager_arc;
    let date_filter_styles = if tab.filter.enabled {
        tab.filter.date_styles.clone()
    } else {
        Vec::new()
    };
    let field_filter_styles = if tab.filter.enabled {
        tab.filter.field_styles.clone()
    } else {
        Vec::new()
    };
    let detected_format_arc: Option<Arc<dyn crate::parser::LogFormatParser>> = if raw_mode {
        None
    } else {
        tab.display.format.clone()
    };

    let (process_style_start, styles) = build_style_table(tab, theme);
    let process_colors_len = theme.process_colors.len();

    let search_results = tab.search.query.get_results();
    let current_search_info: Option<(usize, usize)> = if search_results.is_empty() {
        None
    } else {
        let ri = tab.search.query.get_current_match_index();
        Some((
            search_results[ri].line_idx,
            tab.search.query.get_current_occurrence_index(),
        ))
    };
    let find_search_result = |line_idx: usize| -> Option<&SearchResult> {
        search_results
            .binary_search_by_key(&line_idx, |r| r.line_idx)
            .ok()
            .map(|i| &search_results[i])
    };
    let search_regex = tab.search.query.get_compiled_pattern().cloned();

    let level_colors_disabled = tab.display.level_colors_disabled.clone();
    let current_scroll = tab.scroll.scroll_offset;
    let visual_range: Option<(usize, usize)> = visual_anchor.map(|anchor| {
        let lo = anchor.min(current_scroll);
        let hi = anchor.max(current_scroll);
        (lo, hi)
    });
    let visual_style = Style::default()
        .fg(theme.visual_select_fg)
        .bg(theme.visual_select_bg);

    let comments_for_render: Vec<(Vec<usize>, String)> = tab
        .log_manager
        .get_comments()
        .iter()
        .map(|a| (a.line_indices.clone(), a.text.clone()))
        .collect();

    let (banner_at, vis_comment_map) = prepare_comment_maps(
        &comments_for_render,
        &tab.filter.visible_indices,
        start,
        end,
    );

    let banner_dash_style = Style::default()
        .fg(theme.comment_fg)
        .add_modifier(Modifier::DIM);
    let banner_text_style = Style::default()
        .fg(theme.comment_fg)
        .add_modifier(Modifier::BOLD);
    let banner_cont_style = Style::default().fg(theme.comment_fg);

    let render_gen = tab.cache.render_gen;
    let search_gen = tab.cache.search_result_gen;
    let parse_gen = tab.cache.parse_gen;

    let mut render_cache_misses: Vec<(usize, Option<usize>, Line<'static>)> = Vec::new();
    let mut log_lines: Vec<Line> = Vec::new();

    for abs_vis_idx in start..end {
        let line_idx = tab.filter.visible_indices.get(abs_vis_idx);
        let line_bytes = tab.file_reader.get_line(line_idx);
        let is_current = abs_vis_idx == current_scroll;
        let is_marked = tab.log_manager.is_marked(line_idx);
        let is_visual_selected = visual_range
            .map(|(lo, hi)| abs_vis_idx >= lo && abs_vis_idx <= hi)
            .unwrap_or(false);

        let cached = tab
            .cache
            .parse
            .get(&line_idx)
            .filter(|(g, _)| *g == parse_gen)
            .map(|(_, c)| c);

        let mut base_style = Style::default().fg(theme.text);
        if level_colors_disabled.len() < 7 {
            let level = cached
                .and_then(|c| c.level.as_deref())
                .map(LogLevel::parse_level)
                .unwrap_or_else(|| {
                    // For continuation lines (parse_line returned None), inherit
                    // the parent entry's level so the whole multiline block gets
                    // the same color (e.g. a stack trace stays red under ERROR).
                    if let Some(cmap) = &tab.continuation_map {
                        let parent = cmap.get(line_idx).copied().unwrap_or(line_idx);
                        if parent != line_idx {
                            // Try the parent's cache entry first.
                            if let Some(lvl) = tab
                                .cache
                                .parse
                                .get(&parent)
                                .filter(|(g, _)| *g == parse_gen)
                                .and_then(|(_, c)| c.level.as_deref())
                            {
                                return LogLevel::parse_level(lvl);
                            }
                            // Parent not cached (outside viewport) — parse just
                            // the level from its raw bytes without full layout.
                            if let Some(parser) = tab.display.format.as_deref()
                                && let Some(parts) =
                                    parser.parse_line(tab.file_reader.get_line(parent))
                                && let Some(lvl) = parts.level
                            {
                                return LogLevel::parse_level(lvl);
                            }
                        }
                    }
                    LogLevel::detect_from_bytes(line_bytes)
                });
            match level {
                LogLevel::Trace if !level_colors_disabled.contains("trace") => {
                    base_style = base_style.fg(theme.trace_fg)
                }
                LogLevel::Debug if !level_colors_disabled.contains("debug") => {
                    base_style = base_style.fg(theme.debug_fg)
                }
                LogLevel::Info if !level_colors_disabled.contains("info") => {
                    base_style = base_style.fg(theme.info_fg)
                }
                LogLevel::Notice if !level_colors_disabled.contains("notice") => {
                    base_style = base_style.fg(theme.notice_fg)
                }
                LogLevel::Warning if !level_colors_disabled.contains("warning") => {
                    base_style = base_style.bg(theme.warning_bg)
                }
                LogLevel::Error if !level_colors_disabled.contains("error") => {
                    base_style = base_style.bg(theme.error_bg)
                }
                LogLevel::Fatal if !level_colors_disabled.contains("fatal") => {
                    base_style = base_style.bg(theme.fatal_bg)
                }
                _ => {}
            }
        }
        if is_marked {
            base_style = base_style.fg(theme.mark_fg).bg(theme.mark_bg);
        }
        if is_visual_selected {
            base_style = visual_style;
        }

        let render_style = if is_current && visual_char_selection.is_none() {
            base_style
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            base_style
        };

        let current_occ =
            current_search_info.and_then(|(cl, co)| if cl == line_idx { Some(co) } else { None });

        let content_line: Line<'static> = if let Some((_, _, _, cached_line)) = tab
            .cache
            .render_line
            .get(&line_idx)
            .filter(|(rg, sg, occ, _)| {
                *rg == render_gen && *sg == search_gen && *occ == current_occ
            }) {
            cached_line.clone()
        } else {
            let structured_line: Option<Line<'static>> = cached.filter(|_| !raw_mode).map(|c| {
                if c.all_cols_hidden {
                    let mut collector = MatchCollector::new(line_bytes);
                    if let Ok(text) = std::str::from_utf8(line_bytes) {
                        for (s, e, sid) in collect_value_color_spans(text, &theme.value_colors) {
                            collector.push(s, e, sid);
                        }
                    }
                    collector.with_priority(500);
                    filter_manager.evaluate_into(&mut collector);
                    if let Some(sr) = find_search_result(line_idx) {
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
                    let rendered = &c.rendered;
                    let mut collector = MatchCollector::new(rendered.as_bytes());
                    for (s, e, sid) in collect_value_color_spans(rendered, &theme.value_colors) {
                        collector.push(s, e, sid);
                    }
                    collector.with_priority(10);
                    if process_colors_len > 0
                        && !theme.value_colors.is_disabled("process_colors")
                        && let Some(target) = c.target.as_deref()
                    {
                        let idx = stable_hash(target) % process_colors_len;
                        let sid = process_style_start.saturating_add(idx as u8);
                        if let Some(pos) = c.target_offset {
                            collector.push(pos, pos + target.len(), sid);
                        }
                        if let Some(pid_val) = c.pid.as_deref() {
                            let pid_sid = process_style_start
                                .saturating_add((stable_hash(target) % process_colors_len) as u8);
                            if let Some(pos) = c.pid_offset {
                                collector.push(pos, pos + pid_val.len(), pid_sid);
                            }
                        }
                    }
                    collector.with_priority(500);
                    filter_manager.evaluate_into(&mut collector);
                    if let Some(ts) = c.timestamp.as_deref() {
                        for dfs in &date_filter_styles {
                            if dfs.filter.matches(ts, None) {
                                collector.with_priority(500);
                                if dfs.match_only {
                                    if let Some(ts_pos) = c.timestamp_offset {
                                        collector.push(ts_pos, ts_pos + ts.len(), dfs.style_id);
                                    }
                                } else {
                                    collector.push(0, rendered.len(), dfs.style_id);
                                }
                            }
                        }
                    }
                    if !field_filter_styles.is_empty()
                        && let Some(ref parser_arc) = detected_format_arc
                    {
                        let ffs_parser: &dyn crate::parser::LogFormatParser = parser_arc.as_ref();
                        if let Some(parts) = ffs_parser.parse_line(line_bytes) {
                            collector.with_priority(500);
                            for ffs in &field_filter_styles {
                                if let Some(val) =
                                    crate::filters::resolve_field(&ffs.field_filter.field, &parts)
                                        .filter(|v| v.contains(ffs.field_filter.pattern.as_str()))
                                {
                                    if ffs.match_only {
                                        if let Some(pos) = rendered.find(val) {
                                            collector.push(pos, pos + val.len(), ffs.style_id);
                                        }
                                    } else {
                                        collector.push(0, rendered.len(), ffs.style_id);
                                    }
                                }
                            }
                        }
                    }
                    if let Some(ref regex) = search_regex {
                        collector.with_priority(1000);
                        for (i, m) in regex.find_iter(rendered).enumerate() {
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

            let line = if let Some(sl) = structured_line {
                sl
            } else {
                let mut collector = MatchCollector::new(line_bytes);
                if let Ok(text) = std::str::from_utf8(line_bytes) {
                    for (s, e, sid) in collect_value_color_spans(text, &theme.value_colors) {
                        collector.push(s, e, sid);
                    }
                }
                collector.with_priority(500);
                filter_manager.evaluate_into(&mut collector);
                if let Some(sr) = find_search_result(line_idx) {
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
            render_cache_misses.push((line_idx, current_occ, line.clone()));
            line
        };

        let mut line = content_line.style(render_style);

        if is_current && let Some((lo, hi)) = visual_char_selection {
            line = crate::mode::visual_char_mode::apply_char_selection(line, lo, hi);
        }

        if show_line_numbers {
            let is_annotated = vis_comment_map.contains_key(&abs_vis_idx);
            line = prepend_line_number(
                line,
                line_idx,
                line_number_width,
                is_annotated,
                theme.comment_fg,
                theme.line_number_fg,
                render_style,
            );
        }

        if let Some(&cmt_idx) = banner_at.get(&abs_vis_idx) {
            let (_, text) = &comments_for_render[cmt_idx];
            let total_width = inner_width + ln_prefix_width;
            for banner_line in build_comment_banner_lines(
                text,
                inner_width,
                total_width,
                banner_dash_style,
                banner_text_style,
                banner_cont_style,
            ) {
                log_lines.push(banner_line);
            }
        }
        log_lines.push(line);
    }

    for (line_idx, current_occ, content_line) in render_cache_misses {
        tab.cache.render_line.insert(
            line_idx,
            (render_gen, search_gen, current_occ, content_line),
        );
    }

    let tail_mode = tab.stream.tail_mode;
    let paused = tab.stream.paused;
    let logs_title = if show_tab_bar {
        String::new()
    } else {
        format!(
            "{}{} ({}){}{}{}",
            mode_name.map(|m| format!("[{}] ", m)).unwrap_or_default(),
            tab.log_manager
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
            if tail_mode { " [TAIL]" } else { "" },
            if raw_mode { " [RAW]" } else { "" },
            if paused { " [PAUSED]" } else { "" },
        )
    };

    LogPanelData {
        log_lines,
        num_visible,
        visible_height,
        start,
        horizontal_scroll: tab.scroll.horizontal_scroll,
        logs_title,
        show_borders,
        show_tab_bar,
        wrap,
        theme_border: theme.border,
        theme_border_title: theme.border_title,
        extraction_progress: tab.extraction_progress,
        archive_name: tab.title.clone(),
    }
}

pub struct LogPanel<'a> {
    pub data: &'a LogPanelData,
}

impl<'a> Widget for LogPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let d = self.data;
        let logs_borders = if d.show_borders {
            if d.show_tab_bar {
                Borders::LEFT | Borders::RIGHT | Borders::BOTTOM
            } else {
                Borders::ALL
            }
        } else {
            Borders::NONE
        };
        let title_style = Style::default().fg(d.theme_border_title);
        let logs_block = if d.show_borders {
            let block = Block::default()
                .borders(logs_borders)
                .border_style(Style::default().fg(d.theme_border));
            if d.logs_title.is_empty() {
                block
            } else {
                block.title(d.logs_title.clone()).title_style(title_style)
            }
        } else {
            let block = Block::default()
                .borders(Borders::NONE)
                .padding(Padding::new(1, 0, 0, 0));
            if d.logs_title.is_empty() {
                block
            } else {
                block.title(d.logs_title.clone()).title_style(title_style)
            }
        };

        let mut paragraph = Paragraph::new(d.log_lines.clone())
            .block(logs_block)
            .scroll((0, d.horizontal_scroll as u16));

        if d.wrap {
            paragraph = paragraph.wrap(Wrap { trim: false });
        }

        paragraph.render(area, buf);

        let inner = if d.show_borders {
            area.inner(ratatui::layout::Margin {
                horizontal: 1,
                vertical: 1,
            })
        } else {
            area
        };
        if let Some(fraction) = d.extraction_progress {
            let (bar, pct) = crate::ui::render::progress_bar_str(fraction);
            let text = format!("{}\n{bar}  {pct}%", d.archive_name);
            let overlay = Paragraph::new(text).alignment(Alignment::Center);
            let mid_y = (inner.y + inner.height / 2).saturating_sub(1);
            let overlay_area = Rect {
                x: inner.x,
                y: mid_y,
                width: inner.width,
                height: 2,
            };
            overlay.render(overlay_area, buf);
        }

        if d.num_visible > 0 {
            let max_scroll = d.num_visible.saturating_sub(d.visible_height);
            let mut scrollbar_state = ScrollbarState::new(max_scroll.max(1)).position(d.start);
            Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .style(Style::default().fg(d.theme_border))
                .render(area, buf, &mut scrollbar_state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Keybindings;
    use crate::db::Database;
    use crate::db::LogManager;
    use crate::ingestion::FileReader;
    use crate::theme::Theme;
    use crate::ui::App;
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
    async fn test_log_panel_basic_render() {
        let mut app = make_app(&["INFO line one", "WARN line two", "ERROR line three"]).await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("INFO"));
    }

    #[tokio::test]
    async fn test_log_panel_empty_file() {
        let mut app = make_app(&[]).await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_with_borders() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].display.show_borders = true;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_without_line_numbers() {
        let mut app = make_app(&["INFO hello", "DEBUG world"]).await;
        app.tabs[0].display.show_line_numbers = false;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_with_wrap() {
        let long_line = "A".repeat(200);
        let mut app = make_app(&[&long_line]).await;
        app.tabs[0].display.wrap = true;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_with_horizontal_scroll() {
        let long_line = "A".repeat(200);
        let mut app = make_app(&[&long_line]).await;
        app.tabs[0].scroll.horizontal_scroll = 10;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_json_structured_lines() {
        let mut app = make_app(&[
            r#"{"level":"INFO","msg":"hello world","target":"myapp"}"#,
            r#"{"level":"ERROR","msg":"something failed"}"#,
        ])
        .await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_with_mark() {
        let mut app = make_app(&["line one", "line two", "line three"]).await;
        app.tabs[0].log_manager.toggle_mark(0);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_scroll_offset_mid() {
        let lines: Vec<&str> = (0..30).map(|_| "line content here").collect();
        let mut app = make_app(&lines).await;
        app.tabs[0].scroll.scroll_offset = 15;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_raw_mode() {
        let mut app = make_app(&[
            r#"{"level":"INFO","msg":"hello"}"#,
            r#"{"level":"WARN","msg":"warning"}"#,
        ])
        .await;
        app.tabs[0].display.raw_mode = true;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[test]
    fn test_prepend_line_number_basic() {
        let line = Line::from("hello");
        let result = prepend_line_number(
            line,
            0,
            3,
            false,
            Color::Yellow,
            Color::Gray,
            Style::default(),
        );
        let combined: String = result.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(combined.contains("hello"));
        assert!(combined.contains("1"));
    }

    #[test]
    fn test_prepend_line_number_annotated() {
        let line = Line::from("annotated");
        let result = prepend_line_number(
            line,
            4,
            2,
            true,
            Color::Yellow,
            Color::Gray,
            Style::default(),
        );
        let bar_span = &result.spans[0];
        assert_eq!(bar_span.content, "\u{2502}");
    }

    #[test]
    fn test_build_comment_banner_single_line() {
        let lines = build_comment_banner_lines(
            "my comment",
            60,
            70,
            Style::default(),
            Style::default(),
            Style::default(),
        );
        assert_eq!(lines.len(), 1);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("my comment"));
    }

    #[test]
    fn test_build_comment_banner_multi_line() {
        let lines = build_comment_banner_lines(
            "first\nsecond",
            60,
            70,
            Style::default(),
            Style::default(),
            Style::default(),
        );
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_stable_hash_deterministic() {
        assert_eq!(stable_hash("hello"), stable_hash("hello"));
        assert_ne!(stable_hash("hello"), stable_hash("world"));
    }

    #[test]
    fn test_find_token_offset_found() {
        assert_eq!(find_token_offset("hello world foo", "world"), Some(6));
    }

    #[test]
    fn test_find_token_offset_not_found() {
        assert_eq!(find_token_offset("helloworld", "world"), None);
    }

    #[test]
    fn test_find_token_offset_empty_needle() {
        assert_eq!(find_token_offset("hello world", ""), None);
    }

    #[test]
    fn test_find_token_offset_at_start() {
        assert_eq!(find_token_offset("foo bar", "foo"), Some(0));
    }

    #[test]
    fn test_find_token_offset_at_end() {
        assert_eq!(find_token_offset("foo bar", "bar"), Some(4));
    }

    #[test]
    fn test_log_panel_widget_no_borders_no_title() {
        let data = LogPanelData {
            log_lines: vec![Line::from("hello")],
            num_visible: 1,
            visible_height: 10,
            start: 0,
            horizontal_scroll: 0,
            logs_title: String::new(),
            show_borders: false,
            show_tab_bar: false,
            wrap: false,
            theme_border: Color::Gray,
            theme_border_title: Color::White,
            extraction_progress: None,
            archive_name: String::new(),
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal
            .draw(|f| f.render_widget(LogPanel { data: &data }, f.area()))
            .unwrap();
    }

    #[test]
    fn test_log_panel_widget_borders_with_tab_bar() {
        let data = LogPanelData {
            log_lines: vec![Line::from("content")],
            num_visible: 5,
            visible_height: 4,
            start: 1,
            horizontal_scroll: 0,
            logs_title: String::new(),
            show_borders: true,
            show_tab_bar: true,
            wrap: false,
            theme_border: Color::Gray,
            theme_border_title: Color::White,
            extraction_progress: None,
            archive_name: String::new(),
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal
            .draw(|f| f.render_widget(LogPanel { data: &data }, f.area()))
            .unwrap();
    }

    #[test]
    fn test_log_panel_widget_with_title_no_borders() {
        let data = LogPanelData {
            log_lines: vec![],
            num_visible: 0,
            visible_height: 10,
            start: 0,
            horizontal_scroll: 0,
            logs_title: "myfile.log (0)".to_string(),
            show_borders: false,
            show_tab_bar: false,
            wrap: true,
            theme_border: Color::Gray,
            theme_border_title: Color::White,
            extraction_progress: None,
            archive_name: String::new(),
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal
            .draw(|f| f.render_widget(LogPanel { data: &data }, f.area()))
            .unwrap();
    }

    #[test]
    fn test_log_panel_widget_borders_with_title() {
        let data = LogPanelData {
            log_lines: vec![Line::from("line a"), Line::from("line b")],
            num_visible: 10,
            visible_height: 5,
            start: 2,
            horizontal_scroll: 5,
            logs_title: "app.log (10)".to_string(),
            show_borders: true,
            show_tab_bar: false,
            wrap: false,
            theme_border: Color::Gray,
            theme_border_title: Color::Cyan,
            extraction_progress: None,
            archive_name: String::new(),
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal
            .draw(|f| f.render_widget(LogPanel { data: &data }, f.area()))
            .unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_with_comment_banner() {
        let mut app = make_app(&["line one", "line two", "line three"]).await;
        app.tabs[0]
            .log_manager
            .add_comment("my annotation".to_string(), vec![0]);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[test]
    fn test_prepare_comment_maps_empty() {
        let comments: Vec<(Vec<usize>, String)> = vec![];
        let visible = crate::ui::VisibleLines::default();
        let (banner_at, vis_comment_map) = prepare_comment_maps(&comments, &visible, 0, 0);
        assert!(banner_at.is_empty());
        assert!(vis_comment_map.is_empty());
    }

    #[tokio::test]
    async fn test_log_panel_visual_selection() {
        let mut app = make_app(&["line one", "line two", "line three"]).await;
        app.tabs[0].scroll.scroll_offset = 1;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_with_wrap_and_long_json() {
        let long_msg = "x".repeat(100);
        let line = format!(
            r#"{{"level":"INFO","msg":"{}","target":"myapp"}}"#,
            long_msg
        );
        let mut app = make_app(&[&line]).await;
        app.tabs[0].display.wrap = true;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_raw_mode_no_format() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello"}"#, "plain line"]).await;
        app.tabs[0].display.raw_mode = true;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_all_level_colors_disabled() {
        let mut app = make_app(&[
            "TRACE low",
            "DEBUG detail",
            "INFO info",
            "NOTICE note",
            "WARNING caution",
            "ERROR bad",
            "FATAL critical",
        ])
        .await;
        for level in &[
            "trace", "debug", "info", "notice", "warning", "error", "fatal",
        ] {
            app.tabs[0]
                .display
                .level_colors_disabled
                .insert(level.to_string());
        }
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_with_many_lines_scroll_offset() {
        let lines: Vec<String> = (0..50).map(|i| format!("line {}", i)).collect();
        let lines_ref: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let mut app = make_app(&lines_ref).await;
        app.tabs[0].scroll.scroll_offset = 40;
        app.tabs[0].scroll.viewport_offset = 38;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_show_tab_bar_suppresses_title() {
        let mut app = make_app(&["line one"]).await;
        app.tabs[0].log_manager =
            crate::db::LogManager::new(app.db.clone(), Some("test.log".to_string())).await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_comment_banner_with_line_numbers() {
        let mut app = make_app(&["line one", "line two", "line three"]).await;
        app.tabs[0]
            .log_manager
            .add_comment("note\ncontinuation".to_string(), vec![1]);
        app.tabs[0].display.show_line_numbers = true;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_json_with_hidden_fields() {
        let mut app = make_app(&[r#"{"level":"INFO","msg":"hello","target":"app"}"#]).await;
        app.tabs[0]
            .display
            .hidden_fields
            .insert("level".to_string());
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_tail_mode_label() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].stream.tail_mode = true;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[tokio::test]
    async fn test_log_panel_paused_label() {
        let mut app = make_app(&["line one", "line two"]).await;
        app.tabs[0].stream.paused = true;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    #[test]
    fn test_find_token_offset_partial_match() {
        assert_eq!(find_token_offset("helloworld foo", "world"), None);
        assert_eq!(find_token_offset("world hello", "world"), Some(0));
    }

    #[test]
    fn test_prepare_comment_maps_with_comments() {
        let comments = vec![(vec![1usize, 2usize], "note".to_string())];
        let visible = crate::ui::VisibleLines::Filtered(vec![0, 1, 2]);
        let (banner_at, vis_comment_map) = prepare_comment_maps(&comments, &visible, 0, 3);
        assert!(banner_at.contains_key(&1));
        assert!(vis_comment_map.contains_key(&1));
        assert!(vis_comment_map.contains_key(&2));
        assert!(!banner_at.contains_key(&2));
    }

    #[test]
    fn test_log_panel_widget_wrap_mode() {
        let data = LogPanelData {
            log_lines: vec![Line::from("a very long line that wraps")],
            num_visible: 1,
            visible_height: 5,
            start: 0,
            horizontal_scroll: 0,
            logs_title: String::new(),
            show_borders: false,
            show_tab_bar: false,
            wrap: true,
            theme_border: Color::Gray,
            theme_border_title: Color::White,
            extraction_progress: None,
            archive_name: String::new(),
        };
        let mut terminal = Terminal::new(TestBackend::new(20, 10)).unwrap();
        terminal
            .draw(|f| f.render_widget(LogPanel { data: &data }, f.area()))
            .unwrap();
    }

    #[test]
    fn test_log_panel_renders_extraction_progress_overlay() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let data = LogPanelData {
            log_lines: vec![],
            num_visible: 0,
            visible_height: 8,
            start: 0,
            horizontal_scroll: 0,
            logs_title: String::new(),
            show_borders: false,
            show_tab_bar: false,
            wrap: false,
            theme_border: ratatui::style::Color::White,
            theme_border_title: ratatui::style::Color::White,
            extraction_progress: Some(0.5),
            archive_name: "logs.tar.gz".to_string(),
        };
        terminal
            .draw(|frame| {
                frame.render_widget(LogPanel { data: &data }, frame.area());
            })
            .unwrap();
        let rendered = terminal.backend().buffer().clone();
        let has_progress = rendered
            .content()
            .iter()
            .any(|c| c.symbol() == "\u{2588}" || c.symbol() == "\u{2591}");
        assert!(
            has_progress,
            "expected progress bar characters in rendered output"
        );
    }
}
