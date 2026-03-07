//! Benchmarks for the render pipeline optimisations.
//!
//! Four groups, each targeting a specific improvement:
//!
//! 1. `collect_field_names` — parser-level field discovery (the operation
//!    memoised by Item 3). Compares the raw cost of a fresh call against the
//!    cost of returning a cached `Vec` clone.
//!
//! 2. `date_filter_timestamp_parse` — parsing every visible line to extract a
//!    timestamp (the double-parse that Item 4 eliminates). The bench isolates
//!    `parse_line` × N lines so the regression from the second identical pass
//!    is visible.
//!
//! 3. `incremental_include_vs_full` — compares `FilterManager::compute_visible`
//!    (full O(file_lines) scan) against `VisibleLines::retain` (O(visible_lines)
//!    scan) when adding the first include filter to an already-filtered set
//!    (Item 2).
//!
//! 4. `render_line_pipeline` — the per-line render cost that the full Line
//!    cache (Item 1) would skip: `evaluate_line` + `render_line` +
//!    `colorize_known_values`.
//!
//! Run with:
//!   cargo bench --bench render_pipeline

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use logana::file_reader::FileReader;
use logana::filters::{FilterDecision, FilterManager, MatchCollector, build_filter, render_line};
use logana::parser::detect_format;
use logana::theme::ValueColors;
use logana::ui::VisibleLines;
use logana::value_colors::colorize_known_values;
use ratatui::style::Style;

// ---------------------------------------------------------------------------
// Data generators
// ---------------------------------------------------------------------------

fn json_log_bytes(lines: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(lines * 120);
    for i in 0..lines {
        let level = if i % 10 == 0 { "ERROR" } else { "INFO" };
        std::io::Write::write_fmt(
            &mut buf,
            format_args!(
                "{{\"ts\":\"2024-01-01T{:02}:{:02}:{:02}Z\",\"level\":\"{}\",\
                 \"target\":\"myapp::server\",\"msg\":\"request id={} status=200 latency=42ms\",\
                 \"req_id\":\"{:08x}\",\"pid\":{}}}\n",
                (i / 3600) % 24,
                (i / 60) % 60,
                i % 60,
                level,
                i,
                i,
                1000 + (i % 8),
            ),
        )
        .unwrap();
    }
    buf
}

/// Build a 256-slot style table suitable for `render_line`.
fn default_styles() -> Vec<Style> {
    vec![Style::default(); 256]
}

// ---------------------------------------------------------------------------
// 1. collect_field_names: raw call vs Vec clone (cache hit)
//
// The memoisation in Item 3 means repeated calls within the same
// filter/layout generation return a clone instead of reparsing up to 200
// lines through the format parser.
// ---------------------------------------------------------------------------

