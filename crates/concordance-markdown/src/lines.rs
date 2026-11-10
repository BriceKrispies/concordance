//! Byte offset -> 1-based line/column conversion.

use concordance_core::{RepoPath, SourceRange};

pub struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
    line_count: u32,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        let line_count = text.lines().count() as u32;
        LineIndex {
            text,
            starts,
            line_count,
        }
    }

    /// The whole source text.
    pub fn source(&self) -> &'a str {
        self.text
    }

    /// Number of lines in the document.
    pub fn line_count(&self) -> u32 {
        self.line_count
    }

    /// Highest valid 1-based line number (at least 1).
    pub fn max_line(&self) -> u32 {
        self.line_count.max(1)
    }

    /// 1-based line containing byte `b` (clamped).
    pub fn line(&self, b: usize) -> u32 {
        let b = b.min(self.text.len());
        let idx = self.starts.partition_point(|&s| s <= b).saturating_sub(1);
        ((idx as u32) + 1).min(self.max_line())
    }

    /// 1-based (line, column in chars) of byte `b` (clamped to a char boundary).
    pub fn line_col(&self, b: usize) -> (u32, u32) {
        let mut b = b.min(self.text.len());
        while b > 0 && !self.text.is_char_boundary(b) {
            b -= 1;
        }
        let idx = self.starts.partition_point(|&s| s <= b).saturating_sub(1);
        let ls = self.starts[idx];
        let col = self.text[ls..b].chars().count() as u32 + 1;
        (((idx as u32) + 1).min(self.max_line()), col)
    }

    /// Range covering bytes `start..end` (end exclusive); end column points at the last char.
    pub fn range(&self, path: &RepoPath, start: usize, end: usize) -> SourceRange {
        let (sl, sc) = self.line_col(start);
        let last = end.saturating_sub(1).max(start);
        let (el, ec) = self.line_col(last);
        if el < sl {
            return SourceRange {
                path: path.clone(),
                start_line: sl,
                start_col: sc,
                end_line: sl,
                end_col: sc,
            };
        }
        SourceRange {
            path: path.clone(),
            start_line: sl,
            start_col: sc,
            end_line: el,
            end_col: ec,
        }
    }

    /// Whole-line range for bytes `start..end`.
    pub fn line_range(&self, path: &RepoPath, start: usize, end: usize) -> SourceRange {
        let sl = self.line(start);
        let el = self.line(end.saturating_sub(1).max(start)).max(sl);
        SourceRange::lines(path.clone(), sl, el)
    }
}
