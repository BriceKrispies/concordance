//! Repository paths and source ranges.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// A path relative to the analyzed repository root, always with `/` separators and
/// no `.`/`..` components. This is the only path form that crosses subsystem
/// boundaries; absolute paths stay inside discovery and file access.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepoPath(String);

impl RepoPath {
    /// Build from an absolute (or root-relative) filesystem path. Returns `None` if the
    /// path is outside `root`. Handles Windows separators and drive-letter case.
    pub fn from_fs(root: &Path, path: &Path) -> Option<Self> {
        let rel = if path.is_absolute() {
            strip_prefix_case_insensitive(path, root)?
        } else {
            path.to_path_buf()
        };
        Self::from_relative_str(&rel.to_string_lossy())
    }

    /// Normalize a root-relative path string (`a\b\..\c.md` -> `a/c.md`). Returns
    /// `None` if it escapes the root, is absolute, or is empty.
    pub fn from_relative_str(s: &str) -> Option<Self> {
        let s = s.replace('\\', "/");
        if s.starts_with('/') || has_drive_prefix(&s) {
            return None;
        }
        let mut out: Vec<&str> = Vec::new();
        for part in s.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    out.pop()?;
                }
                p => out.push(p),
            }
        }
        if out.is_empty() {
            return None;
        }
        Some(RepoPath(out.join("/")))
    }

    /// Resolve `target` (as written in the document at `self`) relative to that
    /// document's directory.
    pub fn join_relative(&self, target: &str) -> Option<Self> {
        match self.0.rfind('/') {
            Some(i) => Self::from_relative_str(&format!("{}/{target}", &self.0[..i])),
            None => Self::from_relative_str(target),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_fs(&self, root: &Path) -> PathBuf {
        let mut p = root.to_path_buf();
        for part in self.0.split('/') {
            p.push(part);
        }
        p
    }

    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    pub fn extension(&self) -> Option<&str> {
        self.file_name().rsplit_once('.').map(|(_, e)| e)
    }

    /// True if `self` is `dir` or lies beneath it.
    pub fn starts_with_dir(&self, dir: &RepoPath) -> bool {
        self.0 == dir.0
            || (self.0.starts_with(&dir.0) && self.0.as_bytes().get(dir.0.len()) == Some(&b'/'))
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn has_drive_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn strip_prefix_case_insensitive(path: &Path, root: &Path) -> Option<PathBuf> {
    if let Ok(rel) = path.strip_prefix(root) {
        return Some(rel.to_path_buf());
    }
    // Windows: compare component-wise ignoring ASCII case (drive letters, user dirs).
    let mut pc = path.components();
    for rc in root.components() {
        let p = pc.next()?;
        let eq = match (rc, p) {
            (Component::Prefix(a), Component::Prefix(b)) => {
                a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
            }
            (a, b) => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
        };
        if !eq {
            return None;
        }
    }
    Some(pc.as_path().to_path_buf())
}

/// A 1-based, inclusive line/column range within a repository file.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceRange {
    pub path: RepoPath,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl SourceRange {
    pub fn lines(path: RepoPath, start_line: u32, end_line: u32) -> Self {
        SourceRange {
            path,
            start_line,
            start_col: 1,
            end_line,
            end_col: 1,
        }
    }
    pub fn contains_lines(&self, start: u32, end: u32) -> bool {
        start <= end && start >= self.start_line && end <= self.end_line
    }
}

impl fmt::Display for SourceRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start_line == self.end_line {
            write!(f, "{}:{}", self.path, self.start_line)
        } else {
            write!(f, "{}:{}-{}", self.path, self.start_line, self.end_line)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_paths_normalize_and_refuse_escape() {
        assert_eq!(
            RepoPath::from_relative_str("docs\\a\\..\\b.md")
                .unwrap()
                .as_str(),
            "docs/b.md"
        );
        assert!(RepoPath::from_relative_str("../x.md").is_none());
        assert!(RepoPath::from_relative_str("C:/x.md").is_none());
        let doc = RepoPath::from_relative_str("docs/spec/a.md").unwrap();
        assert_eq!(doc.join_relative("../b.md").unwrap().as_str(), "docs/b.md");
        assert!(doc.join_relative("../../../x").is_none());
    }

    #[test]
    fn windows_absolute_paths_relativize_case_insensitively() {
        if cfg!(windows) {
            let root = Path::new(r"C:\dev\example");
            let p = Path::new(r"c:\Dev\example\crates\a\src\lib.rs");
            assert_eq!(
                RepoPath::from_fs(root, p).unwrap().as_str(),
                "crates/a/src/lib.rs"
            );
            assert!(RepoPath::from_fs(root, Path::new(r"C:\other\x.rs")).is_none());
        }
    }
}
