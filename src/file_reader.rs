use crate::parser::dlt_binary;
use memchr::{memchr_iter, memchr2, memchr3_iter};

fn is_any_dlt_binary(data: &[u8]) -> bool {
    dlt_binary::is_dlt_binary(data) || dlt_binary::is_dlt_wire_format(data)
}
use memmap2::Mmap;
#[cfg(unix)]
use memmap2::{Advice, UncheckedAdvice};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs::File, io};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    spawn,
    sync::{oneshot, watch},
    task::spawn_blocking,
};

pub type VisibilityPredicate = Box<dyn Fn(&[u8]) -> bool + Send + Sync>;
pub struct FileLoadResult {
    pub reader: FileReader,
    pub precomputed_visible: Option<Vec<usize>>,
}

pub struct FileLoadHandle {
    pub progress_rx: watch::Receiver<f64>,
    pub result_rx: oneshot::Receiver<io::Result<FileLoadResult>>,
    pub total_bytes: u64,
}

#[derive(Clone)]
enum Storage {
    Mmap(std::sync::Arc<Mmap>),
    Bytes(std::sync::Arc<Vec<u8>>),
}

impl Storage {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Storage::Mmap(m) => m.as_ref(),
            Storage::Bytes(v) => v.as_slice(),
        }
    }
}

#[derive(Clone)]
pub struct FileReader {
    storage: Storage,
    line_starts: std::sync::Arc<Vec<usize>>,
    is_dlt: bool,
}

