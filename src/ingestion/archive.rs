use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tempfile::NamedTempFile;

#[derive(Debug, Clone, Copy)]
pub struct ArchiveExtractionProgress {
    pub file_index: usize,
    pub fraction: f64,
}

pub(crate) struct ProgressReader<R: Read> {
    inner: R,
    bytes_read: u64,
    total_bytes: u64,
    file_index: Arc<AtomicUsize>,
    progress_tx: tokio::sync::watch::Sender<ArchiveExtractionProgress>,
}

impl<R: Read> ProgressReader<R> {
    pub fn new(
        inner: R,
        total_bytes: u64,
        file_index: Arc<AtomicUsize>,
        progress_tx: tokio::sync::watch::Sender<ArchiveExtractionProgress>,
    ) -> Self {
        Self {
            inner,
            bytes_read: 0,
            total_bytes,
            file_index,
            progress_tx,
        }
    }
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes_read = self.bytes_read.saturating_add(n as u64);
        let fraction = if self.total_bytes > 0 {
            (self.bytes_read as f64 / self.total_bytes as f64).min(1.0)
        } else {
            0.0
        };
        let _ = self.progress_tx.send(ArchiveExtractionProgress {
            file_index: self.file_index.load(Ordering::Relaxed),
            fraction,
        });
        Ok(n)
    }
}

impl<R: Read + io::Seek> io::Seek for ProgressReader<R> {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        let new_pos = self.inner.seek(pos)?;
        self.bytes_read = new_pos;
        Ok(new_pos)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArchiveType {
    Gz,
    Bz2,
    Xz,
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
}

pub struct ExtractedFile {
    pub name: String,
    pub temp_file: NamedTempFile,
}

pub fn detect_archive_type(path: &str) -> Option<ArchiveType> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(ArchiveType::TarGz)
    } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") {
        Some(ArchiveType::TarBz2)
    } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        Some(ArchiveType::TarXz)
    } else if lower.ends_with(".tar") {
        Some(ArchiveType::Tar)
    } else if lower.ends_with(".gz") {
        Some(ArchiveType::Gz)
    } else if lower.ends_with(".bz2") {
        Some(ArchiveType::Bz2)
    } else if lower.ends_with(".xz") {
        Some(ArchiveType::Xz)
    } else if lower.ends_with(".zip") {
        Some(ArchiveType::Zip)
    } else {
        None
    }
}

pub fn extract(path: &str) -> Result<Vec<ExtractedFile>, String> {
    let (tx, _rx) = tokio::sync::watch::channel(ArchiveExtractionProgress {
        file_index: 0,
        fraction: 0.0,
    });
    extract_with_progress(path, tx, None)
}

pub fn extract_with_progress(
    path: &str,
    progress_tx: tokio::sync::watch::Sender<ArchiveExtractionProgress>,
    name_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
) -> Result<Vec<ExtractedFile>, String> {
    let archive_type = detect_archive_type(path)
        .ok_or_else(|| format!("'{}' is not a recognised archive format", path))?;
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let file_index = Arc::new(AtomicUsize::new(0));

    match archive_type {
        ArchiveType::Gz => {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let reader = ProgressReader::new(file, file_size, file_index, progress_tx);
            let mut decoder = flate2::read::GzDecoder::new(reader);
            Ok(vec![decompress_to_temp(&mut decoder, stem(&stem(path)))?])
        }
        ArchiveType::Bz2 => {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let reader = ProgressReader::new(file, file_size, file_index, progress_tx);
            let mut decoder = bzip2::read::BzDecoder::new(reader);
            Ok(vec![decompress_to_temp(&mut decoder, stem(&stem(path)))?])
        }
        ArchiveType::Xz => {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let reader = ProgressReader::new(file, file_size, file_index, progress_tx);
            let mut decoder = xz2::read::XzDecoder::new(reader);
            Ok(vec![decompress_to_temp(&mut decoder, stem(&stem(path)))?])
        }
        ArchiveType::Zip => {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let reader =
                ProgressReader::new(file, file_size, file_index.clone(), progress_tx.clone());
            let mut archive = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
            let mut results = Vec::new();
            let mut logical_idx = 0usize;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
                if entry.is_dir() {
                    continue;
                }
                let name = entry
                    .enclosed_name()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| format!("file_{i}"));
                file_index.store(logical_idx, Ordering::Relaxed);
                let _ = progress_tx.send(ArchiveExtractionProgress {
                    file_index: logical_idx,
                    fraction: 0.0,
                });
                results.push(decompress_to_temp(&mut entry, name)?);
                let _ = progress_tx.send(ArchiveExtractionProgress {
                    file_index: logical_idx,
                    fraction: 1.0,
                });
                logical_idx += 1;
            }
            Ok(results)
        }
        ArchiveType::Tar => {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let reader = ProgressReader::new(file, file_size, file_index.clone(), progress_tx);
            let mut archive = tar::Archive::new(reader);
            extract_tar_entries_with_progress(&mut archive, &file_index, &name_tx)
        }
        ArchiveType::TarGz => {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let reader = ProgressReader::new(file, file_size, file_index.clone(), progress_tx);
            let decoder = flate2::read::GzDecoder::new(reader);
            let mut archive = tar::Archive::new(decoder);
            extract_tar_entries_with_progress(&mut archive, &file_index, &name_tx)
        }
        ArchiveType::TarBz2 => {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let reader = ProgressReader::new(file, file_size, file_index.clone(), progress_tx);
            let decoder = bzip2::read::BzDecoder::new(reader);
            let mut archive = tar::Archive::new(decoder);
            extract_tar_entries_with_progress(&mut archive, &file_index, &name_tx)
        }
        ArchiveType::TarXz => {
            let file = File::open(path).map_err(|e| e.to_string())?;
            let reader = ProgressReader::new(file, file_size, file_index.clone(), progress_tx);
            let decoder = xz2::read::XzDecoder::new(reader);
            let mut archive = tar::Archive::new(decoder);
            extract_tar_entries_with_progress(&mut archive, &file_index, &name_tx)
        }
    }
}

