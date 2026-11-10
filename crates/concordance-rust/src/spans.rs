//! Conversion from `proc_macro2` spans to contract [`SourceRange`]s.

use concordance_core::{RepoPath, SourceRange};
use proc_macro2::{LineColumn, Span, TokenStream};
use quote::ToTokens;

pub(crate) struct Locator {
    pub path: RepoPath,
}

impl Locator {
    pub fn new(path: &RepoPath) -> Self {
        Locator { path: path.clone() }
    }

    /// Lines are 1-based already; columns are 0-based in `proc_macro2`, 1-based in the
    /// contract. `end` is exclusive 0-based, which equals the inclusive 1-based column.
    pub fn range(&self, start: LineColumn, end: LineColumn) -> SourceRange {
        SourceRange {
            path: self.path.clone(),
            start_line: start.line.max(1) as u32,
            start_col: start.column as u32 + 1,
            end_line: end.line.max(1) as u32,
            end_col: (end.column as u32).max(1),
        }
    }

    #[allow(dead_code)]
    pub fn of_span(&self, s: Span) -> SourceRange {
        self.range(s.start(), s.end())
    }

    /// Range from the first token's start to the last token's end.
    pub fn of_tokens(&self, ts: TokenStream) -> SourceRange {
        let mut it = ts.into_iter();
        let Some(first) = it.next() else {
            return self.range(
                LineColumn { line: 1, column: 0 },
                LineColumn { line: 1, column: 1 },
            );
        };
        let start = first.span().start();
        let end = it
            .last()
            .map_or_else(|| first.span().end(), |l| l.span().end());
        self.range(start, end)
    }

    pub fn of<T: ToTokens + ?Sized>(&self, t: &T) -> SourceRange {
        self.of_tokens(t.to_token_stream())
    }
}
