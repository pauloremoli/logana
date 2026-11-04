//! Presentation only: retain canonical paths for identity and filesystem operations.
use std::path::Path;

pub(super) fn display_path(path: impl AsRef<Path>) -> String {
    let value = path.as_ref().to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        rest.to_owned()
    } else {
        value.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_path_display_preserves_identity_and_unc_roots() {
        for (raw, expected) in [
            (r"\\?\C:\logs\東京.log", r"C:\logs\東京.log"),
            (r"\\?\UNC\server\share\logs", r"\\server\share\logs"),
            (r"\\?\UNC\server\share\", r"\\server\share\"),
            (r"\\server\share\logs", r"\\server\share\logs"),
            (r"C:\logs", r"C:\logs"),
            (r"\\?\C:\", r"C:\"),
            (r"\\?\z:\café\日志.log", r"z:\café\日志.log"),
            (r"\\?\UNC\服务器\共有\日志.log", r"\\服务器\共有\日志.log"),
            ("/tmp/logs", "/tmp/logs"),
            ("relative.log", "relative.log"),
            (r"\\?\Volume{abc}\", r"\\?\Volume{abc}\"),
            (r"\\.\pipe\logana", r"\\.\pipe\logana"),
        ] {
            let path = Path::new(raw).to_path_buf();
            assert_eq!(display_path(&path), expected);
            assert_eq!(path.as_os_str(), raw);
        }
    }
}