fn extract_tar_entries_with_progress<R: Read>(
    archive: &mut tar::Archive<R>,
    file_index: &Arc<AtomicUsize>,
    name_tx: &Option<tokio::sync::mpsc::UnboundedSender<String>>,
) -> Result<Vec<ExtractedFile>, String> {
    let mut results = Vec::new();
    let mut idx = 0usize;
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let name = entry
            .path()
            .map_err(|e| e.to_string())?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        if let Some(tx) = name_tx {
            let _ = tx.send(name.clone());
        }
        file_index.store(idx, Ordering::Relaxed);
        results.push(decompress_to_temp(&mut entry, name)?);
        idx += 1;
    }
    Ok(results)
}

/// Returns `true` for archive types where listing filenames requires decompressing
/// (compressed TAR variants). These use the streaming single-pass extraction path.
pub fn uses_streaming_path(archive_type: &ArchiveType) -> bool {
    matches!(
        archive_type,
        ArchiveType::TarGz | ArchiveType::TarBz2 | ArchiveType::TarXz
    )
}

/// List file names contained in an archive without full decompression.
/// Only valid for two-pass formats (GZ, BZ2, XZ single-file; ZIP; uncompressed TAR).
/// Returns an error for streaming TAR variants — use `uses_streaming_path` to check first.
pub fn list_archive_files(path: &str) -> Result<Vec<String>, String> {
    let archive_type = detect_archive_type(path)
        .ok_or_else(|| format!("'{}' is not a recognised archive format", path))?;
    match archive_type {
        ArchiveType::Gz | ArchiveType::Bz2 | ArchiveType::Xz => Ok(vec![stem(&stem(path))]),
        ArchiveType::Zip => list_zip_names(path),
        ArchiveType::Tar => list_tar_names(path),
        ArchiveType::TarGz | ArchiveType::TarBz2 | ArchiveType::TarXz => {
            Err("use streaming path for compressed TAR archives".to_string())
        }
    }
}

fn list_zip_names(path: &str) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| format!("file_{i}"));
        names.push(name);
    }
    Ok(names)
}

fn list_tar_names(path: &str) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(file);
    let mut names = Vec::new();
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let name = entry
            .path()
            .map_err(|e| e.to_string())?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        names.push(name);
    }
    Ok(names)
}

pub(crate) fn stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string()
}

pub(crate) fn decompress_to_temp(
    reader: &mut dyn Read,
    name: String,
) -> Result<ExtractedFile, String> {
    let mut tmp = NamedTempFile::new().map_err(|e| e.to_string())?;
    io::copy(reader, &mut tmp).map_err(|e| e.to_string())?;
    tmp.flush().map_err(|e| e.to_string())?;
    Ok(ExtractedFile {
        name,
        temp_file: tmp,
    })
}

