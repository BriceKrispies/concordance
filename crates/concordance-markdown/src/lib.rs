//! Markdown specification ingestion: pulldown-cmark events -> [`DocumentIndex`].
//!
//! CONTRACT (implemented by this crate, relied on by the engine):
//! * `index_document` is pure and deterministic: same input -> same output.
//! * Never panics on any input text.
//! * All ranges are 1-based lines into `text`, `path` = the given RepoPath.

mod authority;
mod classify;
mod index;
mod inline;
mod lines;
mod parse;
mod refs;
mod sections;
mod text;

#[cfg(test)]
mod tests;

use concordance_core::doc::{AuthorityAssessment, DocumentIndex};
use concordance_core::lang::Languages;

/// Bump when extraction output changes for identical input (forces re-indexing).
pub const EXTRACTOR_VERSION: &str = "md-6";
use concordance_core::RepoPath;

/// Extract sections, claims, references and authority from one Markdown file.
///
/// `configured` is the authority from the configuration file's `[authority]` globs,
/// if one matched; it takes precedence over every heuristic.
pub fn index_document(
    path: &RepoPath,
    text: &str,
    configured: Option<AuthorityAssessment>,
    languages: &Languages,
) -> DocumentIndex {
    index::build(path, text, configured, languages)
}

/// GitHub-style heading anchor: lowercase, spaces -> `-`, drop punctuation except `-`/`_`.
pub fn slugify(heading: &str) -> String {
    text::slugify(heading)
}