fn bench_collect_field_names(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_pipeline/collect_field_names");

    for &lines in &[50usize, 200] {
        let data = json_log_bytes(lines);
        let reader = FileReader::from_bytes(data);
        let sample: Vec<&[u8]> = (0..lines).map(|i| reader.get_line(i)).collect();
        let parser = detect_format(&sample).expect("JSON must be detected");
        let bench_lines: Vec<&[u8]> = sample.clone();

        group.throughput(Throughput::Elements(lines as u64));

        // Fresh call: re-parse all sample lines through the format parser.
        group.bench_with_input(
            BenchmarkId::new("fresh_call", lines),
            &bench_lines,
            |b, lines| b.iter(|| parser.collect_field_names(black_box(lines))),
        );

        // Cache hit: just clone the pre-computed Vec.
        let cached: Vec<String> = parser.collect_field_names(&bench_lines);
        group.bench_function(BenchmarkId::new("cache_hit_clone", lines), |b| {
            b.iter(|| black_box(cached.clone()))
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 2. date_filter_timestamp_parse: single pass vs double pass
//
// Before Item 4, refresh_visible and the first render frame each parsed every
// visible line independently to extract timestamps. The bench shows the cost
// of one pass vs two passes so the saving of pre-populating the parse cache
// during refresh_visible is quantified.
// ---------------------------------------------------------------------------

fn bench_date_filter_timestamp_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_pipeline/date_filter_parse");

    for &lines in &[1_000usize, 10_000, 100_000] {
        let data = json_log_bytes(lines);
        let reader = FileReader::from_bytes(data);
        let sample_limit = lines.min(200);
        let sample: Vec<&[u8]> = (0..sample_limit).map(|i| reader.get_line(i)).collect();
        let parser = detect_format(&sample).expect("JSON must be detected");

        group.throughput(Throughput::Elements(lines as u64));

        // Single pass: what refresh_visible pays when date filters are active
        // (and what the render frame now skips thanks to the pre-populated cache).
        group.bench_with_input(
            BenchmarkId::new("single_pass", lines),
            &reader,
            |b, reader| {
                b.iter(|| {
                    let mut hits = 0usize;
                    for i in 0..lines {
                        let line = reader.get_line(black_box(i));
                        if let Some(parts) = parser.parse_line(line) {
                            if parts.timestamp.is_some() {
                                hits += 1;
                            }
                        }
                    }
                    black_box(hits)
                })
            },
        );

        // Double pass: the old behaviour — refresh_visible then first render
        // both parse every visible line.
        group.bench_with_input(
            BenchmarkId::new("double_pass", lines),
            &reader,
            |b, reader| {
                b.iter(|| {
                    let mut hits = 0usize;
                    for i in 0..lines {
                        let line = reader.get_line(black_box(i));
                        if let Some(parts) = parser.parse_line(line) {
                            if parts.timestamp.is_some() {
                                hits += 1;
                            }
                        }
                    }
                    for i in 0..lines {
                        let line = reader.get_line(black_box(i));
                        if let Some(parts) = parser.parse_line(line) {
                            if parts.timestamp.is_some() {
                                hits += 1;
                            }
                        }
                    }
                    black_box(hits)
                })
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 3. incremental_include_vs_full: retain O(visible) vs compute_visible O(all)
//
// Scenario: a file with N total lines where 90 % are hidden by an exclude
// filter (visible set ≈ N/10). Adding the first include filter:
//   - Full path:        compute_visible scans all N lines.
//   - Incremental path: retain scans only the ≈ N/10 visible lines.
// ---------------------------------------------------------------------------

fn bench_incremental_include_vs_full(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_pipeline/incremental_include");

    for &total_lines in &[10_000usize, 100_000] {
        let data = json_log_bytes(total_lines);
        let reader = FileReader::from_bytes(data);

        // Exclude 90 % of lines (INFO), leaving only ERROR lines (~10 %).
        let exclude_fm = FilterManager::new(
            vec![build_filter("INFO", FilterDecision::Exclude, false, 0).unwrap()],
            false,
        );
        let pre_filtered: Vec<usize> = exclude_fm.compute_visible(&reader);
        let visible_count = pre_filtered.len();

        // Include filter being added.
        let include_filter = build_filter("ERROR", FilterDecision::Include, false, 0).unwrap();

        group.throughput(Throughput::Elements(total_lines as u64));

        // Full path: compute_visible rebuilds from scratch (scans all N lines).
        let include_fm = FilterManager::new(
            vec![
                build_filter("INFO", FilterDecision::Exclude, false, 0).unwrap(),
                build_filter("ERROR", FilterDecision::Include, false, 1).unwrap(),
            ],
            true,
        );
        group.bench_with_input(
            BenchmarkId::new(
                format!("full_compute_visible/{visible_count}vis"),
                total_lines,
            ),
            &reader,
            |b, reader| b.iter(|| include_fm.compute_visible(black_box(reader))),
        );

        // Incremental path: retain scans only the pre-filtered visible lines.
        group.bench_with_input(
            BenchmarkId::new(
                format!("incremental_retain/{visible_count}vis"),
                total_lines,
            ),
            &pre_filtered,
            |b, pre_filtered| {
                b.iter(|| {
                    let mut visible = VisibleLines::Filtered(black_box(pre_filtered.clone()));
                    visible.retain(|li| {
                        let line = reader.get_line(li);
                        let mut dummy = MatchCollector::new(line);
                        matches!(
                            include_filter.evaluate(line, &mut dummy),
                            FilterDecision::Include
                        )
                    });
                    black_box(visible)
                })
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 4. render_line_pipeline: per-line cost skipped by the full Line cache
//
// Measures the sequence that runs for every viewport line every render frame:
//   evaluate_line → render_line → colorize_known_values
// The full Line cache (Item 1) skips this entire sequence on cache hits.
// Individual sub-steps are also benched separately for attribution.
// ---------------------------------------------------------------------------

fn bench_render_line_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_pipeline/per_line");

    let data = json_log_bytes(1);
    let reader = FileReader::from_bytes(data);
    let line_bytes = reader.get_line(0);
    let styles = default_styles();
    let value_colors = ValueColors::default();

    // No active filters (common when browsing a large file without filtering).
    let fm_empty = FilterManager::empty();
    group.bench_function("no_filters/full_pipeline", |b| {
        b.iter(|| {
            let collector = fm_empty.evaluate_line(black_box(line_bytes));
            let rendered = render_line(&collector, &styles);
            black_box(colorize_known_values(rendered, &value_colors))
        })
    });

    // One include filter (Aho-Corasick, the common filtered case).
    let fm_one = FilterManager::new(
        vec![build_filter("ERROR", FilterDecision::Include, false, 0).unwrap()],
        true,
    );
    group.bench_function("one_filter/full_pipeline", |b| {
        b.iter(|| {
            let collector = fm_one.evaluate_line(black_box(line_bytes));
            let rendered = render_line(&collector, &styles);
            black_box(colorize_known_values(rendered, &value_colors))
        })
    });

    // Five filters (heavier realistic scenario).
    let fm_five = FilterManager::new(
        vec![
            build_filter("ERROR", FilterDecision::Include, false, 0).unwrap(),
            build_filter("WARN", FilterDecision::Include, false, 1).unwrap(),
            build_filter("myapp", FilterDecision::Include, false, 2).unwrap(),
            build_filter("debug", FilterDecision::Exclude, false, 3).unwrap(),
            build_filter("healthcheck", FilterDecision::Exclude, false, 4).unwrap(),
        ],
        true,
    );
    group.bench_function("five_filters/full_pipeline", |b| {
        b.iter(|| {
            let collector = fm_five.evaluate_line(black_box(line_bytes));
            let rendered = render_line(&collector, &styles);
            black_box(colorize_known_values(rendered, &value_colors))
        })
    });

    // Isolated sub-steps for cost attribution.
    group.bench_function("five_filters/evaluate_line_only", |b| {
        b.iter(|| fm_five.evaluate_line(black_box(line_bytes)))
    });
    group.bench_function("render_line_only", |b| {
        let collector = fm_empty.evaluate_line(line_bytes);
        b.iter(|| render_line(black_box(&collector), &styles))
    });
    group.bench_function("colorize_known_values_only", |b| {
        let collector = fm_empty.evaluate_line(line_bytes);
        b.iter(|| {
            let line = render_line(&collector, &styles);
            black_box(colorize_known_values(line, &value_colors))
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_collect_field_names,
    bench_date_filter_timestamp_parse,
    bench_incremental_include_vs_full,
    bench_render_line_pipeline,
);
criterion_main!(benches);
