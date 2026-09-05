use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use logana::ingestion::file_reader::FileReader;
use logana::ingestion::paged_file::PagedFile;
use memchr::memmem;
use std::io::Write as _;
use tempfile::NamedTempFile;

const PAGE_SIZE: usize = 2 * 1024 * 1024;
const MAX_CACHED_PAGES: usize = 32;
const NEEDLE: &[u8] = b"status=500";

fn plain_log_bytes(lines: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(lines * 90);
    for i in 0..lines {
        let status = if i % 97 == 0 { 500 } else { 200 };
        writeln!(
            buf,
            "2024-01-01T00:00:00Z INFO  myapp::server: processing request id={i} status={status} latency=42ms"
        )
        .unwrap();
    }
    buf
}

fn write_tmp(data: &[u8]) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(data).unwrap();
    f.flush().unwrap();
    f
}

/// Index construction: `PagedFile::build_line_starts` (streaming, page-sized
/// chunks discarded after scanning) vs. `FileReader::new` (full
/// materialization + single-pass scan over the resident buffer).
fn bench_index_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("paged_file/index_construction");

    for &lines in &[100_000usize, 1_000_000] {
        let data = plain_log_bytes(lines);
        let bytes = data.len() as u64;
        let tmp = write_tmp(&data);
        let path = tmp.path().to_str().unwrap().to_string();

        group.throughput(Throughput::Bytes(bytes));
        group.bench_with_input(BenchmarkId::new("paged", lines), &path, |b, path| {
            b.iter(|| {
                let pf = PagedFile::open(path, PAGE_SIZE, MAX_CACHED_PAGES).unwrap();
                pf.build_line_starts().unwrap()
            })
        });
        group.bench_with_input(
            BenchmarkId::new("full_materialization", lines),
            &path,
            |b, path| b.iter(|| FileReader::new(path).unwrap()),
        );
    }

    group.finish();
}

/// Whole-file substring scan: chunked `PagedFile::read_range` + `memmem`
/// (approximates what a paged whole-file filter fast path would need) vs.
/// scanning the fully-resident `FileReader::data()` buffer directly.
fn bench_wholefile_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("paged_file/wholefile_scan");

    for &lines in &[100_000usize, 1_000_000] {
        let data = plain_log_bytes(lines);
        let bytes = data.len() as u64;
        let tmp = write_tmp(&data);
        let path = tmp.path().to_str().unwrap().to_string();

        group.throughput(Throughput::Bytes(bytes));
        group.bench_with_input(BenchmarkId::new("paged", lines), &path, |b, path| {
            b.iter(|| {
                let pf = PagedFile::open(path, PAGE_SIZE, MAX_CACHED_PAGES).unwrap();
                let total = pf.size() as usize;
                let mut offset = 0usize;
                let mut matches = 0usize;
                while offset < total {
                    let end = (offset + PAGE_SIZE).min(total);
                    let chunk = pf.read_range(offset..end).unwrap();
                    matches += memmem::find_iter(&chunk, NEEDLE).count();
                    offset = end;
                }
                matches
            })
        });
        group.bench_with_input(
            BenchmarkId::new("full_materialization", lines),
            &path,
            |b, path| {
                b.iter(|| {
                    let reader = FileReader::new(path).unwrap();
                    memmem::find_iter(reader.data(), NEEDLE).count()
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_index_construction, bench_wholefile_scan);
criterion_main!(benches);
