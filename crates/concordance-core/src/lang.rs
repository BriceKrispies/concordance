//! Source-language plugins.
//!
//! Everything language-specific about source code sits behind [`LanguagePlugin`]:
//! which files a language owns, how its projects are laid out, how a file becomes
//! symbols and facts, and how documented signatures are read and compared. The rest
//! of Concordance (claims, evidence, status, storage, events, UI) only sees the
//! language-neutral [`SourceIndex`].
//!
//! Plugins are registered in a [`Languages`] registry that the engine builds once and
//! shares with the Markdown extractor (documented signatures in fenced code blocks)
//! and the reconciler (signature comparison).

use crate::code::{SourceIndex, WorkspaceInfo};
use crate::RepoPath;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// An item signature found in a fenced code block of a specification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocSignature {
    pub name: String,
    /// Header verbatim (without the body), multi-line allowed.
    pub text: String,
    /// 0-based line index within the block content where the item starts / its header ends.
    pub start: usize,
    pub end: usize,
    /// Char column (0-based) of `name` on the start line.
    pub name_col: usize,
}

pub trait LanguagePlugin: Send + Sync {
    /// Stable identifier recorded on files (`rust`, `typescript`).
    fn id(&self) -> &'static str;
    /// File extensions this language owns, lower-case, without the dot.
    fn extensions(&self) -> &'static [&'static str];
    /// Markdown fence tags whose blocks hold this language's code. `""` claims
    /// untagged blocks (at most one plugin should).
    fn fences(&self) -> &'static [&'static str];
    /// Bump when indexing output changes for identical input (forces re-indexing).
    fn extractor_version(&self) -> &'static str;
    /// Packages under the source roots (Cargo crates, npm packages, ...).
    fn discover(&self, repo_root: &Path, src_roots: &[PathBuf]) -> WorkspaceInfo;
    /// Index one file. Must not panic; a file that does not parse yields a
    /// `SourceFile` with `parse_error` set and no symbols.
    fn index(&self, path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceIndex;
    /// Item signatures in a fenced code block of this language.
    fn documented_signatures(&self, code: &str) -> Vec<DocSignature>;
    /// `(item name, normalized signature)` of one signature, comparable with
    /// `Symbol::signature` of this language.
    fn normalize_signature(&self, text: &str) -> Option<(String, String)>;
    /// The signature without visibility/export qualifiers (documentation often omits them).
    fn strip_visibility(&self, sig: &str) -> String {
        sig.to_string()
    }
}

/// The registered plugins.
#[derive(Clone, Default)]
pub struct Languages {
    plugins: Vec<Arc<dyn LanguagePlugin>>,
}

impl fmt::Debug for Languages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.plugins.iter().map(|p| p.id()))
            .finish()
    }
}

impl Languages {
    pub fn new(plugins: Vec<Arc<dyn LanguagePlugin>>) -> Self {
        Languages { plugins }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn LanguagePlugin>> {
        self.plugins.iter()
    }

    pub fn by_id(&self, id: &str) -> Option<&dyn LanguagePlugin> {
        self.plugins
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
    }

    /// The plugin owning files with extension `ext` (case-insensitive).
    pub fn for_extension(&self, ext: &str) -> Option<&dyn LanguagePlugin> {
        let ext = ext.to_ascii_lowercase();
        self.plugins
            .iter()
            .find(|p| p.extensions().contains(&ext.as_str()))
            .map(|p| p.as_ref())
    }

    /// The plugin owning a Markdown fence tag (first word of the info string).
    pub fn for_fence(&self, info: &str) -> Option<&dyn LanguagePlugin> {
        let tag = info
            .trim()
            .split(|c: char| c == ',' || c.is_whitespace() || c == '{')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        self.plugins
            .iter()
            .find(|p| p.fences().contains(&tag.as_str()))
            .map(|p| p.as_ref())
    }

    /// One string naming every plugin's extractor version (`rust:rs-3,typescript:ts-1`).
    pub fn extractor_versions(&self) -> String {
        self.plugins
            .iter()
            .map(|p| format!("{}:{}", p.id(), p.extractor_version()))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Packages of every language, merged. Warnings are kept per plugin.
    pub fn discover(&self, repo_root: &Path, src_roots: &[PathBuf]) -> WorkspaceInfo {
        let mut out = WorkspaceInfo {
            from_cargo_metadata: true,
            ..Default::default()
        };
        for p in &self.plugins {
            let w = p.discover(repo_root, src_roots);
            if p.id() == "rust" {
                out.from_cargo_metadata = w.from_cargo_metadata;
            }
            out.crates.extend(w.crates);
            out.warnings.extend(w.warnings);
        }
        out
    }
}
