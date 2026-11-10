//! C# language plugin (tree-sitter).
//!
//! * Symbols: namespaces (block and file-scoped), classes, records, structs, interfaces,
//!   enums and their members, delegates, methods, constructors, properties, indexers,
//!   operators, fields (`const` as constants) and events. Paths are
//!   `[project, namespace segments..., enclosing types..., name]`.
//! * Visibility follows C#'s modifiers and defaults: top-level types are `internal`,
//!   members `private`, interface and enum members `public`. A `public` member of a
//!   `public` chain of types is public API.
//! * Signatures are rebuilt from the declaration's parts (attributes, accessibility and
//!   bodies dropped), so formatting never matters. Documented signatures in ```csharp
//!   fences go through the same code.
//! * Doc text comes from `///` XML comments (tags stripped, `<summary>` text kept).
//! * Facts: `throw` (failure site), `unsafe`, `Environment.GetEnvironmentVariable`,
//!   comparisons with numeric literals, `Math.Clamp` / `Math.Min` / `Math.Max`, numeric
//!   `const` values.
//! * Tests: methods carrying `[Fact]`, `[Theory]`, `[Test]`, `[TestCase]` or
//!   `[TestMethod]`; a `Verifies:` line in the doc comment or the `//` comment directly
//!   above links them to requirements.
//! * Projects: every `.csproj` under the repository, plus those named by `.sln` files.

mod index;
mod signature;
mod workspace;

#[cfg(test)]
mod tests;

use concordance_core::code::{SourceIndex, WorkspaceInfo};
use concordance_core::lang::{DocSignature, LanguagePlugin};
use concordance_core::RepoPath;
use std::path::{Path, PathBuf};

pub const EXTRACTOR_VERSION: &str = "cs-1";

#[derive(Debug, Default, Clone, Copy)]
pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn id(&self) -> &'static str {
        "csharp"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["cs"]
    }
    fn fences(&self) -> &'static [&'static str] {
        &["csharp", "cs", "c#"]
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
