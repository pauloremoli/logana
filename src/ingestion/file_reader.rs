use crate::parser::dlt_binary;
#[cfg(target_os = "linux")]
use libc;
use memchr::{memchr_iter, memchr2, memchr3_iter};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs::File, io};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    spawn,
    sync::{oneshot, watch},
    task::spawn_blocking,
};

fn is_any_dlt_binary(data: &[u8]) -> bool {
    dlt_binary::is_dlt_binary(data) || dlt_binary::is_dlt_wire_format(data)
}

/// Files at or above this size use `Storage::Paged` (disk-backed, bounded
/// memory) instead of reading the whole file into one resident buffer.
pub const PAGED_STORAGE_THRESHOLD_BYTES: u64 = 256 * 1024 * 1024;
const PAGED_PAGE_SIZE: usize = 2 * 1024 * 1024;
const PAGED_MAX_CACHED_PAGES: usize = 64;
/// Fixed (not thread-count-scaled) chunk size for the parallel index-build
/// scan in `FileReader::try_new_paged`. Deliberately small and constant —
/// unlike `index_chunked`'s full-materialization scan, which sizes chunks as
/// `file_size / num_threads` because it keeps every byte anyway, this scan
/// discards each chunk after scanning, so peak extra memory here is bounded
/// by `num_threads * PAGED_INDEX_SCAN_CHUNK_SIZE`, not file size.
const PAGED_INDEX_SCAN_CHUNK_SIZE: usize = 8 * 1024 * 1024;

/// A line's bytes, returned by [`FileReader::get_line`]. `Borrowed` for
/// small/fully-resident storage (zero-copy, exactly as before); `Owned` for
/// `Storage::Paged`, where the bytes can't be borrowed from `&self` since a
/// later cache eviction could otherwise invalidate them.
#[derive(Clone)]
pub enum LineBytes<'a> {
    Borrowed(&'a [u8]),
    Owned(Arc<[u8]>),
}

impl<'a> std::ops::Deref for LineBytes<'a> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            LineBytes::Borrowed(b) => b,
            LineBytes::Owned(a) => a,
        }
    }
}

impl<'a> AsRef<[u8]> for LineBytes<'a> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl<'a> std::fmt::Debug for LineBytes<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

impl<'a> PartialEq for LineBytes<'a> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<'a> PartialEq<[u8]> for LineBytes<'a> {
    fn eq(&self, other: &[u8]) -> bool {
        **self == *other
    }
}

impl<'a> PartialEq<&[u8]> for LineBytes<'a> {
    fn eq(&self, other: &&[u8]) -> bool {
        **self == **other
    }
}

impl<'a, const N: usize> PartialEq<&[u8; N]> for LineBytes<'a> {
    fn eq(&self, other: &&[u8; N]) -> bool {
        **self == other[..]
    }
}

#[derive(Clone)]
pub struct VisibilityPredicate {
    fm: std::sync::Arc<crate::filters::FilterManager>,
}

impl VisibilityPredicate {
    pub fn new(fm: crate::filters::FilterManager) -> Self {
        Self {
            fm: std::sync::Arc::new(fm),
        }
    }

    pub fn is_visible(&self, line: &[u8]) -> bool {
        self.fm.is_visible(line)
    }
}

pub struct FileLoadResult {
    pub reader: FileReader,
    pub precomputed_visible: Option<Vec<usize>>,
    pub precomputed_text_counts: Option<Vec<usize>>,
}

pub struct FileLoadHandle {
    pub progress_rx: watch::Receiver<f64>,
    pub result_rx: oneshot::Receiver<io::Result<FileLoadResult>>,
    pub total_bytes: u64,
}

/// One line in a merged sorted view.  The `sort_key` is a 23-byte canonical
/// timestamp (`"YYYY-MM-DD HH:MM:SS.fff"`) used for lexicographic ordering.
#[derive(Clone)]
pub struct MergedEntry {
    pub sort_key: crate::filters::CanonicalTs,
    pub source_idx: usize,
    pub line_idx: usize,
}

#[derive(Clone)]
enum Storage {
    /// Heap-owned file content. The `File` handle stays open so
    /// `try_extend_from_read` can `pread` new bytes without re-opening the
    /// path. `inode`/`device` detect rotation-by-rename on Unix.
    File {
        data: Arc<Vec<u8>>,
        file: Arc<File>,
        path: Arc<std::path::PathBuf>,
        inode: u64,
        device: u64,
        mtime: Option<std::time::SystemTime>,
    },
    Bytes(Arc<Vec<u8>>),
    /// Virtual storage for a merged view: line access is delegated to one of
    /// the backing `sources` readers using the positional index into `entries`.
    Merged {
        entries: Arc<Vec<MergedEntry>>,
        sources: Arc<Vec<FileReader>>,
    },
    /// Disk-backed, bounded-memory storage for large files (see
    /// `PAGED_STORAGE_THRESHOLD_BYTES`). No resident byte buffer — reads go
    /// through `paged`'s page cache. `inode`/`device`/`mtime` mirror `File`'s
    /// rotation-detection fields, but growth (tail-follow) isn't supported
    /// yet: `try_extend_from_read` falls back to a full reload, same as
    /// `Bytes`/`Merged` today.
    Paged {
        paged: Arc<crate::ingestion::paged_file::PagedFile>,
        path: Arc<std::path::PathBuf>,
        inode: u64,
        device: u64,
        mtime: Option<std::time::SystemTime>,
    },
}

impl Storage {
    /// Panics for `Paged` — there is no resident buffer to hand back; use
    /// `FileReader::materialize_range` instead. Every caller of this method
    /// dispatches around `Paged` first, so this arm should be unreachable.
    fn as_bytes(&self) -> &[u8] {
        match self {
            Storage::File { data, .. } => data.as_slice(),
            Storage::Bytes(v) => v.as_slice(),
            Storage::Merged { .. } => &[],
            Storage::Paged { .. } => {
                unreachable!("Storage::Paged has no resident buffer; use materialize_range")
            }
        }
    }
}

#[derive(Clone)]
pub struct FileReader {
    storage: Storage,
    line_starts: std::sync::Arc<Vec<usize>>,
    pub is_binary: bool,
}

