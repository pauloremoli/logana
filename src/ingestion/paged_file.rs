//! A disk-backed, bounded-memory file reader used for large files.
//!
//! Reads happen through `&self`, not `&mut self`: the page cache is an
//! [`ArcSwap`]-backed immutable snapshot, so concurrent readers (the
//! interactive TUI's parallel re-filter path, `FilterManager::compute_visible`,
//! reads many lines from many threads at once) never block on a lock. A
//! cache MISS reads the page from disk and republishes a new snapshot via
//! `rcu` (a lock-free compare-and-retry loop); a cache HIT never swaps
//! anything, it just clones an `Arc` out of the currently-published snapshot.
//! Safe against the file shrinking or being replaced after opening: a page
//! read that comes back shorter than expected surfaces as an `io::Error`
//! instead of the `SIGBUS` an mmap-backed reader would risk.

use arc_swap::ArcSwap;
use memchr::memchr_iter;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io;
use std::ops::Range;
use std::sync::Arc;

#[cfg(unix)]
pub(crate) fn pread(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

#[cfg(windows)]
pub(crate) fn pread(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buf, offset)
}

/// Immutable page-cache snapshot. FIFO eviction (not LRU) is deliberate: a
/// hit never needs to touch `order`, so it never needs to publish a new
/// snapshot — only misses do. Strict LRU would need to bump recency on every
/// hit too, forcing a swap per read and defeating the lock-free read path.
#[derive(Clone, Default)]
struct PageTable {
    pages: HashMap<usize, Arc<[u8]>>,
    order: VecDeque<usize>,
}

impl PageTable {
    fn with_page(&self, idx: usize, page: Arc<[u8]>, max_pages: usize) -> Self {
        if self.pages.contains_key(&idx) {
            return self.clone();
        }
        let mut pages = self.pages.clone();
        let mut order = self.order.clone();
        pages.insert(idx, page);
        order.push_back(idx);
        while pages.len() > max_pages {
            if let Some(evict) = order.pop_front() {
                pages.remove(&evict);
            } else {
                break;
            }
        }
        Self { pages, order }
    }
}

pub struct PagedFile {
    file: File,
    size: std::sync::atomic::AtomicU64,
    page_size: usize,
    max_cached_pages: usize,
    table: ArcSwap<PageTable>,
}

