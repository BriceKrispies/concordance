//! TypeScript / JavaScript language plugin (oxc parser).
//!
//! * Symbols: functions (including `const f = () => ...`), classes and their methods,
//!   constructors and properties, interfaces and their members, type aliases, enums and
//!   members, other `const`s, namespaces. Paths are
//!   `[package, module segments..., enclosing names..., name]`, where module segments
//!   come from the file path inside its package (`src/` dropped, `index` dropped).
//! * Signatures are rebuilt from the declaration's parts (`function name<T>(params): R`,
//!   `name(params): R` for members), so `export`, bodies and formatting never matter.
//!   Documented signatures in ```ts fences go through the same code.
//! * Facts: `throw` (failure site), `any` / `@ts-ignore` (unchecked code),
//!   `process.env.X` (environment read), comparisons with numeric literals,
//!   `Math.min`/`Math.max` with literals (clamps), numeric `const` values.
//! * Tests: `it(...)` / `test(...)` calls (inside any `describe(...)` nesting) become
//!   test symbols named by their titles; a `Verifies:` line in the comment above the
//!   call links them to requirements.

mod index;
mod signature;
mod workspace;

#[cfg(test)]
mod tests;

use concordance_core::code::{SourceIndex, WorkspaceInfo};
use concordance_core::lang::{DocSignature, LanguagePlugin};
use concordance_core::RepoPath;
use std::path::{Path, PathBuf};

pub const EXTRACTOR_VERSION: &str = "ts-1";

#[derive(Debug, Default, Clone, Copy)]
pub struct TypeScriptPlugin;

impl LanguagePlugin for TypeScriptPlugin {
    fn id(&self) -> &'static str {
        "typescript"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"]
    }
    fn fences(&self) -> &'static [&'static str] {
        &["ts", "typescript", "tsx", "js", "javascript", "jsx"]
    }
    fn extractor_version(&self) -> &'static str {
        EXTRACTOR_VERSION
    }
    fn discover(&self, repo_root: &Path, src_roots: &[PathBuf]) -> WorkspaceInfo {
        workspace::discover(repo_root, src_roots)
    }
    fn index(&self, path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceIndex {
        let attempt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            index::index(path, text, ws)
        }));
        attempt.unwrap_or_else(|_| index::failed(path, text, ws, "internal indexer error".into()))
    }
    fn documented_signatures(&self, code: &str) -> Vec<DocSignature> {
        signature::documented(code)
    }
    fn normalize_signature(&self, text: &str) -> Option<(String, String)> {
        signature::normalize_documented(text)
    }
    fn strip_visibility(&self, sig: &str) -> String {
        signature::strip_modifiers(sig)
    }
}
