//! Typed backend events. Every subscriber (SSE clients, `watch --jsonl`, the scan
//! command's progress printer) consumes exactly these.
//!
//! Wire form: `{"seq":42,"revision":3,"at":1731100000000,"type":"finding.created","data":{...}}`.

use crate::code::SymbolKind;
use crate::doc::{ClaimKind, Modality};
use crate::evidence::{Evidence, Stance, Strength};
use crate::finding::{Counts, Finding, FindingStatus};
use crate::graph::{Edge, NodeRef};
use crate::{
    AnalysisId, ClaimId, DocumentId, EdgeId, EvidenceId, FileId, FindingId, RepoPath, Revision,
    SymbolId, Timestamp,
};
use serde::{Deserialize, Serialize};

/// An event as distributed: sequence number assigned by the bus.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub seq: u64,
    /// Model revision current when the event was published.
    pub revision: Revision,
    pub at: Timestamp,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanReason {
    Initial,
    Manual,
    Watch,
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanPhase {
    Discover,
    Parse,
    Link,
    Reconcile,
    Persist,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisOutcome {
    /// Result validated and applied at the current revision.
    Applied,
    /// A valid cached result for identical inputs was reused.
    Cached,
    /// Inputs changed while running; the result was discarded and never published.
    Superseded,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Created,
    Modified,
    Removed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimSummary {
    pub id: ClaimId,
    pub document: DocumentId,
    pub text: String,
    pub kind: ClaimKind,
    pub modality: Modality,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolSummary {
    pub id: SymbolId,
    pub file: FileId,
    pub kind: SymbolKind,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvidenceSummary {
    pub id: EvidenceId,
    pub subject: NodeRef,
    pub stance: Stance,
    pub strength: Strength,
    pub summary: String,
}

impl From<&Evidence> for EvidenceSummary {
    fn from(e: &Evidence) -> Self {
        EvidenceSummary {
            id: e.id.clone(),
            subject: e.subject.clone(),
            stance: e.stance,
            strength: e.strength,
            summary: e.summary.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Event {
    #[serde(rename = "repository.discovered")]
    RepositoryDiscovered {
        root: String,
        spec_roots: Vec<String>,
        src_roots: Vec<String>,
        markdown_files: u32,
        rust_files: u32,
        crates: u32,
    },
    #[serde(rename = "scan.started")]
    ScanStarted {
        scan: u64,
        reason: ScanReason,
        files: u32,
    },
    #[serde(rename = "scan.progress")]
    ScanProgress {
        scan: u64,
        phase: ScanPhase,
        done: u32,
        total: u32,
    },
    #[serde(rename = "scan.completed")]
    ScanCompleted {
        scan: u64,
        revision: Revision,
        duration_ms: u64,
        changed_files: u32,
        counts: Counts,
    },
    #[serde(rename = "document.indexed")]
    DocumentIndexed {
        document: DocumentId,
        path: RepoPath,
        claims: u32,
    },
    #[serde(rename = "document.removed")]
    DocumentRemoved {
        document: DocumentId,
        path: RepoPath,
    },
    #[serde(rename = "claim.created")]
    ClaimCreated { claim: ClaimSummary },
    #[serde(rename = "claim.updated")]
    ClaimUpdated { claim: ClaimSummary },
    #[serde(rename = "claim.removed")]
    ClaimRemoved { claim: ClaimId },
    #[serde(rename = "source.indexed")]
    SourceIndexed {
        file: FileId,
        path: RepoPath,
        symbols: u32,
        parse_error: Option<String>,
    },
    #[serde(rename = "source.removed")]
    SourceRemoved { file: FileId, path: RepoPath },
    #[serde(rename = "symbol.created")]
    SymbolCreated { symbol: SymbolSummary },
    #[serde(rename = "symbol.updated")]
    SymbolUpdated {
        symbol: SymbolSummary,
        signature_changed: bool,
    },
    #[serde(rename = "symbol.removed")]
    SymbolRemoved { symbol: SymbolId },
    #[serde(rename = "relationship.created")]
    RelationshipCreated { edge: Edge },
    #[serde(rename = "relationship.invalidated")]
    RelationshipInvalidated {
        edge: EdgeId,
        from: NodeRef,
        reason: String,
    },
    #[serde(rename = "evidence.created")]
    EvidenceCreated { evidence: EvidenceSummary },
    #[serde(rename = "evidence.invalidated")]
    EvidenceInvalidated {
        evidence: EvidenceId,
        subject: NodeRef,
        reason: String,
    },
    #[serde(rename = "analysis.queued")]
    AnalysisQueued {
        analysis: AnalysisId,
        claim: ClaimId,
    },
    #[serde(rename = "analysis.started")]
    AnalysisStarted {
        analysis: AnalysisId,
        claim: ClaimId,
        model: String,
        challenge: bool,
    },
    #[serde(rename = "analysis.progress")]
    AnalysisProgress { analysis: AnalysisId, stage: String },
    #[serde(rename = "analysis.completed")]
    AnalysisCompleted {
        analysis: AnalysisId,
        claim: ClaimId,
        outcome: AnalysisOutcome,
        verdict: Option<String>,
        detail: Option<String>,
    },
    #[serde(rename = "finding.created")]
    FindingCreated { finding: Finding },
    #[serde(rename = "finding.updated")]
    FindingUpdated {
        finding: Finding,
        previous_status: FindingStatus,
    },
    #[serde(rename = "finding.resolved")]
    FindingResolved {
        finding: FindingId,
        previous_status: FindingStatus,
    },
    #[serde(rename = "watch.change_detected")]
    WatchChangeDetected {
        changes: Vec<(RepoPath, ChangeKind)>,
    },
    #[serde(rename = "system.warning")]
    SystemWarning { message: String },
    #[serde(rename = "system.error")]
    SystemError { message: String },
}

impl Event {
    pub fn type_name(&self) -> &'static str {
        match self {
            Event::RepositoryDiscovered { .. } => "repository.discovered",
            Event::ScanStarted { .. } => "scan.started",
            Event::ScanProgress { .. } => "scan.progress",
            Event::ScanCompleted { .. } => "scan.completed",
            Event::DocumentIndexed { .. } => "document.indexed",
            Event::DocumentRemoved { .. } => "document.removed",
            Event::ClaimCreated { .. } => "claim.created",
            Event::ClaimUpdated { .. } => "claim.updated",
            Event::ClaimRemoved { .. } => "claim.removed",
            Event::SourceIndexed { .. } => "source.indexed",
            Event::SourceRemoved { .. } => "source.removed",
            Event::SymbolCreated { .. } => "symbol.created",
            Event::SymbolUpdated { .. } => "symbol.updated",
            Event::SymbolRemoved { .. } => "symbol.removed",
            Event::RelationshipCreated { .. } => "relationship.created",
            Event::RelationshipInvalidated { .. } => "relationship.invalidated",
            Event::EvidenceCreated { .. } => "evidence.created",
            Event::EvidenceInvalidated { .. } => "evidence.invalidated",
            Event::AnalysisQueued { .. } => "analysis.queued",
            Event::AnalysisStarted { .. } => "analysis.started",
            Event::AnalysisProgress { .. } => "analysis.progress",
            Event::AnalysisCompleted { .. } => "analysis.completed",
            Event::FindingCreated { .. } => "finding.created",
            Event::FindingUpdated { .. } => "finding.updated",
            Event::FindingResolved { .. } => "finding.resolved",
            Event::WatchChangeDetected { .. } => "watch.change_detected",
            Event::SystemWarning { .. } => "system.warning",
            Event::SystemError { .. } => "system.error",
        }
    }

    /// Events that describe committed model state (as opposed to progress/telemetry).
    /// Only these are persisted to event history.
    pub fn is_state_change(&self) -> bool {
        !matches!(
            self,
            Event::ScanProgress { .. }
                | Event::AnalysisProgress { .. }
                | Event::AnalysisQueued { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_wire_format_is_flat_and_typed() {
        let env = Envelope {
            seq: 7,
            revision: Revision(2),
            at: Timestamp(1),
            event: Event::SystemWarning {
                message: "x".into(),
            },
        };
        let v: serde_json::Value = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "system.warning");
        assert_eq!(v["seq"], 7);
        assert_eq!(v["data"]["message"], "x");
        assert_eq!(env.event.type_name(), "system.warning");
        let back: Envelope = serde_json::from_value(v).unwrap();
        assert_eq!(back.seq, 7);
    }
}