impl FileReader {
    pub fn new(path: &str) -> io::Result<Self> {
        let canonical_path = Arc::new(
            std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path)),
        );
        let file = Arc::new(File::open(path)?);
        let size = file.metadata()?.len() as usize;

        if size as u64 >= PAGED_STORAGE_THRESHOLD_BYTES
            && let Some(reader) = Self::try_new_paged(path, Arc::clone(&canonical_path))?
        {
            return Ok(reader);
        }

        // Parallel pread + MADV_POPULATE_WRITE, same as index_chunked.
        #[cfg(unix)]
        let data: Vec<u8> = {
            use rayon::prelude::*;
            use std::os::unix::fs::FileExt;
            let mut v = vec![0u8; size];
            // SAFETY: madvise only touches kernel page tables for this valid,
            // writable `size`-byte mapping; it never reads/writes user memory.
            // EINVAL on kernels < 5.14 is ignored (demand-paging fallback).
            #[cfg(target_os = "linux")]
            unsafe {
                libc::madvise(
                    v.as_mut_ptr() as *mut libc::c_void,
                    size,
                    libc::MADV_POPULATE_WRITE,
                );
            }
            let num_threads = rayon::current_num_threads().max(1);
            let chunk_size = size.div_ceil(num_threads).max(4 * 1024 * 1024);
            v.par_chunks_mut(chunk_size).enumerate().try_for_each(
                |(i, chunk)| -> io::Result<()> {
                    let offset = (i * chunk_size) as u64;
                    let mut filled = 0;
                    while filled < chunk.len() {
                        match file.read_at(&mut chunk[filled..], offset + filled as u64) {
                            Ok(0) => break,
                            Ok(n) => filled += n,
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                            Err(e) => return Err(e),
                        }
                    }
                    Ok(())
                },
            )?;
            v
        };
        #[cfg(not(unix))]
        let data: Vec<u8> = {
            use std::io::Read;
            let mut v = Vec::with_capacity(size);
            (&*file).read_to_end(&mut v)?;
            v
        };

        let len = data.len();

        if is_any_dlt_binary(&data) {
            let text = dlt_binary::convert_dlt_binary_to_text(&data);
            let mut reader = Self::from_bytes(text);
            reader.is_binary = true;
            return Ok(reader);
        }

        // Single pass: scan for '\n', '\x1b', '\r' simultaneously.
        let mut starts = vec![0usize];
        let mut has_ansi = false;
        for pos in memchr3_iter(b'\n', b'\x1b', b'\r', &data) {
            if data[pos] == b'\n' {
                let next = pos + 1;
                if next <= len {
                    starts.push(next);
                }
            } else {
                has_ansi = true;
                break;
            }
        }

        if has_ansi {
            let (stripped, line_starts) = strip_ansi_and_index(&data);
            return Ok(FileReader {
                storage: Storage::Bytes(std::sync::Arc::new(stripped)),
                line_starts: std::sync::Arc::new(line_starts),
                is_binary: false,
            });
        }

        // Capture file identity for rotation detection, and mtime for year inference.
        #[cfg(unix)]
        let (inode, device, mtime) = {
            use std::os::unix::fs::MetadataExt;
            let m = file.metadata()?;
            (m.ino(), m.dev(), m.modified().ok())
        };
        #[cfg(not(unix))]
        let (inode, device, mtime) = (
            0u64,
            0u64,
            file.metadata().ok().and_then(|m| m.modified().ok()),
        );

        Ok(FileReader {
            storage: Storage::File {
                data: Arc::new(data),
                file,
                path: canonical_path,
                inode,
                device,
                mtime,
            },
            line_starts: std::sync::Arc::new(starts),
            is_binary: false,
        })
    }

    /// Attempts to build a `Storage::Paged`-backed reader for a large,
    /// plain-text file via a streaming pass — bounded memory, never reads
    /// the whole file into one resident buffer. Returns `Ok(None)` if the
    /// file turns out to be DLT-binary or contains ANSI escapes/`\r`: both
    /// need a full materialization pass anyway
    /// (`convert_dlt_binary_to_text`/`strip_ansi_and_index`), so the caller
    /// falls back to the ordinary full-read path for those, exactly as
    /// before.
    pub(crate) fn try_new_paged(
        path: &str,
        canonical_path: Arc<std::path::PathBuf>,
    ) -> io::Result<Option<Self>> {
        use crate::ingestion::paged_file::{PagedFile, pread};
        use rayon::prelude::*;

        let file_handle = File::open(path)?;
        let paged = PagedFile::open(path, PAGED_PAGE_SIZE, PAGED_MAX_CACHED_PAGES)?;
        let total = paged.size() as usize;

        // Parallel pread + scan, same idea as `index_chunked`'s phase 1, but
        // with a small FIXED chunk size (not file_size / num_threads): each
        // chunk's bytes are discarded once scanned, so peak extra memory is
        // bounded by num_threads * PAGED_INDEX_SCAN_CHUNK_SIZE regardless of
        // file size, instead of needing the whole file resident.
        let chunk_size = PAGED_INDEX_SCAN_CHUNK_SIZE;
        let num_chunks = total.div_ceil(chunk_size).max(1);

        // Each element: (needs full-materialization fallback, i.e. DLT/ANSI
        // detected, absolute next-line offsets for this chunk).
        let chunk_results: Vec<io::Result<(bool, Vec<usize>)>> = (0..num_chunks)
            .into_par_iter()
            .map(|chunk_idx| -> io::Result<(bool, Vec<usize>)> {
                let chunk_start = chunk_idx * chunk_size;
                let this_len = chunk_size.min(total - chunk_start);
                let mut buf = vec![0u8; this_len];
                let mut filled = 0usize;
                while filled < buf.len() {
                    match pread(
                        &file_handle,
                        &mut buf[filled..],
                        (chunk_start + filled) as u64,
                    ) {
                        Ok(0) => break,
                        Ok(n) => filled += n,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(e) => return Err(e),
                    }
                }
                let chunk = &buf[..filled];
                let needs_fallback = (chunk_idx == 0 && is_any_dlt_binary(chunk))
                    || memchr2(b'\x1b', b'\r', chunk).is_some();
                let mut local_starts = Vec::new();
                if !needs_fallback {
                    for pos in memchr_iter(b'\n', chunk) {
                        let next = chunk_start + pos + 1;
                        if next <= total {
                            local_starts.push(next);
                        }
                    }
                }
                Ok((needs_fallback, local_starts))
            })
            .collect();
        let chunk_results = chunk_results.into_iter().collect::<io::Result<Vec<_>>>()?;

        if chunk_results
            .iter()
            .any(|(needs_fallback, _)| *needs_fallback)
        {
            return Ok(None);
        }

        // Chunks are non-overlapping and processed in order, so simple
        // concatenation preserves ascending order — no sort needed.
        let mut starts =
            Vec::with_capacity(1 + chunk_results.iter().map(|(_, v)| v.len()).sum::<usize>());
        starts.push(0usize);
        for (_, local) in chunk_results {
            starts.extend(local);
        }

        #[cfg(unix)]
        let (inode, device, mtime) = {
            use std::os::unix::fs::MetadataExt;
            let m = file_handle.metadata()?;
            (m.ino(), m.dev(), m.modified().ok())
        };
        #[cfg(not(unix))]
        let (inode, device, mtime) = (
            0u64,
            0u64,
            file_handle.metadata().ok().and_then(|m| m.modified().ok()),
        );

        Ok(Some(FileReader {
            storage: Storage::Paged {
                paged: Arc::new(paged),
                path: canonical_path,
                inode,
                device,
                mtime,
            },
            line_starts: Arc::new(starts),
            is_binary: false,
        }))
    }

    /// Build a `FileReader` from an in-memory byte buffer (e.g. stdin content).
    pub fn from_bytes(data: Vec<u8>) -> Self {
        let has_ansi = memchr2(b'\x1b', b'\r', &data).is_some();
        let mut starts = vec![0usize];
        if !has_ansi {
            for pos in memchr_iter(b'\n', &data) {
                let next = pos + 1;
                if next <= data.len() {
                    starts.push(next);
                }
            }
        }

        if has_ansi {
            let (stripped, line_starts) = strip_ansi_and_index(&data);
            return FileReader {
                storage: Storage::Bytes(std::sync::Arc::new(stripped)),
                line_starts: std::sync::Arc::new(line_starts),
                is_binary: false,
            };
        }

        FileReader {
            storage: Storage::Bytes(std::sync::Arc::new(data)),
            line_starts: std::sync::Arc::new(starts),
            is_binary: false,
        }
    }

    /// Build a `FileReader` backed by a merged sorted index over multiple
    /// sources. Line access is delegated to the source reader named by the
    /// compound key in each `MergedEntry`.
    pub fn from_merged(entries: Arc<Vec<MergedEntry>>, sources: Arc<Vec<FileReader>>) -> Self {
        let line_count = entries.len();
        FileReader {
            storage: Storage::Merged { entries, sources },
            line_starts: Arc::new((0..=line_count).collect()),
            is_binary: false,
        }
    }

    /// Read the last `preview_bytes` of `path` and return a `FileReader`
    /// with just those lines — the `--tail` fast path, showing the file's
    /// end immediately while the full background index builds. Seeking
    /// near the end keeps this fast regardless of file size; the first
    /// (likely partial) line of the read chunk is dropped.
    pub async fn from_file_tail(path: &str, preview_bytes: u64) -> io::Result<Self> {
        let mut file = tokio::fs::File::open(path).await?;
        let total_len = file.metadata().await?.len();

        // DLT binary: read the whole file and convert, then take tail lines.
        // Peek the start to check for the DLT magic.
        let mut magic_buf = [0u8; 4];
        let is_binary = if total_len >= 4 {
            file.read_exact(&mut magic_buf).await?;
            file.seek(io::SeekFrom::Start(0)).await?;
            is_any_dlt_binary(&magic_buf)
        } else {
            false
        };

        if is_binary {
            let mut full_buf = vec![0u8; total_len as usize];
            file.read_exact(&mut full_buf).await?;
            let text = dlt_binary::convert_dlt_binary_to_text(&full_buf);
            let mut reader = Self::from_bytes(text);
            reader.is_binary = true;
            return Ok(reader);
        }

        let offset = total_len.saturating_sub(preview_bytes);
        file.seek(io::SeekFrom::Start(offset)).await?;
        let read_len = (total_len - offset) as usize;
        let mut buf = vec![0u8; read_len];
        file.read_exact(&mut buf).await?;
        // Drop the first (likely partial) line so every line is complete.
        let start = if offset > 0 {
            buf.iter()
                .position(|&b| b == b'\n')
                .map(|p| p + 1)
                .unwrap_or(buf.len())
        } else {
            0
        };
        Ok(Self::from_bytes(buf[start..].to_vec()))
    }

    /// Read the first `preview_bytes` of `path` and return a `FileReader`
    /// with just those complete lines — the non-tail fast path, showing the
    /// file's start while the full background index builds. The last
    /// (likely partial) line of the read chunk is dropped.
    pub async fn from_file_head(path: &str, preview_bytes: u64) -> io::Result<Self> {
        let mut file = tokio::fs::File::open(path).await?;
        let total_len = file.metadata().await?.len();
        let read_len = total_len.min(preview_bytes) as usize;
        let mut buf = vec![0u8; read_len];
        file.read_exact(&mut buf).await?;

        if is_any_dlt_binary(&buf) {
            let text = dlt_binary::convert_dlt_binary_to_text(&buf);
            let mut reader = Self::from_bytes(text);
            reader.is_binary = true;
            return Ok(reader);
        }

        // Truncate to the last complete line so no partial line leaks out.
        if let Some(last_nl) = buf.iter().rposition(|&b| b == b'\n') {
            buf.truncate(last_nl + 1);
        } else {
            buf.clear();
        }
        Ok(Self::from_bytes(buf))
    }

    /// Extend this reader from a growing file, scanning only new bytes via `pread`.
    ///
    /// Returns `true` when incremental extension succeeded (file grew or unchanged).
    /// Returns `false` when:
    ///   - storage is `Bytes` (ANSI/DLT) — caller falls back to `FileReader::new()`.
    ///   - file was truncated (`new_size < old_size`) — caller does a full reload.
    ///   - file identity changed (inode/device mismatch) — caller does a full reload.
    pub fn try_extend_from_read(&mut self) -> io::Result<bool> {
        if let Storage::Paged {
            paged,
            path,
            inode,
            device,
            ..
        } = &self.storage
        {
            return self.try_extend_paged(Arc::clone(paged), Arc::clone(path), *inode, *device);
        }

        let (file, data, path, old_size, old_inode, old_device, old_mtime) = match &self.storage {
            Storage::File {
                file,
                data,
                path,
                inode,
                device,
                mtime,
            } => (
                Arc::clone(file),
                Arc::clone(data),
                Arc::clone(path),
                data.len(),
                *inode,
                *device,
                *mtime,
            ),
            // ANSI/merged storage still has no incremental-extend path — a
            // full reload is needed for those.
            Storage::Bytes(_) | Storage::Merged { .. } | Storage::Paged { .. } => {
                return Ok(false);
            }
        };

        // Stat the path (not the fd) so rotation-by-rename is detectable:
        // after `mv app.log app.log.1`, fstat on the old fd still reports the
        // original inode, but stat on the path sees the new file's inode.
        #[cfg(unix)]
        let (new_size, current_inode, current_device) = {
            use std::os::unix::fs::MetadataExt;
            match std::fs::metadata(&*path) {
                Ok(m) => (m.len() as usize, m.ino(), m.dev()),
                Err(_) => return Ok(false), // path gone — rotation or deletion
            }
        };
        #[cfg(not(unix))]
        let (new_size, current_inode, current_device) = {
            let sz = file
                .metadata()
                .map(|m| m.len() as usize)
                .unwrap_or(old_size);
            (sz, 0u64, 0u64)
        };

        // Inode/device mismatch: file was replaced (rotation by rename).
        #[cfg(unix)]
        if current_inode != old_inode || current_device != old_device {
            return Ok(false);
        }
        #[cfg(not(unix))]
        let _ = (old_inode, old_device, current_inode, current_device);

        if new_size == old_size {
            return Ok(true);
        }
        if new_size < old_size {
            // Truncation: caller must do a full reload.
            return Ok(false);
        }

        // Obtain an owned Vec, avoiding a copy when this is the only Arc handle.
        let mut new_data = Arc::try_unwrap(data).unwrap_or_else(|arc| (*arc).clone());

        let starts = Arc::make_mut(&mut self.line_starts);

        // Read only the new bytes via pread and append them to the buffer.
        #[cfg(unix)]
        {
            let extra = new_size - old_size;
            use std::os::unix::fs::FileExt;
            let mut buf = vec![0u8; extra];
            file.read_at(&mut buf, old_size as u64)?;
            for pos in memchr_iter(b'\n', &buf) {
                starts.push(old_size + pos + 1);
            }
            new_data.extend_from_slice(&buf);
        }
        #[cfg(not(unix))]
        {
            use std::io::{Read, Seek};
            (&*file).seek(io::SeekFrom::Start(old_size as u64))?;
            (&*file).read_to_end(&mut new_data)?;
            for pos in memchr_iter(b'\n', &new_data[old_size..]) {
                starts.push(old_size + pos + 1);
            }
        }

        self.storage = Storage::File {
            data: Arc::new(new_data),
            file,
            path,
            inode: current_inode,
            device: current_device,
            mtime: old_mtime,
        };
        Ok(true)
    }

    /// `Storage::Paged` counterpart of `try_extend_from_read`'s growth
    /// handling. Unlike `File`, there's no resident buffer to reallocate:
    /// just bump the known size and stream-scan the newly-added bytes for
    /// `\n` (via `paged.read_range`, which pulls only that span from disk).
    /// Already-cached pages stay valid since content before the old size
    /// never changes.
    fn try_extend_paged(
        &mut self,
        paged: Arc<crate::ingestion::paged_file::PagedFile>,
        path: Arc<std::path::PathBuf>,
        old_inode: u64,
        old_device: u64,
    ) -> io::Result<bool> {
        let old_size = paged.size();

        #[cfg(unix)]
        let (new_size, current_inode, current_device) = {
            use std::os::unix::fs::MetadataExt;
            match std::fs::metadata(&*path) {
                Ok(m) => (m.len(), m.ino(), m.dev()),
                Err(_) => return Ok(false), // path gone — rotation or deletion
            }
        };
        #[cfg(not(unix))]
        let (new_size, current_inode, current_device) = {
            let sz = std::fs::metadata(&*path)
                .map(|m| m.len())
                .unwrap_or(old_size);
            (sz, 0u64, 0u64)
        };

        #[cfg(unix)]
        if current_inode != old_inode || current_device != old_device {
            return Ok(false);
        }
        #[cfg(not(unix))]
        let _ = (old_inode, old_device, current_inode, current_device);

        if new_size == old_size {
            return Ok(true);
        }
        if new_size < old_size {
            // Truncation: caller must do a full reload.
            return Ok(false);
        }

        paged.set_size(new_size);
        let new_bytes = paged.read_range(old_size as usize..new_size as usize)?;

        let starts = Arc::make_mut(&mut self.line_starts);
        for pos in memchr_iter(b'\n', &new_bytes) {
            starts.push(old_size as usize + pos + 1);
        }
        Ok(true)
    }

    /// Stream stdin asynchronously, appending complete lines to a temp file
    /// every second. Returns a `watch::Receiver<()>` that fires on each
    /// write and the `NamedTempFile` owning the bytes. When stdin closes,
    /// the sender drops — callers detect this via `has_changed() == Err(_)`.
    pub async fn stream_stdin() -> (watch::Receiver<()>, tempfile::NamedTempFile) {
        use std::io::Write as _;
        use std::time::Duration;

        let temp_file =
            tempfile::NamedTempFile::new().expect("failed to create temp file for stdin stream");
        let temp_path = temp_file.path().to_owned();
        let (snapshot_tx, snapshot_rx) = watch::channel(());

        spawn(async move {
            use tokio::io::AsyncReadExt;

            let mut stdin = tokio::io::stdin();
            let mut partial: Vec<u8> = Vec::new();
            let mut buf = vec![0u8; 4096];
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await; // skip the initial immediate tick

            loop {
                tokio::select! {
                    result = stdin.read(&mut buf) => {
                        match result {
                            Ok(0) | Err(_) => {
                                if !partial.is_empty()
                                    && let Ok(mut f) = std::fs::OpenOptions::new()
                                        .append(true)
                                        .open(&temp_path)
                                {
                                    let _ = f.write_all(&partial);
                                    let _ = f.flush();
                                }
                                let _ = snapshot_tx.send(());
                                return;
                            }
                            Ok(n) => partial.extend_from_slice(&buf[..n]),
                        }
                    }
                    _ = interval.tick() => {
                        if let Some(last_nl) = partial.iter().rposition(|&b| b == b'\n')
                            && let Ok(mut f) = std::fs::OpenOptions::new()
                                .append(true)
                                .open(&temp_path)
                        {
                            let complete = partial[..=last_nl].to_vec();
                            partial.drain(..=last_nl);
                            if f.write_all(&complete).is_ok() {
                                let _ = f.flush();
                                let _ = snapshot_tx.send(());
                            }
                        }
                    }
                }
            }
        });

        (snapshot_rx, temp_file)
    }

    /// Start loading `path` on tokio's blocking thread pool. Returns a
    /// [`FileLoadHandle`] immediately; indexing happens in the background,
    /// polled via `handle.result_rx`/`progress_rx`.
    ///
    /// `predicate`, when `Some`, is tested per line during indexing and
    /// stored in [`FileLoadResult::precomputed_visible`], skipping a
    /// separate `compute_visible` call. `tail` evaluates it from the last
    /// line backward so tail lines are confirmed first; the result is
    /// still returned in ascending order.
    pub async fn load(
        path: String,
        predicate: Option<VisibilityPredicate>,
        tail: bool,
        cancel: Arc<AtomicBool>,
        keep_pages: bool,
    ) -> io::Result<FileLoadHandle> {
        let total_bytes = std::fs::metadata(&path)?.len();
        let (progress_tx, progress_rx) = watch::channel(0.0_f64);
        let (result_tx, result_rx) = oneshot::channel();

        spawn_blocking(move || {
            let result = Self::index_chunked(
                &path,
                total_bytes,
                progress_tx,
                predicate,
                tail,
                &cancel,
                keep_pages,
            );
            // Ignore send error — UI may have quit before we finish.
            let _ = result_tx.send(result);
        });

        Ok(FileLoadHandle {
            progress_rx,
            result_rx,
            total_bytes,
        })
    }

    /// Index the file using a parallel Rayon scan, sending progress updates
    /// as chunks complete. Produces the same `line_starts` as
    /// `compute_line_starts`.
    ///
    /// Phase 1 (always): parallel scan building `line_starts` + ANSI
    /// detection, one equal chunk per thread, concatenated in order (no
    /// sort needed). Any ESC/CR byte found triggers a serial ANSI fallback
    /// over the full mmap.
    ///
    /// Phase 2 (when `predicate` is `Some`): forward parallel scan, or for
    /// `tail=true` a backward sequential scan (tail lines evaluated first,
    /// result reversed back to ascending order).
    /// Async-load counterpart of `try_new_paged`: builds a `Storage::Paged`
    /// reader via the same streaming pass (bounded memory), then evaluates
    /// `predicate` (if any) using `reader.get_line()`, which is safe to call
    /// from many rayon threads at once since it returns owned `Arc` data.
    /// Returns `Ok(None)` to signal "fall back to the full-materialization
    /// path" for DLT-binary/ANSI files, same as `try_new_paged`.
    fn try_index_paged(
        path: &str,
        progress_tx: &watch::Sender<f64>,
        predicate: Option<VisibilityPredicate>,
        tail: bool,
        cancel: &AtomicBool,
    ) -> io::Result<Option<FileLoadResult>> {
        use rayon::prelude::*;

        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "load cancelled"));
        }
        let canonical_path = Arc::new(
            std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path)),
        );
        let Some(reader) = Self::try_new_paged(path, canonical_path)? else {
            return Ok(None);
        };
        let _ = progress_tx.send(0.5);

        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "load cancelled"));
        }

        let (precomputed_visible, precomputed_text_counts) = if let Some(pred) = predicate {
            let count = reader.line_count();
            let n = pred.fm.filter_count();
            let has_include = pred.fm.has_include();
            let (visible, text_counts) = if tail {
                let mut text_counts = vec![0usize; n];
                let mut visible: Vec<usize> = (0..count)
                    .rev()
                    .filter(|&i| {
                        pred.fm
                            .evaluate_and_count(&reader.get_line(i), &mut text_counts)
                            .to_visibility(has_include)
                    })
                    .collect();
                visible.reverse();
                (visible, text_counts)
            } else {
                (0..count)
                    .into_par_iter()
                    .fold(
                        || (Vec::new(), vec![0usize; n]),
                        |(mut vis, mut tc), i| {
                            let dec = pred.fm.evaluate_and_count(&reader.get_line(i), &mut tc);
                            if dec.to_visibility(has_include) {
                                vis.push(i);
                            }
                            (vis, tc)
                        },
                    )
                    .reduce(
                        || (Vec::new(), vec![0usize; n]),
                        |(mut va, mut ta), (vb, tb)| {
                            va.extend(vb);
                            for (a, b) in ta.iter_mut().zip(tb) {
                                *a += b;
                            }
                            (va, ta)
                        },
                    )
            };
            (Some(visible), Some(text_counts))
        } else {
            (None, None)
        };

        let _ = progress_tx.send(1.0);
        Ok(Some(FileLoadResult {
            reader,
            precomputed_visible,
            precomputed_text_counts,
        }))
    }

    fn index_chunked(
        path: &str,
        total_bytes: u64,
        progress_tx: watch::Sender<f64>,
        predicate: Option<VisibilityPredicate>,
        tail: bool,
        cancel: &AtomicBool,
        _keep_pages: bool,
    ) -> io::Result<FileLoadResult> {
        use rayon::prelude::*;
        use std::sync::atomic::AtomicUsize;

        if total_bytes >= PAGED_STORAGE_THRESHOLD_BYTES
            && let Some(result) =
                Self::try_index_paged(path, &progress_tx, predicate.clone(), tail, cancel)?
        {
            return Ok(result);
        }

        let canonical_path = Arc::new(
            std::fs::canonicalize(path).unwrap_or_else(|_| std::path::PathBuf::from(path)),
        );
        let file = Arc::new(File::open(path)?);
        let size = total_bytes as usize;

        // Pre-allocate the file buffer. On Unix we fill it via parallel pread
        // (one chunk per rayon worker) so I/O and the newline scan run in the
        // same pass; non-Unix falls back to sequential read_to_end.
        #[cfg(unix)]
        let mut file_data: Vec<u8> = {
            let mut v = vec![0u8; size];
            // SAFETY: madvise only touches kernel page tables for this valid,
            // writable `size`-byte mapping. Pre-faulting pages before the
            // parallel pread avoids per-page fault stalls during memcpy.
            // EINVAL on kernels < 5.14 is ignored (demand-paging fallback).
            #[cfg(target_os = "linux")]
            unsafe {
                libc::madvise(
                    v.as_mut_ptr() as *mut libc::c_void,
                    size,
                    libc::MADV_POPULATE_WRITE,
                );
            }
            v
        };
        #[cfg(not(unix))]
        let file_data: Vec<u8> = {
            use std::io::Read;
            let mut v = Vec::with_capacity(size);
            (&*file).read_to_end(&mut v)?;
            v
        };

        let len = file_data.len();

        if is_any_dlt_binary(&file_data) {
            let text = dlt_binary::convert_dlt_binary_to_text(&file_data);
            drop(file_data);
            let _ = progress_tx.send(1.0);
            let mut reader = Self::from_bytes(text);
            reader.is_binary = true;

            let (precomputed_visible, precomputed_text_counts) = if let Some(pred) = predicate {
                let count = reader.line_count();
                let n = pred.fm.filter_count();
                let has_include = pred.fm.has_include();
                if tail {
                    let mut text_counts = vec![0usize; n];
                    let mut visible: Vec<usize> = (0..count)
                        .rev()
                        .filter(|&i| {
                            pred.fm
                                .evaluate_and_count(&reader.get_line(i), &mut text_counts)
                                .to_visibility(has_include)
                        })
                        .collect();
                    visible.reverse();
                    (Some(visible), Some(text_counts))
                } else {
                    use rayon::prelude::*;
                    let (visible, text_counts) = (0..count)
                        .into_par_iter()
                        .fold(
                            || (Vec::new(), vec![0usize; n]),
                            |(mut vis, mut tc), i| {
                                let dec = pred.fm.evaluate_and_count(&reader.get_line(i), &mut tc);
                                if dec.to_visibility(has_include) {
                                    vis.push(i);
                                }
                                (vis, tc)
                            },
                        )
                        .reduce(
                            || (Vec::new(), vec![0usize; n]),
                            |(mut va, mut ta), (vb, tb)| {
                                va.extend(vb);
                                for (a, b) in ta.iter_mut().zip(tb) {
                                    *a += b;
                                }
                                (va, ta)
                            },
                        );
                    (Some(visible), Some(text_counts))
                }
            } else {
                (None, None)
            };

            return Ok(FileLoadResult {
                reader,
                precomputed_visible,
                precomputed_text_counts,
            });
        }

        // Phase 1: parallel pread + scan for '\n', '\x1b', '\r' in one pass.
        // Each worker fills its chunk via pread (parallel I/O) then scans it
        // while hot in cache — same parallelism as mmap page-faulting, but
        // without the SIGBUS risk. chunk_size is one slice per rayon thread
        // (min 4 MiB); bytes_done is a shared counter for progress updates.
        let num_threads = rayon::current_num_threads().max(1);
        let chunk_size = len.div_ceil(num_threads).max(4 * 1024 * 1024);
        let bytes_done = AtomicUsize::new(0);

        // Each element: (has_ansi, Vec<absolute next-line offsets for this chunk>)
        #[cfg(unix)]
        let chunk_results: Vec<(bool, Vec<usize>)> = {
            use std::os::unix::fs::FileExt;
            file_data
                .par_chunks_mut(chunk_size)
                .enumerate()
                .map(|(chunk_idx, chunk)| -> io::Result<(bool, Vec<usize>)> {
                    if cancel.load(Ordering::Relaxed) {
                        return Ok((false, vec![]));
                    }
                    // Fill the chunk via pread (does not move the file cursor).
                    let offset = (chunk_idx * chunk_size) as u64;
                    let mut filled = 0;
                    while filled < chunk.len() {
                        match file.read_at(&mut chunk[filled..], offset + filled as u64) {
                            Ok(0) => break, // EOF before expected — file shrank
                            Ok(n) => filled += n,
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                            Err(e) => return Err(e),
                        }
                    }
                    let chunk = &chunk[..filled];
                    let chunk_start = chunk_idx * chunk_size;
                    let has_ansi = memchr2(b'\x1b', b'\r', chunk).is_some();
                    let mut local_starts: Vec<usize> = Vec::new();
                    if !has_ansi {
                        for pos in memchr_iter(b'\n', chunk) {
                            let next = chunk_start + pos + 1;
                            if next <= len {
                                local_starts.push(next);
                            }
                        }
                    }
                    let done = bytes_done.fetch_add(chunk.len(), Ordering::Relaxed) + chunk.len();
                    if len > 0 {
                        let _ = progress_tx.send(done as f64 / len as f64);
                    }
                    Ok((has_ansi, local_starts))
                })
                .collect::<io::Result<Vec<_>>>()?
        };

        #[cfg(not(unix))]
        let chunk_results: Vec<(bool, Vec<usize>)> = file_data
            .par_chunks(chunk_size)
            .enumerate()
            .map(|(chunk_idx, chunk)| {
                if cancel.load(Ordering::Relaxed) {
                    return (false, vec![]);
                }
                let chunk_start = chunk_idx * chunk_size;
                let has_ansi = memchr2(b'\x1b', b'\r', chunk).is_some();
                let mut local_starts: Vec<usize> = Vec::new();
                if !has_ansi {
                    for pos in memchr_iter(b'\n', chunk) {
                        let next = chunk_start + pos + 1;
                        if next <= len {
                            local_starts.push(next);
                        }
                    }
                }
                let done = bytes_done.fetch_add(chunk.len(), Ordering::Relaxed) + chunk.len();
                if len > 0 {
                    let _ = progress_tx.send(done as f64 / len as f64);
                }
                (has_ansi, local_starts)
            })
            .collect();

        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "load cancelled"));
        }

        let has_ansi = chunk_results.iter().any(|(a, _)| *a);

        let reader = if has_ansi {
            let (stripped, line_starts) = strip_ansi_and_index(&file_data);
            drop(file_data);
            let _ = progress_tx.send(1.0);
            FileReader {
                storage: Storage::Bytes(std::sync::Arc::new(stripped)),
                line_starts: std::sync::Arc::new(line_starts),
                is_binary: false,
            }
        } else {
            // Merge per-chunk newline positions into the final line_starts.
            // Chunks are non-overlapping and ordered, so simple concatenation
            // preserves ascending order — no sort needed.
            let total_starts: usize = chunk_results.iter().map(|(_, v)| v.len()).sum();
            let mut starts = Vec::with_capacity(1 + total_starts);
            starts.push(0usize); // sentinel: first line always starts at byte 0
            for (_, local) in chunk_results {
                starts.extend(local);
            }

            #[cfg(unix)]
            let (inode, device, mtime) = {
                use std::os::unix::fs::MetadataExt;
                let m = file.metadata()?;
                (m.ino(), m.dev(), m.modified().ok())
            };
            #[cfg(not(unix))]
            let (inode, device, mtime) = (
                0u64,
                0u64,
                file.metadata().ok().and_then(|m| m.modified().ok()),
            );

            FileReader {
                storage: Storage::File {
                    data: Arc::new(file_data),
                    file,
                    path: canonical_path,
                    inode,
                    device,
                    mtime,
                },
                line_starts: Arc::new(starts),
                is_binary: false,
            }
        };

        // Phase 2: evaluate the predicate on each line when provided.
        let (precomputed_visible, precomputed_text_counts) = if let Some(pred) = predicate {
            let count = reader.line_count();
            let n = pred.fm.filter_count();
            let has_include = pred.fm.has_include();
            let (visible, text_counts) = if tail {
                // Evaluate from the last line backward so lines near the tail
                // are confirmed first; reverse at the end to restore ascending order.
                let mut text_counts = vec![0usize; n];
                let mut visible: Vec<usize> = (0..count)
                    .rev()
                    .filter(|&i| {
                        pred.fm
                            .evaluate_and_count(&reader.get_line(i), &mut text_counts)
                            .to_visibility(has_include)
                    })
                    .collect();
                visible.reverse();
                (visible, text_counts)
            } else {
                use rayon::prelude::*;
                (0..count)
                    .into_par_iter()
                    .fold(
                        || (Vec::new(), vec![0usize; n]),
                        |(mut vis, mut tc), i| {
                            let dec = pred.fm.evaluate_and_count(&reader.get_line(i), &mut tc);
                            if dec.to_visibility(has_include) {
                                vis.push(i);
                            }
                            (vis, tc)
                        },
                    )
                    .reduce(
                        || (Vec::new(), vec![0usize; n]),
                        |(mut va, mut ta), (vb, tb)| {
                            va.extend(vb);
                            for (a, b) in ta.iter_mut().zip(tb) {
                                *a += b;
                            }
                            (va, ta)
                        },
                    )
            };
            (Some(visible), Some(text_counts))
        } else {
            (None, None)
        };

        Ok(FileLoadResult {
            reader,
            precomputed_visible,
            precomputed_text_counts,
        })
    }

    /// File modification time, if available (only for files opened via `new()`).
    pub fn mtime(&self) -> Option<std::time::SystemTime> {
        match &self.storage {
            Storage::File { mtime, .. } => *mtime,
            Storage::Paged { mtime, .. } => *mtime,
            _ => None,
        }
    }

    /// Total byte length of the backing content. Unlike `data().len()`, this
    /// works for `Storage::Paged` too, which has no resident buffer.
    pub fn total_len(&self) -> usize {
        match &self.storage {
            Storage::Paged { paged, .. } => paged.size() as usize,
            _ => self.storage.as_bytes().len(),
        }
    }

    /// Total number of lines (including any final partial line without a trailing newline).
    pub fn line_count(&self) -> usize {
        if let Storage::Merged { entries, .. } = &self.storage {
            return entries.len();
        }
        let total = self.total_len();
        if total == 0 {
            return 0;
        }
        // line_starts has one entry per newline + the initial 0.
        // If the file ends with '\n', the last start points to data.len() (empty slice).
        // We skip that phantom empty line.
        let n = self.line_starts.len();
        if n > 0 && self.line_starts[n - 1] == total {
            n - 1
        } else {
            n
        }
    }

    /// True when this reader is backed by `Storage::Paged` (a large,
    /// disk-backed file) — no resident buffer, so [`Self::data`] can't be
    /// used and the whole-file Aho-Corasick fast path isn't available.
    pub fn is_paged(&self) -> bool {
        matches!(self.storage, Storage::Paged { .. })
    }

    /// Like [`Self::get_line`], but requires a genuine zero-copy borrow
    /// tied to `&self`'s lifetime rather than `get_line`'s owned-or-borrowed
    /// [`LineBytes`]. Needed by continuation-group handling, which collects
    /// borrowed lines into a caller-lifetime'd `DisplayParts` — something an
    /// owned `Arc<[u8]>` can't satisfy. Not supported for `Storage::Paged`
    /// (large files): callers must check `reader.is_paged()` first and skip
    /// continuation handling for those instead (see `build_continuation_map`).
    ///
    /// # Panics
    /// Panics if `idx >= line_count()`, or if this reader is `Storage::Paged`.
    pub fn get_line_zero_copy(&self, idx: usize) -> &[u8] {
        match self.get_line(idx) {
            LineBytes::Borrowed(b) => b,
            LineBytes::Owned(_) => {
                panic!("get_line_zero_copy is not supported for paged (large-file) storage")
            }
        }
    }

    /// If this is a merged reader, return the Arc to the sorted entries; otherwise `None`.
    pub fn merged_entries(&self) -> Option<&Arc<Vec<MergedEntry>>> {
        if let Storage::Merged { entries, .. } = &self.storage {
            Some(entries)
        } else {
            None
        }
    }

    /// Return the raw bytes of line `idx` (without the trailing newline).
    ///
    /// For `Storage::Merged`, `idx` is interpreted as a compound key
    /// `source_idx << SOURCE_IDX_SHIFT | line_idx`.
    ///
    /// Returns owned, `Arc`-backed bytes for `Storage::Paged` (large files);
    /// a zero-copy borrow otherwise. See [`LineBytes`].
    ///
    /// # Panics
    /// Panics if `idx >= line_count()`, or if the underlying file shrank out
    /// from under a `Storage::Paged` reader (rotation mid-read).
    pub fn get_line(&self, idx: usize) -> LineBytes<'_> {
        if let Storage::Merged { entries, sources } = &self.storage {
            let entry = &entries[idx];
            return sources[entry.source_idx].get_line(entry.line_idx);
        }
        if let Storage::Paged { paged, .. } = &self.storage {
            return LineBytes::Owned(
                paged
                    .get_line(&self.line_starts, idx)
                    .unwrap_or_else(|e| panic!("paged line {idx} read failed: {e}")),
            );
        }
        LineBytes::Borrowed(&self.storage.as_bytes()[self.line_byte_range(idx)])
    }

    /// Byte range of line `idx` within [`data()`] (without the trailing
    /// newline). Only meaningful for non-`Merged` storage — a `Merged`
    /// reader has no single flat buffer for `idx` to index into (see
    /// [`data()`]).
    ///
    /// # Panics
    /// Panics if `idx >= line_count()`.
    pub fn line_byte_range(&self, idx: usize) -> std::ops::Range<usize> {
        let start = self.line_starts[idx];
        let end = if idx + 1 < self.line_starts.len() {
            let next = self.line_starts[idx + 1];
            match &self.storage {
                // `line_starts` entries are always right after a '\n' by
                // construction (see `PagedFile::build_line_starts`), so no
                // need to read a byte back just to confirm it — that would
                // cost an extra page fetch per line for no benefit.
                Storage::Paged { .. } => next - 1,
                _ => {
                    let data = self.storage.as_bytes();
                    if next > 0 && data.get(next - 1) == Some(&b'\n') {
                        next - 1
                    } else {
                        next
                    }
                }
            }
        } else {
            self.total_len()
        };
        start..end
    }

    /// The contiguous backing data buffer (mmap or in-memory bytes).
    ///
    /// Used by whole-file scanning paths (e.g. Aho-Corasick over the entire
    /// buffer) that avoid per-line `get_line()` overhead. Not supported for
    /// `Storage::Paged` (large files) — use [`Self::materialize_range`]
    /// instead, which only fetches the bytes actually needed.
    ///
    /// # Panics
    /// Panics if this reader is backed by `Storage::Paged`.
    pub fn data(&self) -> &[u8] {
        assert!(
            !matches!(self.storage, Storage::Paged { .. }),
            "FileReader::data() is not supported for paged (large-file) storage — use materialize_range() instead"
        );
        self.storage.as_bytes()
    }

    /// Returns an owned copy of the bytes in `byte_range`. For small/resident
    /// storage this is a cheap slice-copy; for `Storage::Paged` (large files)
    /// it fetches/concatenates only the needed pages, unlike [`Self::data`].
    pub fn materialize_range(&self, byte_range: std::ops::Range<usize>) -> Arc<[u8]> {
        match &self.storage {
            Storage::Paged { paged, .. } => paged
                .read_range(byte_range)
                .unwrap_or_else(|e| panic!("paged range read failed: {e}")),
            _ => Arc::from(&self.storage.as_bytes()[byte_range]),
        }
    }

    /// Materializes the byte range for lines `[chunk_start, chunk_end)` and
    /// returns it alongside a *rebased* line_starts table — index 0 lines up
    /// with `chunk_start`, and every value is relative to the chunk's first
    /// byte — suitable for `FilterManager::evaluate_chunk_wholefile`, which
    /// only ever indexes within the `line_range` it's given.
    ///
    /// Works for any storage kind, but exists specifically so paged
    /// (large-file) readers can use the whole-buffer fast path one bounded
    /// chunk at a time instead of needing the whole file resident. Callers
    /// must add `chunk_start` back onto any line indices
    /// `evaluate_chunk_wholefile` returns to get real, file-absolute ones.
    ///
    /// # Panics
    /// Panics if `chunk_start >= chunk_end` or `chunk_end > line_count()`.
    pub fn materialize_chunk(
        &self,
        chunk_start: usize,
        chunk_end: usize,
    ) -> (Arc<[u8]>, Vec<usize>) {
        assert!(chunk_start < chunk_end, "empty or inverted chunk range");
        assert!(chunk_end <= self.line_count(), "chunk_end past line_count");

        let byte_start = self.line_starts[chunk_start];
        let byte_end = if chunk_end < self.line_starts.len() {
            self.line_starts[chunk_end]
        } else {
            self.total_len()
        };

        // Bypasses the page cache for `Storage::Paged`: a whole-file filter
        // chunk reads every byte exactly once (nothing is reused), so the
        // cache's per-page bookkeeping is pure overhead here — see
        // `PagedFile::read_range_uncached`.
        let chunk_bytes = match &self.storage {
            Storage::Paged { paged, .. } => paged
                .read_range_uncached(byte_start..byte_end)
                .unwrap_or_else(|e| panic!("paged range read failed: {e}")),
            _ => self.materialize_range(byte_start..byte_end),
        };
        // One entry per line's start in [chunk_start, chunk_end), plus a
        // trailing sentinel at the chunk's end — `chunk_end` may or may not
        // land on a real `line_starts` entry (no trailing newline on the
        // file's last line), so the sentinel is computed explicitly from
        // `byte_end` rather than reused from `line_starts` itself.
        let mut local_starts: Vec<usize> = self.line_starts[chunk_start..chunk_end]
            .iter()
            .map(|&s| s - byte_start)
            .collect();
        local_starts.push(byte_end - byte_start);

        (chunk_bytes, local_starts)
    }

    /// The sorted byte-offset table: `line_starts()[i]` is the byte offset
    /// where line `i` begins in [`data()`].
    pub fn line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    #[cfg(unix)]
    pub fn advise_for_scan(&self, _line_range: std::ops::Range<usize>) {
        // Data is already in RAM (Vec<u8>); no prefetch hint needed.
    }

    /// No-op: data is already in RAM so no prefetch hint is needed.
    #[cfg(unix)]
    pub fn advise_viewport(&self, _first_line: usize, _last_line: usize) {}

    /// Iterate over `(line_index, line_bytes)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (usize, LineBytes<'_>)> {
        (0..self.line_count()).map(move |i| (i, self.get_line(i)))
    }

    pub fn append_bytes(&mut self, new_data: &[u8]) {
        if new_data.is_empty() {
            return;
        }

        let effective_data;
        let converted;
        if self.is_binary {
            converted = dlt_binary::convert_dlt_binary_to_text(new_data);
            effective_data = converted.as_slice();
        } else {
            effective_data = new_data;
        }

        if effective_data.is_empty() {
            return;
        }

        let old_storage = std::mem::replace(
            &mut self.storage,
            Storage::Bytes(std::sync::Arc::new(Vec::new())),
        );
        let mut data: Vec<u8> = match old_storage {
            Storage::Bytes(v) => std::sync::Arc::try_unwrap(v).unwrap_or_else(|arc| (*arc).clone()),
            Storage::File { data, .. } => {
                std::sync::Arc::try_unwrap(data).unwrap_or_else(|arc| (*arc).clone())
            }
            // Streaming/append use is only for small in-memory (Bytes/File)
            // readers (e.g. `:run` command output); large paged/merged
            // readers don't support in-place growth this way.
            Storage::Merged { .. } | Storage::Paged { .. } => return,
        };
        let offset = data.len();
        data.extend_from_slice(effective_data);
        // Extend line_starts incrementally — only scan the new bytes.
        let starts = std::sync::Arc::make_mut(&mut self.line_starts);
        for pos in memchr_iter(b'\n', &data[offset..]) {
            let abs = offset + pos + 1;
            if abs <= data.len() {
                starts.push(abs);
            }
        }
        self.storage = Storage::Bytes(std::sync::Arc::new(data));
    }

    /// Spawn a child process and stream its output. Appends ANSI-stripped
    /// complete lines to a `NamedTempFile` every 500 ms, returning a
    /// `watch::Receiver<()>` that fires on each flush; the sender drops
    /// when the process exits.
    ///
    /// When `tag_stderr` is `true`, stderr lines are prefixed with
    /// `"ERROR "` so log parsers show them at error level; stdout is
    /// written unchanged.
    pub async fn spawn_process_stream(
        program: &str,
        args: &[&str],
        tag_stderr: bool,
    ) -> io::Result<(watch::Receiver<()>, tempfile::NamedTempFile)> {
        use std::io::Write as _;
        use tokio::process::Command;

        let mut child = Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            .spawn()?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let temp_file = tempfile::NamedTempFile::new()?;
        let temp_path = temp_file.path().to_owned();
        let (tx, rx) = watch::channel(());

        // Each chunk carries a flag: `true` means it came from stderr.
        let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<(Vec<u8>, bool)>(64);

        if let Some(mut out) = stdout {
            let sender = line_tx.clone();
            spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = vec![0u8; 4096];
                loop {
                    match out.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if sender.send((buf[..n].to_vec(), false)).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }

        if let Some(mut err) = stderr {
            let sender = line_tx.clone();
            spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = vec![0u8; 4096];
                loop {
                    match err.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if sender.send((buf[..n].to_vec(), true)).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }

        // Drop the original sender so line_rx closes when both readers finish.
        drop(line_tx);

        spawn(async move {
            use std::time::Duration;

            // Prefix every line in `data` with "ERROR ".
            fn prefix_error_lines(data: &[u8]) -> Vec<u8> {
                let mut out = Vec::with_capacity(data.len() + 32);
                for line in data.split_inclusive(|&b| b == b'\n') {
                    out.extend_from_slice(b"ERROR ");
                    out.extend_from_slice(line);
                }
                out
            }

            // Flush complete lines (up to the last `\n`) from `partial` into `f`.
            // Returns true if anything was written.
            fn flush_partial(
                partial: &mut Vec<u8>,
                is_stderr: bool,
                tag_stderr: bool,
                f: &mut std::fs::File,
            ) -> bool {
                let Some(last_nl) = partial.iter().rposition(|&b| b == b'\n') else {
                    return false;
                };
                let stripped = strip_ansi_escapes(&partial[..=last_nl]);
                partial.drain(..=last_nl);
                let to_write = if tag_stderr && is_stderr {
                    prefix_error_lines(&stripped)
                } else {
                    stripped
                };
                f.write_all(&to_write).is_ok()
            }

            // Separate buffers per stream to prevent interleaving at chunk
            // boundaries from corrupting line-prefix insertion.
            let mut partial_out: Vec<u8> = Vec::new();
            let mut partial_err: Vec<u8> = Vec::new();

            let mut interval = tokio::time::interval(Duration::from_millis(500));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await; // skip initial immediate tick

            loop {
                tokio::select! {
                    chunk = line_rx.recv() => {
                        match chunk {
                            Some((data, is_stderr)) => {
                                if is_stderr {
                                    partial_err.extend_from_slice(&data);
                                } else {
                                    partial_out.extend_from_slice(&data);
                                }
                            }
                            None => {
                                // Both readers done — flush all remaining bytes.
                                if (!partial_out.is_empty() || !partial_err.is_empty())
                                    && let Ok(mut f) = std::fs::OpenOptions::new()
                                        .append(true)
                                        .open(&temp_path)
                                {
                                    if !partial_out.is_empty() {
                                        let stripped = strip_ansi_escapes(&partial_out);
                                        let _ = f.write_all(&stripped);
                                    }
                                    if !partial_err.is_empty() {
                                        let stripped = strip_ansi_escapes(&partial_err);
                                        let to_write = if tag_stderr {
                                            prefix_error_lines(&stripped)
                                        } else {
                                            stripped
                                        };
                                        let _ = f.write_all(&to_write);
                                    }
                                    let _ = f.flush();
                                }
                                let _ = tx.send(());
                                return;
                            }
                        }
                    }
                    _ = interval.tick() => {
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .append(true)
                            .open(&temp_path)
                        {
                            let mut wrote = false;
                            wrote |= flush_partial(&mut partial_out, false, tag_stderr, &mut f);
                            wrote |= flush_partial(&mut partial_err, true, tag_stderr, &mut f);
                            if wrote {
                                let _ = f.flush();
                                let _ = tx.send(());
                            }
                        }
                    }
                }
            }
        });

        Ok((rx, temp_file))
    }

    pub async fn spawn_dlt_tcp_stream(
        host: String,
        port: u16,
    ) -> io::Result<(watch::Receiver<()>, tempfile::NamedTempFile)> {
        use std::io::Write as _;
        use tokio::net::TcpStream;

        let stream = TcpStream::connect((host.as_str(), port)).await?;
        let temp_file = tempfile::NamedTempFile::new()?;
        let temp_path = temp_file.path().to_owned();
        let (tx, rx) = watch::channel(());
        let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

        spawn(async move {
            use tokio::io::AsyncReadExt;
            // Keep the full stream alive — splitting and dropping the write
            // half sends a FIN that causes dlt-daemon to disconnect.
            let mut stream = stream;
            let mut buf = vec![0u8; 8192];
            loop {
                match stream.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if chunk_tx.send(buf[..n].to_vec()).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        spawn(async move {
            use std::time::Duration;

            let mut partial: Vec<u8> = Vec::new();
            let mut format_confirmed = false;

            let mut interval = tokio::time::interval(Duration::from_millis(500));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;

            let flush = |data: &[u8]| -> bool {
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&temp_path)
                    .ok()
                    .and_then(|mut f| {
                        f.write_all(data).ok()?;
                        f.flush().ok()
                    })
                    .is_some()
            };

            loop {
                tokio::select! {
                    chunk = chunk_rx.recv() => {
                        match chunk {
                            Some(data) => partial.extend_from_slice(&data),
                            None => {
                                if !partial.is_empty() {
                                    let now_ts = {
                                        let d = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default();
                                        dlt_binary::format_storage_timestamp(
                                            d.as_secs() as u32,
                                            d.subsec_micros(),
                                        )
                                    };
                                    let (text, consumed) =
                                        dlt_binary::convert_wire_streaming(&partial, &now_ts);
                                    let mut out = text;
                                    if consumed < partial.len() {
                                        out.extend_from_slice(
                                            String::from_utf8_lossy(&partial[consumed..]).as_bytes(),
                                        );
                                    }
                                    if !out.is_empty() {
                                        flush(&out);
                                    }
                                }
                                let _ = tx.send(());
                                return;
                            }
                        }
                    }
                    _ = interval.tick() => {
                        if partial.is_empty() {
                            continue;
                        }
                        if !format_confirmed {
                            if dlt_binary::is_dlt_wire_format(&partial)
                                || dlt_binary::is_dlt_binary(&partial)
                            {
                                format_confirmed = true;
                            } else {
                                continue;
                            }
                        }
                        if dlt_binary::is_dlt_binary(&partial) {
                            let text = dlt_binary::convert_dlt_binary_to_text(&partial);
                            if !text.is_empty() && flush(&text) {
                                partial.clear();
                                let _ = tx.send(());
                            }
                        } else {
                            let now_ts = {
                                let d = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default();
                                dlt_binary::format_storage_timestamp(
                                    d.as_secs() as u32,
                                    d.subsec_micros(),
                                )
                            };
                            let (text, consumed) =
                                dlt_binary::convert_wire_streaming(&partial, &now_ts);
                            if !text.is_empty() && flush(&text) {
                                partial.drain(..consumed);
                                let _ = tx.send(());
                            }
                        }
                    }
                }
            }
        });

        Ok((rx, temp_file))
    }

    /// Spawn a background task that polls `path` for new bytes every 50 ms.
    /// `initial_offset` must be the original (unstripped) file size at load
    /// time. Returns a `watch::Receiver<()>` that fires when the file
    /// grows, is truncated, or is replaced (inode change on Unix); the
    /// caller applies the update via `FileReader::try_extend_from_read`
    /// (or, for ANSI files where that returns `false`, a full reload via
    /// `FileReader::new`).
    pub async fn spawn_file_watcher(path: String, initial_offset: u64) -> watch::Receiver<()> {
        let (tx, rx) = watch::channel(());

        tokio::spawn(async move {
            use tokio::time::MissedTickBehavior;

            let mut last_offset = initial_offset;

            // Capture initial file identity for rotation-by-rename detection.
            #[cfg(unix)]
            let mut last_identity: Option<(u64, u64)> = {
                use std::os::unix::fs::MetadataExt;
                std::fs::metadata(&path).ok().map(|m| (m.ino(), m.dev()))
            };

            let mut interval = tokio::time::interval(std::time::Duration::from_millis(50));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            interval.tick().await; // skip initial immediate tick

            loop {
                interval.tick().await;

                let path_clone = path.clone();
                let result = tokio::task::spawn_blocking(move || -> io::Result<(u64, u64, u64)> {
                    let meta = std::fs::metadata(&path_clone)?;
                    let size = meta.len();
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        Ok((size, meta.ino(), meta.dev()))
                    }
                    #[cfg(not(unix))]
                    Ok((size, 0, 0))
                })
                .await;

                #[allow(unused_variables)]
                if let Ok(Ok((current_size, ino, dev))) = result {
                    // Detect rotation-by-rename: inode changed under the path.
                    #[cfg(unix)]
                    {
                        let identity = (ino, dev);
                        if let Some(last) = last_identity {
                            if identity != last {
                                last_identity = Some(identity);
                                last_offset = current_size;
                                if tx.send(()).is_err() {
                                    break;
                                }
                                continue;
                            }
                        } else {
                            last_identity = Some(identity);
                        }
                    }

                    if current_size < last_offset {
                        // File was truncated (e.g. log rotation) — reset offset.
                        last_offset = current_size;
                        if tx.send(()).is_err() {
                            break;
                        }
                    } else if current_size > last_offset {
                        last_offset = current_size;
                        if tx.send(()).is_err() {
                            break; // Receiver dropped — stop watching.
                        }
                    }
                }
                // Else: transient I/O error or task panic — retry next tick.
            }
        });

        rx
    }
}

/// Strip ANSI/VT escape sequences and bare `\r` from `input`: CSI
/// sequences (`ESC [` … final byte 0x40–0x7E), OSC sequences (`ESC ]` …
/// BEL or `ESC \`), other two-byte ESC sequences, and bare `\r` (so
/// `\r\n` becomes `\n`).
fn strip_ansi_escapes(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        match input[i] {
            b'\x1b' => {
                i += 1;
                if i >= input.len() {
                    break;
                }
                match input[i] {
                    b'[' => {
                        // CSI: ESC [ {param/intermediate bytes} {final byte 0x40–0x7E}
                        i += 1;
                        while i < input.len() {
                            let b = input[i];
                            i += 1;
                            if (0x40..=0x7E).contains(&b) {
                                break;
                            }
                        }
                    }
                    b']' => {
                        // OSC: ESC ] … BEL  or  ESC ] … ESC \
                        i += 1;
                        while i < input.len() {
                            let b = input[i];
                            i += 1;
                            if b == b'\x07' {
                                break;
                            }
                            if b == b'\x1b' && i < input.len() && input[i] == b'\\' {
                                i += 1;
                                break;
                            }
                        }
                    }
                    _ => {
                        i += 1;
                    } // two-byte ESC sequence (e.g. ESC M, ESC =)
                }
            }
            b'\r' => {
                i += 1;
            } // strip CR so \r\n becomes \n
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Strip ANSI escape sequences from `input` and collect line-start offsets in
/// one pass — eliminating the separate [`compute_line_starts`] scan over the
/// stripped output.
///
/// Returns `(stripped_bytes, line_starts)` where `line_starts[i]` is the byte
/// offset of the first byte of line `i` in the returned `Vec<u8>`.
fn strip_ansi_and_index(input: &[u8]) -> (Vec<u8>, Vec<usize>) {
    let mut out = Vec::with_capacity(input.len());
    let mut starts = vec![0usize];
    let mut i = 0;

    while i < input.len() {
        // Fast path: scan ahead for the next ESC or CR, bulk-copy everything before it.
        let safe_end = memchr2(b'\x1b', b'\r', &input[i..])
            .map(|p| i + p)
            .unwrap_or(input.len());

        if safe_end > i {
            let segment = &input[i..safe_end];
            let out_base = out.len();
            out.extend_from_slice(segment);
            // Record line starts for every '\n' in the bulk-copied segment.
            for nl in memchr_iter(b'\n', segment) {
                starts.push(out_base + nl + 1);
            }
            i = safe_end;
        }

        if i >= input.len() {
            break;
        }

        // Slow path: handle the control byte at `i`.
        match input[i] {
            b'\x1b' => {
                i += 1;
                if i >= input.len() {
                    break;
                }
                match input[i] {
                    b'[' => {
                        // CSI: ESC [ {param/intermediate bytes} {final byte 0x40–0x7E}
                        i += 1;
                        while i < input.len() {
                            let b = input[i];
                            i += 1;
                            if (0x40..=0x7E).contains(&b) {
                                break;
                            }
                        }
                    }
                    b']' => {
                        // OSC: ESC ] … BEL  or  ESC ] … ESC \
                        i += 1;
                        while i < input.len() {
                            let b = input[i];
                            i += 1;
                            if b == b'\x07' {
                                break;
                            }
                            if b == b'\x1b' && i < input.len() && input[i] == b'\\' {
                                i += 1;
                                break;
                            }
                        }
                    }
                    _ => {
                        i += 1; // two-byte ESC sequence (e.g. ESC M, ESC =)
                    }
                }
            }
            b'\r' => {
                i += 1; // strip CR so \r\n becomes \n
            }
            _ => unreachable!("memchr2 only stops at ESC or CR"),
        }
    }

    (out, starts)
}

/// Computes the byte offsets of the start of every line in `data`.
/// The first element is always `0`.  The last element points one past the
/// final newline (i.e. to the beginning of a potential final partial line).
#[cfg(test)]
fn compute_line_starts(data: &[u8]) -> Vec<usize> {
    let mut starts = vec![0usize];
    for pos in memchr_iter(b'\n', data) {
        if pos < data.len() {
            starts.push(pos + 1);
        }
    }
    // If the last byte is NOT a newline, the last element already points past
    // the data, so no extra push is needed.  If it IS a newline, the starts vec
    // ends with `data.len()`, and `get_line` will return an empty slice there —
    // which is fine because we only iterate `0..line_count()`.
    starts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make(content: &[u8]) -> FileReader {
        FileReader::from_bytes(content.to_vec())
    }

    #[test]
    fn test_line_byte_range_matches_get_line() {
        let reader = make(b"first\nsecond\nthird\n");
        for idx in 0..reader.line_count() {
            let range = reader.line_byte_range(idx);
            assert_eq!(reader.get_line(idx), &reader.data()[range]);
        }
    }

    #[test]
    fn test_line_byte_range_last_line_without_trailing_newline() {
        let reader = make(b"first\nsecond");
        assert_eq!(reader.line_count(), 2);
        let range = reader.line_byte_range(1);
        assert_eq!(&reader.data()[range], b"second");
    }

    #[test]
    fn test_line_byte_range_is_contiguous_across_consecutive_lines() {
        // The range for line N ends exactly where line N+1's range starts
        // (modulo the trailing newline byte) — the zero-copy multiline merge
        // relies on this contiguity.
        let reader = make(b"first\nsecond\nthird\n");
        let r0 = reader.line_byte_range(0);
        let r1 = reader.line_byte_range(1);
        assert_eq!(r0.end + 1, r1.start); // +1 skips the '\n' between lines
    }

    fn make_tmp(lines: &[&str]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        f
    }

    #[tokio::test]
    async fn test_load_no_predicate_no_precomputed_visible() {
        let f = make_tmp(&["line1", "line2"]);
        let path = f.path().to_str().unwrap().to_string();
        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)), false)
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert!(result.precomputed_visible.is_none());
        assert_eq!(result.reader.line_count(), 2);
    }

    #[tokio::test]
    async fn test_load_predicate_forward_filters_correctly() {
        use crate::filters::{FilterDecision, FilterManager, SubstringFilter};
        let f = make_tmp(&["ERROR: bad", "INFO: ok", "ERROR: also bad"]);
        let path = f.path().to_str().unwrap().to_string();
        let filter =
            SubstringFilter::new("ERROR", FilterDecision::Include, false, 0, false).unwrap();
        let fm = FilterManager::new(vec![Box::new(filter)], true);
        let pred = VisibilityPredicate::new(fm);
        let handle = FileReader::load(
            path,
            Some(pred),
            false,
            Arc::new(AtomicBool::new(false)),
            false,
        )
        .await
        .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(result.precomputed_visible, Some(vec![0, 2]));
        assert_eq!(result.precomputed_text_counts, Some(vec![2]));
    }

    #[tokio::test]
    async fn test_load_predicate_tail_result_is_ascending() {
        use crate::filters::{FilterDecision, FilterManager, SubstringFilter};
        let f = make_tmp(&["ERROR: first", "INFO: skip", "ERROR: last"]);
        let path = f.path().to_str().unwrap().to_string();
        let filter =
            SubstringFilter::new("ERROR", FilterDecision::Include, false, 0, false).unwrap();
        let fm = FilterManager::new(vec![Box::new(filter)], true);
        let pred = VisibilityPredicate::new(fm);
        let handle = FileReader::load(
            path,
            Some(pred),
            true,
            Arc::new(AtomicBool::new(false)),
            false,
        )
        .await
        .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        let visible = result.precomputed_visible.unwrap();
        // Backward evaluation, but result must be sorted ascending.
        assert_eq!(visible, vec![0, 2]);
        assert!(visible.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(result.precomputed_text_counts, Some(vec![2]));
    }

    #[tokio::test]
    async fn test_load_predicate_ansi_file_indices_correct() {
        // ANSI file: predicate must evaluate against stripped bytes and indices
        // must reference stripped-line positions (same as get_line returns).
        use crate::filters::{FilterDecision, FilterManager, SubstringFilter};
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "\x1b[32mERROR\x1b[0m: red").unwrap(); // line 0 — contains ERROR
        writeln!(f, "\x1b[32mINFO\x1b[0m: green").unwrap(); // line 1 — skipped
        writeln!(f, "\x1b[31mERROR\x1b[0m: also red").unwrap(); // line 2 — contains ERROR
        let path = f.path().to_str().unwrap().to_string();

        let filter =
            SubstringFilter::new("ERROR", FilterDecision::Include, false, 0, false).unwrap();
        let fm = FilterManager::new(vec![Box::new(filter)], true);
        let pred = VisibilityPredicate::new(fm);
        let handle = FileReader::load(
            path,
            Some(pred),
            false,
            Arc::new(AtomicBool::new(false)),
            false,
        )
        .await
        .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();

        // Predicate operates on stripped bytes ("ERROR: red", etc.)
        assert_eq!(result.precomputed_visible, Some(vec![0, 2]));
        // Verify get_line also returns stripped bytes — indices are consistent.
        assert_eq!(result.reader.get_line(0), b"ERROR: red");
        assert_eq!(result.reader.get_line(2), b"ERROR: also red");
    }

    #[tokio::test]
    async fn test_load_predicate_tail_all_match() {
        let f = make_tmp(&["a", "b", "c"]);
        let path = f.path().to_str().unwrap().to_string();
        let pred = VisibilityPredicate::new(crate::filters::FilterManager::empty());
        let handle = FileReader::load(
            path,
            Some(pred),
            true,
            Arc::new(AtomicBool::new(false)),
            false,
        )
        .await
        .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(result.precomputed_visible, Some(vec![0, 1, 2]));
        assert_eq!(result.precomputed_text_counts, Some(vec![]));
    }

    #[tokio::test]
    async fn test_load_predicate_none_match() {
        use crate::filters::{FilterDecision, FilterManager, SubstringFilter};
        let f = make_tmp(&["INFO: ok", "DEBUG: verbose"]);
        let path = f.path().to_str().unwrap().to_string();
        let filter =
            SubstringFilter::new("ERROR", FilterDecision::Include, false, 0, false).unwrap();
        let fm = FilterManager::new(vec![Box::new(filter)], true);
        let pred = VisibilityPredicate::new(fm);
        let handle = FileReader::load(
            path,
            Some(pred),
            false,
            Arc::new(AtomicBool::new(false)),
            false,
        )
        .await
        .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(result.precomputed_visible, Some(vec![]));
        assert_eq!(result.precomputed_text_counts, Some(vec![0]));
    }

    #[test]
    fn test_empty_file() {
        let r = make(b"");
        assert_eq!(r.line_count(), 0);
    }

    #[test]
    fn test_single_line_no_newline() {
        let r = make(b"hello");
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b"hello");
    }

    #[test]
    fn test_single_line_with_newline() {
        let r = make(b"hello\n");
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b"hello");
    }

    #[test]
    fn test_multiple_lines() {
        let r = make(b"line1\nline2\nline3\n");
        assert_eq!(r.line_count(), 3);
        assert_eq!(r.get_line(0), b"line1");
        assert_eq!(r.get_line(1), b"line2");
        assert_eq!(r.get_line(2), b"line3");
    }

    #[test]
    fn test_multiple_lines_no_trailing_newline() {
        let r = make(b"line1\nline2\nline3");
        assert_eq!(r.line_count(), 3);
        assert_eq!(r.get_line(0), b"line1");
        assert_eq!(r.get_line(1), b"line2");
        assert_eq!(r.get_line(2), b"line3");
    }

    #[test]
    fn test_iter() {
        let r = make(b"a\nb\nc\n");
        let collected: Vec<(usize, LineBytes<'_>)> = r.iter().collect();
        assert_eq!(collected.len(), 3);
        assert_eq!(collected[0].0, 0);
        assert_eq!(collected[0].1, b"a".as_ref());
        assert_eq!(collected[1].0, 1);
        assert_eq!(collected[1].1, b"b".as_ref());
        assert_eq!(collected[2].0, 2);
        assert_eq!(collected[2].1, b"c".as_ref());
    }

    #[test]
    fn test_file_reader_from_path() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "[2024-07-24T10:00:00Z] INFO myhost: line 1").unwrap();
        writeln!(f, "[2024-07-24T10:01:00Z] DEBUG myhost: line 2").unwrap();
        let path = f.path().to_str().unwrap();

        let reader = FileReader::new(path).unwrap();
        assert_eq!(reader.line_count(), 2);

        let line0 = reader.get_line(0);
        let l0 = std::str::from_utf8(&line0).unwrap();
        assert!(l0.contains("INFO"));
        let line1 = reader.get_line(1);
        let l1 = std::str::from_utf8(&line1).unwrap();
        assert!(l1.contains("DEBUG"));
    }

    #[test]
    fn test_empty_lines_in_content() {
        let r = make(b"first\n\nthird\n");
        assert_eq!(r.line_count(), 3);
        assert_eq!(r.get_line(0), b"first");
        assert_eq!(r.get_line(1), b"");
        assert_eq!(r.get_line(2), b"third");
    }

    #[test]
    fn test_strip_ansi_csi_color_codes() {
        let r = make(b"\x1b[32m INFO\x1b[0m message\n");
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b" INFO message");
    }

    #[test]
    fn test_strip_carriage_return() {
        let r = make(b"line1\r\nline2\r\n");
        assert_eq!(r.line_count(), 2);
        assert_eq!(r.get_line(0), b"line1");
        assert_eq!(r.get_line(1), b"line2");
    }

    #[test]
    fn test_strip_ansi_real_log_line() {
        // Simulates a tracing-subscriber log line with dim/color codes
        let input = b"\x1b[2m2026-02-20T15:06:28Z\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2mtodo_app\x1b[0m: message\n";
        let r = make(input);
        assert_eq!(r.line_count(), 1);
        assert_eq!(
            r.get_line(0),
            b"2026-02-20T15:06:28Z  INFO todo_app: message"
        );
    }

    #[test]
    fn test_no_ansi_unchanged() {
        let r = make(b"plain log line\n");
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b"plain log line");
    }

    #[test]
    fn test_append_bytes_basic() {
        let mut r = make(b"line1\nline2\n");
        assert_eq!(r.line_count(), 2);
        r.append_bytes(b"line3\nline4\n");
        assert_eq!(r.line_count(), 4);
        assert_eq!(r.get_line(2), b"line3");
        assert_eq!(r.get_line(3), b"line4");
    }

    #[test]
    fn test_append_bytes_extends_partial_last_line() {
        let mut r = make(b"partial");
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b"partial");
        r.append_bytes(b"ly done\nnext\n");
        assert_eq!(r.line_count(), 2);
        assert_eq!(r.get_line(0), b"partially done");
        assert_eq!(r.get_line(1), b"next");
    }

    #[test]
    fn test_append_bytes_empty_is_noop() {
        let mut r = make(b"line1\n");
        r.append_bytes(b"");
        assert_eq!(r.line_count(), 1);
    }

    #[test]
    fn test_strip_osc_terminated_by_bel() {
        // OSC: ESC ] ... BEL (0x07)
        let input = b"\x1b]0;my title\x07rest of line\n";
        let r = make(input);
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b"rest of line");
    }

    #[test]
    fn test_strip_osc_terminated_by_st() {
        // OSC: ESC ] ... ESC backslash (ST)
        let input = b"\x1b]0;my title\x1b\\rest\n";
        let r = make(input);
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b"rest");
    }

    #[test]
    fn test_strip_osc_mixed_with_csi() {
        let input = b"\x1b]0;title\x07\x1b[32mGREEN\x1b[0m\n";
        let r = make(input);
        assert_eq!(r.get_line(0), b"GREEN");
    }

    #[test]
    fn test_strip_two_byte_esc_sequence() {
        // ESC M (reverse index), ESC = (keypad mode), etc.
        let input = b"before\x1bMafter\n";
        let r = make(input);
        assert_eq!(r.get_line(0), b"beforeafter");
    }

    #[test]
    fn test_strip_multiple_two_byte_esc() {
        let input = b"\x1b=\x1b>hello\n";
        let r = make(input);
        assert_eq!(r.get_line(0), b"hello");
    }

    #[test]
    fn test_strip_esc_at_end_of_input() {
        // Truncated: ESC is the very last byte
        let out = strip_ansi_escapes(b"hello\x1b");
        assert_eq!(out, b"hello");
    }

    #[test]
    fn test_strip_truncated_csi() {
        // CSI that never gets a final byte (0x40-0x7E) — consume until end
        let out = strip_ansi_escapes(b"hi\x1b[31");
        assert_eq!(out, b"hi");
    }

    #[test]
    fn test_strip_empty_input() {
        let out = strip_ansi_escapes(b"");
        assert!(out.is_empty());
    }

    #[test]
    fn test_strip_only_escapes() {
        let out = strip_ansi_escapes(b"\x1b[32m\x1b[0m\r");
        assert!(out.is_empty());
    }

    #[test]
    fn test_strip_complex_csi_with_params() {
        // CSI with multiple params: ESC [ 38;5;196 m (256-color red)
        let input = b"\x1b[38;5;196mred text\x1b[0m\n";
        let r = make(input);
        assert_eq!(r.get_line(0), b"red text");
    }

    #[test]
    fn test_strip_cr_only_lines() {
        // Lines with only CR (no LF)
        let out = strip_ansi_escapes(b"hello\rworld");
        assert_eq!(out, b"helloworld");
    }

    #[test]
    fn test_only_newlines() {
        let r = make(b"\n\n\n");
        assert_eq!(r.line_count(), 3);
        assert_eq!(r.get_line(0), b"");
        assert_eq!(r.get_line(1), b"");
        assert_eq!(r.get_line(2), b"");
    }

    #[test]
    fn test_single_newline() {
        let r = make(b"\n");
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b"");
    }

    #[test]
    fn test_large_number_of_lines() {
        let mut data = Vec::new();
        for i in 0..10_000 {
            data.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        let r = make(&data);
        assert_eq!(r.line_count(), 10_000);
        assert_eq!(r.get_line(0), b"line 0");
        assert_eq!(r.get_line(9_999), b"line 9999");
    }

    #[test]
    fn test_long_single_line() {
        let line = vec![b'x'; 100_000];
        let r = make(&line);
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0).len(), 100_000);
    }

    #[test]
    fn test_binary_content_no_newlines() {
        let data: Vec<u8> = (0..=255).collect();
        // This has 0x0a (newline) at position 10 and 0x1b (ESC) at position 27
        // After stripping, there should be content split at newline positions
        let r = make(&data);
        assert!(r.line_count() >= 1);
    }

    #[test]
    fn test_append_bytes_multiple_times() {
        let mut r = make(b"a\n");
        r.append_bytes(b"b\n");
        r.append_bytes(b"c\n");
        r.append_bytes(b"d\n");
        assert_eq!(r.line_count(), 4);
        assert_eq!(r.get_line(0), b"a");
        assert_eq!(r.get_line(1), b"b");
        assert_eq!(r.get_line(2), b"c");
        assert_eq!(r.get_line(3), b"d");
    }

    #[test]
    fn test_append_bytes_no_newline_then_newline() {
        let mut r = make(b"start");
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b"start");

        r.append_bytes(b" middle");
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b"start middle");

        r.append_bytes(b" end\nnew\n");
        assert_eq!(r.line_count(), 2);
        assert_eq!(r.get_line(0), b"start middle end");
        assert_eq!(r.get_line(1), b"new");
    }

    #[test]
    fn test_append_to_empty() {
        let mut r = make(b"");
        assert_eq!(r.line_count(), 0);
        r.append_bytes(b"hello\n");
        assert_eq!(r.line_count(), 1);
        assert_eq!(r.get_line(0), b"hello");
    }

    #[test]
    fn test_file_reader_from_path_with_ansi() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"\x1b[32mgreen\x1b[0m\nplain\n").unwrap();
        let path = f.path().to_str().unwrap();
        let reader = FileReader::new(path).unwrap();
        assert_eq!(reader.line_count(), 2);
        assert_eq!(reader.get_line(0), b"green");
        assert_eq!(reader.get_line(1), b"plain");
    }

    #[test]
    fn test_file_reader_from_path_with_crlf() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"line1\r\nline2\r\n").unwrap();
        let path = f.path().to_str().unwrap();
        let reader = FileReader::new(path).unwrap();
        assert_eq!(reader.line_count(), 2);
        assert_eq!(reader.get_line(0), b"line1");
        assert_eq!(reader.get_line(1), b"line2");
    }

    #[test]
    fn test_file_reader_nonexistent_path() {
        let result = FileReader::new("/tmp/nonexistent_logana_test_file.log");
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_line_starts_empty() {
        let starts = compute_line_starts(b"");
        assert_eq!(starts, vec![0]);
    }

    #[test]
    fn test_compute_line_starts_no_newline() {
        let starts = compute_line_starts(b"hello");
        assert_eq!(starts, vec![0]);
    }

    #[test]
    fn test_compute_line_starts_one_newline() {
        let starts = compute_line_starts(b"hello\n");
        assert_eq!(starts, vec![0, 6]);
    }

    #[test]
    fn test_compute_line_starts_multiple() {
        let starts = compute_line_starts(b"ab\ncd\nef\n");
        assert_eq!(starts, vec![0, 3, 6, 9]);
    }

    #[test]
    fn test_compute_line_starts_consecutive_newlines() {
        let starts = compute_line_starts(b"\n\n\n");
        assert_eq!(starts, vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn test_load_basic() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line 1").unwrap();
        writeln!(f, "line 2").unwrap();
        writeln!(f, "line 3").unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)), false)
            .await
            .unwrap();
        assert!(handle.total_bytes > 0);

        let result = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(result.reader.line_count(), 3);
        assert_eq!(result.reader.get_line(0), b"line 1");
    }

    #[tokio::test]
    async fn test_load_progress_reaches_one() {
        let mut f = NamedTempFile::new().unwrap();
        for i in 0..100 {
            writeln!(f, "line {i}").unwrap();
        }
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)), false)
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(result.reader.line_count(), 100);

        // After completion, progress should be 1.0
        let progress = *handle.progress_rx.borrow();
        assert!((progress - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_load_with_ansi() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"\x1b[31mred\x1b[0m\nplain\n").unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)), false)
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(result.reader.line_count(), 2);
        assert_eq!(result.reader.get_line(0), b"red");
        assert_eq!(result.reader.get_line(1), b"plain");
    }

    #[tokio::test]
    async fn test_load_nonexistent() {
        let result = FileReader::load(
            "/tmp/nonexistent_logana_load_test.log".to_string(),
            None,
            false,
            Arc::new(AtomicBool::new(false)),
            false,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_load_empty_file() {
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)), false)
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(result.reader.line_count(), 0);
    }

    #[tokio::test]
    async fn test_load_cancel_returns_error() {
        let f = make_tmp(&["line1", "line2", "line3"]);
        let path = f.path().to_str().unwrap().to_string();
        let cancel = Arc::new(AtomicBool::new(true)); // pre-cancelled
        let handle = FileReader::load(path, None, false, cancel, false)
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap();
        // The load should have returned an Interrupted error because cancel was pre-set.
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().kind(),
            std::io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn test_try_extend_from_read_bytes_returns_false() {
        let mut reader = make(b"line1\nline2\n");
        assert!(!reader.try_extend_from_read().unwrap());
    }

    #[test]
    fn test_try_extend_from_read_appends_new_lines() {
        use std::io::Write;

        let mut f = NamedTempFile::new().unwrap();
        write!(f, "line1\nline2\n").unwrap();
        f.flush().unwrap();

        let mut reader = FileReader::new(f.path().to_str().unwrap()).unwrap();
        assert_eq!(reader.line_count(), 2);

        write!(f, "line3\nline4\n").unwrap();
        f.flush().unwrap();

        assert!(reader.try_extend_from_read().unwrap());
        assert_eq!(reader.line_count(), 4);
        assert_eq!(reader.get_line(2), b"line3");
        assert_eq!(reader.get_line(3), b"line4");
    }

    #[test]
    fn test_try_extend_from_read_unchanged_returns_true() {
        use std::io::Write;

        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line1").unwrap();
        f.flush().unwrap();

        let mut reader = FileReader::new(f.path().to_str().unwrap()).unwrap();
        // No new data — should return true (no-op success).
        assert!(reader.try_extend_from_read().unwrap());
        assert_eq!(reader.line_count(), 1);
    }

    #[test]
    fn test_try_extend_from_read_truncation_returns_false() {
        use std::io::Write;

        let mut f = NamedTempFile::new().unwrap();
        write!(f, "line1\nline2\nline3\n").unwrap();
        f.flush().unwrap();

        let mut reader = FileReader::new(f.path().to_str().unwrap()).unwrap();
        assert_eq!(reader.line_count(), 3);

        // Truncate the file — try_extend_from_read must signal a full reload.
        f.as_file().set_len(0).unwrap();

        assert!(!reader.try_extend_from_read().unwrap());
    }

    #[test]
    fn test_try_new_paged_builds_storage_paged_and_matches_full_read() {
        let data = (0..5000)
            .map(|i| format!("line-{i:05}-payload"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let mut f = NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut f, data.as_bytes()).unwrap();
        f.flush().unwrap();

        let path = f.path().to_str().unwrap();
        let canonical = Arc::new(std::fs::canonicalize(path).unwrap());
        let paged_reader = FileReader::try_new_paged(path, canonical)
            .unwrap()
            .expect("plain text file should build Storage::Paged");
        let full_reader = FileReader::new(path).unwrap();

        assert!(paged_reader.is_paged());
        assert_eq!(paged_reader.line_count(), full_reader.line_count());
        for idx in [0usize, 1, 2500, 4999] {
            assert_eq!(paged_reader.get_line(idx), full_reader.get_line(idx));
        }
    }

    #[test]
    fn test_try_new_paged_index_build_spans_multiple_parallel_chunks() {
        // Each line is ~20 bytes; enough lines to exceed
        // PAGED_INDEX_SCAN_CHUNK_SIZE (8MB) several times over, so the
        // parallel index-build scan actually exercises multiple chunks
        // merged back together in order — not just the single-chunk case
        // every other small-file test in this module hits.
        let line_count = 1_500_000;
        let data = (0..line_count)
            .map(|i| format!("line-{i:07}-payload"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert!(
            data.len() > PAGED_INDEX_SCAN_CHUNK_SIZE * 3,
            "test data too small to force multiple index-scan chunks"
        );

        let mut f = NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut f, data.as_bytes()).unwrap();
        f.flush().unwrap();
        let path = f.path().to_str().unwrap();
        let canonical = Arc::new(std::fs::canonicalize(path).unwrap());

        let paged_reader = FileReader::try_new_paged(path, canonical)
            .unwrap()
            .expect("plain text file should build Storage::Paged");
        let full_reader = FileReader::new(path).unwrap();

        assert_eq!(paged_reader.line_starts(), full_reader.line_starts());
        assert_eq!(paged_reader.line_count(), line_count);
        for idx in [0usize, 1, 250_000, line_count - 1] {
            assert_eq!(paged_reader.get_line(idx), full_reader.get_line(idx));
        }
    }

    fn make_multiline_reader(lines: usize) -> (FileReader, FileReader, String) {
        let data = (0..lines)
            .map(|i| format!("line-{i:06}-payload"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let mut f = NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut f, data.as_bytes()).unwrap();
        f.flush().unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let canonical = Arc::new(std::fs::canonicalize(&path).unwrap());
        let paged = FileReader::try_new_paged(&path, canonical)
            .unwrap()
            .expect("plain text file should build Storage::Paged");
        let full = FileReader::new(&path).unwrap();
        // Keep the temp file alive for the caller by leaking it into `path`'s
        // scope via NamedTempFile's Drop — instead, persist it so it isn't
        // deleted before the caller is done using the readers.
        let _ = f.into_temp_path().keep().unwrap();
        (paged, full, path)
    }

    #[test]
    fn test_materialize_chunk_matches_manual_slice_mid_file() {
        let (paged, full, path) = make_multiline_reader(5000);
        let chunk_start = 1200;
        let chunk_end = 1800;

        let (chunk_bytes, local_starts) = paged.materialize_chunk(chunk_start, chunk_end);

        let global_starts = full.line_starts();
        let byte_start = global_starts[chunk_start];
        let byte_end = global_starts[chunk_end];
        let expected_bytes = full.materialize_range(byte_start..byte_end);
        assert_eq!(&*chunk_bytes, &*expected_bytes);

        let expected_local_starts: Vec<usize> = global_starts[chunk_start..=chunk_end]
            .iter()
            .map(|&s| s - byte_start)
            .collect();
        assert_eq!(local_starts, expected_local_starts);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_materialize_chunk_last_chunk_with_trailing_newline() {
        // File ends with '\n', so line_starts has a phantom entry at
        // data.len() past the last real line — chunk_end lands exactly on
        // it (the `chunk_end < line_starts.len()` branch).
        let (paged, full, path) = make_multiline_reader(100);
        let line_count = paged.line_count();
        let chunk_start = 90;
        let chunk_end = line_count;

        let (chunk_bytes, local_starts) = paged.materialize_chunk(chunk_start, chunk_end);

        let byte_start = full.line_starts()[chunk_start];
        let expected_bytes = full.materialize_range(byte_start..full.total_len());
        assert_eq!(&*chunk_bytes, &*expected_bytes);
        assert_eq!(*local_starts.last().unwrap(), chunk_bytes.len());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_materialize_chunk_last_chunk_without_trailing_newline() {
        // No trailing '\n' on the last line — line_starts has NO phantom
        // entry past it, so chunk_end == line_starts.len(), exercising the
        // `chunk_end >= line_starts.len()` fallback to total_len().
        let data = (0..100)
            .map(|i| format!("line-{i:06}-payload"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut f = NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut f, data.as_bytes()).unwrap();
        f.flush().unwrap();
        let path = f.path().to_str().unwrap().to_string();
        let canonical = Arc::new(std::fs::canonicalize(&path).unwrap());
        let paged = FileReader::try_new_paged(&path, canonical)
            .unwrap()
            .expect("plain text file should build Storage::Paged");
        let full = FileReader::new(&path).unwrap();
        assert_eq!(paged.line_starts().len(), paged.line_count());

        let chunk_start = 90;
        let chunk_end = paged.line_count();
        let (chunk_bytes, local_starts) = paged.materialize_chunk(chunk_start, chunk_end);

        let byte_start = full.line_starts()[chunk_start];
        let expected_bytes = full.materialize_range(byte_start..full.total_len());
        assert_eq!(&*chunk_bytes, &*expected_bytes);
        assert_eq!(*local_starts.last().unwrap(), chunk_bytes.len());
    }

    /// End-to-end check of the paged whole-file fast path: a file with more
    /// lines than `headless::run_headless_to_writer`'s internal
    /// `CHUNK_LINES` (16_384) forces at least 2 chunks — including a match
    /// straddling the chunk boundary — and the paged reader's output must be
    /// byte-identical to the same filter run against a fully-resident
    /// reader over the same content.
    #[tokio::test]
    async fn test_run_headless_wholefile_paged_matches_non_paged_across_chunks() {
        use crate::db::{Database, LogManager};
        use crate::filters::{FilterOptions, FilterType};

        let total_lines = 20_000;
        let mut data = String::new();
        for i in 0..total_lines {
            // Deliberately place matches right around the 16_384-line chunk
            // boundary, not just scattered uniformly, so a rebasing bug at
            // the boundary would actually be exercised.
            let marker = if (16_380..16_388).contains(&i) || i % 37 == 0 {
                "MATCH"
            } else {
                "skip"
            };
            data.push_str(&format!("line-{i:06}-{marker}\n"));
        }
        let mut f = NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut f, data.as_bytes()).unwrap();
        f.flush().unwrap();
        let path = f.path().to_str().unwrap().to_string();
        let canonical = Arc::new(std::fs::canonicalize(&path).unwrap());

        let paged_reader = FileReader::try_new_paged(&path, canonical)
            .unwrap()
            .expect("plain text file should build Storage::Paged");
        assert!(paged_reader.is_paged());
        let plain_reader = FileReader::new(&path).unwrap();
        assert!(!plain_reader.is_paged());

        let mut lm_paged =
            LogManager::new(Arc::new(Database::in_memory().await.unwrap()), None).await;
        lm_paged
            .add_filter_with_color(
                "MATCH".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;
        let mut lm_plain =
            LogManager::new(Arc::new(Database::in_memory().await.unwrap()), None).await;
        lm_plain
            .add_filter_with_color(
                "MATCH".to_string(),
                FilterType::Include,
                FilterOptions::default(),
            )
            .await;

        let mut paged_out = Vec::new();
        crate::headless::run_headless_to_writer(paged_reader, &lm_paged, &mut paged_out).unwrap();
        let mut plain_out = Vec::new();
        crate::headless::run_headless_to_writer(plain_reader, &lm_plain, &mut plain_out).unwrap();

        assert!(!paged_out.is_empty());
        assert_eq!(paged_out, plain_out);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_try_extend_from_read_paged_growth_appends_new_lines() {
        use std::io::Write;

        let mut f = NamedTempFile::new().unwrap();
        write!(f, "line1\nline2\n").unwrap();
        f.flush().unwrap();

        let path = f.path().to_str().unwrap();
        let canonical = Arc::new(std::fs::canonicalize(path).unwrap());
        let mut reader = FileReader::try_new_paged(path, canonical)
            .unwrap()
            .expect("plain text file should build Storage::Paged");
        assert!(reader.is_paged());
        assert_eq!(reader.line_count(), 2);

        write!(f, "line3\nline4\n").unwrap();
        f.flush().unwrap();

        assert!(reader.try_extend_from_read().unwrap());
        assert!(reader.is_paged(), "growth must stay on Storage::Paged");
        assert_eq!(reader.line_count(), 4);
        assert_eq!(reader.get_line(2), b"line3");
        assert_eq!(reader.get_line(3), b"line4");
    }

    #[test]
    fn test_try_extend_from_read_paged_growth_within_cached_page_is_visible() {
        use std::io::Write;

        // The whole file is a handful of bytes — well within page 0's
        // 2MB span — so reading line 0 caches page 0 at its OLD (short)
        // length. Growing the file and re-reading exercises exactly the
        // stale-cached-page bug `PagedFile::set_size` guards against: the
        // grown bytes must be visible, not silently dropped by a stale
        // cache hit.
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "line1\nline2\n").unwrap();
        f.flush().unwrap();
        let path = f.path().to_str().unwrap();
        let canonical = Arc::new(std::fs::canonicalize(path).unwrap());
        let mut reader = FileReader::try_new_paged(path, canonical)
            .unwrap()
            .expect("plain text file should build Storage::Paged");

        // Touch line 0 so its (default-sized, 2MB) page gets cached.
        assert_eq!(reader.get_line(0), b"line1");

        writeln!(f, "line3").unwrap();
        f.flush().unwrap();
        assert!(reader.try_extend_from_read().unwrap());
        assert_eq!(reader.line_count(), 3);
        assert_eq!(reader.get_line(2), b"line3");
    }

    #[tokio::test]
    async fn test_spawn_file_watcher_detects_new_data() {
        use std::io::{Seek, SeekFrom};
        use tokio::time::{Duration, sleep};

        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "initial").unwrap();
        f.flush().unwrap();
        let initial_size = f.as_file().metadata().unwrap().len();
        let path = f.path().to_str().unwrap().to_string();

        let rx = FileReader::spawn_file_watcher(path, initial_size).await;

        // Append new data to the file
        f.seek(SeekFrom::End(0)).unwrap();
        writeln!(f, "appended").unwrap();
        f.flush().unwrap();

        // Wait for the watcher to detect the change (polls every 50ms)
        sleep(Duration::from_millis(200)).await;

        assert!(
            rx.has_changed().unwrap(),
            "watcher should have sent a notification"
        );
        let text = std::fs::read_to_string(f.path()).unwrap();
        assert!(
            text.contains("appended"),
            "file should contain appended data, got: {text}"
        );
    }

    #[test]
    fn test_iter_empty() {
        let r = make(b"");
        let collected: Vec<_> = r.iter().collect();
        assert!(collected.is_empty());
    }

    #[test]
    fn test_iter_single_no_newline() {
        let r = make(b"only");
        let collected: Vec<_> = r.iter().collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].0, 0);
        assert_eq!(collected[0].1, b"only".as_ref());
    }

    #[tokio::test]
    async fn test_spawn_process_stream_basic() {
        let (mut rx, tmp) = FileReader::spawn_process_stream("echo", &["hello world"], false)
            .await
            .unwrap();

        // Wait for the process to finish and the final flush.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        rx.borrow_and_update();
        let text = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            text.contains("hello world"),
            "stdout should be captured, got: {text}"
        );
    }

    #[tokio::test]
    async fn test_spawn_process_stream_stderr() {
        // Use sh -c to write to stderr
        // tag_stderr=false: stderr merged as-is (no prefix)
        let (mut rx, tmp) =
            FileReader::spawn_process_stream("sh", &["-c", "echo error_output >&2"], false)
                .await
                .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        rx.borrow_and_update();
        let text = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            text.contains("error_output"),
            "stderr should be captured, got: {text}"
        );
        assert!(
            !text.contains("ERROR "),
            "tag_stderr=false should not prefix stderr, got: {text}"
        );

        // tag_stderr=true: each stderr line is prefixed with "ERROR "
        let (mut rx2, tmp2) =
            FileReader::spawn_process_stream("sh", &["-c", "echo error_output >&2"], true)
                .await
                .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        rx2.borrow_and_update();
        let text2 = std::fs::read_to_string(tmp2.path()).unwrap();
        assert!(
            text2.contains("ERROR error_output"),
            "tag_stderr=true should prefix stderr lines, got: {text2}"
        );
    }

    #[tokio::test]
    async fn test_spawn_process_stream_strips_ansi() {
        // printf outputs ANSI codes; they should be stripped
        let (mut rx, tmp) =
            FileReader::spawn_process_stream("printf", &["\x1b[31mred text\x1b[0m\n"], false)
                .await
                .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        rx.borrow_and_update();
        let text = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            text.contains("red text"),
            "should contain stripped text, got: {text}"
        );
        assert!(
            !text.contains("\x1b["),
            "ANSI codes should be stripped, got: {text}"
        );
    }

    fn strip_bytes(input: &[u8]) -> Vec<u8> {
        strip_ansi_and_index(input).0
    }

    fn index_bytes(input: &[u8]) -> Vec<usize> {
        strip_ansi_and_index(input).1
    }

    #[test]
    fn test_strip_ansi_and_index_plain() {
        // No ANSI codes — output equals input, starts are identical to
        // compute_line_starts.
        let input = b"hello\nworld\n";
        assert_eq!(strip_bytes(input), input);
        assert_eq!(index_bytes(input), compute_line_starts(input));
    }

    #[test]
    fn test_strip_ansi_and_index_csi() {
        // CSI colour codes are stripped; content bytes and newlines kept.
        let input = b"\x1b[32mgreen\x1b[0m\nplain\n";
        let expected_bytes = b"green\nplain\n";
        let (out, starts) = strip_ansi_and_index(input);
        assert_eq!(out, expected_bytes);
        assert_eq!(starts, compute_line_starts(expected_bytes));
    }

    #[test]
    fn test_strip_ansi_and_index_osc_bel() {
        // OSC sequence terminated by BEL.
        let input = b"\x1b]0;title\x07line\n";
        let expected_bytes = b"line\n";
        let (out, starts) = strip_ansi_and_index(input);
        assert_eq!(out, expected_bytes);
        assert_eq!(starts, compute_line_starts(expected_bytes));
    }

    #[test]
    fn test_strip_ansi_and_index_osc_string_terminator() {
        // OSC sequence terminated by ESC \.
        let input = b"\x1b]0;title\x1b\\line\n";
        let expected_bytes = b"line\n";
        let (out, starts) = strip_ansi_and_index(input);
        assert_eq!(out, expected_bytes);
        assert_eq!(starts, compute_line_starts(expected_bytes));
    }

    #[test]
    fn test_strip_ansi_and_index_two_byte_esc() {
        // Two-byte escape sequence (ESC + one byte, not [ or ]).
        let input = b"\x1b=text\n";
        let expected_bytes = b"text\n";
        let (out, starts) = strip_ansi_and_index(input);
        assert_eq!(out, expected_bytes);
        assert_eq!(starts, compute_line_starts(expected_bytes));
    }

    #[test]
    fn test_strip_ansi_and_index_cr_stripped() {
        // Bare \r is stripped; \r\n becomes just \n.
        let input = b"line1\r\nline2\r\n";
        let expected_bytes = b"line1\nline2\n";
        let (out, starts) = strip_ansi_and_index(input);
        assert_eq!(out, expected_bytes);
        assert_eq!(starts, compute_line_starts(expected_bytes));
    }

    #[test]
    fn test_strip_ansi_and_index_multiline_ansi() {
        // Multiple lines each with ANSI codes — matches separate strip + index.
        let input = b"\x1b[32mfoo\x1b[0m\n\x1b[34mbar\x1b[0m\nbaz\n";
        let stripped = strip_ansi_escapes(input);
        let expected_starts = compute_line_starts(&stripped);
        let (out, starts) = strip_ansi_and_index(input);
        assert_eq!(out, stripped);
        assert_eq!(starts, expected_starts);
    }

    #[test]
    fn test_strip_ansi_and_index_no_trailing_newline() {
        // Last line has no newline — starts has one entry (just 0).
        let input = b"\x1b[1mhello\x1b[0m";
        let stripped = strip_ansi_escapes(input);
        let expected_starts = compute_line_starts(&stripped);
        let (out, starts) = strip_ansi_and_index(input);
        assert_eq!(out, stripped);
        assert_eq!(starts, expected_starts);
    }

    #[test]
    fn test_strip_ansi_and_index_esc_at_end() {
        // Dangling ESC at end of input is silently dropped.
        let input = b"text\n\x1b";
        let stripped = strip_ansi_escapes(input);
        let (out, starts) = strip_ansi_and_index(input);
        assert_eq!(out, stripped);
        assert_eq!(starts, compute_line_starts(&stripped));
    }

    #[test]
    fn test_strip_ansi_and_index_empty() {
        let (out, starts) = strip_ansi_and_index(b"");
        assert!(out.is_empty());
        assert_eq!(starts, vec![0usize]);
    }

    #[test]
    fn test_strip_ansi_and_index_bulk_copy_long_plain_segment() {
        // A long plain segment (> 32 bytes) with no control bytes exercises the
        // memchr2 fast path that bulk-copies the safe region.
        let plain = b"abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ\n";
        let input: Vec<u8> = plain.repeat(10);
        let (out, starts) = strip_ansi_and_index(&input);
        assert_eq!(out, input.as_slice());
        assert_eq!(starts, compute_line_starts(&input));
    }

    #[test]
    fn test_strip_ansi_and_index_bulk_copy_ansi_surrounded_by_long_plain() {
        // Long plain prefix → ANSI escape → long plain suffix; verifies that both
        // bulk-copy segments and the slow escape-parser produce correct output.
        let prefix = b"a".repeat(100);
        let suffix = b"b".repeat(100);
        let mut input = prefix.clone();
        input.extend_from_slice(b"\x1b[32m");
        input.extend_from_slice(&suffix);
        input.push(b'\n');

        let mut expected = prefix;
        expected.extend_from_slice(&suffix);
        expected.push(b'\n');

        let (out, starts) = strip_ansi_and_index(&input);
        assert_eq!(out, expected);
        assert_eq!(starts, compute_line_starts(&expected));
    }

    #[test]
    fn test_strip_ansi_and_index_cr_only_no_newline() {
        // Bare \r with no following \n — CR is stripped, no new line_start emitted.
        let input = b"foo\rbar\n";
        let expected = b"foobar\n";
        let (out, starts) = strip_ansi_and_index(input);
        assert_eq!(out, expected);
        assert_eq!(starts, compute_line_starts(expected));
    }

    #[tokio::test]
    async fn test_from_file_tail_returns_last_lines() {
        let mut f = NamedTempFile::new().unwrap();
        for i in 0..1000usize {
            writeln!(f, "line {i}").unwrap();
        }
        f.flush().unwrap();
        let path = f.path().to_str().unwrap();

        let reader = FileReader::from_file_tail(path, 512).await.unwrap();
        let n = reader.line_count();
        assert!(n > 0, "should have at least one line");
        // The last line of the preview must match the last line of the full file.
        let last_preview = reader.get_line(n - 1);
        assert_eq!(last_preview, b"line 999");
    }

    #[tokio::test]
    async fn test_from_file_tail_all_lines_complete() {
        let mut f = NamedTempFile::new().unwrap();
        for i in 0..500usize {
            writeln!(f, "entry {i} data").unwrap();
        }
        f.flush().unwrap();
        let path = f.path().to_str().unwrap();

        // Every line returned must be a complete "entry N data" line.
        let reader = FileReader::from_file_tail(path, 1024).await.unwrap();
        for i in 0..reader.line_count() {
            let line = reader.get_line(i);
            assert!(
                line.starts_with(b"entry "),
                "partial line leaked: {:?}",
                std::str::from_utf8(&line)
            );
        }
    }

    #[tokio::test]
    async fn test_from_file_tail_small_file_fits_in_preview() {
        // When the file is smaller than preview_bytes the whole file is returned.
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "only line").unwrap();
        f.flush().unwrap();
        let path = f.path().to_str().unwrap();

        let reader = FileReader::from_file_tail(path, 64 * 1024).await.unwrap();
        assert_eq!(reader.line_count(), 1);
        assert_eq!(reader.get_line(0), b"only line");
    }

    #[tokio::test]
    async fn test_from_file_tail_nonexistent_returns_error() {
        let result = FileReader::from_file_tail("/tmp/logana_no_such_file_tail.log", 1024).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_from_file_head_returns_first_lines() {
        let mut f = NamedTempFile::new().unwrap();
        for i in 0..1000usize {
            writeln!(f, "line {i}").unwrap();
        }
        f.flush().unwrap();
        let path = f.path().to_str().unwrap();

        let reader = FileReader::from_file_head(path, 512).await.unwrap();
        let n = reader.line_count();
        assert!(n > 0, "should have at least one line");
        // The first line of the preview must match the first line of the full file.
        assert_eq!(reader.get_line(0), b"line 0");
    }

    #[tokio::test]
    async fn test_from_file_head_all_lines_complete() {
        let mut f = NamedTempFile::new().unwrap();
        for i in 0..500usize {
            writeln!(f, "entry {i} data").unwrap();
        }
        f.flush().unwrap();
        let path = f.path().to_str().unwrap();

        // Every line returned must be a complete "entry N data" line.
        let reader = FileReader::from_file_head(path, 1024).await.unwrap();
        for i in 0..reader.line_count() {
            let line = reader.get_line(i);
            assert!(
                line.starts_with(b"entry "),
                "partial line leaked: {:?}",
                std::str::from_utf8(&line)
            );
        }
    }

    #[tokio::test]
    async fn test_from_file_head_small_file_fits_in_preview() {
        // When the file is smaller than preview_bytes the whole file is returned.
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "only line").unwrap();
        f.flush().unwrap();
        let path = f.path().to_str().unwrap();

        let reader = FileReader::from_file_head(path, 64 * 1024).await.unwrap();
        assert_eq!(reader.line_count(), 1);
        assert_eq!(reader.get_line(0), b"only line");
    }

    #[tokio::test]
    async fn test_from_file_head_nonexistent_returns_error() {
        let result = FileReader::from_file_head("/tmp/logana_no_such_file_head.log", 1024).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_append_bytes_on_file_backed_reader() {
        // FileReader::new uses File storage. Appending should convert to Bytes.
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"mmap line\n").unwrap();
        f.flush().unwrap();
        let path = f.path().to_str().unwrap();

        let mut reader = FileReader::new(path).unwrap();
        assert_eq!(reader.line_count(), 1);
        assert_eq!(reader.get_line(0), b"mmap line");

        // This triggers the Mmap→Vec conversion in append_bytes (line 272)
        reader.append_bytes(b"appended\n");
        assert_eq!(reader.line_count(), 2);
        assert_eq!(reader.get_line(0), b"mmap line");
        assert_eq!(reader.get_line(1), b"appended");
    }

    #[tokio::test]
    async fn test_spawn_file_watcher_truncation() {
        use std::io::{Seek, SeekFrom};
        use tokio::time::{Duration, sleep};

        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "original data that is fairly long").unwrap();
        f.flush().unwrap();
        let initial_size = f.as_file().metadata().unwrap().len();
        let path = f.path().to_str().unwrap().to_string();

        let rx = FileReader::spawn_file_watcher(path.clone(), initial_size).await;

        // Step 1: truncate to 0 bytes — the watcher detects this and resets
        // its internal offset to 0.
        f.as_file().set_len(0).unwrap();
        sleep(Duration::from_millis(200)).await;

        // Step 2: write new data — now the file grows past the reset offset
        // and the watcher picks up the new content.
        f.seek(SeekFrom::Start(0)).unwrap();
        writeln!(f, "after truncation").unwrap();
        f.flush().unwrap();
        sleep(Duration::from_millis(200)).await;

        assert!(
            rx.has_changed().unwrap(),
            "watcher should have sent a notification after truncation"
        );
        let text = std::fs::read_to_string(f.path()).unwrap();
        assert!(
            text.contains("after truncation"),
            "file should contain data written after truncation, got: {text}"
        );
    }

    /// Build a file whose byte size exceeds the 4 MiB minimum chunk size so
    /// the parallel scan exercises at least two chunks on any machine.
    fn make_large_tmp(line: &str, target_bytes: usize) -> (NamedTempFile, usize) {
        let line_with_newline = format!("{line}\n");
        let n = (target_bytes / line_with_newline.len()).max(1);
        let mut f = NamedTempFile::new().unwrap();
        for _ in 0..n {
            f.write_all(line_with_newline.as_bytes()).unwrap();
        }
        f.flush().unwrap();
        (f, n)
    }

    #[tokio::test]
    async fn test_load_large_file_line_count_correct() {
        // Target ~6 MiB so the file spans at least two 4 MiB chunks regardless
        // of the rayon thread count.
        let line = "hello world this is a reasonably long log line for testing";
        let (f, expected_lines) = make_large_tmp(line, 6 * 1024 * 1024);
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)), false)
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();

        assert_eq!(result.reader.line_count(), expected_lines);
        assert_eq!(result.reader.get_line(0), line.as_bytes());
        assert_eq!(result.reader.get_line(expected_lines - 1), line.as_bytes());
    }

    #[tokio::test]
    async fn test_load_large_file_matches_reference_implementation() {
        // Verify that the parallel index produces line_starts identical to the
        // sequential reference implementation by round-tripping every line.
        let line = "2024-01-15T10:00:00Z INFO service: request processed id=42 dur=3ms";
        let (f, n) = make_large_tmp(line, 6 * 1024 * 1024);
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)), false)
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();

        assert_eq!(result.reader.line_count(), n);
        // Spot-check first, middle, and last lines — these cross chunk boundaries
        // when the file is larger than 4 MiB.
        for &idx in &[0, n / 4, n / 2, 3 * n / 4, n - 1] {
            assert_eq!(
                result.reader.get_line(idx),
                line.as_bytes(),
                "line {idx} mismatch"
            );
        }
    }

    #[tokio::test]
    async fn test_load_large_file_predicate_correct() {
        // Every even line starts with "EVEN"; every odd line with "ODD".
        // The predicate selects only even lines — verify the correct indices
        // across chunk boundaries.
        let mut f = NamedTempFile::new().unwrap();
        let n = 200_000usize;
        for i in 0..n {
            if i % 2 == 0 {
                writeln!(f, "EVEN line {i}").unwrap();
            } else {
                writeln!(f, "ODD line {i}").unwrap();
            }
        }
        f.flush().unwrap();
        let path = f.path().to_str().unwrap().to_string();

        use crate::filters::{FilterDecision, FilterManager, SubstringFilter};
        let filter =
            SubstringFilter::new("EVEN", FilterDecision::Include, false, 0, false).unwrap();
        let fm = FilterManager::new(vec![Box::new(filter)], true);
        let pred = VisibilityPredicate::new(fm);
        let handle = FileReader::load(
            path,
            Some(pred),
            false,
            Arc::new(AtomicBool::new(false)),
            false,
        )
        .await
        .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();

        let visible = result.precomputed_visible.unwrap();
        assert_eq!(visible.len(), n / 2);
        // All returned indices must point to "EVEN" lines.
        for &idx in visible
            .iter()
            .take(100)
            .chain(visible.iter().rev().take(100))
        {
            assert!(
                result.reader.get_line(idx).starts_with(b"EVEN"),
                "index {idx} should be an EVEN line"
            );
        }
        // Indices must be strictly ascending.
        assert!(visible.windows(2).all(|w| w[0] < w[1]));
    }

    #[tokio::test]
    async fn test_load_newline_at_chunk_boundary() {
        // Construct a file where a '\n' falls exactly at a 4 MiB boundary so the
        // parallel merger is forced to handle an offset of exactly chunk_size.
        // We write lines of a fixed width to place a newline at byte 4_194_304.
        const BOUNDARY: usize = 4 * 1024 * 1024;
        // A line of 63 bytes + '\n' = 64 bytes.  BOUNDARY / 64 = 65536 lines land
        // the (65536th) newline exactly at byte 4_194_304.
        let line = "A".repeat(63);
        let lines_to_boundary = BOUNDARY / 64;
        let mut f = NamedTempFile::new().unwrap();
        for _ in 0..lines_to_boundary {
            writeln!(f, "{line}").unwrap();
        }
        // Write a few more lines past the boundary to verify the second chunk.
        for i in 0..10 {
            writeln!(f, "extra{i}").unwrap();
        }
        f.flush().unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)), false)
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();

        let expected = lines_to_boundary + 10;
        assert_eq!(result.reader.line_count(), expected);
        assert_eq!(result.reader.get_line(0), line.as_bytes());
        assert_eq!(result.reader.get_line(lines_to_boundary), b"extra0");
        assert_eq!(result.reader.get_line(expected - 1), b"extra9");
    }

    fn build_dlt_storage_header(secs: u32, usecs: u32, ecu: &[u8; 4]) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(b"DLT\x01");
        h.extend_from_slice(&secs.to_le_bytes());
        h.extend_from_slice(&usecs.to_le_bytes());
        h.extend_from_slice(ecu);
        h
    }

    fn build_dlt_std_header(htyp: u8, mcnt: u8, length: u16) -> Vec<u8> {
        let mut h = Vec::new();
        h.push(htyp);
        h.push(mcnt);
        h.extend_from_slice(&length.to_be_bytes());
        h
    }

    fn build_dlt_ext_header(msin: u8, noar: u8, apid: &[u8; 4], ctid: &[u8; 4]) -> Vec<u8> {
        let mut h = Vec::new();
        h.push(msin);
        h.push(noar);
        h.extend_from_slice(apid);
        h.extend_from_slice(ctid);
        h
    }

    fn make_dlt_binary_data(count: usize) -> Vec<u8> {
        let mut data = Vec::new();
        for i in 0..count {
            data.extend_from_slice(&build_dlt_storage_header(1705312245 + i as u32, 0, b"ECU1"));
            let htyp = 0x01; // UEH
            let msin = 0x01 | (4 << 4); // verbose, log, info
            let ext = build_dlt_ext_header(msin, 0, b"APP1", b"CTX1");
            let msg_len = (4 + ext.len()) as u16;
            let mut msg = build_dlt_std_header(htyp, i as u8, msg_len);
            msg.extend_from_slice(&ext);
            data.extend_from_slice(&msg);
        }
        data
    }

    #[test]
    fn test_file_reader_new_with_dlt_binary() {
        let dlt_data = make_dlt_binary_data(3);
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&dlt_data).unwrap();
        f.flush().unwrap();

        let reader = FileReader::new(f.path().to_str().unwrap()).unwrap();
        assert!(reader.is_binary);
        assert_eq!(reader.line_count(), 3);
    }

    #[test]
    fn test_file_reader_new_non_dlt_unchanged() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line1").unwrap();
        writeln!(f, "line2").unwrap();
        f.flush().unwrap();

        let reader = FileReader::new(f.path().to_str().unwrap()).unwrap();
        assert!(!reader.is_binary);
        assert_eq!(reader.line_count(), 2);
    }

    #[test]
    fn test_dlt_binary_lines_parseable() {
        use crate::parser::dlt::DltParser;
        use crate::parser::types::LogFormatParser;

        let dlt_data = make_dlt_binary_data(2);
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&dlt_data).unwrap();
        f.flush().unwrap();

        let reader = FileReader::new(f.path().to_str().unwrap()).unwrap();
        let parser = DltParser;

        for i in 0..reader.line_count() {
            let line = reader.get_line(i);
            let parts = parser.parse_line(&line);
            assert!(
                parts.is_some(),
                "Line {} should be parseable: {:?}",
                i,
                std::str::from_utf8(&line)
            );
        }
    }

    #[tokio::test]
    async fn test_from_file_head_with_dlt_binary() {
        let dlt_data = make_dlt_binary_data(5);
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&dlt_data).unwrap();
        f.flush().unwrap();

        let reader = FileReader::from_file_head(f.path().to_str().unwrap(), 1024 * 1024)
            .await
            .unwrap();
        assert!(reader.is_binary);
        assert_eq!(reader.line_count(), 5);
    }

    #[test]
    fn test_append_bytes_with_dlt_flag() {
        let dlt_data = make_dlt_binary_data(2);
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&dlt_data).unwrap();
        f.flush().unwrap();

        let mut reader = FileReader::new(f.path().to_str().unwrap()).unwrap();
        assert_eq!(reader.line_count(), 2);

        let more_data = make_dlt_binary_data(1);
        reader.append_bytes(&more_data);
        assert_eq!(reader.line_count(), 3);
    }
}
