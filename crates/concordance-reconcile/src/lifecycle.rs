//! Finding lifecycle: stable identity across scans, history-preserving updates,
//! retained resolution.

use crate::BatchContext;
use concordance_core::finding::{Finding, FindingStatus, Lifecycle};

/// What happened to a finding in this batch.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Created,
    /// Status changed (`from` is the previous status) or the finding reopened.
    StatusChanged {
        from: FindingStatus,
    },
    /// Same status, different content (evidence, explanation, locations ...).
    Updated,
    Unchanged,
}

/// Fields that, when equal, mean the finding is materially unchanged.
fn material_eq(a: &Finding, b: &Finding) -> bool {
    a.status == b.status
        && a.category == b.category
        && a.severity == b.severity
        && a.title == b.title
        && a.explanation == b.explanation
        && a.locations == b.locations
        && a.supporting == b.supporting
        && a.contradicting == b.contradicting
        && a.symbols == b.symbols
        && a.limitations == b.limitations
        && (a.confidence - b.confidence).abs() < f32::EPSILON
        && a.proposals == b.proposals
        && a.quality == b.quality
        && a.methods == b.methods
        && a.dependencies == b.dependencies
        && a.documents == b.documents
        && a.claims == b.claims
        && a.subject == b.subject
        && a.is_open() == b.is_open()
}

/// Merge a freshly derived finding with its previous version.
pub fn merge(prev: Option<&Finding>, mut new: Finding, ctx: &BatchContext) -> (Finding, Change) {
    let Some(prev) = prev else {
        return (new, Change::Created);
    };
    new.first_seen = prev.first_seen;
    let reopened = !prev.is_open();
    if prev.status == new.status && !reopened {
        new.introduced_in = prev.introduced_in;
        new.introduced_by = prev.introduced_by.clone();
        if material_eq(prev, &new) {
            let mut kept = prev.clone();
            kept.revision = ctx.revision;
            return (kept, Change::Unchanged);
        }
        return (new, Change::Updated);
    }
    (new, Change::StatusChanged { from: prev.status })
}

/// Mark a finding resolved (its subject no longer exhibits the issue or no longer exists).
/// Returns `None` if it was already resolved.
pub fn resolve(prev: &Finding, ctx: &BatchContext) -> Option<Finding> {
    if !prev.is_open() {
        return None;
    }
    let mut f = prev.clone();
    f.lifecycle = Lifecycle::Resolved {
        revision: ctx.revision,
        at: ctx.now,
    };
    f.last_updated = ctx.now;
    f.revision = ctx.revision;
    Some(f)
}