impl FileReader {
    pub fn new(path: &str) -> io::Result<Self> {
        let file = File::open(path)?;
        let scan_mmap = unsafe { Mmap::map(&file)? };
        let len = scan_mmap.len();

        // Hint sequential access so the kernel prefetches ahead during the scan.
        #[cfg(unix)]
        let _ = scan_mmap.advise(Advice::Sequential);

        if is_any_dlt_binary(&scan_mmap) {
            let text = dlt_binary::convert_dlt_binary_to_text(&scan_mmap);
            drop(scan_mmap);
            let mut reader = Self::from_bytes(text);
            reader.is_dlt = true;
            return Ok(reader);
        }

        let mut starts = vec![0usize];
        let mut has_ansi = false;
        for pos in memchr3_iter(b'\n', b'\x1b', b'\r', &scan_mmap) {
            if scan_mmap[pos] == b'\n' {
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
            let (stripped, line_starts) = strip_ansi_and_index(&scan_mmap);
            return Ok(FileReader {
                storage: Storage::Bytes(std::sync::Arc::new(stripped)),
                line_starts: std::sync::Arc::new(line_starts),
                is_dlt: false,
            });
        }

        // Drop the scan mmap. munmap() is guaranteed to remove all its pages
        // from the process RSS immediately — unlike MADV_DONTNEED which is
        // merely advisory and can be ignored by the kernel for file-backed
        // shared mappings.
        drop(scan_mmap);

        // Fresh mmap for on-demand access: zero pages in RSS until get_line
        // faults in only the specific 4 KiB page(s) it needs.
        let access_mmap = unsafe { Mmap::map(&file)? };
        #[cfg(unix)]
        let _ = access_mmap.advise(Advice::Random);

        Ok(FileReader {
            storage: Storage::Mmap(std::sync::Arc::new(access_mmap)),
            line_starts: std::sync::Arc::new(starts),
            is_dlt: false,
        })
    }

    /// Build a `FileReader` from an in-memory byte buffer (e.g. stdin content).
    pub fn from_bytes(data: Vec<u8>) -> Self {
        // Single pass: scan for '\n', '\x1b', '\r' simultaneously.
        let mut starts = vec![0usize];
        let mut has_ansi = false;
        for pos in memchr3_iter(b'\n', b'\x1b', b'\r', &data) {
            if data[pos] == b'\n' {
                let next = pos + 1;
                if next <= data.len() {
                    starts.push(next);
                }
            } else {
                has_ansi = true;
                break;
            }
        }

        if has_ansi {
            let (stripped, line_starts) = strip_ansi_and_index(&data);
            return FileReader {
                storage: Storage::Bytes(std::sync::Arc::new(stripped)),
                line_starts: std::sync::Arc::new(line_starts),
                is_dlt: false,
            };
        }

        FileReader {
            storage: Storage::Bytes(std::sync::Arc::new(data)),
            line_starts: std::sync::Arc::new(starts),
            is_dlt: false,
        }
    }

    /// Read the last `preview_bytes` of `path` synchronously and return a
    /// `FileReader` containing only those lines.
    ///
    /// This is used by the `--tail` fast path to display the end of a large
    /// file immediately while the full background index is still being built.
    /// Because we seek to near the end of the file the call returns in
    /// milliseconds regardless of file size.
    ///
    /// The first (potentially partial) line of the read chunk is dropped so
    /// that every line in the returned reader is complete.
    pub async fn from_file_tail(path: &str, preview_bytes: u64) -> io::Result<Self> {
        let mut file = tokio::fs::File::open(path).await?;
        let total_len = file.metadata().await?.len();

        // For DLT binary files, read the full file and convert, then take tail lines.
        // We need to peek at the beginning to check for the DLT magic.
        let mut magic_buf = [0u8; 4];
        let is_dlt = if total_len >= 4 {
            file.read_exact(&mut magic_buf).await?;
            file.seek(io::SeekFrom::Start(0)).await?;
            is_any_dlt_binary(&magic_buf)
        } else {
            false
        };

        if is_dlt {
            let mut full_buf = vec![0u8; total_len as usize];
            file.read_exact(&mut full_buf).await?;
            let text = dlt_binary::convert_dlt_binary_to_text(&full_buf);
            let mut reader = Self::from_bytes(text);
            reader.is_dlt = true;
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

    /// Read the first `preview_bytes` of `path` synchronously and return a
    /// `FileReader` containing only those complete lines.
    ///
    /// Used by the non-tail fast path to display the beginning of a large file
    /// immediately while the full background index is still being built.
    /// The last (potentially partial) line of the read chunk is dropped so that
    /// every line in the returned reader is complete.
    pub async fn from_file_head(path: &str, preview_bytes: u64) -> io::Result<Self> {
        let mut file = tokio::fs::File::open(path).await?;
        let total_len = file.metadata().await?.len();
        let read_len = total_len.min(preview_bytes) as usize;
        let mut buf = vec![0u8; read_len];
        file.read_exact(&mut buf).await?;

        if is_any_dlt_binary(&buf) {
            let text = dlt_binary::convert_dlt_binary_to_text(&buf);
            let mut reader = Self::from_bytes(text);
            reader.is_dlt = true;
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

    /// Stream stdin asynchronously, flushing complete lines every second.
    ///
    /// Returns a `watch::Receiver<Vec<u8>>` whose value is replaced each time a
    /// batch of complete lines is ready.  When stdin closes the sender is
    /// dropped, which callers can detect via `has_changed() == Err(_)`.
    ///
    /// Only complete lines (up to the last `\n`) are flushed on each interval
    /// tick; any trailing partial line is held until EOF.
    pub async fn stream_stdin() -> watch::Receiver<Vec<u8>> {
        let (snapshot_tx, snapshot_rx) = watch::channel(Vec::<u8>::new());

        spawn(async move {
            use std::time::Duration;
            use tokio::io::AsyncReadExt;

            let mut stdin = tokio::io::stdin();
            let mut accumulated: Vec<u8> = Vec::new();
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
                                // EOF or error — flush everything including any partial line.
                                accumulated.extend_from_slice(&partial);
                                let _ = snapshot_tx.send(accumulated);
                                return;
                            }
                            Ok(n) => partial.extend_from_slice(&buf[..n]),
                        }
                    }
                    _ = interval.tick() => {
                        // Flush only complete lines (up to the last '\n').
                        if let Some(last_nl) = partial.iter().rposition(|&b| b == b'\n') {
                            accumulated.extend_from_slice(&partial[..=last_nl]);
                            partial.drain(..=last_nl);
                            let _ = snapshot_tx.send(accumulated.clone());
                        }
                    }
                }
            }
        });

        snapshot_rx
    }

    /// Start loading `path` on tokio's blocking thread pool.
    ///
    /// Returns a [`FileLoadHandle`] immediately; the actual indexing happens
    /// in the background.  The caller polls `handle.result_rx.try_recv()` each
    /// frame and reads `*handle.progress_rx.borrow()` for live progress.
    ///
    /// `predicate` — when `Some`, each line is tested after indexing and the
    /// matching indices are stored in [`FileLoadResult::precomputed_visible`],
    /// avoiding a separate `compute_visible` call after the load completes.
    ///
    /// `tail` — when `true`, the predicate is evaluated from the last line
    /// backward so that lines near the end of the file are confirmed visible
    /// first; the result is always returned in ascending order.
    pub async fn load(
        path: String,
        predicate: Option<VisibilityPredicate>,
        tail: bool,
        cancel: Arc<AtomicBool>,
    ) -> io::Result<FileLoadHandle> {
        let total_bytes = std::fs::metadata(&path)?.len();
        let (progress_tx, progress_rx) = watch::channel(0.0_f64);
        let (result_tx, result_rx) = oneshot::channel();

        spawn_blocking(move || {
            let result =
                Self::index_chunked(&path, total_bytes, progress_tx, predicate, tail, &cancel);
            // Ignore send error — UI may have quit before we finish.
            let _ = result_tx.send(result);
        });

        Ok(FileLoadHandle {
            progress_rx,
            result_rx,
            total_bytes,
        })
    }

