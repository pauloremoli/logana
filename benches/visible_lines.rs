//! Benchmarks for visible-line computation.
//!
//! Two targets:
//!
//! 1. `collect_all_visible` — the `(0..n).collect::<Vec<usize>>()` call inside
//!    `refresh_visible` when no filters are active.  This is the baseline for
//!    the `VisibleLines::All` optimisation.
//!
//! 2. `compute_visible` — `FilterManager::compute_visible` with zero, one include,
//!    and one exclude filter.  Measures rayon parallel throughput.
//!
//! Run with:
//!   cargo bench --bench visible_lines

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use logana::file_reader::FileReader;
use logana::filters::{build_filter, FilterDecision, FilterManager};

// ---------------------------------------------------------------------------
// Data generator
// ---------------------------------------------------------------------------

fn plain_log_bytes(lines: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(lines * 90);
    for i in 0..lines {
        // Alternate INFO / ERROR so include/exclude filters have real work to do.
        let level = if i % 10 == 0 { "ERROR" } else { "INFO " };
        std::io::Write::write_fmt(
            &mut buf,
            format_args!(
                "2024-01-01T00:00:00Z {level} myapp::server: request id={i} status=200\n"
            ),
        )
        .unwrap();
    }
    buf
}

// ---------------------------------------------------------------------------
// Benchmark 1: collect_all_visible
//
// Isolates the cost of `(0..n).collect::<Vec<usize>>()` — the no-filter fast
// path in `refresh_visible`.  After the VisibleLines::All change, this call
// disappears entirely; comparing new vs old baselines shows the saving.
// ---------------------------------------------------------------------------

fn bench_collect_all_visible(c: &mut Criterion) {
    let mut group = c.benchmark_group("visible_lines/collect_all");

    for &n in &[10_000usize, 100_000, 1_000_000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| (0..black_box(n)).collect::<Vec<usize>>())
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark 2: FilterManager::compute_visible
// ---------------------------------------------------------------------------

fn bench_compute_visible(c: &mut Criterion) {
    let mut group = c.benchmark_group("visible_lines/compute_visible");

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let reader = FileReader::from_bytes(plain_log_bytes(lines));
        group.throughput(Throughput::Elements(lines as u64));

        // No filters — currently also calls collect() inside compute_visible
        // via the rayon parallel iterator.
        group.bench_with_input(
            BenchmarkId::new("no_filters", lines),
            &reader,
            |b, reader| b.iter(|| FilterManager::empty().compute_visible(reader)),
        );

        // One include filter (Aho-Corasick substring).
        let fm_include = FilterManager::new(
            vec![build_filter("INFO", FilterDecision::Include, false, 0).unwrap()],
            true,
        );
        group.bench_with_input(
            BenchmarkId::new("one_include", lines),
            &reader,
            |b, reader| b.iter(|| fm_include.compute_visible(reader)),
        );

        // One exclude filter.
        let fm_exclude = FilterManager::new(
            vec![build_filter("ERROR", FilterDecision::Exclude, false, 0).unwrap()],
            false,
        );
        group.bench_with_input(
            BenchmarkId::new("one_exclude", lines),
            &reader,
            |b, reader| b.iter(|| fm_exclude.compute_visible(reader)),
        );
    }

    group.finish();
}

criterion_group!(benches, bench_collect_all_visible, bench_compute_visible);
criterion_main!(benches);
