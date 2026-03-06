//! Benchmarks for [`FileReader`] line indexing.
//!
//! Covers both the plain (no ANSI) and ANSI-stripped paths, and the
//! `from_bytes` (stdin) variant. These are the targets for the single-pass
//! ANSI+index optimisation.
//!
//! Run with:
//!   cargo bench --bench file_reader

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use logana::file_reader::FileReader;
use std::io::Write as _;
use tempfile::NamedTempFile;

// ---------------------------------------------------------------------------
// Data generators
// ---------------------------------------------------------------------------

fn plain_log_bytes(lines: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(lines * 90);
    for i in 0..lines {
        writeln!(
            buf,
            "2024-01-01T00:00:00Z INFO  myapp::server: processing request id={i} status=200 latency=42ms"
        )
        .unwrap();
    }
    buf
}

fn ansi_log_bytes(lines: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(lines * 110);
    for i in 0..lines {
        writeln!(
            buf,
            "\x1b[32m2024-01-01T00:00:00Z\x1b[0m \x1b[34mINFO\x1b[0m  myapp::server: request id={i} status=200"
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

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

/// `FileReader::new` — reads from a file path (OS page cache warm after first
/// iteration, so this measures the CPU-bound memchr scan + Vec allocation).
fn bench_new_plain(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_reader/new/plain");

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let data = plain_log_bytes(lines);
        let bytes = data.len() as u64;
        let tmp = write_tmp(&data);
        let path = tmp.path().to_str().unwrap().to_string();

        group.throughput(Throughput::Bytes(bytes));
        group.bench_with_input(BenchmarkId::from_parameter(lines), &path, |b, path| {
            b.iter(|| FileReader::new(path).unwrap())
        });
    }

    group.finish();
}

/// `FileReader::new` — ANSI path: memchr2 detection + strip_ansi + reindex.
fn bench_new_ansi(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_reader/new/ansi");

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let data = ansi_log_bytes(lines);
        let bytes = data.len() as u64;
        let tmp = write_tmp(&data);
        let path = tmp.path().to_str().unwrap().to_string();

        group.throughput(Throughput::Bytes(bytes));
        group.bench_with_input(BenchmarkId::from_parameter(lines), &path, |b, path| {
            b.iter(|| FileReader::new(path).unwrap())
        });
    }

    group.finish();
}

/// `FileReader::from_bytes` — in-memory path (stdin / test data).
/// Uses `iter_batched` so the Vec clone is outside the measured region.
fn bench_from_bytes_plain(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_reader/from_bytes/plain");

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let data = plain_log_bytes(lines);
        let bytes = data.len() as u64;

        group.throughput(Throughput::Bytes(bytes));
        group.bench_with_input(BenchmarkId::from_parameter(lines), &data, |b, data| {
            b.iter_batched(
                || data.clone(),
                |bytes| FileReader::from_bytes(bytes),
                BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

/// `FileReader::from_bytes` — ANSI path.
fn bench_from_bytes_ansi(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_reader/from_bytes/ansi");

    for &lines in &[10_000usize, 100_000, 1_000_000] {
        let data = ansi_log_bytes(lines);
        let bytes = data.len() as u64;

        group.throughput(Throughput::Bytes(bytes));
        group.bench_with_input(BenchmarkId::from_parameter(lines), &data, |b, data| {
            b.iter_batched(
                || data.clone(),
                |bytes| FileReader::from_bytes(bytes),
                BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_new_plain,
    bench_new_ansi,
    bench_from_bytes_plain,
    bench_from_bytes_ansi,
);
criterion_main!(benches);
