//! "Ignored Files and Folders" (IDEA: Settings › File Types): file names the IDE
//! treats as nonexistent — hidden from the project view and navigation, never
//! indexed or searched, never offered as unversioned changes.

/// IDEA's default list, plus `.jj` and the Windows shell files.
pub const DEFAULT_IGNORED_NAMES: &[&str] = &[
    "*.pyc",
    "*.pyo",
    "*.rbc",
    "*.yarb",
    "*~",
    ".DS_Store",
    ".git",
    ".hg",
    ".jj",
    ".svn",
    "CVS",
    "__pycache__",
    "_svn",
    "vssver.scc",
    "vssver2.scc",
    "Thumbs.db",
    "desktop.ini",
];

/// Matches single path segments against name patterns (`*` and `?` wildcards).
#[derive(Clone, Debug, Default)]
pub struct IgnoredNames {
    exact: Vec<String>,
    wildcards: Vec<String>,
}

impl IgnoredNames {
    pub fn new(patterns: &[String]) -> Self {
        let mut out = Self::default();
        for p in patterns.iter().map(|p| p.trim()).filter(|p| !p.is_empty() && !p.contains('/')) {
            if p.contains(['*', '?']) {
                out.wildcards.push(p.to_owned());
            } else {
                out.exact.push(p.to_owned());
            }
        }
        out
    }

    pub fn is_ignored_name(&self, name: &str) -> bool {
        self.exact.iter().any(|e| e == name) || self.wildcards.iter().any(|w| wildcard(w.as_bytes(), name.as_bytes()))
    }

    /// Whether any segment of a `/`-separated relative path is ignored.
    pub fn is_ignored_path(&self, rel: &str) -> bool {
        rel.split('/').any(|seg| !seg.is_empty() && self.is_ignored_name(seg))
    }
}

fn wildcard(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si, mut star, mut mark) = (0, 0, None, 0);
    while si < s.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            pi += 1;
            mark = si;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> IgnoredNames {
        IgnoredNames::new(&DEFAULT_IGNORED_NAMES.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn matches_names_and_wildcards() {
        let m = defaults();
        for name in [".DS_Store", "__pycache__", "a.pyc", "notes.txt~", "Thumbs.db"] {
            assert!(m.is_ignored_name(name), "{name}");
        }
        for name in ["DS_Store", "a.py", "pyc", "src", ".gitignore"] {
            assert!(!m.is_ignored_name(name), "{name}");
        }
        assert!(wildcard(b"a?c*", b"abcdef"));
        assert!(!wildcard(b"a?c", b"abcd"));
    }

    #[test]
    fn matches_any_path_segment() {
        let m = defaults();
        assert!(m.is_ignored_path("src/.DS_Store"));
        assert!(m.is_ignored_path("pkg/__pycache__/"));
        assert!(m.is_ignored_path("pkg/__pycache__/mod.cpython-312.pyc"));
        assert!(!m.is_ignored_path("src/main.rs"));
    }
}
