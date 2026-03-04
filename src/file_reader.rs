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

/// Handle returned by [`FileReader::load`].
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
    /// calling this (e.g. [`FileReader::spawn_file_watcher`] does it automatically).
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

        let handle = FileReader::load(path).await.unwrap();
        assert!(handle.total_bytes > 0);

        let reader = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(reader.line_count(), 3);
        assert_eq!(reader.get_line(0), b"line 1");
    }

    #[tokio::test]
    async fn test_load_progress_reaches_one() {
        let mut f = NamedTempFile::new().unwrap();
        for i in 0..100 {
            writeln!(f, "line {i}").unwrap();
        }
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path).await.unwrap();
        let reader = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(reader.line_count(), 100);

        // After completion, progress should be 1.0
        let progress = *handle.progress_rx.borrow();
        assert!((progress - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_load_with_ansi() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"\x1b[31mred\x1b[0m\nplain\n").unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path).await.unwrap();
        let reader = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(reader.line_count(), 2);
        assert_eq!(reader.get_line(0), b"red");
        assert_eq!(reader.get_line(1), b"plain");
    }

    #[tokio::test]
    async fn test_load_nonexistent() {
        let result = FileReader::load("/tmp/nonexistent_logana_load_test.log".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_load_empty_file() {
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_str().unwrap().to_string();

        let handle = FileReader::load(path).await.unwrap();
        let reader = handle.result_rx.await.unwrap().unwrap();
        assert_eq!(reader.line_count(), 0);
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
}
