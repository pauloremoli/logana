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
use crate::theme::Theme;
use crate::ui::field_layout::{apply_field_layout, effective_row_count, reconstructed_line_text};
use crate::ui::{CachedParsedLine, TabState, VisibleLines};
use crate::utils::search::SearchResult;
use crate::value_colors::{
    VALUE_STYLE_HTTP_DELETE, VALUE_STYLE_HTTP_GET, VALUE_STYLE_HTTP_OTHER, VALUE_STYLE_HTTP_PATCH,
    VALUE_STYLE_HTTP_POST, VALUE_STYLE_HTTP_PUT, VALUE_STYLE_IP, VALUE_STYLE_STATUS_2XX,
    VALUE_STYLE_STATUS_3XX, VALUE_STYLE_STATUS_4XX, VALUE_STYLE_STATUS_5XX, VALUE_STYLE_UUID,
    collect_value_color_spans,
};

/// Picks the number shown in the line-number gutter for one row: the
/// absolute file line number, or — in relative mode, on rows other than the
/// selected one — the row's distance from the selected row.
fn line_number_for_row(
    line_idx: usize,
    abs_vis_idx: usize,
    current_scroll: usize,
    relative: bool,
) -> usize {
    if relative && abs_vis_idx != current_scroll {
        abs_vis_idx.abs_diff(current_scroll)
    } else {
        line_idx + 1
    }
}

fn prepend_line_number(
    line: Line<'static>,
    line_num: usize,
    line_number_width: usize,
    is_annotated: bool,
    comment_fg: Color,
    line_number_fg: Color,
    render_style: Style,
) -> Line<'static> {
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

/// Prepends a 1-char collapse indicator: `+` for a parent line whose
/// continuation lines are currently hidden (whether because
/// `collapse_continuations` is on or because `<` individually collapsed
/// just this entry), `-` for a parent individually flipped away from the
/// default via `<`/`>` but currently expanded, or a blank space otherwise.
/// Independent of `prepend_line_number`'s comment-bar slot so a line that's
/// both commented and collapsed can show both indicators.
fn prepend_collapse_indicator(
    line: Line<'static>,
    marker: Option<char>,
    fg: Color,
    render_style: Style,
) -> Line<'static> {
    let marker_span = match marker {
        Some(c) => Span::styled(c.to_string(), Style::default().fg(fg)),
        None => Span::raw(" "),
    };
    let mut all_spans = vec![marker_span];
    all_spans.extend(line.spans);
    Line::from(all_spans).style(render_style)
}

/// Whether — and how — to render the collapse indicator for file line
/// `line_idx`. `<`/`>` work regardless of `collapse_continuations`'s value
/// (see `TabState::set_continuation_collapsed`), so this always reflects
/// the entry's actual effective state — `default_collapsed XOR
/// overridden_groups.contains(line_idx)` — rather than gating on the
/// global default: `Some('+')` when currently collapsed, `Some('-')` when
/// individually flipped away from the default but currently expanded,
/// `None` when it's not a collapsible parent line or is expanded with no
/// override (the common, unaffected case — kept marker-free to avoid
/// clutter on ordinary multiline entries).
///
/// Uses raw `cmap` adjacency rather than the post-filter visible set, so a
/// marker can show even if every continuation line of that entry happens to
/// be excluded by an unrelated active filter — a known, accepted
/// simplification that mirrors how the comment-bar indicator already
/// ignores filter state.
fn collapse_indicator_for_line(
    default_collapsed: bool,
    cmap: Option<&[usize]>,
    overridden_groups: &HashSet<usize>,
    line_idx: usize,
) -> Option<char> {
    let cmap = cmap?;
    if cmap.get(line_idx) != Some(&line_idx) {
        return None; // not a parent line
    }
    if cmap.get(line_idx + 1) != Some(&line_idx) {
        return None; // no continuation lines
    }
    let overridden = overridden_groups.contains(&line_idx);
    if default_collapsed ^ overridden {
        Some('+')
    } else if overridden {
        Some('-')
    } else {
        None
    }
}

/// Prepends a merged tab's per-line source label — purely a visual gutter
/// decoration, exactly like `prepend_line_number`. Applied *after* all
/// content-based highlighting (char selection, search, filters) so the
/// label is never part of the addressable/matchable line text: it can't
/// shift word motions, search offsets, export, or yank, the same way line
/// numbers can't.
fn prepend_source_label(
    line: Line<'static>,
    label: &str,
    label_col_width: usize,
    label_fg: Color,
    render_style: Style,
) -> Line<'static> {
    let label_span = Span::styled(
        format!("{:<width$} ", label, width = label_col_width),
        Style::default().fg(label_fg),
    );
    let mut all_spans = vec![label_span];
    all_spans.extend(line.spans);
    Line::from(all_spans).style(render_style)
}

/// Row height a comment banner occupies once rendered: one row per line of
/// the comment's text, mirroring [`build_comment_banner_lines`].
pub(crate) fn comment_banner_row_count(text: &str) -> usize {
    text.lines().count()
}

