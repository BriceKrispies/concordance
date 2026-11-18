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

use concordance_core::doc::{AuthorityAssessment, DocumentIndex};
use concordance_core::lang::Languages;

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
    index::build(path, text, configured, languages, whitepaper)
}

pub use citations::{extract_citations, parse_citation, CitationMatch};

/// GitHub-style heading anchor (see [`concordance_core::anchor::slugify`]).
pub fn slugify(heading: &str) -> String {
    concordance_core::anchor::slugify(heading)
}

pub use requirements::{
    structure, AttributeItem, CodeSpan, LooseText, RequirementBlock, SpecStructure,
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
