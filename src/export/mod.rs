//! Writing references and tables out of the library.

pub mod bibtex;
pub mod matrix;

use std::path::Path;

use anyhow::{Context, Result};

/// Writes `content` to `path` through a temporary file and a rename, so a
/// LaTeX build never sees half a file. Does nothing if the file already has
/// exactly this content (avoids waking up file watchers). Returns whether it wrote.
pub fn write_if_changed(path: &Path, content: &str) -> Result<bool> {
    if std::fs::read(path).is_ok_and(|old| old == content.as_bytes()) {
        return Ok(false);
    }
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().context("The export path has no file name")?.to_string_lossy();
    let tmp = dir.join(format!(".{name}.litanatomy-tmp"));
    std::fs::write(&tmp, content).with_context(|| format!("Could not write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("Could not replace {}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_only_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("refs.bib");
        assert!(write_if_changed(&path, "a").unwrap());
        assert!(!write_if_changed(&path, "a").unwrap());
        assert!(write_if_changed(&path, "b").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
