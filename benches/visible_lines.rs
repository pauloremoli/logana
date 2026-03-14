use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use logana::file_reader::FileReader;
use logana::filters::{FilterDecision, FilterManager, build_filter};

fn plain_log_bytes(lines: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(lines * 90);
    for i in 0..lines {
        // Alternate INFO / ERROR so include/exclude filters have real work to do.
        let level = if i % 10 == 0 { "ERROR" } else { "INFO " };
        std::io::Write::write_fmt(
            &mut buf,
            format_args!("2024-01-01T00:00:00Z {level} myapp::server: request id={i} status=200\n"),
        )
        .unwrap();
    }
    buf
}

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

fn bench_compute_visible(c: &mut Criterion) {
    let mut group = c.benchmark_group("visible_lines/compute_visible");

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let reader = FileReader::from_bytes(plain_log_bytes(lines));
        group.throughput(Throughput::Elements(lines as u64));

        group.bench_with_input(
            BenchmarkId::new("no_filters", lines),
            &reader,
            |b, reader| b.iter(|| FilterManager::empty().compute_visible(reader)),
        );

        let fm_include = FilterManager::new(
            vec![build_filter("INFO", FilterDecision::Include, false, 0).unwrap()],
            true,
        );
        group.bench_with_input(
            BenchmarkId::new("one_include", lines),
            &reader,
            |b, reader| b.iter(|| fm_include.compute_visible(reader)),
        );

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

fn bench_compute_visible_combined(c: &mut Criterion) {
    let mut group = c.benchmark_group("visible_lines/compute_visible_combined");

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let reader = FileReader::from_bytes(plain_log_bytes(lines));
        group.throughput(Throughput::Elements(lines as u64));

        let fm_combined = FilterManager::new(
            vec![
                build_filter("INFO", FilterDecision::Exclude, false, 0).unwrap(),
                build_filter("ERROR", FilterDecision::Include, false, 1).unwrap(),
            ],
            true,
        );
        group.bench_with_input(
            BenchmarkId::new("exclude_info_include_error", lines),
            &reader,
            |b, reader| b.iter(|| fm_combined.compute_visible(reader)),
        );

        let fm_five = FilterManager::new(
            vec![
                build_filter("ERROR", FilterDecision::Include, false, 0).unwrap(),
                build_filter("WARN", FilterDecision::Include, false, 1).unwrap(),
                build_filter("INFO", FilterDecision::Exclude, false, 2).unwrap(),
                build_filter("DEBUG", FilterDecision::Exclude, false, 3).unwrap(),
                build_filter("myapp", FilterDecision::Exclude, false, 4).unwrap(),
            ],
            true,
        );
        group.bench_with_input(
            BenchmarkId::new("five_filters", lines),
            &reader,
            |b, reader| b.iter(|| fm_five.compute_visible(reader)),
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_collect_all_visible,
    bench_compute_visible,
    bench_compute_visible_combined
);
criterion_main!(benches);
