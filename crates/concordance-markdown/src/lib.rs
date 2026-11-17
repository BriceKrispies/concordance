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

use concordance_core::doc::{Authority, AuthorityAssessment, ClaimKind, DocumentIndex};
use concordance_core::lang::Languages;
use std::collections::HashMap;

/// Bump when extraction output changes for identical input (forces re-indexing).
pub const EXTRACTOR_VERSION: &str = "md-8";
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
    index::build(path, text, configured, languages, &Hints::default())
}

/// What a caller that generated the Markdown knows about some of its blocks, keyed by
/// the 1-based line a paragraph, list item or table row starts on. Used by readers of
/// structured specifications (Exegra) whose entities carry identity the text does not.
#[derive(Clone, Debug, Default)]
pub struct Hints {
    pub blocks: HashMap<u32, BlockHint>,
}

#[derive(Clone, Debug, Default)]
pub struct BlockHint {
    /// The block defines this requirement ID, as a bold opener would.
    pub requirement: Option<String>,
    /// The block is one statement and always yields one claim, even without a modal verb.
    pub whole_claim: bool,
    /// Overrides the classified kind.
    pub kind: Option<ClaimKind>,
    /// Overrides the effective authority.
    pub authority: Option<Authority>,
}

/// [`index_document`] with block hints and an authority decided by the caller.
pub fn index_document_with(
    path: &RepoPath,
    text: &str,
    authority: AuthorityAssessment,
    languages: &Languages,
    hints: &Hints,
) -> DocumentIndex {
    index::build(path, text, Some(authority), languages, hints)
}

/// GitHub-style heading anchor: lowercase, spaces -> `-`, drop punctuation except `-`/`_`.
pub fn slugify(heading: &str) -> String {
    text::slugify(heading)
}