/// Synthetic-archive builders shared by this module's tests and by
/// `archive_tree`'s tests (which need the same fixtures to test recursion
/// into nested archives).
#[cfg(test)]
pub(crate) mod test_helpers {
    use super::*;
    use std::io::Write;

    pub(crate) fn make_gz(content: &[u8]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        let mut enc = flate2::write::GzEncoder::new(&mut tmp, flate2::Compression::default());
        enc.write_all(content).unwrap();
        enc.finish().unwrap();
        tmp
    }

    pub(crate) fn make_bz2(content: &[u8]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        let mut enc = bzip2::write::BzEncoder::new(&mut tmp, bzip2::Compression::default());
        enc.write_all(content).unwrap();
        enc.finish().unwrap();
        tmp
    }

    pub(crate) fn make_xz(content: &[u8]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        let mut enc = xz2::write::XzEncoder::new(&mut tmp, 1);
        enc.write_all(content).unwrap();
        enc.finish().unwrap();
        tmp
    }

    pub(crate) fn make_zip(entries: &[(&str, &[u8])]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        let mut zip = zip::ZipWriter::new(&mut tmp);
        for (name, content) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content).unwrap();
        }
        zip.finish().unwrap();
        tmp
    }

    pub(crate) fn make_tar(entries: &[(&str, &[u8])]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        {
            let mut builder = tar::Builder::new(&mut tmp);
            for (name, content) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append_data(&mut header, name, *content).unwrap();
            }
            builder.finish().unwrap();
        }
        tmp
    }

    pub(crate) fn make_tar_gz(entries: &[(&str, &[u8])]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        let enc = flate2::write::GzEncoder::new(&mut tmp, flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *content).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        tmp
    }

    pub(crate) fn make_tar_bz2(entries: &[(&str, &[u8])]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        let enc = bzip2::write::BzEncoder::new(&mut tmp, bzip2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *content).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        tmp
    }

    pub(crate) fn make_tar_xz(entries: &[(&str, &[u8])]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        let enc = xz2::write::XzEncoder::new(&mut tmp, 1);
        let mut builder = tar::Builder::new(enc);
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *content).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
        tmp
    }
}

#[cfg(test)]
mod tests {
    use super::test_helpers::*;
    use super::*;
    use std::io::{Seek, SeekFrom, Write};

    fn read_extracted(file: &mut ExtractedFile) -> String {
        let mut content = String::new();
        file.temp_file.seek(SeekFrom::Start(0)).unwrap();
        file.temp_file.read_to_string(&mut content).unwrap();
        content
    }

    // detect_archive_type

    #[test]
    fn test_detect_gz() {
        assert_eq!(detect_archive_type("app.log.gz"), Some(ArchiveType::Gz));
    }

    #[test]
    fn test_detect_bz2() {
        assert_eq!(detect_archive_type("app.log.bz2"), Some(ArchiveType::Bz2));
    }

    #[test]
    fn test_detect_xz() {
        assert_eq!(detect_archive_type("app.log.xz"), Some(ArchiveType::Xz));
    }

    #[test]
    fn test_detect_zip() {
        assert_eq!(detect_archive_type("logs.zip"), Some(ArchiveType::Zip));
    }

    #[test]
    fn test_detect_tar() {
        assert_eq!(detect_archive_type("logs.tar"), Some(ArchiveType::Tar));
    }

    #[test]
    fn test_detect_tar_gz_long() {
        assert_eq!(detect_archive_type("logs.tar.gz"), Some(ArchiveType::TarGz));
    }

    #[test]
    fn test_detect_tgz_short() {
        assert_eq!(detect_archive_type("logs.tgz"), Some(ArchiveType::TarGz));
    }

    #[test]
    fn test_detect_tar_bz2_long() {
        assert_eq!(
            detect_archive_type("logs.tar.bz2"),
            Some(ArchiveType::TarBz2)
        );
    }

    #[test]
    fn test_detect_tbz2_short() {
        assert_eq!(detect_archive_type("logs.tbz2"), Some(ArchiveType::TarBz2));
    }

    #[test]
    fn test_detect_tar_xz_long() {
        assert_eq!(detect_archive_type("logs.tar.xz"), Some(ArchiveType::TarXz));
    }

    #[test]
    fn test_detect_txz_short() {
        assert_eq!(detect_archive_type("logs.txz"), Some(ArchiveType::TarXz));
    }

