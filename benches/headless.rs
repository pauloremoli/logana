use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use logana::db::Database;
use logana::file_reader::FileReader;
use logana::headless::run_headless_to_writer;
use logana::log_manager::LogManager;
use logana::types::FilterType;
use std::sync::Arc;

fn plain_log_bytes(lines: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(lines * 90);
    for i in 0..lines {
        let level = if i % 10 == 0 { "ERROR" } else { "INFO " };
        std::io::Write::write_fmt(
            &mut buf,
            format_args!("2024-01-01T00:00:00Z {level} myapp::server: request id={i} status=200\n"),
        )
        .unwrap();
    }
    buf
}

fn json_log_bytes(lines: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(lines * 120);
    for i in 0..lines {
        let level = if i % 10 == 0 { "error" } else { "info" };
        std::io::Write::write_fmt(
            &mut buf,
            format_args!("{{\"level\":\"{level}\",\"msg\":\"request id={i}\",\"status\":200}}\n"),
        )
        .unwrap();
    }
    buf
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn make_manager() -> LogManager {
    let db = Arc::new(Database::in_memory().await.unwrap());
    LogManager::new(db, None).await
}

async fn make_manager_with(filters: &[(&str, FilterType)]) -> LogManager {
    let db = Arc::new(Database::in_memory().await.unwrap());
    let mut lm = LogManager::new(db, None).await;
    for (pattern, ft) in filters {
        lm.add_filter_with_color((*pattern).to_string(), ft.clone(), None, None, true)
            .await;
    }
    lm
}

fn bench_headless_no_filters(c: &mut Criterion) {
    let mut group = c.benchmark_group("headless/no_filters");
    let runtime = rt();

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let data = plain_log_bytes(lines);
        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(BenchmarkId::from_parameter(lines), &data, |b, data| {
            b.iter(|| {
                let reader = FileReader::from_bytes(data.clone());
                let lm = runtime.block_on(make_manager());
                let mut sink = Vec::new();
                run_headless_to_writer(reader, lm, &mut sink).unwrap();
                sink
            })
        });
    }

    group.finish();
}

fn bench_headless_one_include(c: &mut Criterion) {
    let mut group = c.benchmark_group("headless/one_include");
    let runtime = rt();

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let data = plain_log_bytes(lines);
        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(BenchmarkId::from_parameter(lines), &data, |b, data| {
            b.iter(|| {
                let reader = FileReader::from_bytes(data.clone());
                let lm = runtime.block_on(make_manager_with(&[("ERROR", FilterType::Include)]));
                let mut sink = Vec::new();
                run_headless_to_writer(reader, lm, &mut sink).unwrap();
                sink
            })
        });
    }

    group.finish();
}

fn bench_headless_one_exclude(c: &mut Criterion) {
    let mut group = c.benchmark_group("headless/one_exclude");
    let runtime = rt();

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let data = plain_log_bytes(lines);
        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(BenchmarkId::from_parameter(lines), &data, |b, data| {
            b.iter(|| {
                let reader = FileReader::from_bytes(data.clone());
                let lm = runtime.block_on(make_manager_with(&[("INFO", FilterType::Exclude)]));
                let mut sink = Vec::new();
                run_headless_to_writer(reader, lm, &mut sink).unwrap();
                sink
            })
        });
    }

    group.finish();
}

fn bench_headless_include_and_exclude(c: &mut Criterion) {
    let mut group = c.benchmark_group("headless/include_and_exclude");
    let runtime = rt();

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let data = plain_log_bytes(lines);
        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(BenchmarkId::from_parameter(lines), &data, |b, data| {
            b.iter(|| {
                let reader = FileReader::from_bytes(data.clone());
                let lm = runtime.block_on(make_manager_with(&[
                    ("myapp", FilterType::Include),
                    ("DEBUG", FilterType::Exclude),
                ]));
                let mut sink = Vec::new();
                run_headless_to_writer(reader, lm, &mut sink).unwrap();
                sink
            })
        });
    }

    group.finish();
}

fn bench_headless_field_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("headless/field_filter");
    let runtime = rt();

    for &lines in &[10_000usize, 100_000] {
        let data = json_log_bytes(lines);
        group.throughput(Throughput::Elements(lines as u64));
        group.bench_with_input(BenchmarkId::from_parameter(lines), &data, |b, data| {
            b.iter(|| {
                let reader = FileReader::from_bytes(data.clone());
                let lm = runtime.block_on(make_manager_with(&[(
                    "@field:level:error",
                    FilterType::Include,
                )]));
                let mut sink = Vec::new();
                run_headless_to_writer(reader, lm, &mut sink).unwrap();
                sink
            })
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_headless_no_filters,
    bench_headless_one_include,
    bench_headless_one_exclude,
    bench_headless_include_and_exclude,
    bench_headless_field_filter,
);
criterion_main!(benches);
