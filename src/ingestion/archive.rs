use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use tempfile::NamedTempFile;

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
    let archive_type = detect_archive_type(path)
        .ok_or_else(|| format!("'{}' is not a recognised archive format", path))?;
    match archive_type {
        ArchiveType::Gz => extract_gz(path),
        ArchiveType::Bz2 => extract_bz2(path),
        ArchiveType::Xz => extract_xz(path),
        ArchiveType::Zip => extract_zip(path),
        ArchiveType::Tar => extract_tar(path),
        ArchiveType::TarGz => extract_tar_gz(path),
        ArchiveType::TarBz2 => extract_tar_bz2(path),
        ArchiveType::TarXz => extract_tar_xz(path),
    }
}

fn stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string()
}

fn decompress_to_temp(reader: &mut dyn Read, name: String) -> Result<ExtractedFile, String> {
    let mut tmp = NamedTempFile::new().map_err(|e| e.to_string())?;
    io::copy(reader, &mut tmp).map_err(|e| e.to_string())?;
    tmp.flush().map_err(|e| e.to_string())?;
    Ok(ExtractedFile {
        name,
        temp_file: tmp,
    })
}

fn extract_gz(path: &str) -> Result<Vec<ExtractedFile>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = flate2::read::GzDecoder::new(file);
    let name = stem(&stem(path));
    Ok(vec![decompress_to_temp(&mut decoder, name)?])
}

fn extract_bz2(path: &str) -> Result<Vec<ExtractedFile>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = bzip2::read::BzDecoder::new(file);
    let name = stem(&stem(path));
    Ok(vec![decompress_to_temp(&mut decoder, name)?])
}

fn extract_xz(path: &str) -> Result<Vec<ExtractedFile>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = xz2::read::XzDecoder::new(file);
    let name = stem(&stem(path));
    Ok(vec![decompress_to_temp(&mut decoder, name)?])
}

fn extract_zip(path: &str) -> Result<Vec<ExtractedFile>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .enclosed_name()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| format!("file_{}", i));
        results.push(decompress_to_temp(&mut entry, name)?);
    }
    Ok(results)
}

fn extract_tar_entries<R: Read>(
    archive: &mut tar::Archive<R>,
) -> Result<Vec<ExtractedFile>, String> {
    let mut results = Vec::new();
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
        results.push(decompress_to_temp(&mut entry, name)?);
    }
    Ok(results)
}

fn extract_tar(path: &str) -> Result<Vec<ExtractedFile>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(file);
    extract_tar_entries(&mut archive)
}

fn extract_tar_gz(path: &str) -> Result<Vec<ExtractedFile>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    extract_tar_entries(&mut archive)
}

fn extract_tar_bz2(path: &str) -> Result<Vec<ExtractedFile>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    extract_tar_entries(&mut archive)
}

fn extract_tar_xz(path: &str) -> Result<Vec<ExtractedFile>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let decoder = xz2::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    extract_tar_entries(&mut archive)
}

#[cfg(test)]
mod tests {
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

    fn make_gz(content: &[u8]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        let mut enc = flate2::write::GzEncoder::new(&mut tmp, flate2::Compression::default());
        enc.write_all(content).unwrap();
        enc.finish().unwrap();
        tmp
    }

    fn make_bz2(content: &[u8]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        let mut enc = bzip2::write::BzEncoder::new(&mut tmp, bzip2::Compression::default());
        enc.write_all(content).unwrap();
        enc.finish().unwrap();
        tmp
    }

    fn make_xz(content: &[u8]) -> NamedTempFile {
        let mut tmp = NamedTempFile::new().unwrap();
        let mut enc = xz2::write::XzEncoder::new(&mut tmp, 1);
        enc.write_all(content).unwrap();
        enc.finish().unwrap();
        tmp
    }

    fn make_zip(entries: &[(&str, &[u8])]) -> NamedTempFile {
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

    fn make_tar(entries: &[(&str, &[u8])]) -> NamedTempFile {
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

    fn make_tar_gz(entries: &[(&str, &[u8])]) -> NamedTempFile {
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

    fn make_tar_bz2(entries: &[(&str, &[u8])]) -> NamedTempFile {
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

    fn make_tar_xz(entries: &[(&str, &[u8])]) -> NamedTempFile {
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
}
