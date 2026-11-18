//! Lean 4 language plugin (hand-written lexical scanner; no Lean parser is available).
//!
//! * Symbols: `def`, `theorem`, `lemma`, `abbrev`, `structure` (and its fields),
//!   `inductive` (and its constructors), `class`, `instance` (named or anonymous),
//!   `axiom`, `opaque`, `example`. Paths are `[package, module segments..., namespaces...,
//!   name]`; `namespace A.B` opens two levels, `section`/`end` nest. A declaration's range
//!   runs from its first modifier to the next top-level command.
//! * Signatures are the header up to `:=`, `where` or `|`, whitespace-normalized and
//!   without modifiers or attributes. `private` / `protected` set visibility; other
//!   modifiers and `@[attr]`s are recorded as attributes.
//! * Facts: `sorry` (`Unproven`), `unsafe` (`UnsafeCode`), `panic!` / `unreachable!`.
//! * Tests: a `theorem` / `lemma` carrying `Verifies: ID` in its `/-- -/` doc comment or in
//!   the `--` comment lines directly above it.
//! * Evidence: a `lake build` log is turned into per-theorem outcomes through
//!   `LanguagePlugin::build_log_outcomes` (see [`buildlog`]).

mod buildlog;
mod index;
mod lex;
mod signature;
mod workspace;

use concordance_core::code::{SourceIndex, WorkspaceInfo};
use concordance_core::lang::{DocSignature, LanguagePlugin};
use concordance_core::testrun::{TestOutcome, TestSite};
use concordance_core::RepoPath;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const EXTRACTOR_VERSION: &str = "lean-2";

#[derive(Debug, Default, Clone, Copy)]
pub struct LeanPlugin;

impl LanguagePlugin for LeanPlugin {
    fn id(&self) -> &'static str {
        "lean"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &["lean"]
    }
    fn fences(&self) -> &'static [&'static str] {
        &["lean", "lean4"]
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
    fn build_log_outcomes(&self, log: &str, tests: &[TestSite]) -> BTreeMap<String, TestOutcome> {
        buildlog::outcomes(log, tests)
    }
}
