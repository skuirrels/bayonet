//! The per-project config folder (`<root>/.openidea/`).

use crate::IdeResult;
use std::path::{Path, PathBuf};

/// Folder name under the project root.
pub const DIR: &str = ".openidea";

/// Where `rel` (e.g. `settings.toml`) is read from and written to.
pub fn path(root: &Path, rel: impl AsRef<Path>) -> PathBuf {
    root.join(DIR).join(rel)
}

/// Adds `entries` (e.g. `/shelf/`) to `<root>/.openidea/.gitignore` unless already listed, so
/// per-user state in the project folder stays out of version control (IDEA writes
/// `.idea/.gitignore` the same way).
pub fn ignore(root: &Path, entries: &[&str]) -> IdeResult<()> {
    let file = path(root, ".gitignore");
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    let missing: Vec<&str> = entries.iter().copied().filter(|e| !current.lines().any(|l| l.trim() == *e)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut text = current;
    if text.is_empty() {
        text.push_str("# Per-user IDE state\n");
    } else if !text.ends_with('\n') {
        text.push('\n');
    }
    for e in missing {
        text.push_str(e);
        text.push('\n');
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitignore_entries_are_added_once() {
        let tmp = tempfile::tempdir().unwrap();
        ignore(tmp.path(), &["/shelf/"]).unwrap();
        ignore(tmp.path(), &["/shelf/", "/changelists.json"]).unwrap();
        let text = std::fs::read_to_string(path(tmp.path(), ".gitignore")).unwrap();
        assert_eq!(text, "# Per-user IDE state\n/shelf/\n/changelists.json\n");
        std::fs::write(path(tmp.path(), ".gitignore"), "/shelf/").unwrap();
        ignore(tmp.path(), &["/x"]).unwrap();
        assert_eq!(std::fs::read_to_string(path(tmp.path(), ".gitignore")).unwrap(), "/shelf/\n/x\n");
    }
}
