//! Documented signatures in ```lean fences. They go through the same scanner as source
//! files, so a documented header and an indexed one are normalized identically
//! (whitespace and comments never matter).

use crate::index::index_snippet;
use concordance_core::lang::DocSignature;
use concordance_core::RepoPath;

fn snippet_path() -> RepoPath {
    RepoPath::from_relative_str("__doc__.lean").expect("valid path")
}

/// Signatures of the items declared in a documented code block.
pub fn documented(code: &str) -> Vec<DocSignature> {
    let idx = index_snippet(&snippet_path(), code);
    let lines: Vec<&str> = code.lines().collect();
    idx.symbols
        .iter()
        .filter(|s| !s.is_test && s.signature.is_some())
        .map(|s| {
            let start = s.range.start_line.saturating_sub(1) as usize;
            let end = s.range.end_line.saturating_sub(1) as usize;
            let line = lines.get(start).copied().unwrap_or("");
            let name_col = line
                .find(s.name.as_str())
                .map(|b| line[..b].chars().count())
                .unwrap_or(0);
            DocSignature {
                name: s.name.clone(),
                text: s.signature.clone().unwrap_or_default(),
                start,
                end: end.max(start),
                name_col,
            }
        })
        .collect()
}

/// `(name, normalized)` for one documented signature.
pub fn normalize_documented(text: &str) -> Option<(String, String)> {
    index_snippet(&snippet_path(), text.trim())
        .symbols
        .into_iter()
        .find(|s| s.signature.is_some() && !s.is_test)
        .map(|s| (s.name, s.signature.unwrap_or_default()))
}