    /// Index the file using a parallel Rayon scan, sending progress updates as
    /// chunks complete.  Produces the same `line_starts` as `compute_line_starts`.
    ///
    /// Phase 1 (always): parallel scan building `line_starts` + ANSI detection.
    ///   The mmap is divided into `rayon::current_num_threads()` equal chunks;
    ///   each thread runs `memchr3_iter` independently.  When all threads finish
    ///   the per-chunk results are concatenated in order (no sort needed) to form
    ///   the final `line_starts`.  If any chunk detects an ESC/CR byte the ANSI
    ///   fallback runs serially over the full mmap.
    ///
    /// Phase 2 (when `predicate` is `Some`): evaluate visibility on each line.
    ///   - `tail=false`: forward parallel scan via rayon.
    ///   - `tail=true`: backward sequential scan so tail lines are evaluated first;
    ///     result is reversed to restore ascending order.
    fn index_chunked(
        path: &str,
        _total_bytes: u64,
        progress_tx: watch::Sender<f64>,
        predicate: Option<VisibilityPredicate>,
        tail: bool,
        cancel: &AtomicBool,
    ) -> io::Result<FileLoadResult> {
        use rayon::prelude::*;
        use std::sync::atomic::AtomicUsize;

        let file = File::open(path)?;
        let scan_mmap = unsafe { Mmap::map(&file)? };
        let len = scan_mmap.len();

        if is_any_dlt_binary(&scan_mmap) {
            let text = dlt_binary::convert_dlt_binary_to_text(&scan_mmap);
            drop(scan_mmap);
            let _ = progress_tx.send(1.0);
            let mut reader = Self::from_bytes(text);
            reader.is_dlt = true;

            let precomputed_visible = predicate.map(|pred| {
                let count = reader.line_count();
                if tail {
                    let mut visible: Vec<usize> = (0..count)
                        .rev()
                        .filter(|&i| pred(reader.get_line(i)))
                        .collect();
                    visible.reverse();
                    visible
                } else {
                    use rayon::prelude::*;
                    (0..count)
                        .into_par_iter()
                        .filter(|&i| pred(reader.get_line(i)))
                        .collect()
                }
            });

            return Ok(FileLoadResult {
                reader,
                precomputed_visible,
            });
        }

        // Phase 1: parallel chunk scan for '\n', '\x1b', '\r'.
        //
        // Divide the mmap into one chunk per rayon thread (minimum 4 MiB each so
        // tiny files don't spawn more tasks than lines).  Each thread independently
        // finds newline positions and reports them as absolute byte offsets.
        // progress_tx is Sync so it can be referenced from multiple threads;
        // bytes_done is a shared counter for fractional progress updates.
        let num_threads = rayon::current_num_threads().max(1);
        let chunk_size = len.div_ceil(num_threads).max(4 * 1024 * 1024);
        let bytes_done = AtomicUsize::new(0);

        // Each element: (has_ansi, Vec<absolute next-line offsets for this chunk>)
        let chunk_results: Vec<(bool, Vec<usize>)> = scan_mmap
            .par_chunks(chunk_size)
            .enumerate()
            .map(|(chunk_idx, chunk)| {
                if cancel.load(Ordering::Relaxed) {
                    return (false, vec![]);
                }
                let chunk_start = chunk_idx * chunk_size;
                let mut has_ansi = false;
                let mut local_starts: Vec<usize> = Vec::new();

                for pos in memchr3_iter(b'\n', b'\x1b', b'\r', chunk) {
                    match chunk[pos] {
                        b'\n' => {
                            let next = chunk_start + pos + 1;
                            if next <= len {
                                local_starts.push(next);
                            }
                        }
                        _ => {
                            has_ansi = true;
                            break;
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
            // Strip into a Vec<u8>; scan_mmap is dropped here so munmap
            // reclaims the pages — no explicit DontNeed needed.
            let (stripped, line_starts) = strip_ansi_and_index(&scan_mmap);
            drop(scan_mmap);
            let _ = progress_tx.send(1.0);
            FileReader {
                storage: Storage::Bytes(std::sync::Arc::new(stripped)),
                line_starts: std::sync::Arc::new(line_starts),
                is_dlt: false,
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

            // Drop the scan mmap. munmap() is guaranteed to remove all its
            // pages from process RSS immediately — unlike MADV_DONTNEED which
            // is advisory and can be ignored for file-backed shared mappings.
            drop(scan_mmap);

            // Fresh mmap for on-demand access: zero pages in RSS until
            // get_line faults in only the specific page(s) it needs.
            let access_mmap = unsafe { Mmap::map(&file)? };
            #[cfg(unix)]
            let _ = access_mmap.advise(Advice::Random);

            FileReader {
                storage: Storage::Mmap(std::sync::Arc::new(access_mmap)),
                line_starts: std::sync::Arc::new(starts),
                is_dlt: false,
            }
        };

        // Phase 2: evaluate the predicate on each line when provided.
        let precomputed_visible = predicate.map(|pred| {
            let count = reader.line_count();
            if tail {
                // Evaluate from the last line backward so lines near the tail
                // are confirmed first; reverse at the end to restore ascending order.
                let mut visible: Vec<usize> = (0..count)
                    .rev()
                    .filter(|&i| pred(reader.get_line(i)))
                    .collect();
                visible.reverse();
                visible
            } else {
                use rayon::prelude::*;
                (0..count)
                    .into_par_iter()
                    .filter(|&i| pred(reader.get_line(i)))
                    .collect()
            }
        });

        // If phase 2 ran, the predicate accessed every line and re-faulted all
        // pages into RSS. Release them now — only the viewport pages will be
        // re-faulted during rendering.
        if precomputed_visible.is_some()
            && let Storage::Mmap(ref m) = reader.storage
        {
            // SAFETY: phase 2 is complete; no borrows of the mmap data remain
            // (the predicate closures are done and the Arc has count 1 here).
            #[cfg(unix)]
            let _ = unsafe { (**m).unchecked_advise(UncheckedAdvice::DontNeed) };
        }

        Ok(FileLoadResult {
            reader,
            precomputed_visible,
        })
    }

    /// Total number of lines (including any final partial line without a trailing newline).
    pub fn line_count(&self) -> usize {
        let data = self.storage.as_bytes();
        if data.is_empty() {
            return 0;
        }
        // line_starts has one entry per newline + the initial 0.
        // If the file ends with '\n', the last start points to data.len() (empty slice).
        // We skip that phantom empty line.
        let n = self.line_starts.len();
        if n > 0 && self.line_starts[n - 1] == data.len() {
            n - 1
        } else {
            n
        }
    }

    /// Return the raw bytes of line `idx` (without the trailing newline).
    ///
    /// # Panics
    /// Panics if `idx >= line_count()`.
    pub fn get_line(&self, idx: usize) -> &[u8] {
        let data = self.storage.as_bytes();
        let start = self.line_starts[idx];
        let end = if idx + 1 < self.line_starts.len() {
            // End is the start of the next line, minus the newline character.
            let next = self.line_starts[idx + 1];
            if next > 0 && data.get(next - 1) == Some(&b'\n') {
                next - 1
            } else {
                next
            }
        } else {
            data.len()
        };
        &data[start..end]
    }

    /// The contiguous backing data buffer (mmap or in-memory bytes).
    ///
    /// Used by whole-file scanning paths (e.g. Aho-Corasick over the entire
    /// buffer) that avoid per-line `get_line()` overhead.
    pub fn data(&self) -> &[u8] {
        self.storage.as_bytes()
    }

    /// The sorted byte-offset table: `line_starts()[i]` is the byte offset
    /// where line `i` begins in [`data()`].
    pub fn line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    /// Hint the kernel to prefetch the mmap pages covering lines `first_line..=last_line`.
    /// Called before the render loop so async I/O can overlap with CPU work.
    /// No-op for in-memory (stdin/test) readers or on non-Unix platforms.
    #[cfg(unix)]
    pub fn advise_viewport(&self, first_line: usize, last_line: usize) {
        if let Storage::Mmap(ref m) = self.storage {
            let len = m.len();
            let start = self.line_starts.get(first_line).copied().unwrap_or(0);
            let end = self
                .line_starts
                .get(last_line + 1)
                .copied()
                .unwrap_or(len)
                .min(len);
            if end > start {
                let _ = m.advise_range(Advice::WillNeed, start, end - start);
            }
        }
    }

    /// Iterate over `(line_index, line_bytes)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &[u8])> {
        (0..self.line_count()).map(move |i| (i, self.get_line(i)))
    }

    /// Append pre-stripped bytes to this reader, extending the line index.
    ///
    /// The caller is responsible for stripping ANSI escape sequences before
    /// calling this (e.g. [`FileReader::spawn_file_watcher`] does it automatically).
    /// Converting an mmap-backed reader to heap-owned bytes on first call is
    /// unavoidable but cheap relative to the file I/O that precedes it.
    /// Returns `true` if this reader was constructed from a DLT binary file.
    pub fn is_dlt(&self) -> bool {
        self.is_dlt
    }

    pub fn append_bytes(&mut self, new_data: &[u8]) {
        if new_data.is_empty() {
            return;
        }

        let effective_data;
        let converted;
        if self.is_dlt {
            converted = dlt_binary::convert_dlt_binary_to_text(new_data);
            effective_data = converted.as_slice();
        } else {
            effective_data = new_data;
        }

        if effective_data.is_empty() {
            return;
        }

        // Convert mmap to owned bytes before extending.
        let old_storage = std::mem::replace(
            &mut self.storage,
            Storage::Bytes(std::sync::Arc::new(Vec::new())),
        );
        let mut data: Vec<u8> = match old_storage {
            Storage::Bytes(v) => std::sync::Arc::try_unwrap(v).unwrap_or_else(|arc| (*arc).clone()),
            Storage::Mmap(m) => m.to_vec(),
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

    /// Spawn a child process and stream its combined stdout+stderr output.
    ///
    /// Returns a `watch::Receiver<Vec<u8>>` whose value is replaced whenever
    /// new complete lines arrive (flushed every 500 ms).  ANSI escape
    /// sequences are stripped from the output.  When the process exits the
    /// sender is dropped.
    pub async fn spawn_process_stream(
        program: &str,
        args: &[&str],
    ) -> io::Result<watch::Receiver<Vec<u8>>> {
        use tokio::process::Command;

        let mut child = Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            .spawn()?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let (tx, rx) = watch::channel(Vec::<u8>::new());

        // Merge stdout and stderr by spawning a reader task for each,
        // both writing into a shared mpsc channel.
        let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

        if let Some(mut out) = stdout {
            let sender = line_tx.clone();
            spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = vec![0u8; 4096];
                loop {
                    match out.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if sender.send(buf[..n].to_vec()).await.is_err() {
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
                            if sender.send(buf[..n].to_vec()).await.is_err() {
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

            let mut accumulated: Vec<u8> = Vec::new();
            let mut partial: Vec<u8> = Vec::new();

            let mut interval = tokio::time::interval(Duration::from_millis(500));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await; // skip initial immediate tick

            loop {
                tokio::select! {
                    chunk = line_rx.recv() => {
                        match chunk {
                            Some(data) => partial.extend_from_slice(&data),
                            None => {
                                // Both readers done — flush remaining.
                                accumulated.extend_from_slice(&strip_ansi_escapes(&partial));
                                let _ = tx.send(accumulated);
                                return;
                            }
                        }
                    }
                    _ = interval.tick() => {
                        if let Some(last_nl) = partial.iter().rposition(|&b| b == b'\n') {
                            let chunk = &partial[..=last_nl];
                            accumulated.extend_from_slice(&strip_ansi_escapes(chunk));
                            partial.drain(..=last_nl);
                            let _ = tx.send(accumulated.clone());
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    pub async fn spawn_dlt_tcp_stream(
        host: String,
        port: u16,
    ) -> io::Result<watch::Receiver<Vec<u8>>> {
        use tokio::net::TcpStream;

        let stream = TcpStream::connect((host.as_str(), port)).await?;
        let (tx, rx) = watch::channel(Vec::<u8>::new());
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

            let mut accumulated: Vec<u8> = Vec::new();
            let mut partial: Vec<u8> = Vec::new();
            let mut format_confirmed = false;

            let mut interval = tokio::time::interval(Duration::from_millis(500));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;

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
                                    if !text.is_empty() {
                                        accumulated.extend_from_slice(&text);
                                    }
                                    if consumed < partial.len() {
                                        let remainder = &partial[consumed..];
                                        accumulated.extend_from_slice(
                                            String::from_utf8_lossy(remainder).as_bytes(),
                                        );
                                    }
                                }
                                let _ = tx.send(accumulated);
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
                            let text =
                                dlt_binary::convert_dlt_binary_to_text(&partial);
                            if !text.is_empty() {
                                accumulated.extend_from_slice(&text);
                                partial.clear();
                                let _ = tx.send(accumulated.clone());
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
                            if !text.is_empty() {
                                accumulated.extend_from_slice(&text);
                                partial.drain(..consumed);
                                let _ = tx.send(accumulated.clone());
                            }
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    /// Spawn a background task that polls `path` for new bytes every 500 ms.
    ///
    /// `initial_offset` must be the **original** (unstripped) file size in
    /// bytes at the time the file was first loaded (from
    /// `std::fs::metadata(path)?.len()`).  The watcher reads from that offset
    /// onwards, strips ANSI escape sequences, and delivers each chunk via the
    /// returned watch channel.
    ///
    /// The channel value is replaced on each new chunk.  When the background
    /// task stops (receiver dropped), the sender is also dropped.
    pub async fn spawn_file_watcher(path: String, initial_offset: u64) -> watch::Receiver<Vec<u8>> {
        let (tx, rx) = watch::channel(Vec::<u8>::new());
        tokio::spawn(async move {
            use tokio::time::MissedTickBehavior;

            let mut last_offset = initial_offset;
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            interval.tick().await; // skip initial immediate tick

            loop {
                interval.tick().await;

                let path_clone = path.clone();
                let result = tokio::task::spawn_blocking(move || -> io::Result<(u64, Vec<u8>)> {
                    use std::io::{Read, Seek};
                    let current_size = std::fs::metadata(&path_clone)?.len();
                    if current_size <= last_offset {
                        return Ok((current_size, vec![]));
                    }
                    let mut file = File::open(&path_clone)?;
                    file.seek(std::io::SeekFrom::Start(last_offset))?;
                    let new_len = (current_size - last_offset) as usize;
                    let mut buf = vec![0u8; new_len];
                    file.read_exact(&mut buf)?;
                    Ok((current_size, buf))
                })
                .await;

                if let Ok(Ok((new_size, buf))) = result {
                    if new_size < last_offset {
                        // File was truncated (e.g. log rotation) — reset offset.
                        last_offset = new_size;
                    } else if !buf.is_empty() {
                        last_offset = new_size;
                        let stripped = strip_ansi_escapes(&buf);
                        if tx.send(stripped).is_err() {
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

// ---------------------------------------------------------------------------
// strip_ansi_escapes
// ---------------------------------------------------------------------------

/// Strip ANSI/VT escape sequences and bare `\r` characters from `input`.
///
/// Handles:
/// * CSI sequences  (`ESC [` … final_byte in 0x40–0x7E)
/// * OSC sequences  (`ESC ]` … BEL or `ESC \`)
/// * All other two-byte ESC sequences (`ESC` + one byte)
/// * Bare `\r` (so `\r\n` line endings become `\n`)
///
/// Returns a new `Vec<u8>` with the sequences removed.
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

// ---------------------------------------------------------------------------
// strip_ansi_and_index
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// compute_line_starts (test reference implementation)
// ---------------------------------------------------------------------------

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
        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert!(result.precomputed_visible.is_none());
        assert_eq!(result.reader.line_count(), 2);
    }

    #[tokio::test]
    async fn test_load_predicate_forward_filters_correctly() {
        let f = make_tmp(&["ERROR: bad", "INFO: ok", "ERROR: also bad"]);
        let path = f.path().to_str().unwrap().to_string();
        let pred: Box<dyn Fn(&[u8]) -> bool + Send + Sync> =
            Box::new(|line: &[u8]| line.starts_with(b"ERROR"));
        let handle = FileReader::load(path, Some(pred), false, Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(result.precomputed_visible, Some(vec![0, 2]));
    }

    #[tokio::test]
    async fn test_load_predicate_tail_result_is_ascending() {
        let f = make_tmp(&["ERROR: first", "INFO: skip", "ERROR: last"]);
        let path = f.path().to_str().unwrap().to_string();
        let pred: Box<dyn Fn(&[u8]) -> bool + Send + Sync> =
            Box::new(|line: &[u8]| line.starts_with(b"ERROR"));
        let handle = FileReader::load(path, Some(pred), true, Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        let visible = result.precomputed_visible.unwrap();
        // Backward evaluation, but result must be sorted ascending.
        assert_eq!(visible, vec![0, 2]);
        assert!(visible.windows(2).all(|w| w[0] < w[1]));
    }

    #[tokio::test]
    async fn test_load_predicate_ansi_file_indices_correct() {
        // ANSI file: predicate must evaluate against stripped bytes and indices
        // must reference stripped-line positions (same as get_line returns).
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "\x1b[32mERROR\x1b[0m: red").unwrap(); // line 0 — contains ERROR
        writeln!(f, "\x1b[32mINFO\x1b[0m: green").unwrap(); // line 1 — skipped
        writeln!(f, "\x1b[31mERROR\x1b[0m: also red").unwrap(); // line 2 — contains ERROR
        let path = f.path().to_str().unwrap().to_string();

        let pred: Box<dyn Fn(&[u8]) -> bool + Send + Sync> =
            Box::new(|line: &[u8]| line.starts_with(b"ERROR"));
        let handle = FileReader::load(path, Some(pred), false, Arc::new(AtomicBool::new(false)))
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
        let pred: Box<dyn Fn(&[u8]) -> bool + Send + Sync> = Box::new(|_| true);
        let handle = FileReader::load(path, Some(pred), true, Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(result.precomputed_visible, Some(vec![0, 1, 2]));
    }

    #[tokio::test]
    async fn test_load_predicate_none_match() {
        let f = make_tmp(&["INFO: ok", "DEBUG: verbose"]);
        let path = f.path().to_str().unwrap().to_string();
        let pred: Box<dyn Fn(&[u8]) -> bool + Send + Sync> =
            Box::new(|line: &[u8]| line.starts_with(b"ERROR"));
        let handle = FileReader::load(path, Some(pred), false, Arc::new(AtomicBool::new(false)))
            .await
            .unwrap();
        let result = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(result.precomputed_visible, Some(vec![]));
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
        let collected: Vec<(usize, &[u8])> = r.iter().collect();
        assert_eq!(collected.len(), 3);
        assert_eq!(collected[0], (0, b"a".as_ref()));
        assert_eq!(collected[1], (1, b"b".as_ref()));
        assert_eq!(collected[2], (2, b"c".as_ref()));
    }

    #[test]
    fn test_file_reader_from_path() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "[2024-07-24T10:00:00Z] INFO myhost: line 1").unwrap();
        writeln!(f, "[2024-07-24T10:01:00Z] DEBUG myhost: line 2").unwrap();
        let path = f.path().to_str().unwrap();

        let reader = FileReader::new(path).unwrap();
        assert_eq!(reader.line_count(), 2);

        let l0 = std::str::from_utf8(reader.get_line(0)).unwrap();
        assert!(l0.contains("INFO"));
        let l1 = std::str::from_utf8(reader.get_line(1)).unwrap();
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

    // -----------------------------------------------------------------------
    // strip_ansi_escapes – OSC sequences
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // strip_ansi_escapes – two-byte ESC sequences
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // strip_ansi_escapes – edge / truncation cases
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // FileReader – content edge cases
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // append_bytes – advanced scenarios
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // FileReader::new – file with ANSI codes
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // compute_line_starts – direct tests
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Async: load + index_chunked
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_load_basic() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line 1").unwrap();
        writeln!(f, "line 2").unwrap();
        writeln!(f, "line 3").unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)))
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

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)))
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

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)))
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
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_load_empty_file() {
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)))
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
        let handle = FileReader::load(path, None, false, cancel).await.unwrap();
        let result = handle.result_rx.await.unwrap();
        // The load should have returned an Interrupted error because cancel was pre-set.
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().kind(),
            std::io::ErrorKind::Interrupted
        );
    }

    // -----------------------------------------------------------------------
    // Async: spawn_file_watcher
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_spawn_file_watcher_detects_new_data() {
        use std::io::{Seek, SeekFrom};
        use tokio::time::{Duration, sleep};

        let mut f = NamedTempFile::new().unwrap();
        write!(f, "initial\n").unwrap();
        f.flush().unwrap();
        let initial_size = f.as_file().metadata().unwrap().len();
        let path = f.path().to_str().unwrap().to_string();

        let mut rx = FileReader::spawn_file_watcher(path, initial_size).await;

        // Append new data to the file
        f.seek(SeekFrom::End(0)).unwrap();
        write!(f, "appended\n").unwrap();
        f.flush().unwrap();

        // Wait for the watcher to detect the change (polls every 500ms)
        sleep(Duration::from_millis(1500)).await;

        let data = rx.borrow_and_update().clone();
        let text = String::from_utf8_lossy(&data);
        assert!(
            text.contains("appended"),
            "watcher should detect appended data, got: {text}"
        );
    }

    // -----------------------------------------------------------------------
    // iter – additional coverage
    // -----------------------------------------------------------------------

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
        assert_eq!(collected, vec![(0, b"only".as_ref())]);
    }

    // -----------------------------------------------------------------------
    // spawn_process_stream
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_spawn_process_stream_basic() {
        let mut rx = FileReader::spawn_process_stream("echo", &["hello world"])
            .await
            .unwrap();

        // Wait for the process to finish and the final flush.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        let data = rx.borrow_and_update().clone();
        let text = String::from_utf8_lossy(&data);
        assert!(
            text.contains("hello world"),
            "stdout should be captured, got: {text}"
        );
    }

    #[tokio::test]
    async fn test_spawn_process_stream_stderr() {
        // Use sh -c to write to stderr
        let mut rx = FileReader::spawn_process_stream("sh", &["-c", "echo error_output >&2"])
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        let data = rx.borrow_and_update().clone();
        let text = String::from_utf8_lossy(&data);
        assert!(
            text.contains("error_output"),
            "stderr should be captured, got: {text}"
        );
    }

    #[tokio::test]
    async fn test_spawn_process_stream_strips_ansi() {
        // printf outputs ANSI codes; they should be stripped
        let mut rx = FileReader::spawn_process_stream("printf", &["\x1b[31mred text\x1b[0m\n"])
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        let data = rx.borrow_and_update().clone();
        let text = String::from_utf8_lossy(&data);
        assert!(
            text.contains("red text"),
            "should contain stripped text, got: {text}"
        );
        assert!(
            !text.contains("\x1b["),
            "ANSI codes should be stripped, got: {text}"
        );
    }

    // -----------------------------------------------------------------------
    // strip_ansi_and_index
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // from_file_tail
    // -----------------------------------------------------------------------

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
                std::str::from_utf8(line)
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

    // -----------------------------------------------------------------------
    // from_file_head
    // -----------------------------------------------------------------------

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
                std::str::from_utf8(line)
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

    // -----------------------------------------------------------------------
    // append_bytes – Mmap → Vec conversion branch
    // -----------------------------------------------------------------------

    #[test]
    fn test_append_bytes_on_file_backed_reader() {
        // FileReader::new uses Mmap storage. Appending should convert to Vec.
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

    // -----------------------------------------------------------------------
    // spawn_file_watcher – truncation detection
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_spawn_file_watcher_truncation() {
        use std::io::{Seek, SeekFrom};
        use tokio::time::{Duration, sleep};

        let mut f = NamedTempFile::new().unwrap();
        write!(f, "original data that is fairly long\n").unwrap();
        f.flush().unwrap();
        let initial_size = f.as_file().metadata().unwrap().len();
        let path = f.path().to_str().unwrap().to_string();

        let mut rx = FileReader::spawn_file_watcher(path.clone(), initial_size).await;

        // Step 1: truncate to 0 bytes — the watcher detects this and resets
        // its internal offset to 0.
        f.as_file().set_len(0).unwrap();
        sleep(Duration::from_millis(1500)).await;

        // Step 2: write new data — now the file grows past the reset offset
        // and the watcher picks up the new content.
        f.seek(SeekFrom::Start(0)).unwrap();
        write!(f, "after truncation\n").unwrap();
        f.flush().unwrap();
        sleep(Duration::from_millis(1500)).await;

        let data = rx.borrow_and_update().clone();
        let text = String::from_utf8_lossy(&data);
        assert!(
            text.contains("after truncation"),
            "watcher should detect data after truncation, got: {text}"
        );
    }

    // -----------------------------------------------------------------------
    // Parallel Phase-1 indexing via index_chunked
    // -----------------------------------------------------------------------

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

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)))
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

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)))
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

        let pred: VisibilityPredicate = Box::new(|line| line.starts_with(b"EVEN"));
        let handle = FileReader::load(path, Some(pred), false, Arc::new(AtomicBool::new(false)))
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

        let handle = FileReader::load(path, None, false, Arc::new(AtomicBool::new(false)))
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
            let msin = 0x01 | (0 << 1) | (4 << 4); // verbose, log, info
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
        assert!(reader.is_dlt());
        assert_eq!(reader.line_count(), 3);
    }

    #[test]
    fn test_file_reader_new_non_dlt_unchanged() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line1").unwrap();
        writeln!(f, "line2").unwrap();
        f.flush().unwrap();

        let reader = FileReader::new(f.path().to_str().unwrap()).unwrap();
        assert!(!reader.is_dlt());
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
            let parts = parser.parse_line(line);
            assert!(
                parts.is_some(),
                "Line {} should be parseable: {:?}",
                i,
                std::str::from_utf8(line)
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
        assert!(reader.is_dlt());
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