    #[test]
    fn test_detect_plain_log_returns_none() {
        assert_eq!(detect_archive_type("app.log"), None);
        assert_eq!(detect_archive_type("app.json"), None);
    }

    #[test]
    fn test_detect_case_insensitive() {
        assert_eq!(detect_archive_type("APP.LOG.GZ"), Some(ArchiveType::Gz));
        assert_eq!(detect_archive_type("LOGS.TAR.GZ"), Some(ArchiveType::TarGz));
        assert_eq!(detect_archive_type("LOGS.ZIP"), Some(ArchiveType::Zip));
    }

    // extract roundtrips

    #[test]
    fn test_extract_gz_roundtrip() {
        let content = b"hello from gz\n";
        let tmp = make_gz(content);
        let path = tmp.path().to_str().unwrap().to_string() + ".gz";
        std::fs::copy(tmp.path(), &path).unwrap();
        let mut files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
    }

    #[test]
    fn test_extract_bz2_roundtrip() {
        let content = b"hello from bz2\n";
        let tmp = make_bz2(content);
        let path = tmp.path().to_str().unwrap().to_string() + ".bz2";
        std::fs::copy(tmp.path(), &path).unwrap();
        let mut files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
    }

    #[test]
    fn test_extract_xz_roundtrip() {
        let content = b"hello from xz\n";
        let tmp = make_xz(content);
        let path = tmp.path().to_str().unwrap().to_string() + ".xz";
        std::fs::copy(tmp.path(), &path).unwrap();
        let mut files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
    }

