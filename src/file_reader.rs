use memchr::memchr_iter;
use memmap2::Mmap;
use std::{fs::File, io};

/// Computes the byte offsets of the start of every line in `data`.
/// The first element is always `0`.  The last element points one past the
/// final newline (i.e. to the beginning of a potential final partial line).
fn compute_line_starts(data: &[u8]) -> Vec<usize> {
    let mut starts = vec![0usize];
    for pos in memchr_iter(b'\n', data) {
        if pos + 1 <= data.len() {
            starts.push(pos + 1);
        }
    }
    // If the last byte is NOT a newline, the last element already points past
    // the data, so no extra push is needed.  If it IS a newline, the starts vec
    // ends with `data.len()`, and `get_line` will return an empty slice there —
    // which is fine because we only iterate `0..line_count()`.
    starts
}

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
    /// Memory-map `path` and index all line starts.
    pub fn new(path: &str) -> io::Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        let line_starts = compute_line_starts(&mmap);
        Ok(FileReader {
            storage: Storage::Mmap(mmap),
            line_starts,
        })
    }

    /// Build a `FileReader` from an in-memory byte buffer (e.g. stdin content).
    pub fn from_bytes(data: Vec<u8>) -> Self {
        let line_starts = compute_line_starts(&data);
        FileReader {
            storage: Storage::Bytes(data),
            line_starts,
        }
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
}