/// Maps each commented file line to the index of its comment in `comments`,
/// independent of any viewport window.
fn build_line_comment_map(comments: &[(Vec<usize>, String)]) -> HashMap<usize, usize> {
    let mut line_cmt_map: HashMap<usize, usize> = HashMap::new();
    for (cmt_idx, (line_indices, _)) in comments.iter().enumerate() {
        for &li in line_indices {
            line_cmt_map.entry(li).or_insert(cmt_idx);
        }
    }
    line_cmt_map
}

pub(crate) fn prepare_comment_maps(
    comments: &[(Vec<usize>, String)],
    visible_indices: &VisibleLines,
    start: usize,
    end: usize,
) -> (HashMap<usize, usize>, HashMap<usize, usize>) {
    let line_cmt_map = build_line_comment_map(comments);
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
    comments: &[(Vec<usize>, String)],
) -> (usize, usize) {
    let parser = if raw_mode {
        None
    } else {
        tab.display.format.as_deref()
    };
    let use_wrap = wrap && inner_width > 0;
    let content_rows = |li: usize| -> usize {
        if use_wrap {
            effective_row_count(
                tab.file_reader.get_line(li),
                inner_width,
                parser,
                field_layout,
                hidden_fields,
                show_keys,
            )
        } else {
            1
        }
    };
    let line_cmt_map = build_line_comment_map(comments);
    // Row height of visible-position `pos`, plus its comment banner the first
    // time `seen` encounters that comment — mirrors how `prepare_comment_maps`
    // places a banner once per comment, at its first line within the window.
    let row_height = |pos: usize, seen: &mut HashSet<usize>| -> usize {
        let li = tab.filter.visible_indices.get(pos);
        let mut h = content_rows(li);
        if let Some(&cmt_idx) = line_cmt_map.get(&li)
            && seen.insert(cmt_idx)
        {
            h += comment_banner_row_count(&comments[cmt_idx].1);
        }
        h
    };
    // Grows a window forward from `from`, returning the exclusive end that
    // still fits within `visible_height` rows (always at least one line).
    let forward_fill = |from: usize| -> usize {
        let mut rows = 0usize;
        let mut seen: HashSet<usize> = HashSet::new();
        let mut e = from;
        while e < num_visible {
            let h = row_height(e, &mut seen);
            if rows + h > visible_height {
                break;
            }
            rows += h;
            e += 1;
        }
        if e == from && from < num_visible {
            e = from + 1;
        }
        e
    };
    // Grows a window backward from `upto` (exclusive), returning the
    // inclusive start that still fits within `visible_height` rows.
    let backward_fill = |upto: usize| -> usize {
        let mut rows = 0usize;
        let mut seen: HashSet<usize> = HashSet::new();
        let mut s = upto;
        while s > 0 {
            let h = row_height(s - 1, &mut seen);
            if rows + h > visible_height {
                break;
            }
            rows += h;
            s -= 1;
        }
        s
    };

    let new_viewport = if scroll_offset < viewport_offset {
        scroll_offset
    } else if num_visible == 0 {
        viewport_offset
    } else {
        let gap = scroll_offset.saturating_sub(viewport_offset);
        let overflowed = gap > visible_height || {
            let mut rows = 0usize;
            let mut seen: HashSet<usize> = HashSet::new();
            for pos in viewport_offset..=scroll_offset {
                rows += row_height(pos, &mut seen);
            }
            rows > visible_height
        };
        if overflowed {
            backward_fill(scroll_offset + 1).min(scroll_offset)
        } else {
            viewport_offset
        }
    };

    let start = new_viewport;
    let end = forward_fill(start);

    if end == num_visible && num_visible > 0 {
        let filled_start = backward_fill(num_visible);
        if filled_start < new_viewport {
            (filled_start, forward_fill(filled_start))
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

    // For merged tabs, resolve per-source parsers (source *labels* are
    // rendered separately, purely visually — see `prepend_source_label`;
    // they never enter `rendered`, so they can't affect word motions,
    // search, filters, export, or yank).
    let merged_entries_arc: Option<Arc<Vec<crate::ingestion::MergedEntry>>> =
        tab.file_reader.merged_entries().cloned();
    let merged_parsers: Option<Vec<Option<Arc<dyn crate::parser::LogFormatParser>>>> =
        tab.merged.as_ref().map(|m| m.source_parsers.clone());
    let has_any_parser = merged_parsers.is_some() || tab.display.format.is_some();
    let is_merged = merged_entries_arc.is_some();

    if raw_mode {
        return;
    }

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

        let parser: Option<&dyn crate::parser::LogFormatParser> =
            if let (Some(entries), Some(parsers)) =
                (merged_entries_arc.as_ref(), merged_parsers.as_ref())
            {
                entries
                    .get(line_idx)
                    .and_then(|e| parsers.get(e.source_idx))
                    .and_then(|p| p.as_deref())
            } else {
                tab.display.format.as_deref()
            };
        let year_override = if is_merged {
            None
        } else {
            tab.year_map.as_deref().map(|ym| ym.year_for_line(line_idx))
        };

        if has_any_parser
            && let Some(parser) = parser
            && let Some(parts) = parser.parse_line(line_bytes)
        {
            // Only use the parser's own field order/separators (e.g. a custom
            // schema's `{level}/{component}/{feature}` template) when there's
            // no explicit *reordered* column layout — genuinely moving a
            // field via `:select-fields` can't be represented in a fixed
            // template, so that still falls back to the generic column
            // layout. A hidden field, however, is handled by
            // `reconstructed_line_text` itself: it drops the field's value
            // and collapses the separator that follows it, instead of
            // disabling reconstruction outright. This is the same function
            // Visual Char Mode's word motions use (`render_line_text`), so
            // the two can never disagree about what's on screen.
            let reconstructed =
                reconstructed_line_text(parser, &parts, field_layout, hidden_fields);
            let cols = if reconstructed.is_none() {
                apply_field_layout(
                    &parts,
                    field_layout,
                    hidden_fields,
                    show_keys,
                    year_override,
                )
            } else {
                Vec::new()
            };
            let all_cols_hidden = reconstructed
                .as_deref()
                .map(str::is_empty)
                .unwrap_or_else(|| cols.is_empty());
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
            } else if let Some(recon) = &reconstructed {
                recon.clone()
            } else {
                cols.join(" ")
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
    for (line_idx, entry) in new_entries {
        tab.cache.parse.insert(line_idx, (cache_gen, entry));
    }
}

/// Classifies a raw captured `level` value via `parser`'s own mapping (e.g. a
/// custom schema's error/warning value overrides) when a parser is
/// available, otherwise via the built-in `LogLevel::parse_level` keywords.
fn classify_level(parser: Option<&dyn crate::parser::LogFormatParser>, raw: &str) -> LogLevel {
    parser
        .map(|p| p.classify_level(raw))
        .unwrap_or_else(|| LogLevel::parse_level(raw))
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
    let relative_line_numbers = tab.display.relative_line_numbers;
    let collapse_continuations = tab.display.collapse_continuations;
    let cmap = tab.active_continuation_map().cloned();
    let overridden_groups = tab.filter.overridden_groups.clone();
    let total_lines = tab.file_reader.line_count();
    let line_number_width = if show_line_numbers {
        total_lines.max(1).to_string().len()
    } else {
        0
    };
    let ln_prefix_width = if show_line_numbers {
        line_number_width + 3
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

    // Merged-tab source label, resolved per row purely for the visual
    // gutter prepend below — see `prepend_source_label`.
    let merged_entries_for_labels: Option<Arc<Vec<crate::ingestion::MergedEntry>>> =
        tab.file_reader.merged_entries().cloned();
    let merged_source_labels: Option<Vec<String>> =
        tab.merged.as_ref().map(|m| m.source_labels.clone());
    let merged_label_col_width: usize = tab.merged.as_ref().map(|m| m.label_col_width).unwrap_or(0);
    let source_label_hidden = hidden_fields.contains("source");

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

    let comments_for_render: Vec<(Vec<usize>, String)> = tab
        .comment_manager
        .get()
        .iter()
        .map(|a| (a.line_indices.clone(), a.text.clone()))
        .collect();

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
        &comments_for_render,
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
        let is_marked = tab.mark_manager.is_marked(line_idx);
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
            let format_parser = tab.display.format.as_deref();
            let level = cached
                .and_then(|c| c.level.as_deref())
                .map(|lvl| classify_level(format_parser, lvl))
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
                                return classify_level(format_parser, lvl);
                            }
                            // Parent not cached (outside viewport) — parse just
                            // the level from its raw bytes without full layout.
                            if let Some(parser) = format_parser
                                && let Some(parts) =
                                    parser.parse_line(tab.file_reader.get_line(parent))
                                && let Some(lvl) = parts.level
                            {
                                return classify_level(format_parser, lvl);
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
                                if !crate::filters::field_filter_matches(
                                    &ffs.field_filter,
                                    &parts,
                                    line_bytes,
                                ) {
                                    continue;
                                }
                                if !ffs.match_only {
                                    collector.push(0, rendered.len(), ffs.style_id);
                                    continue;
                                }
                                for (field, pattern) in &ffs.field_filter.conditions {
                                    if let Some(val) = crate::filters::resolve_field(field, &parts)
                                        .filter(|v| v.contains(pattern.as_str()))
                                        && let Some(pos) = rendered.find(val)
                                    {
                                        collector.push(pos, pos + val.len(), ffs.style_id);
                                    }
                                }
                                if let Some(text) = &ffs.field_filter.text
                                    && let Some(pos) = rendered.find(text.as_str())
                                {
                                    collector.push(pos, pos + text.len(), ffs.style_id);
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

        if !source_label_hidden
            && let Some(entries) = merged_entries_for_labels.as_ref()
            && let Some(labels) = merged_source_labels.as_ref()
            && let Some(entry) = entries.get(line_idx)
            && let Some(label) = labels.get(entry.source_idx)
        {
            line = prepend_source_label(
                line,
                label,
                merged_label_col_width,
                theme.line_number_fg,
                render_style,
            );
        }

        if show_line_numbers {
            let is_annotated = vis_comment_map.contains_key(&abs_vis_idx);
            let line_num =
                line_number_for_row(line_idx, abs_vis_idx, current_scroll, relative_line_numbers);
            line = prepend_line_number(
                line,
                line_num,
                line_number_width,
                is_annotated,
                theme.comment_fg,
                theme.line_number_fg,
                render_style,
            );

            let marker = collapse_indicator_for_line(
                collapse_continuations,
                cmap.as_deref().map(Vec::as_slice),
                &overridden_groups,
                line_idx,
            );
            line = prepend_collapse_indicator(line, marker, theme.line_number_fg, render_style);
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
    let is_temp = tab.is_temp_backed();
    let logs_title = if show_tab_bar {
        String::new()
    } else {
        format!(
            "{}{} ({}){}{}{}{}",
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
            if is_temp { " [TEMP]" } else { "" },
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

    #[test]
    fn test_collapse_indicator_none_when_default_expanded_no_override() {
        let cmap = vec![0usize, 0, 0];
        assert_eq!(
            collapse_indicator_for_line(false, Some(&cmap), &HashSet::new(), 0),
            None
        );
    }

    #[test]
    fn test_collapse_indicator_plus_on_collapsed_parent() {
        let cmap = vec![0usize, 0, 0];
        assert_eq!(
            collapse_indicator_for_line(true, Some(&cmap), &HashSet::new(), 0),
            Some('+')
        );
    }

    #[test]
    fn test_collapse_indicator_minus_on_expanded_parent() {
        let cmap = vec![0usize, 0, 0];
        let overridden: HashSet<usize> = [0].into_iter().collect();
        assert_eq!(
            collapse_indicator_for_line(true, Some(&cmap), &overridden, 0),
            Some('-')
        );
    }

    /// A parent line individually collapsed via `<` while the global
    /// default is expanded must still show `+`, regardless of
    /// `collapse_continuations` — the indicator must reflect the entry's
    /// real state, not gate on the global mode.
    #[test]
    fn test_collapse_indicator_plus_on_overridden_parent_when_default_expanded() {
        let cmap = vec![0usize, 0, 0];
        let overridden: HashSet<usize> = [0].into_iter().collect();
        assert_eq!(
            collapse_indicator_for_line(false, Some(&cmap), &overridden, 0),
            Some('+')
        );
    }

    #[test]
    fn test_collapse_indicator_none_on_continuation_line() {
        let cmap = vec![0usize, 0, 0];
        assert_eq!(
            collapse_indicator_for_line(true, Some(&cmap), &HashSet::new(), 1),
            None
        );
    }

    #[test]
    fn test_collapse_indicator_none_on_parent_without_continuations() {
        let cmap = vec![0usize, 1, 2];
        assert_eq!(
            collapse_indicator_for_line(true, Some(&cmap), &HashSet::new(), 1),
            None
        );
    }

    #[test]
    fn test_collapse_indicator_none_when_no_continuation_map() {
        assert_eq!(
            collapse_indicator_for_line(true, None, &HashSet::new(), 0),
            None
        );
    }

    /// Builds an `App` whose single tab is a merged view of `sources`
    /// (each `&[&str]` a source's raw lines, none of which detect a parser
    /// — the common real-world case this bug hit), labelled with `labels`.
    /// Lines are merged in source order (source 0's lines, then source 1's,
    /// ...) via a fixed dummy `sort_key` — good enough for rendering tests,
    /// which don't care about chronological order.
    async fn make_merged_app(sources: &[&[&str]], labels: &[&str]) -> App {
        let file_readers: Vec<FileReader> = sources
            .iter()
            .map(|lines| FileReader::from_bytes(lines.join("\n").into_bytes()))
            .collect();
        let mut entries = Vec::new();
        for (source_idx, lines) in sources.iter().enumerate() {
            for line_idx in 0..lines.len() {
                entries.push(crate::ingestion::MergedEntry {
                    sort_key: *b"2024-01-01 00:00:00.000",
                    source_idx,
                    line_idx,
                });
            }
        }
        let file_reader = FileReader::from_merged(Arc::new(entries), Arc::new(file_readers));

        let db = Arc::new(Database::in_memory().await.unwrap());
        let log_manager = LogManager::new(db, None).await;
        let mut app = App::builder(
            log_manager,
            file_reader,
            Theme::default(),
            Arc::new(Keybindings::default()),
        )
        .build()
        .await;

        // Auto-detect a parser per source from its own lines, mirroring what
        // `open_merge_tab` does in production (it reuses whatever format was
        // already detected for that source's own tab) — this exercises the
        // "line has a real parser" code path, not just the no-parser one.
        let source_parsers: Vec<Option<Arc<dyn crate::parser::LogFormatParser>>> = sources
            .iter()
            .map(|lines| {
                let byte_lines: Vec<&[u8]> = lines.iter().map(|l| l.as_bytes()).collect();
                crate::parser::detect_format(&byte_lines).map(Arc::from)
            })
            .collect();

        let label_col_width = labels.iter().map(|l| l.len()).max().unwrap_or(0);
        app.tabs[0].merged = Some(crate::ui::MergedState {
            source_tab_indices: Vec::new(),
            source_parsers,
            source_labels: labels.iter().map(|s| s.to_string()).collect(),
            source_line_counts: sources.iter().map(|l| l.len()).collect(),
            label_col_width,
            stopped: true,
            building: None,
        });
        app.tabs[0].display.format = None;
        app
    }

    #[tokio::test]
    async fn test_merged_tab_source_label_is_visual_only_not_in_parse_cache() {
        // The actual root cause of the reported bug: for a merged line whose
        // source *does* have a detected parser, the label used to get baked
        // directly into `CachedParsedLine.rendered` — the same text word
        // motions, search, and export treat as "the line". It must not be
        // there anymore; it's a separate, purely visual gutter span. Uses a
        // syslog-shaped line so a format actually gets detected and this
        // exercises the reconstructed/cols cache-population path (not the
        // no-parser fallback, which now leaves no cache entry at all).
        let mut app = make_merged_app(
            &[
                &["Jan  1 10:00:00 host1 app: first line"],
                &["Jan  1 10:00:01 host2 app: second line"],
            ],
            &["workerA.log", "workerB.log"],
        )
        .await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();

        for line_idx in 0..2 {
            let (_, cached) = app.tabs[0]
                .cache
                .parse
                .get(&line_idx)
                .unwrap_or_else(|| panic!("expected a cache entry for line {line_idx}"));
            assert!(
                !cached.rendered.contains("workerA.log")
                    && !cached.rendered.contains("workerB.log"),
                "source label leaked into the parsed/matchable line text: {:?}",
                cached.rendered
            );
        }
    }

    #[tokio::test]
    async fn test_merged_tab_source_label_still_shown_on_screen() {
        // The label must still be visible as a gutter decoration — only its
        // presence in the *matchable content* is the bug being fixed.
        let mut app = make_merged_app(&[&["hello world"]], &["workerA.log"]).await;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("workerA.log"));
        assert!(content.contains("hello world"));
    }

    #[tokio::test]
    async fn test_merged_tab_source_label_hidden_via_hidden_fields() {
        let mut app = make_merged_app(&[&["hello world"]], &["workerA.log"]).await;
        app.tabs[0]
            .display
            .hidden_fields
            .insert("source".to_string());
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(!content.contains("workerA.log"));
        assert!(content.contains("hello world"));
    }

    #[tokio::test]
    async fn test_merged_tab_visual_char_selection_highlights_content_not_label() {
        // End-to-end reproduction of the reported bug: Visual Char Mode's
        // `cursor_col` is computed against content-only text (no source
        // label), so the on-screen highlight it drives must land on the
        // first character of the actual content — never inside the label,
        // however long the label is.
        use crate::mode::visual_char_mode::VisualMode;

        let mut app = make_merged_app(&[&["hello world"]], &["a-very-long-worker-label.log"]).await;
        let line_text = crate::mode::visual_char_mode::display_line_text(&app.tabs[0]);
        assert_eq!(
            line_text, "hello world",
            "sanity: label must not be in line_text"
        );
        app.tabs[0].interaction.mode = Box::new(VisualMode::new(line_text));

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();

        let row_text = |y: u16| -> String {
            (0..buf.area.width)
                .map(|x| {
                    buf.cell(ratatui::prelude::Position::new(x, y))
                        .unwrap()
                        .symbol()
                        .to_string()
                })
                .collect()
        };
        let content_row = (0..buf.area.height)
            .find(|&y| row_text(y).contains("hello world"))
            .expect("content row not found");
        let row = row_text(content_row);
        let label_col = row
            .find("a-very-long-worker-label.log")
            .expect("label not found on screen") as u16;
        let content_col = row
            .find("hello world")
            .expect("content not found on screen") as u16;

        let cell_at = |x: u16| {
            buf.cell(ratatui::prelude::Position::new(x, content_row))
                .unwrap()
        };
        assert!(
            cell_at(content_col)
                .modifier
                .contains(ratatui::style::Modifier::REVERSED),
            "cursor at content position 0 must highlight the first content char, not somewhere else"
        );
        assert!(
            !cell_at(label_col)
                .modifier
                .contains(ratatui::style::Modifier::REVERSED),
            "the source label must never be highlighted by char-selection"
        );
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
        app.tabs[0].mark_manager.toggle(0);
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
            1,
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
            5,
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
    fn test_prepend_line_number_width_padding() {
        let line = Line::from("hello");
        let result = prepend_line_number(
            line,
            3,
            4,
            false,
            Color::Yellow,
            Color::Gray,
            Style::default(),
        );
        let num_span = &result.spans[1];
        assert_eq!(num_span.content, "   3 ");
    }

    #[test]
    fn test_line_number_for_row_absolute_when_relative_off() {
        assert_eq!(line_number_for_row(9, 5, 5, false), 10);
    }

    #[test]
    fn test_line_number_for_row_relative_distance_on_non_current_row() {
        // abs_vis_idx=8, current_scroll=5 -> distance 3, not the absolute line_idx+1.
        assert_eq!(line_number_for_row(99, 8, 5, true), 3);
    }

    #[test]
    fn test_line_number_for_row_absolute_on_current_row_even_when_relative() {
        assert_eq!(line_number_for_row(9, 5, 5, true), 10);
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
    async fn test_relative_line_numbers_end_to_end() {
        let mut app = make_app(&["line one", "line two", "line three"]).await;
        app.tabs[0].display.relative_line_numbers = true;
        app.tabs[0].scroll.scroll_offset = 2;
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains(" 2 line one"), "{content}");
        assert!(content.contains(" 1 line two"), "{content}");
        assert!(content.contains(" 3 line three"), "{content}");
    }

    /// End-to-end regression test for a plain, normally-opened file (no
    /// custom schema, no streaming): after `:collapse`, the parent line's
    /// gutter must show a literal `+` character right before its line
    /// number, and the hidden continuation lines must not appear at all.
    #[tokio::test]
    async fn test_collapse_indicator_renders_end_to_end_on_plain_file() {
        let mut app = make_app(&[
            "2024-07-24T10:00:00Z INFO request processed",
            "2024-07-24T10:00:01Z ERROR something failed",
            "    at module.function (file.rs:42)",
            "    at caller (file.rs:10)",
            "2024-07-24T10:00:02Z INFO another request",
        ])
        .await;
        assert!(
            app.tabs[0].continuation_map.is_some(),
            "format must be detected for this test to exercise the collapse indicator"
        );

        app.run_command("collapse").await.unwrap();
        assert_eq!(
            app.tab().filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1, 4],
            "lines 2 and 3 (continuations of line 1) must be hidden"
        );

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();

        assert!(
            content.contains("+ 2"),
            "expected a '+' immediately before line number 2's gutter, got: {content}"
        );
        assert!(
            !content.contains("module.function"),
            "the collapsed continuation line must not be rendered at all: {content}"
        );
    }

    /// Same end-to-end check as above, but via the normal-mode `<` key
    /// (per-entry collapse) instead of `:collapse` — must work without the
    /// global collapse mode ever being turned on.
    #[tokio::test]
    async fn test_collapse_indicator_renders_end_to_end_via_normal_mode_key() {
        use crate::mode::app_mode::Mode;
        use crate::mode::normal_mode::NormalMode;
        use crossterm::event::{KeyCode, KeyModifiers};

        let mut app = make_app(&[
            "2024-07-24T10:00:00Z INFO request processed",
            "2024-07-24T10:00:01Z ERROR something failed",
            "    at module.function (file.rs:42)",
            "    at caller (file.rs:10)",
            "2024-07-24T10:00:02Z INFO another request",
        ])
        .await;
        assert!(app.tabs[0].continuation_map.is_some());
        app.tabs[0].scroll.scroll_offset = 1; // cursor on the ERROR entry
        assert!(!app.tabs[0].display.collapse_continuations);

        Box::new(NormalMode::default())
            .handle_key(&mut app.tabs[0], KeyCode::Char('<'), KeyModifiers::NONE)
            .await;

        assert_eq!(
            app.tab().filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 1, 4]
        );

        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();

        assert!(
            content.contains("+ 2"),
            "expected a '+' immediately before line number 2's gutter, got: {content}"
        );
    }

    /// A nested, brace-structured multiline schema (mirroring a real HLAPI
    /// transaction log — a header, deeply nested `object { ... }` blocks,
    /// and a distinctly-worded `end_pattern` footer) combined with two
    /// active include filters, matching a real reported "+ doesn't show"
    /// setup. Confirms the gutter marker survives that combination.
    #[tokio::test]
    async fn test_collapse_indicator_renders_with_nested_schema_and_active_filters() {
        let cfg = crate::config::CustomSchemaConfig {
            name: "txn".to_string(),
            template: Some("### Transaction {id} started at: {ts}".to_string()),
            fields: [("id".to_string(), "extra".to_string())]
                .into_iter()
                .collect(),
            continuation: Some(crate::config::ContinuationConfig {
                end_pattern: Some("### Transaction ended at: {ended_at}".to_string()),
                fields: vec![crate::config::ContinuationFieldSpec {
                    template: Some("Status: {status}".to_string()),
                    fields: Default::default(),
                    pattern: None,
                    json: false,
                }],
            }),
            ..Default::default()
        };
        let parser = crate::parser::CustomParser::from_config(&cfg).unwrap();

        let mut app = make_app(&[
            "### Transaction 148 started at: 1980-01-06 00:00:22.234",
            "Status: FAILED",
            "object {",
            "  class_name: \"Widget\"",
            "  nested {",
            "    array: \"a\"",
            "  }",
            "}",
            "### Transaction ended at: 1980-01-06 00:00:22.297",
            "### Transaction 147 started at: 1980-01-06 00:00:22.234",
            "Status: SUCCESS",
            "object {",
            "  class_name: \"Widget\"",
            "}",
            "### Transaction ended at: 1980-01-06 00:00:22.297",
        ])
        .await;
        app.tabs[0].apply_format(Some(Arc::new(parser)));
        assert_eq!(
            app.tabs[0]
                .continuation_map
                .as_deref()
                .map(|c| c.as_slice()),
            Some([0usize, 0, 0, 0, 0, 0, 0, 0, 0, 9, 9, 9, 9, 9, 9].as_slice())
        );

        app.run_command("filter SUCCESS").await.unwrap();
        app.run_command("filter FAILED").await.unwrap();
        while app.tab().filter.handle.is_some() {
            app.advance_filter_computation();
            tokio::task::yield_now().await;
        }
        app.run_command("collapse").await.unwrap();
        assert_eq!(
            app.tab().filter.visible_indices.iter().collect::<Vec<_>>(),
            vec![0, 9]
        );

        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("+  1 ###"),
            "expected a '+' immediately before line number 1's gutter, got: {content}"
        );
        assert!(
            content.contains("+ 10 ###"),
            "expected a '+' immediately before line number 10's gutter, got: {content}"
        );
    }

    #[tokio::test]
    async fn test_collapse_indicator_renders_with_wrap_enabled() {
        let mut app = make_app(&[
            "2024-07-24T10:00:00Z INFO request processed",
            "2024-07-24T10:00:01Z ERROR something failed",
            "    at module.function (file.rs:42)",
            "    at caller (file.rs:10)",
            "2024-07-24T10:00:02Z INFO another request",
        ])
        .await;
        app.tabs[0].display.wrap = true;
        app.run_command("collapse").await.unwrap();
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("+ 2"),
            "expected a '+' immediately before line number 2's gutter with wrap on, got: {content}"
        );
    }

    #[tokio::test]
    async fn test_collapse_indicator_renders_with_active_filter() {
        let mut app = make_app(&[
            "2024-07-24T10:00:00Z INFO request processed",
            "2024-07-24T10:00:01Z ERROR something failed",
            "    at module.function (file.rs:42)",
            "    at caller (file.rs:10)",
            "2024-07-24T10:00:02Z INFO another request",
        ])
        .await;
        app.run_command("filter 2024").await.unwrap();
        for tab in &mut app.tabs {
            if let Some(mut h) = tab.filter.handle.take() {
                while let Some(chunk) = h.result_rx.recv().await {
                    if chunk.is_last {
                        break;
                    }
                }
            }
        }
        app.run_command("collapse").await.unwrap();
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("+ 2"),
            "expected a '+' immediately before line number 2's gutter with an \
             active filter, got: {content}"
        );
    }

    #[tokio::test]
    async fn test_log_panel_with_comment_banner() {
        let mut app = make_app(&["line one", "line two", "line three"]).await;
        app.tabs[0]
            .comment_manager
            .add("my annotation".to_string(), vec![0]);
        let mut terminal = make_terminal();
        terminal.draw(|f| app.ui(f)).unwrap();
    }

    /// Reproduces the bug where jumping far down the file (as `:next-error`
    /// / `:next-warning` do) with wrap enabled and a comment banner sitting
    /// inside the target window left the viewport miscalculated: the
    /// banner's extra rows weren't reserved, so the window computed as
    /// "fitting" `visible_height` actually overflowed it once rendered.
    #[tokio::test]
    async fn test_compute_viewport_reserves_room_for_comment_banner_in_wrap_mode() {
        // Every line is a single 40-char word — no spaces — so at
        // inner_width 20 it wraps to exactly 2 rows, deterministically.
        let lines: Vec<String> = (0..30).map(|_| "X".repeat(40)).collect();
        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let mut app = make_app(&line_refs).await;
        app.tabs[0].display.wrap = true;
        app.tabs[0]
            .comment_manager
            .add("note".to_string(), vec![18]);

        let tab = &app.tabs[0];
        let comments: Vec<(Vec<usize>, String)> = tab
            .comment_manager
            .get()
            .iter()
            .map(|a| (a.line_indices.clone(), a.text.clone()))
            .collect();
        let inner_width = 20;
        let visible_height = 10;
        let num_visible = tab.filter.visible_indices.len();
        let scroll_offset = 20;

        let (start, end) = compute_viewport(
            tab,
            scroll_offset,
            0, // stale viewport_offset from before the jump
            num_visible,
            visible_height,
            true,
            inner_width,
            &HashSet::new(),
            &tab.display.field_layout,
            false,
            false,
            &comments,
        );

        assert!(
            start <= scroll_offset && scroll_offset < end,
            "jump target {scroll_offset} must be inside the rendered viewport [{start}, {end})"
        );

        // Recompute the rows the renderer will actually use for [start, end)
        // — content rows plus the banner, counted once at its first line —
        // and confirm it never exceeds what actually fits on screen.
        let mut total_rows = 0usize;
        let mut seen_comment = false;
        for i in start..end {
            let li = tab.filter.visible_indices.get(i);
            total_rows += effective_row_count(
                tab.file_reader.get_line(li),
                inner_width,
                None,
                &tab.display.field_layout,
                &HashSet::new(),
                false,
            );
            if li == 18 && !seen_comment {
                total_rows += comment_banner_row_count("note");
                seen_comment = true;
            }
        }
        assert!(
            total_rows <= visible_height,
            "window [{start}, {end}) needs {total_rows} rows, more than fits in visible_height {visible_height}"
        );
    }

    /// Same bug as above, but without wrap: the fixed-height branch used
    /// plain index arithmetic that didn't know about comment banners either.
    #[tokio::test]
    async fn test_compute_viewport_reserves_room_for_comment_banner_without_wrap() {
        let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
        let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let mut app = make_app(&line_refs).await;
        app.tabs[0]
            .comment_manager
            .add("note".to_string(), vec![18]);

        let tab = &app.tabs[0];
        let comments: Vec<(Vec<usize>, String)> = tab
            .comment_manager
            .get()
            .iter()
            .map(|a| (a.line_indices.clone(), a.text.clone()))
            .collect();
        let visible_height = 10;
        let num_visible = tab.filter.visible_indices.len();
        let scroll_offset = 20;

        let (start, end) = compute_viewport(
            tab,
            scroll_offset,
            0,
            num_visible,
            visible_height,
            false,
            80,
            &HashSet::new(),
            &tab.display.field_layout,
            false,
            false,
            &comments,
        );

        assert!(
            start <= scroll_offset && scroll_offset < end,
            "jump target {scroll_offset} must be inside the rendered viewport [{start}, {end})"
        );
        // One row per line, plus the single-line banner once.
        let rows = (end - start)
            + usize::from((start..end).any(|i| tab.filter.visible_indices.get(i) == 18));
        assert!(
            rows <= visible_height,
            "window [{start}, {end}) needs {rows} rows, more than fits in visible_height {visible_height}"
        );
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
            .comment_manager
            .add("note\ncontinuation".to_string(), vec![1]);
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

    fn acme_parser() -> Arc<dyn crate::parser::LogFormatParser> {
        Arc::new(
            crate::parser::CustomParser::from_config(&crate::config::CustomSchemaConfig {
                name: "acme".to_string(),
                description: None,
                template: Some(
                    "{id} {service} <{timestamp}> {pid} {level}/{component}/{feature}, {message}"
                        .to_string(),
                ),
                pattern: None,
                fields: [
                    ("id".to_string(), "extra".to_string()),
                    ("service".to_string(), "target".to_string()),
                ]
                .into_iter()
                .collect(),
                levels: Default::default(),
                multiline: false,
                continuation: None,
                ..Default::default()
            })
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn test_log_panel_custom_schema_preserves_template_separators() {
        let line = "04 LINUX-0-syscon <2035-04-04T21:54:53.283856Z> 62A INF/Syscon/StartupMgr, StateChange: ok";
        let mut app = make_app(&[line]).await;
        app.tabs[0].display.format = Some(acme_parser());
        let mut terminal = Terminal::new(TestBackend::new(200, 24)).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("INF/Syscon/StartupMgr,"),
            "expected the schema's own separators in the rendered line: {content}"
        );
    }

    #[tokio::test]
    async fn test_log_panel_custom_schema_hidden_field_keeps_structure() {
        // Hiding a field must not disable template reconstruction outright —
        // its value drops and the template's own separator collapses with
        // it, but the OTHER separators (and fields) stay in template order.
        let line = "04 LINUX-0-syscon <2035-04-04T21:54:53.283856Z> 62A INF/Syscon/StartupMgr, StateChange: ok";
        let mut app = make_app(&[line]).await;
        app.tabs[0].display.format = Some(acme_parser());
        app.tabs[0]
            .display
            .hidden_fields
            .insert("component".to_string());
        let mut terminal = Terminal::new(TestBackend::new(200, 24)).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("INF/StartupMgr,"),
            "expected the hidden field's value and its separator to collapse, other separators kept: {content}"
        );
        assert!(
            !content.contains("Syscon"),
            "hidden field's value must not appear: {content}"
        );
    }

    #[tokio::test]
    async fn test_log_panel_custom_schema_reordered_columns_falls_back() {
        // A genuine reorder (an explicit column list whose order differs
        // from the template's own) can't be represented by the fixed
        // template, so it still falls back to the generic column layout.
        let line = "04 LINUX-0-syscon <2035-04-04T21:54:53.283856Z> 62A INF/Syscon/StartupMgr, StateChange: ok";
        let mut app = make_app(&[line]).await;
        app.tabs[0].display.format = Some(acme_parser());
        app.tabs[0].display.field_layout.columns = Some(vec![
            "message".to_string(),
            "level".to_string(),
            "component".to_string(),
        ]);
        let mut terminal = Terminal::new(TestBackend::new(200, 24)).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            !content.contains("INF/Syscon/StartupMgr,"),
            "an explicit reordered column list must fall back to columns, not the template: {content}"
        );
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
