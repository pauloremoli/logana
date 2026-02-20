use memchr::{memchr_iter, memchr2};
use memmap2::Mmap;
use std::{fs::File, io};
use tokio::{
    spawn,
    sync::{oneshot, watch},
    task::spawn_blocking,
};

// ---------------------------------------------------------------------------
// FileLoadHandle
// ---------------------------------------------------------------------------

/// Handle returned by [`FileReader::load_async`].
///
/// * `progress_rx` — watch channel carrying the current progress fraction
///   (0.0 – 1.0).  Updated in ~4 MiB increments by the background task.
/// * `result_rx`   — oneshot channel; receives the completed [`FileReader`]
///   (or an IO error) when indexing finishes.
/// * `total_bytes` — size of the file in bytes (for display).
pub struct FileLoadHandle {
    pub progress_rx: watch::Receiver<f64>,
    pub result_rx: oneshot::Receiver<io::Result<FileReader>>,
    pub total_bytes: u64,
}

// ---------------------------------------------------------------------------
// FileReader internals
// ---------------------------------------------------------------------------

/// Backing storage for a `FileReader`: either a memory-mapped file or an
/// in-memory buffer (used for stdin / test data).
enum Storage {
    Mmap(Mmap),
    Bytes(Vec<u8>),
}

impl Storage {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Storage::Mmap(m) => m.as_ref(),
            Storage::Bytes(v) => v.as_slice(),
        }
    }
}

/// A fast, random-access log file reader backed by a memory-mapped file or
/// an in-memory byte buffer.
pub struct FileReader {
    storage: Storage,
    line_starts: Vec<usize>,
}

impl FileReader {
    /// Memory-map `path` and index all line starts synchronously.
    pub fn new(path: &str) -> io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        if memchr2(b'\x1b', b'\r', &mmap).is_some() {
            let stripped = strip_ansi_escapes(&mmap);
            let line_starts = compute_line_starts(&stripped);
            Ok(FileReader {
                storage: Storage::Bytes(stripped),
                line_starts,
            })
        } else {
            let line_starts = compute_line_starts(&mmap);
            Ok(FileReader {
                storage: Storage::Mmap(mmap),
                line_starts,
            })
        }
    }

    /// Build a `FileReader` from an in-memory byte buffer (e.g. stdin content).
    pub fn from_bytes(data: Vec<u8>) -> Self {
        let data = if memchr2(b'\x1b', b'\r', &data).is_some() {
            strip_ansi_escapes(&data)
        } else {
            data
        };
        let line_starts = compute_line_starts(&data);
        FileReader {
            storage: Storage::Bytes(data),
            line_starts,
        }
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
    pub async fn load(path: String) -> io::Result<FileLoadHandle> {
        let total_bytes = std::fs::metadata(&path)?.len();
        let (progress_tx, progress_rx) = watch::channel(0.0_f64);
        let (result_tx, result_rx) = oneshot::channel();

        spawn_blocking(move || {
            let result = Self::index_chunked(&path, total_bytes, progress_tx);
            // Ignore send error — UI may have quit before we finish.
            let _ = result_tx.send(result);
        });

        Ok(FileLoadHandle {
            progress_rx,
            result_rx,
            total_bytes,
        })
    }

    /// Index the file in 4 MiB chunks, sending progress updates after each
    /// chunk.  Produces the same `line_starts` as `compute_line_starts`.
    fn index_chunked(
        path: &str,
        total_bytes: u64,
        progress_tx: watch::Sender<f64>,
    ) -> io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        let len = mmap.len();

        // If the file contains ANSI escape sequences or CR characters, strip them
        // and re-index the clean bytes in one pass (stripping is O(n) anyway).
        if memchr2(b'\x1b', b'\r', &mmap).is_some() {
            let stripped = strip_ansi_escapes(&mmap);
            let starts = compute_line_starts(&stripped);
            let _ = progress_tx.send(1.0);
            return Ok(FileReader {
                storage: Storage::Bytes(stripped),
                line_starts: starts,
            });
        }

        const CHUNK: usize = 4 * 1024 * 1024; // 4 MiB
        let mut starts = vec![0usize];

        let mut offset = 0;
        while offset < len {
            let end = (offset + CHUNK).min(len);
            for pos in memchr_iter(b'\n', &mmap[offset..end]) {
                let abs = offset + pos + 1;
                if abs <= len {
                    starts.push(abs);
                }
            }
            let progress = if total_bytes > 0 {
                end as f64 / total_bytes as f64
            } else {
                1.0
            };
            // Ignore send error — receiver may have been dropped.
            let _ = progress_tx.send(progress);
            offset = end;
        }

        Ok(FileReader {
            storage: Storage::Mmap(mmap),
            line_starts: starts,
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

    /// Iterate over `(line_index, line_bytes)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &[u8])> {
        (0..self.line_count()).map(move |i| (i, self.get_line(i)))
    }

    /// Append pre-stripped bytes to this reader, extending the line index.
    ///
    /// The caller is responsible for stripping ANSI escape sequences before
    /// calling this (e.g. [`spawn_file_watcher`] does it automatically).
    /// Converting an mmap-backed reader to heap-owned bytes on first call is
    /// unavoidable but cheap relative to the file I/O that precedes it.
    pub fn append_bytes(&mut self, new_data: &[u8]) {
        if new_data.is_empty() {
            return;
        }
        // Convert mmap to owned bytes before extending.
        let old_storage = std::mem::replace(&mut self.storage, Storage::Bytes(Vec::new()));
        let mut data = match old_storage {
            Storage::Bytes(v) => v,
            Storage::Mmap(m) => m.to_vec(),
        };
        let offset = data.len();
        data.extend_from_slice(new_data);
        // Extend line_starts incrementally — only scan the new bytes.
        for pos in memchr_iter(b'\n', &data[offset..]) {
            let abs = offset + pos + 1;
            if abs <= data.len() {
                self.line_starts.push(abs);
            }
        }
        self.storage = Storage::Bytes(data);
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

                match result {
                    Ok(Ok((new_size, buf))) => {
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
                    _ => {} // Transient I/O error or task panic — retry next tick.
                }
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
// compute_line_starts (used by synchronous constructors)
// ---------------------------------------------------------------------------

/// Computes the byte offsets of the start of every line in `data`.
/// The first element is always `0`.  The last element points one past the
/// final newline (i.e. to the beginning of a potential final partial line).
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
}
