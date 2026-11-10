//! Evidence: what can currently be established about a claim, symbol or document,
//! with provenance, the content it depends on, and validity.

use crate::graph::NodeRef;
use crate::{AnalysisId, ContentHash, EvidenceId, Revision, SourceRange, Timestamp};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stance {
    Supports,
    Contradicts,
    /// Relevant but neither supports nor contradicts (e.g. linkage established, behavior unchecked).
    Inconclusive,
}

/// How strong a single evidence record is. Orders from weakest to strongest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    /// Lexical association or an unvalidated hint.
    Weak,
    /// Model-assessed with validated citations, or syntactic fact with known blind spots.
    Moderate,
    /// Syntactic fact that directly bears on the claim (e.g. `unwrap()` in a no-panic scope).
    Strong,
    /// Deterministic structural fact (symbol absent, signature differs, file missing).
    Proven,
}

/// Deterministic Layer A checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralCheck {
    SymbolResolution,
    SignatureComparison,
    LinkResolution,
    FileResolution,
    DependencyConstraint,
    PanicConstraint,
    UnsafeConstraint,
    ConstantBound,
    ConflictingClaims,
    VagueLanguage,
    PublicApiCoverage,
    /// A `Verifies:` declaration names a requirement no specification defines.
    RequirementResolution,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Method {
    Structural {
        check: StructuralCheck,
    },
    /// Layer B: model interpretation of prose + scoped code. An analytical proposal.
    /// `verdict`: `agrees` | `disagrees` | `ambiguous` | `insufficient_evidence` (after validation).
    Semantic {
        analysis: AnalysisId,
        model: String,
        verdict: String,
    },
    /// Layer C: independent challenge of a Layer B result.
    /// `verdict`: `upheld` | `refuted` | `inconclusive` (after validation).
    Adversarial {
        analysis: AnalysisId,
        model: String,
        verdict: String,
    },
    /// A recorded test outcome for a test that declares `Verifies: <requirement>`.
    /// `outcome`: `passed` | `failed` | `ignored` | `not_run` | `outdated`.
    Test {
        test: String,
        outcome: String,
        run: String,
    },
}

impl Method {
    pub fn kind(&self) -> MethodKind {
        match self {
            Method::Structural { .. } => MethodKind::Structural,
            Method::Semantic { .. } => MethodKind::Semantic,
            Method::Adversarial { .. } => MethodKind::Adversarial,
            Method::Test { .. } => MethodKind::Test,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodKind {
    Structural,
    Semantic,
    Adversarial,
    Test,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationRole {
    Specification,
    Implementation,
}

/// An exact, validated location plus the excerpt that was read there.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Citation {
    pub role: CitationRole,
    pub range: SourceRange,
    /// Bounded excerpt (<= 2000 chars) of the cited text at `content_hash` of its file.
    pub excerpt: String,
    pub node: Option<NodeRef>,
}

/// One piece of content an evidence record was computed from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Dependency {
    pub node: NodeRef,
    /// The node's content hash when the evidence was computed. Evidence is valid only
    /// while every dependency still has this hash.
    pub content_hash: ContentHash,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Validity {
    Valid,
    Invalidated {
        revision: Revision,
        reason: String,
        changed: Vec<NodeRef>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub id: EvidenceId,
    /// What the evidence is about: a claim, a symbol (Layer D) or a document (links).
    pub subject: NodeRef,
    pub stance: Stance,
    pub strength: Strength,
    pub method: Method,
    /// One or two sentences. For semantic evidence: the validated model explanation.
    pub summary: String,
    pub citations: Vec<Citation>,
    pub dependencies: Vec<Dependency>,
    pub validity: Validity,
    pub revision: Revision,
    pub created_at: Timestamp,
}

impl Evidence {
    pub fn is_valid(&self) -> bool {
        matches!(self.validity, Validity::Valid)
    }
}
