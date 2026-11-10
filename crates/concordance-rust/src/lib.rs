//! Rust source indexing: `syn` AST -> [`SourceIndex`], plus Cargo workspace discovery.
//!
//! CONTRACT (implemented by this crate, relied on by the engine):
//! * `index_source` is pure and deterministic; never panics; a parse failure yields a
//!   `SourceIndex` with `file.parse_error = Some(..)` and no symbols.
//! * `SymbolId`s never depend on line numbers, so moving code does not change identity.
//! * `Symbol::signature` and `normalize_signature` produce the *same* normal form, so a
//!   signature copied from source into a Markdown code block compares equal.

mod docsig;
mod facts;
mod index;
mod modpath;
mod signature;
mod spans;
mod workspace;

#[cfg(test)]
mod tests;

use concordance_core::code::{SourceIndex, WorkspaceInfo};
use concordance_core::RepoPath;
use std::path::{Path, PathBuf};

/// Bump when indexing output changes for identical input (forces re-indexing).
pub const EXTRACTOR_VERSION: &str = "rs-3";

pub use signature::{normalize_signature, normalize_tokens, strip_visibility};

/// Discover Cargo packages under `repo_root` that own files in `src_roots`.
/// Runs `cargo metadata --format-version 1 --no-deps --offline`; falls back to
/// reading `Cargo.toml` files directly when cargo is unavailable or fails.
pub fn discover_workspace(repo_root: &Path, src_roots: &[PathBuf]) -> WorkspaceInfo {
    workspace::discover(repo_root, src_roots)
}

/// Index one `.rs` file. `ws` determines crate name and module path.
pub fn index_source(path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceIndex {
    let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        index::index(path, text, ws)
    }));
    match attempt {
        Ok(i) => i,
        Err(_) => index::failed(path, text, ws, "internal indexer error".to_string()),
    }
}

/// The Rust language plugin: `syn` indexing, Cargo package discovery.
#[derive(Debug, Default, Clone, Copy)]
pub struct RustPlugin;

impl concordance_core::lang::LanguagePlugin for RustPlugin {
    fn id(&self) -> &'static str {
        "rust"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }
    fn fences(&self) -> &'static [&'static str] {
        &["", "rust", "rs"]
    }
    fn extractor_version(&self) -> &'static str {
        EXTRACTOR_VERSION
    }
    fn discover(&self, repo_root: &Path, src_roots: &[PathBuf]) -> WorkspaceInfo {
        discover_workspace(repo_root, src_roots)
    }
    fn index(&self, path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceIndex {
        index_source(path, text, ws)
    }
    fn documented_signatures(&self, code: &str) -> Vec<concordance_core::lang::DocSignature> {
        docsig::extract(code)
    }
    fn normalize_signature(&self, text: &str) -> Option<(String, String)> {
        signature::normalize_signature(text)
    }
    fn strip_visibility(&self, sig: &str) -> String {
        signature::strip_visibility(sig)
    }
}
