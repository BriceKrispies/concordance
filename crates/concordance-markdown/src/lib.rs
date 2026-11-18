//! Markdown specification ingestion: pulldown-cmark events -> [`DocumentIndex`].
//!
//! CONTRACT (implemented by this crate, relied on by the engine):
//! * `index_document` is pure and deterministic: same input -> same output.
//! * Never panics on any input text.
//! * All ranges are 1-based lines into `text`, `path` = the given RepoPath.

mod authority;
mod citations;
mod classify;
mod index;
mod inline;
mod lines;
mod parse;
mod refs;
mod requirements;
mod sections;
mod text;

#[cfg(test)]
mod tests;

use concordance_core::doc::{Authority, AuthorityAssessment, ClaimKind, DocumentIndex};
use concordance_core::lang::Languages;
use std::collections::HashMap;

/// Bump when extraction output changes for identical input (forces re-indexing).
pub const EXTRACTOR_VERSION: &str = "md-11";
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
    index_document_for(path, text, configured, languages, false)
}

/// [`index_document`] for a file that may be the white paper (`[lint] whitepaper`). Lines
/// that open with a requirement ID are never requirements in the white paper, whatever its
/// authority.
pub fn index_document_for(
    path: &RepoPath,
    text: &str,
    configured: Option<AuthorityAssessment>,
    languages: &Languages,
    whitepaper: bool,
) -> DocumentIndex {
    index::build(
        path,
        text,
        configured,
        languages,
        &Hints::default(),
        whitepaper,
    )
}

/// What a caller that generated the Markdown knows about some of its blocks, keyed by
/// the 1-based line a paragraph, list item or table row starts on. Nothing in the workspace supplies
/// them: `index_document_for` passes `Hints::default()`, so the hint branches of the indexer are never taken.
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

/// Block hints and an authority decided by the caller. No caller in the workspace uses it.
pub fn index_document_with(
    path: &RepoPath,
    text: &str,
    authority: AuthorityAssessment,
    languages: &Languages,
    hints: &Hints,
) -> DocumentIndex {
    index::build(path, text, Some(authority), languages, hints, false)
}

pub use citations::{extract_citations, parse_citation, CitationMatch};

/// GitHub-style heading anchor: lowercase, spaces -> `-`, drop punctuation except `-`/`_`.
pub fn slugify(heading: &str) -> String {
    text::slugify(heading)
}

pub use requirements::{
    is_strict_id, structure, AttributeItem, CodeSpan, LooseText, RequirementBlock, SpecStructure,
};

/// What an inline code span refers to, if it looks like a code reference (a symbol path,
/// an identifier, a repository file). Spans such as `tool build` or `--out` are not references.
pub fn classify_code_span(
    code: &str,
) -> Option<(
    concordance_core::doc::ReferenceTarget,
    concordance_core::doc::ReferenceStrength,
)> {
    refs::classify_code(code)
}
