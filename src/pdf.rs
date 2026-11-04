//! Linked PDFs: stored relative to the PDF folder when inside it, opened
//! with the system's default viewer.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

/// What to store for `file`: a '/'-separated path relative to `folder` if
/// the file is inside it, else the absolute path.
pub fn to_stored(file: &Path, folder: Option<&Path>) -> String {
    if let Some(folder) = folder
        && let Ok(rel) = file.strip_prefix(folder)
    {
        let parts: Vec<String> = rel
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        if !parts.is_empty() {
            return parts.join("/");
        }
    }
    file.to_string_lossy().into_owned()
}

/// The file on this computer for a stored path.
pub fn resolve(stored: &str, folder: Option<&Path>) -> PathBuf {
    let path = PathBuf::from(stored);
    if path.is_absolute() || looks_absolute(stored) {
        return path;
    }
    match folder {
        Some(folder) => stored.split('/').fold(folder.to_path_buf(), |p, part| p.join(part)),
        None => path,
    }
}

/// Absolute on any system, so a library moved between Windows and
/// Linux/macOS never glues another system's absolute path onto the folder.
/// `to_stored` never writes a relative path starting with '/'.
fn looks_absolute(s: &str) -> bool {
    let b = s.as_bytes();
    s.starts_with('/')
        || s.starts_with("\\\\")
        || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

/// Opens the file with the default application (the PDF viewer).
pub fn open_file(path: &Path) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("{} is missing. Use Relink to find it.", path.display());
    }
    open::that_detached(path).with_context(|| format!("Could not open {}", path.display()))
}

/// Shows the file in the file manager (selected, where the system allows).
pub fn reveal(path: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // Explorer wants `/select,"C:\path with spaces"` as one argument, unescaped.
        std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn()?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg("-R").arg(path).spawn()?;
        Ok(())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let dir = path.parent().context("The file has no folder")?;
        open::that_detached(dir).with_context(|| format!("Could not open {}", dir.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_inside_folder() {
        let folder = Path::new("/home/me/Papers");
        let file = Path::new("/home/me/Papers/RIS/zhang.pdf");
        assert_eq!(to_stored(file, Some(folder)), "RIS/zhang.pdf");
        assert_eq!(resolve("RIS/zhang.pdf", Some(Path::new("/mnt/other"))), Path::new("/mnt/other/RIS/zhang.pdf"));
    }

    #[test]
    fn absolute_outside_folder() {
        let file = Path::new("/tmp/x.pdf");
        assert_eq!(to_stored(file, Some(Path::new("/home/me/Papers"))), "/tmp/x.pdf");
        assert_eq!(to_stored(file, None), "/tmp/x.pdf");
        assert_eq!(resolve("/tmp/x.pdf", Some(Path::new("/home"))), Path::new("/tmp/x.pdf"));
        assert!(looks_absolute(r"C:\Papers\x.pdf"));
        assert!(looks_absolute(r"\\server\share\x.pdf"));
        assert!(looks_absolute("/tmp/x.pdf"));
        assert!(!looks_absolute("RIS/x.pdf"));
    }

    #[test]
    fn missing_file_is_reported() {
        let err = open_file(Path::new("/definitely/not/here.pdf")).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }
}
