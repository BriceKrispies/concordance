//! Reconciliation: turns the graph into evidence and findings.
//!
//! * [`link`]: claim/prose references -> typed edges (with explicit resolution labels).
//! * [`structural`]: Layer A deterministic checks -> [`Evidence`].
//! * [`gaps`]: Layer D structural underspecification -> undocumented findings.
//! * [`derive`]: evidence -> finding status, severity, proposals (the status rules).
//! * [`lifecycle`]: merging a freshly derived finding with its previous version.
//!
//! Semantic evidence (Layers B/C) is produced by the engine from
//! `concordance-semantic` results and consumed by [`derive`]
//! like any other evidence.

pub mod derive;
pub mod gaps;
pub mod lifecycle;
pub mod link;
pub mod structural;
mod text;
pub mod verification;

use concordance_core::{RepoPath, Revision, Timestamp};

/// Bump when reconciliation rules change, so persisted findings are re-derived.
pub const RULES_VERSION: &str = "rules-7";

/// Filesystem existence probe for references that point outside the indexed roots.
/// Implementations must only answer for paths inside the repository.
pub trait FsProbe {
    fn exists(&self, path: &RepoPath) -> bool;
    /// 1-based line of the first line in `path` that satisfies `pred`, if readable.
    fn find_line(&self, path: &RepoPath, pred: &dyn Fn(&str) -> bool) -> Option<u32> {
        let _ = (path, pred);
        None
    }
}

/// Context shared by everything computed in one change batch.
#[derive(Clone, Debug)]
pub struct BatchContext {
    pub revision: Revision,
    pub now: Timestamp,
    /// Files changed by this batch (empty for an initial scan).
    pub changed_files: Vec<RepoPath>,
    /// Whether semantic analysis is enabled (affects explanations and proposals only).
    pub semantic_enabled: bool,
    /// Registered source-language plugins (signature normalisation).
    pub languages: concordance_core::lang::Languages,
}