impl PagedFile {
    pub fn open(path: &str, page_size: usize, max_cached_pages: usize) -> io::Result<Self> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            file,
            size: std::sync::atomic::AtomicU64::new(size),
            page_size,
            max_cached_pages,
            table: ArcSwap::from_pointee(PageTable::default()),
        })
    }

    pub fn size(&self) -> u64 {
        self.size.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Bumps the known size after the underlying file has grown (tail-follow).
    /// Every already-cached page except possibly the last is unaffected —
    /// but if the *old* size fell mid-page, that page was cached short (it
    /// was the last, partial page); grown content landing in that same page
    /// would silently go missing without invalidating it first. `&self`, not
    /// `&mut self`, so growth can be applied through a shared `Arc<PagedFile>`
    /// — matching every other read/write path here, none of which need
    /// exclusive access.
    pub fn set_size(&self, new_size: u64) {
        let old_size = self.size();
        if old_size > 0 && !(old_size as usize).is_multiple_of(self.page_size) {
            let boundary_page = (old_size as usize) / self.page_size;
            self.invalidate_page(boundary_page);
        }
        self.size
            .store(new_size, std::sync::atomic::Ordering::Release);
    }

    /// Drops `page_idx` from the cache (if present) so the next read
    /// re-fetches it from disk. Lock-free, same `rcu` pattern as inserts.
    fn invalidate_page(&self, page_idx: usize) {
        self.table.rcu(move |old| {
            if !old.pages.contains_key(&page_idx) {
                return (**old).clone();
            }
            let mut pages = old.pages.clone();
            let mut order = old.order.clone();
            pages.remove(&page_idx);
            order.retain(|&i| i != page_idx);
            PageTable { pages, order }
        });
    }

    pub fn cached_page_count(&self) -> usize {
        self.table.load().pages.len()
    }

    /// Single streaming forward pass building the same `line_starts`
    /// convention as `FileReader` (index 0 is always the start of the file;
    /// every other entry is the byte right after a `\n`). Each chunk is
    /// discarded once scanned, so peak memory here is O(page size), not
    /// O(file size).
    pub fn build_line_starts(&self) -> io::Result<Vec<usize>> {
        let mut starts = vec![0usize];
        let mut offset: u64 = 0;
        let mut buf = vec![0u8; self.page_size];

        loop {
            let mut filled = 0usize;
            while filled < buf.len() {
                match pread(&self.file, &mut buf[filled..], offset + filled as u64) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            if filled == 0 {
                break;
            }
            for pos in memchr_iter(b'\n', &buf[..filled]) {
                starts.push(offset as usize + pos + 1);
            }
            offset += filled as u64;
            if filled < buf.len() {
                break;
            }
        }

        Ok(starts)
    }

    /// Byte range of line `idx` (without its trailing `\n`), given a
    /// `line_starts` table built by [`Self::build_line_starts`]. Mirrors
    /// `FileReader::line_byte_range`'s convention.
    pub fn line_byte_range(&self, line_starts: &[usize], idx: usize) -> Range<usize> {
        let start = line_starts[idx];
        let end = if idx + 1 < line_starts.len() {
            // `line_starts[idx + 1]` is always right after a '\n' by
            // construction, so subtracting 1 strips exactly that byte.
            line_starts[idx + 1] - 1
        } else {
            self.size() as usize
        };
        start..end
    }

    /// Returns line `idx`'s bytes as an owned, `Arc`-backed slice — never a
    /// borrow tied to `&self`, so a later eviction can't invalidate it, and
    /// safe to call concurrently from many threads at once.
    pub fn get_line(&self, line_starts: &[usize], idx: usize) -> io::Result<Arc<[u8]>> {
        let range = self.line_byte_range(line_starts, idx);
        self.read_owned(range)
    }

    /// Like [`Self::get_line`] but for an arbitrary byte span (models what a
    /// whole-file filter fast path needs per processed chunk).
    pub fn read_range(&self, range: Range<usize>) -> io::Result<Arc<[u8]>> {
        self.read_owned(range)
    }

    /// Like [`Self::read_range`], but bypasses the page cache entirely: one
    /// direct positional read into a single buffer, no per-page `Arc`
    /// allocation and no cache insert/eviction bookkeeping.
    ///
    /// For a range spanning many pages (a whole-file filter's chunk can be
    /// tens to hundreds of MB), going through the page cache costs far more
    /// than the read itself: each page is a guaranteed cache miss (a bulk
    /// sequential scan reads every byte exactly once — nothing is ever
    /// reused), yet the cached path still pays for a `vec![0u8; page_size]`
    /// allocation, an `Arc` wrap, and a page-table clone-and-swap *per
    /// page*, then copies each page's bytes into the caller's buffer on top
    /// of that. Measured on a 3.5GB file: this cut a 4-second chunked
    /// filter scan's `materialize_chunk` cost from ~4s to well under 1s.
    /// `get_line`/small random-access reads should keep using the cached
    /// path, where nearby repeated reads (viewport rendering) actually
    /// benefit from it.
    pub fn read_range_uncached(&self, range: Range<usize>) -> io::Result<Arc<[u8]>> {
        if range.start >= range.end {
            return Ok(Arc::from(Vec::new().into_boxed_slice()));
        }
        let len = range.end - range.start;
        let mut buf = vec![0u8; len];
        let mut filled = 0usize;
        while filled < len {
            match pread(
                &self.file,
                &mut buf[filled..],
                (range.start + filled) as u64,
            ) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if filled < len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("file shrank while reading range: expected {len} bytes, got {filled}"),
            ));
        }
        Ok(Arc::from(buf.into_boxed_slice()))
    }

    fn read_owned(&self, range: Range<usize>) -> io::Result<Arc<[u8]>> {
        if range.start >= range.end {
            return Ok(Arc::from(Vec::new().into_boxed_slice()));
        }
        let start_page = range.start / self.page_size;
        let end_page = (range.end - 1) / self.page_size;

        if start_page == end_page {
            let page = self.load_page(start_page)?;
            let base = start_page * self.page_size;
            return Ok(Arc::from(&page[range.start - base..range.end - base]));
        }

        let mut buf = Vec::with_capacity(range.end - range.start);
        for page_idx in start_page..=end_page {
            let page = self.load_page(page_idx)?;
            let base = page_idx * self.page_size;
            let lo = range.start.max(base) - base;
            let hi = range.end.min(base + page.len()) - base;
            buf.extend_from_slice(&page[lo..hi]);
        }
        Ok(Arc::from(buf.into_boxed_slice()))
    }

    fn expected_page_len(&self, page_idx: usize) -> usize {
        let size = self.size();
        let start = (page_idx * self.page_size) as u64;
        let end = ((page_idx + 1) * self.page_size) as u64;
        (end.min(size) - start.min(size)) as usize
    }

    fn load_page(&self, page_idx: usize) -> io::Result<Arc<[u8]>> {
        // Fast path: lock-free read of the currently-published snapshot.
        if let Some(page) = self.table.load().pages.get(&page_idx) {
            return Ok(Arc::clone(page));
        }

        // Slow path: read from disk, then publish a new snapshot via a
        // lock-free compare-and-retry loop. Concurrent misses on different
        // pages race safely — worst case is a duplicate read, never
        // corruption — since `rcu` retries against whatever the latest
        // snapshot is each time.
        let expected = self.expected_page_len(page_idx);
        let offset = (page_idx * self.page_size) as u64;
        let mut buf = vec![0u8; expected];
        let mut filled = 0usize;
        while filled < expected {
            match pread(&self.file, &mut buf[filled..], offset + filled as u64) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if filled < expected {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "file shrank while reading page {page_idx}: expected {expected} bytes, got {filled}"
                ),
            ));
        }

        let page: Arc<[u8]> = Arc::from(buf.into_boxed_slice());
        let max_pages = self.max_cached_pages;
        let for_rcu = Arc::clone(&page);
        self.table
            .rcu(move |old| old.with_page(page_idx, Arc::clone(&for_rcu), max_pages));
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write_tmp(content: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content).unwrap();
        f.flush().unwrap();
        f
    }

    fn naive_line_starts(data: &[u8]) -> Vec<usize> {
        let mut starts = vec![0usize];
        for pos in memchr_iter(b'\n', data) {
            let next = pos + 1;
            if next <= data.len() {
                starts.push(next);
            }
        }
        starts
    }

    fn sample_lines(n: usize) -> Vec<u8> {
        let mut buf = Vec::new();
        for i in 0..n {
            buf.extend_from_slice(format!("line-{i:06}-payload\n").as_bytes());
        }
        buf
    }

    #[test]
    fn test_build_line_starts_matches_naive_scan() {
        let data = sample_lines(500);
        let tmp = write_tmp(&data);
        let pf = PagedFile::open(tmp.path().to_str().unwrap(), 37, 4).unwrap();

        let starts = pf.build_line_starts().unwrap();
        assert_eq!(starts, naive_line_starts(&data));
    }

    #[test]
    fn test_build_line_starts_empty_file() {
        let tmp = write_tmp(b"");
        let pf = PagedFile::open(tmp.path().to_str().unwrap(), 64, 4).unwrap();
        assert_eq!(pf.build_line_starts().unwrap(), vec![0]);
    }

    #[test]
    fn test_get_line_within_single_page() {
        let data = sample_lines(50);
        let tmp = write_tmp(&data);
        let pf = PagedFile::open(tmp.path().to_str().unwrap(), 4096, 4).unwrap();
        let starts = pf.build_line_starts().unwrap();

        for idx in [0usize, 10, 49] {
            let range = naive_line_range(&data, &starts, idx);
            let expected = &data[range];
            let got = pf.get_line(&starts, idx).unwrap();
            assert_eq!(&*got, expected);
        }
    }

    #[test]
    fn test_get_line_spanning_page_boundary() {
        let data = sample_lines(200);
        let tmp = write_tmp(&data);
        // Small page size guarantees many lines straddle a page boundary.
        let pf = PagedFile::open(tmp.path().to_str().unwrap(), 23, 4).unwrap();
        let starts = pf.build_line_starts().unwrap();

        for idx in 0..starts.len() - 1 {
            let range = naive_line_range(&data, &starts, idx);
            let expected = &data[range];
            let got = pf.get_line(&starts, idx).unwrap();
            assert_eq!(&*got, expected, "line {idx} mismatch");
        }
    }

    #[test]
    fn test_get_line_after_eviction_reload() {
        let data = sample_lines(100);
        let tmp = write_tmp(&data);
        // page_size chosen so the file spans several pages; cache only 1.
        let pf = PagedFile::open(tmp.path().to_str().unwrap(), 64, 1).unwrap();
        let starts = pf.build_line_starts().unwrap();

        let first = pf.get_line(&starts, 0).unwrap().to_vec();
        assert_eq!(pf.cached_page_count(), 1);

        // Force eviction of page 0 by reading from a far later page.
        let last_idx = starts.len() - 2;
        let _ = pf.get_line(&starts, last_idx).unwrap();
        assert_eq!(pf.cached_page_count(), 1, "cache should stay bounded");

        // Re-reading line 0 must reload correctly, not return stale/garbage data.
        let reread = pf.get_line(&starts, 0).unwrap();
        assert_eq!(&*reread, first.as_slice());
    }

    #[test]
    fn test_set_size_invalidates_stale_short_last_page() {
        use std::io::Write as _;

        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        // Page size 64; write exactly one page's worth so the last (only)
        // page is cached at full length, then grow the file within a NEW
        // last page — this doesn't hit the "old page was short" case, so
        // also cover the case where old_size falls mid-page below.
        let initial = b"a".repeat(40); // less than page_size(64) -> short last page
        tmp.write_all(&initial).unwrap();
        tmp.flush().unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let pf = PagedFile::open(&path, 64, 4).unwrap();
        // Cache the short (40-byte) last page.
        let first_read = pf.read_range(0..40).unwrap();
        assert_eq!(first_read.len(), 40);
        assert_eq!(pf.cached_page_count(), 1);

        // Grow the file within the SAME page (still under 64 bytes total).
        let extra = b"b".repeat(20);
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(&extra).unwrap();
        f.flush().unwrap();

        pf.set_size(60);
        // The stale 40-byte cached page must have been invalidated — a
        // fresh read of the grown range must see the appended bytes, not a
        // cached-short-page miss/garbage.
        let grown_read = pf.read_range(0..60).unwrap();
        let mut expected = initial.clone();
        expected.extend_from_slice(&extra);
        assert_eq!(&*grown_read, expected.as_slice());
    }

    #[test]
    fn test_read_range_spanning_pages_is_correct() {
        let data = sample_lines(300);
        let tmp = write_tmp(&data);
        let pf = PagedFile::open(tmp.path().to_str().unwrap(), 50, 4).unwrap();

        let end = data.len();
        let got = pf.read_range(0..end).unwrap();
        assert_eq!(&*got, data.as_slice());
    }

    #[test]
    fn test_read_past_truncation_errors_instead_of_panicking() {
        let data = sample_lines(1000);
        let tmp = write_tmp(&data);
        let path = tmp.path().to_str().unwrap().to_string();
        let pf = PagedFile::open(&path, 256, 4).unwrap();
        let starts = pf.build_line_starts().unwrap();

        // Truncate the file out from under the already-open PagedFile —
        // simulates log rotation while logana is viewing/filtering it.
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(16).unwrap();

        let last_idx = starts.len() - 2;
        let result = pf.get_line(&starts, last_idx);
        assert!(
            result.is_err(),
            "reading a page past the new (shrunk) EOF must error, not panic or return garbage"
        );
    }

    #[test]
    fn test_concurrent_get_line_from_many_threads_is_correct() {
        use std::sync::Arc as StdArc;

        let data = sample_lines(5000);
        let tmp = write_tmp(&data);
        let pf = StdArc::new(PagedFile::open(tmp.path().to_str().unwrap(), 512, 8).unwrap());
        let starts = StdArc::new(pf.build_line_starts().unwrap());

        let handles: Vec<_> = (0..8)
            .map(|t| {
                let pf = StdArc::clone(&pf);
                let starts = StdArc::clone(&starts);
                let data = data.clone();
                std::thread::spawn(move || {
                    for i in 0..starts.len() - 1 {
                        // Every thread hammers every line — heavy overlap on
                        // the same pages is exactly the contention pattern
                        // `FilterManager::compute_visible`'s rayon fan-out
                        // produces against a real FileReader.
                        let idx = (i + t) % (starts.len() - 1);
                        let range = naive_line_range(&data, &starts, idx);
                        let got = pf.get_line(&starts, idx).unwrap();
                        assert_eq!(&*got, &data[range], "thread {t} line {idx} mismatch");
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
    }

    fn naive_line_range(data: &[u8], starts: &[usize], idx: usize) -> Range<usize> {
        let start = starts[idx];
        let end = if idx + 1 < starts.len() {
            starts[idx + 1] - 1
        } else {
            data.len()
        };
        start..end
    }

    #[cfg(target_os = "linux")]
    fn vm_rss_kb() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                return rest
                    .trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse()
                    .unwrap_or(0);
            }
        }
        0
    }

    /// Manual measurement, not asserted on: compares resident memory after
    /// (a) a full `PagedFile` streaming pass and (b) today's
    /// `FileReader::new` full-materialization, against the real large file
    /// from the memory-usage report this module was built to investigate.
    /// Run with `cargo test -- --ignored test_memory_vs_full_materialization`.
    #[test]
    #[ignore]
    #[cfg(target_os = "linux")]
    fn test_memory_vs_full_materialization_on_real_file() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let path = format!("{home}/logs/access.log");
        if !std::path::Path::new(&path).exists() {
            eprintln!("skipping: {path} not found");
            return;
        }

        let before_paged = vm_rss_kb();
        let pf = PagedFile::open(&path, 2 * 1024 * 1024, 32).unwrap();
        let starts = pf.build_line_starts().unwrap();
        let total = pf.size() as usize;
        let mut offset = 0usize;
        let chunk = 4 * 1024 * 1024;
        while offset < total {
            let end = (offset + chunk).min(total);
            let _ = pf.read_range(offset..end).unwrap();
            offset = end;
        }
        let after_paged = vm_rss_kb();
        let paged_line_count = starts.len().saturating_sub(1);
        drop(pf);

        let before_full = vm_rss_kb();
        let reader = crate::ingestion::file_reader::FileReader::new(&path).unwrap();
        let after_full = vm_rss_kb();

        println!(
            "PagedFile:  {paged_line_count} lines, RSS before={before_paged}KB after={after_paged}KB delta={}KB",
            after_paged.saturating_sub(before_paged)
        );
        println!(
            "FileReader: {} lines, RSS before={before_full}KB after={after_full}KB delta={}KB",
            reader.line_count(),
            after_full.saturating_sub(before_full)
        );
    }
}
