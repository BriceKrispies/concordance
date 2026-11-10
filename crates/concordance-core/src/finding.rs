//! Findings: the reconciliation result a human acts on.
//!
//! Status rules (enforced by `concordance-reconcile`, documented in `docs/findings.md`):
//! * `violated` requires valid contradicting evidence of strength >= Strong, or a
//!   semantic contradiction that survived an adversarial challenge with validated
//!   citations. Its `category` says which.
//! * `verified` requires valid supporting evidence that covers the claim's content
//!   (a matching documented signature, a satisfied deterministic constraint, or a
//!   challenged semantic agreement). Existence of a referenced symbol never verifies a
//!   behavioral claim.
//! * Anything else that cannot be established stays `unverified`, `ambiguous` or `stale`.

use crate::evidence::MethodKind;
use crate::graph::NodeRef;
use crate::{
    ClaimId, DocumentId, EvidenceId, FindingId, RepoPath, Revision, SourceRange, SymbolId,
    Timestamp,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    Violated,
    Stale,
    Ambiguous,
    Undocumented,
    Unverified,
    Verified,
}

impl FindingStatus {
    pub const ALL: [FindingStatus; 6] = [
        FindingStatus::Verified,
        FindingStatus::Violated,
        FindingStatus::Unverified,
        FindingStatus::Stale,
        FindingStatus::Ambiguous,
        FindingStatus::Undocumented,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            FindingStatus::Verified => "verified",
            FindingStatus::Violated => "violated",
            FindingStatus::Unverified => "unverified",
            FindingStatus::Stale => "stale",
            FindingStatus::Ambiguous => "ambiguous",
            FindingStatus::Undocumented => "undocumented",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// Why the finding has its status. Distinguishes proof from suspicion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCategory {
    /// Deterministic structural fact contradicts the documentation.
    StructuralContradiction,
    /// Semantic analysis suspects a behavioral contradiction (model-assessed, challenged).
    SuspectedSemanticContradiction,
    /// No evidence establishes compliance either way.
    MissingEvidence,
    /// Documentation is unclear, vague or self-contradictory.
    UnclearDocumentation,
    /// Implementation behavior has no documented contract.
    MissingDocumentation,
    /// Previously valid evidence was invalidated by a change and not yet re-established.
    InvalidatedEvidence,
    /// Evidence supports the claim.
    Supported,
    /// A test that declares it verifies this requirement failed in the recorded run.
    TestFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
            Severity::Info => "info",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [
            Severity::Critical,
            Severity::High,
            Severity::Medium,
            Severity::Low,
            Severity::Info,
        ]
        .into_iter()
        .find(|v| v.as_str() == s)
    }
}

/// Overall quality of the evidence behind a finding's status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceQuality {
    None,
    Weak,
    Moderate,
    Strong,
    Proven,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionAction {
    ChangeImplementation,
    UpdateDescriptiveDocumentation,
    ClarifySpecification,
    EstablishSpecification,
    GatherStrongerEvidence,
    AcceptBehaviorChange,
    NoActionNeeded,
}

/// A reviewable reconciliation proposal. Concordance never applies these itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub action: ResolutionAction,
    pub rationale: String,
    /// True when the proposal changes normative intent and so needs an explicit human decision.
    pub requires_decision: bool,
    /// The files a repair would touch.
    pub targets: Vec<SourceRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Lifecycle {
    Open,
    Resolved { revision: Revision, at: Timestamp },
}

/// What a finding is about. Determines the stable `FindingId`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FindingSubject {
    /// The reconciliation status of one obligation claim.
    Claim { claim: ClaimId },
    /// A reference in non-claim prose (dangling symbol, broken link, missing file).
    ProseReference { document: DocumentId, text: String },
    /// A gap: implementation behavior without a documented contract.
    Undocumented { symbol: SymbolId, aspect: String },
    /// Two or more claims that cannot all hold.
    ClaimConflict { claims: Vec<ClaimId> },
    /// A test declares `Verifies: <requirement>` for a requirement no specification defines.
    Verification {
        symbol: SymbolId,
        requirement: String,
    },
}

impl FindingSubject {
    pub fn finding_id(&self) -> FindingId {
        match self {
            FindingSubject::Claim { claim } => FindingId::from_key(&["claim", claim.as_str()]),
            FindingSubject::ProseReference { document, text } => {
                FindingId::from_key(&["prose", document.as_str(), text])
            }
            FindingSubject::Undocumented { symbol, aspect } => {
                FindingId::from_key(&["undocumented", symbol.as_str(), aspect])
            }
            FindingSubject::ClaimConflict { claims } => {
                let mut ids: Vec<&str> = claims.iter().map(|c| c.as_str()).collect();
                ids.sort();
                let joined = ids.join(",");
                FindingId::from_key(&["conflict", &joined])
            }
            FindingSubject::Verification {
                symbol,
                requirement,
            } => FindingId::from_key(&["verification", symbol.as_str(), requirement]),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: FindingId,
    pub subject: FindingSubject,
    pub status: FindingStatus,
    pub category: FindingCategory,
    pub severity: Severity,
    pub title: String,
    /// What disagrees and why it matters, in plain sentences.
    pub explanation: String,
    pub claims: Vec<ClaimId>,
    pub symbols: Vec<SymbolId>,
    pub documents: Vec<DocumentId>,
    /// Every exact location involved, specification first.
    pub locations: Vec<SourceRange>,
    pub supporting: Vec<EvidenceId>,
    pub contradicting: Vec<EvidenceId>,
    /// 0..=1 confidence that `status` is right, given the evidence.
    pub confidence: f32,
    pub quality: EvidenceQuality,
    pub methods: Vec<MethodKind>,
    /// What this finding's methods cannot see (e.g. "callees not analyzed").
    pub limitations: Vec<String>,
    pub dependencies: Vec<NodeRef>,
    pub proposals: Vec<Proposal>,
    /// Revision the finding was last evaluated at.
    pub revision: Revision,
    /// Revision in which the finding (in its current status) first appeared.
    pub introduced_in: Revision,
    /// Files changed in `introduced_in`; empty for the initial scan.
    pub introduced_by: Vec<RepoPath>,
    pub first_seen: Timestamp,
    pub last_updated: Timestamp,
    pub lifecycle: Lifecycle,
    /// For filtering: crate and module of the first implicated symbol.
    pub crate_name: Option<String>,
    pub module: Option<String>,
}

impl Finding {
    pub fn is_open(&self) -> bool {
        matches!(self.lifecycle, Lifecycle::Open)
    }
}

/// Aggregate counts for dashboards and scan summaries (open findings only).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub documents: u32,
    pub claims: u32,
    pub obligation_claims: u32,
    pub source_files: u32,
    pub symbols: u32,
    pub edges: u32,
    pub verified: u32,
    pub violated: u32,
    pub unverified: u32,
    pub stale: u32,
    pub ambiguous: u32,
    pub undocumented: u32,
    pub resolved: u32,
}