    #[test]
    fn test_extract_zip_single_file() {
        let content = b"hello from zip\n";
        let tmp = make_zip(&[("app.log", content)]);
        let path = tmp.path().to_str().unwrap().to_string() + ".zip";
        std::fs::copy(tmp.path(), &path).unwrap();
        let mut files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "app.log");
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
    }

    #[test]
    fn test_extract_zip_multiple_files() {
        let tmp = make_zip(&[("a.log", b"aaa\n"), ("b.log", b"bbb\n")]);
        let path = tmp.path().to_str().unwrap().to_string() + ".zip";
        std::fs::copy(tmp.path(), &path).unwrap();
        let files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 2);
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"a.log"));
        assert!(names.contains(&"b.log"));
    }

    #[test]
    fn test_extract_zip_skips_directories() {
        let mut tmp = NamedTempFile::new().unwrap();
        let mut zip = zip::ZipWriter::new(&mut tmp);
        zip.add_directory("logs/", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.start_file("logs/app.log", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"content\n").unwrap();
        zip.finish().unwrap();
        let path = tmp.path().to_str().unwrap().to_string() + ".zip";
        std::fs::copy(tmp.path(), &path).unwrap();
        let files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "app.log");
    }

    #[test]
    fn test_extract_tar_single_file() {
        let content = b"hello from tar\n";
        let tmp = make_tar(&[("app.log", content)]);
        let path = tmp.path().to_str().unwrap().to_string() + ".tar";
        std::fs::copy(tmp.path(), &path).unwrap();
        let mut files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "app.log");
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
    }

    #[test]
    fn test_extract_tar_multiple_files() {
        let tmp = make_tar(&[("a.log", b"aaa\n"), ("b.log", b"bbb\n")]);
        let path = tmp.path().to_str().unwrap().to_string() + ".tar";
        std::fs::copy(tmp.path(), &path).unwrap();
        let files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_extract_tar_gz_roundtrip() {
        let content = b"hello from tar.gz\n";
        let tmp = make_tar_gz(&[("app.log", content)]);
        let path = tmp.path().to_str().unwrap().to_string() + ".tar.gz";
        std::fs::copy(tmp.path(), &path).unwrap();
        let mut files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
    }

    #[test]
    fn test_extract_tar_bz2_roundtrip() {
        let content = b"hello from tar.bz2\n";
        let tmp = make_tar_bz2(&[("app.log", content)]);
        let path = tmp.path().to_str().unwrap().to_string() + ".tar.bz2";
        std::fs::copy(tmp.path(), &path).unwrap();
        let mut files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
    }

    #[test]
    fn test_extract_tar_xz_roundtrip() {
        let content = b"hello from tar.xz\n";
        let tmp = make_tar_xz(&[("app.log", content)]);
        let path = tmp.path().to_str().unwrap().to_string() + ".tar.xz";
        std::fs::copy(tmp.path(), &path).unwrap();
        let mut files = extract(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
    }

    #[test]
    fn test_extract_nonexistent_file_returns_err() {
        let result = extract("/nonexistent/path/file.gz");
        assert!(result.is_err());
    }

    #[test]
    fn test_progress_reader_counts_bytes() {
        use std::sync::Arc;
        use tokio::sync::watch;
        let data = b"hello world";
        let (tx, rx) = watch::channel(ArchiveExtractionProgress {
            file_index: 0,
            fraction: 0.0,
        });
        let file_index = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut reader = ProgressReader::new(
            std::io::Cursor::new(data),
            data.len() as u64,
            file_index,
            tx,
        );
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, data);
        let p = rx.borrow();
        assert_eq!(p.file_index, 0);
        assert!((p.fraction - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_progress_reader_partial_read() {
        use std::sync::Arc;
        use tokio::sync::watch;
        let data = b"abcdefghij"; // 10 bytes
        let (tx, rx) = watch::channel(ArchiveExtractionProgress {
            file_index: 0,
            fraction: 0.0,
        });
        let file_index = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut reader = ProgressReader::new(std::io::Cursor::new(data), 10u64, file_index, tx);
        let mut buf = [0u8; 5];
        reader.read_exact(&mut buf).unwrap();
        let p = rx.borrow();
        assert!((p.fraction - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_progress_reader_zero_total_bytes_does_not_panic() {
        use std::sync::Arc;
        use tokio::sync::watch;
        let (tx, rx) = watch::channel(ArchiveExtractionProgress {
            file_index: 0,
            fraction: 0.0,
        });
        let file_index = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut reader = ProgressReader::new(std::io::Cursor::new(b"hi"), 0u64, file_index, tx);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(rx.borrow().fraction, 0.0);
    }

    #[test]
    fn test_list_gz_returns_single_stem() {
        use std::io::Write;
        let tmp = tempfile::Builder::new()
            .prefix("logana_list_gz_")
            .suffix(".log.gz")
            .tempfile()
            .unwrap();
        let path_str = tmp.path().to_str().unwrap().to_string();
        {
            let f = std::fs::OpenOptions::new()
                .write(true)
                .open(tmp.path())
                .unwrap();
            let mut enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            enc.write_all(b"content").unwrap();
            enc.finish().unwrap();
        }
        let names = list_archive_files(&path_str).unwrap();
        let expected = std::path::Path::new(&path_str)
            .file_stem()
            .and_then(|s| std::path::Path::new(s).file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        assert_eq!(names.len(), 1);
        assert_eq!(names, vec![expected]);
    }

    #[test]
    fn test_list_archive_files_errors_for_streaming_formats() {
        assert!(
            list_archive_files("file.tar.gz")
                .unwrap_err()
                .contains("streaming")
        );
        assert!(
            list_archive_files("file.tar.bz2")
                .unwrap_err()
                .contains("streaming")
        );
        assert!(
            list_archive_files("file.tar.xz")
                .unwrap_err()
                .contains("streaming")
        );
    }

    #[test]
    fn test_list_zip_files_without_decompression() {
        let tmp = make_zip(&[("a.log", b"aaa"), ("b.log", b"bbb")]);
        let path = tmp.path().to_str().unwrap().to_string() + ".zip";
        std::fs::copy(tmp.path(), &path).unwrap();
        let names = list_archive_files(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let mut names = names;
        names.sort();
        assert_eq!(names, vec!["a.log", "b.log"]);
    }

    #[test]
    fn test_list_tar_files_without_decompression() {
        let tmp = make_tar(&[("x.log", b"xxx"), ("y.log", b"yyy")]);
        let path = tmp.path().to_str().unwrap().to_string() + ".tar";
        std::fs::copy(tmp.path(), &path).unwrap();
        let names = list_archive_files(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let mut names = names;
        names.sort();
        assert_eq!(names, vec!["x.log", "y.log"]);
    }

    #[test]
    fn test_uses_streaming_path_for_compressed_tar() {
        assert!(uses_streaming_path(&ArchiveType::TarGz));
        assert!(uses_streaming_path(&ArchiveType::TarBz2));
        assert!(uses_streaming_path(&ArchiveType::TarXz));
        assert!(!uses_streaming_path(&ArchiveType::Gz));
        assert!(!uses_streaming_path(&ArchiveType::Zip));
        assert!(!uses_streaming_path(&ArchiveType::Tar));
    }

    #[test]
    fn test_extract_with_progress_gz_sends_progress() {
        use tokio::sync::watch;
        let content = b"hello from gz progress\n";
        let tmp = make_gz(content);
        let path = tmp.path().to_str().unwrap().to_string() + ".log.gz";
        std::fs::copy(tmp.path(), &path).unwrap();
        let (tx, rx) = watch::channel(ArchiveExtractionProgress {
            file_index: 0,
            fraction: 0.0,
        });
        let mut files = extract_with_progress(&path, tx, None).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
        let p = rx.borrow();
        assert_eq!(p.file_index, 0);
        assert!(
            (p.fraction - 1.0).abs() < 1e-9,
            "expected fraction 1.0, got {}",
            p.fraction
        );
    }

    #[test]
    fn test_extract_with_progress_zip_multiple_files() {
        use tokio::sync::watch;
        let tmp = make_zip(&[("a.log", b"aaa\n"), ("b.log", b"bbb\n")]);
        let path = tmp.path().to_str().unwrap().to_string() + ".zip";
        std::fs::copy(tmp.path(), &path).unwrap();
        let (tx, rx) = watch::channel(ArchiveExtractionProgress {
            file_index: 0,
            fraction: 0.0,
        });
        let files = extract_with_progress(&path, tx, None).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 2);
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"a.log"));
        assert!(names.contains(&"b.log"));
        let p = rx.borrow();
        assert_eq!(p.file_index, 1, "last file_index should be 1 (second file)");
        assert!(
            (p.fraction - 1.0).abs() < 1e-9,
            "last fraction should be 1.0"
        );
    }

    #[test]
    fn test_extract_with_progress_bz2_sends_progress() {
        use tokio::sync::watch;
        let content = b"hello from bz2 progress\n";
        let tmp = make_bz2(content);
        let path = tmp.path().to_str().unwrap().to_string() + ".log.bz2";
        std::fs::copy(tmp.path(), &path).unwrap();
        let (tx, rx) = watch::channel(ArchiveExtractionProgress {
            file_index: 0,
            fraction: 0.0,
        });
        let mut files = extract_with_progress(&path, tx, None).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
        let p = rx.borrow();
        assert!((p.fraction - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_extract_with_progress_tar_increments_file_index() {
        use tokio::sync::watch;
        let tmp = make_tar(&[("a.log", b"aaa\n"), ("b.log", b"bbb\n")]);
        let path = tmp.path().to_str().unwrap().to_string() + ".tar";
        std::fs::copy(tmp.path(), &path).unwrap();
        let (tx, rx) = watch::channel(ArchiveExtractionProgress {
            file_index: 0,
            fraction: 0.0,
        });
        let files = extract_with_progress(&path, tx, None).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 2);
        let p = rx.borrow();
        assert_eq!(p.file_index, 1);
    }

    #[test]
    fn test_extract_with_progress_tar_gz_streaming_sends_names() {
        use tokio::sync::{mpsc, watch};
        let content = b"log data\n";
        let tmp = make_tar_gz(&[("app.log", content)]);
        let path = tmp.path().to_str().unwrap().to_string() + ".tar.gz";
        std::fs::copy(tmp.path(), &path).unwrap();
        let (tx, _rx) = watch::channel(ArchiveExtractionProgress {
            file_index: 0,
            fraction: 0.0,
        });
        let (name_tx, mut name_rx) = mpsc::unbounded_channel();
        let mut files = extract_with_progress(&path, tx, Some(name_tx)).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(read_extracted(&mut files[0]).as_bytes(), content);
        assert_eq!(name_rx.try_recv().unwrap(), "app.log");
    }

    #[test]
    fn test_progress_reader_seek_updates_bytes_read() {
        use std::io::{Seek, SeekFrom};
        use std::sync::Arc;
        use tokio::sync::watch;
        let data = b"abcdefghij"; // 10 bytes
        let (tx, _rx) = watch::channel(ArchiveExtractionProgress {
            file_index: 0,
            fraction: 0.0,
        });
        let file_index = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut reader =
            ProgressReader::new(std::io::Cursor::new(data as &[u8]), 10u64, file_index, tx);
        let mut buf = [0u8; 5];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(reader.bytes_read, 5);
        let pos = reader.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(pos, 0);
        assert_eq!(reader.bytes_read, 0);
        let pos = reader.seek(SeekFrom::Start(7)).unwrap();
        assert_eq!(pos, 7);
        assert_eq!(reader.bytes_read, 7);
    }
}
