//! The in-memory semantic graph: every entity of the current revision plus the
//! reverse indexes needed for resolution, change impact and scoped queries.
//!
//! The model is owned by the engine (single writer). Readers (HTTP handlers, the
//! semantic context builder) take shared access through the engine's lock. Nothing in
//! here performs I/O.

mod mutate;
mod neighborhood;
mod resolve;

pub use neighborhood::{Neighborhood, NeighborhoodEdge, NeighborhoodNode};
pub use resolve::{lexical_candidates, Resolved, STD_ROOTS};

use concordance_core::code::{Import, SourceFile, Symbol, WorkspaceInfo};
use concordance_core::doc::{Claim, Document, ProseReference, Section};
use concordance_core::evidence::Evidence;
use concordance_core::finding::{Counts, Finding, FindingStatus, FindingSubject};
use concordance_core::graph::{Edge, NodeRef};
use concordance_core::testrun::TestRun;
use concordance_core::{
    ClaimId, ContentHash, DocumentId, EdgeId, EvidenceId, FileId, FindingId, RepoPath, Revision,
    SectionId, SymbolId,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default, Debug, Clone)]
pub struct Model {
    pub revision: Revision,
    pub workspace: WorkspaceInfo,
    pub documents: BTreeMap<DocumentId, Document>,
    pub sections: BTreeMap<SectionId, Section>,
    pub claims: BTreeMap<ClaimId, Claim>,
    pub prose_refs: BTreeMap<DocumentId, Vec<ProseReference>>,
    pub files: BTreeMap<FileId, SourceFile>,
    pub symbols: BTreeMap<SymbolId, Symbol>,
    pub imports: BTreeMap<FileId, Vec<Import>>,
    pub edges: BTreeMap<EdgeId, Edge>,
    pub evidence: BTreeMap<EvidenceId, Evidence>,
    pub findings: BTreeMap<FindingId, Finding>,
    /// Ingested test-run outputs (reloaded from their files; not persisted).
    pub test_runs: Vec<TestRun>,
    idx: Indexes,
}

/// Derived lookups. Maintained by the mutation methods only.
#[derive(Default, Debug, Clone)]
struct Indexes {
    doc_by_path: HashMap<RepoPath, DocumentId>,
    file_by_path: HashMap<RepoPath, FileId>,
    sections_by_doc: HashMap<DocumentId, BTreeSet<SectionId>>,
    claims_by_doc: HashMap<DocumentId, BTreeSet<ClaimId>>,
    symbols_by_file: HashMap<FileId, BTreeSet<SymbolId>>,
    /// Last path segment -> symbols.
    symbols_by_name: HashMap<String, BTreeSet<SymbolId>>,
    children: HashMap<SymbolId, BTreeSet<SymbolId>>,
    edges_from: HashMap<NodeRef, BTreeSet<EdgeId>>,
    edges_to: HashMap<NodeRef, BTreeSet<EdgeId>>,
    evidence_by_subject: HashMap<NodeRef, BTreeSet<EvidenceId>>,
    evidence_by_dep: HashMap<NodeRef, BTreeSet<EvidenceId>>,
    /// Last segment of every symbol reference written in a claim or prose -> writer.
    /// Lets a symbol appearing/disappearing find the documentation it affects.
    refs_by_name: HashMap<String, BTreeSet<NodeRef>>,
    /// Document path targeted by section links -> writers (claims or documents).
    refs_by_doc_target: HashMap<RepoPath, BTreeSet<NodeRef>>,
    findings_by_claim: HashMap<ClaimId, BTreeSet<FindingId>>,
    findings_by_symbol: HashMap<SymbolId, BTreeSet<FindingId>>,
    /// Requirement ID -> claims tagged with it.
    claims_by_requirement: HashMap<String, BTreeSet<ClaimId>>,
    /// Requirement ID -> symbols whose doc declares `Verifies: <id>`.
    symbols_by_requirement: HashMap<String, BTreeSet<SymbolId>>,
}

impl Model {
    /// Claims tagged with requirement `id`.
    pub fn claims_of_requirement<'a>(&'a self, id: &str) -> impl Iterator<Item = &'a Claim> + 'a {
        self.idx
            .claims_by_requirement
            .get(id)
            .into_iter()
            .flatten()
            .filter_map(|c| self.claims.get(c))
    }

    /// Whether any claim defines requirement `id`.
    pub fn requirement_defined(&self, id: &str) -> bool {
        self.idx
            .claims_by_requirement
            .get(id)
            .is_some_and(|s| !s.is_empty())
    }

    /// Symbols that declare `Verifies: <id>`.
    pub fn verifiers_of<'a>(&'a self, id: &str) -> impl Iterator<Item = &'a Symbol> + 'a {
        self.idx
            .symbols_by_requirement
            .get(id)
            .into_iter()
            .flatten()
            .filter_map(|s| self.symbols.get(s))
    }

    /// Every claim that carries a requirement ID.
    pub fn requirement_claims(&self) -> impl Iterator<Item = &Claim> {
        self.idx
            .claims_by_requirement
            .values()
            .flatten()
            .filter_map(|c| self.claims.get(c))
    }

    /// Every symbol that declares `Verifies:`.
    pub fn verifying_symbols(&self) -> impl Iterator<Item = &Symbol> {
        let ids: BTreeSet<&SymbolId> = self.idx.symbols_by_requirement.values().flatten().collect();
        ids.into_iter().filter_map(|s| self.symbols.get(s))
    }

    /// The recorded outcome for test symbol `sym` across all ingested runs: the most
    /// specific result name (longest `::` suffix of the symbol path) that names no other
    /// test symbol. Returns the run and outcome, or `None` if no run reports it.
    pub fn test_result(
        &self,
        sym: &Symbol,
    ) -> Option<(&TestRun, concordance_core::testrun::TestOutcome)> {
        let mut best: Option<(usize, &TestRun, concordance_core::testrun::TestOutcome)> = None;
        for run in &self.test_runs {
            for k in 1..=sym.path.len() {
                let name = sym.path[sym.path.len() - k..].join("::");
                let Some(o) = run.results.get(&name) else {
                    continue;
                };
                // A result name that is also a suffix of another test's path is ambiguous.
                let segs: Vec<&str> = name.split("::").collect();
                let others = self
                    .idx
                    .symbols_by_name
                    .get(segs[segs.len() - 1])
                    .into_iter()
                    .flatten()
                    .filter_map(|id| self.symbols.get(id))
                    .filter(|o| {
                        o.id != sym.id
                            && o.is_test
                            && o.path.len() >= segs.len()
                            && o.path[o.path.len() - segs.len()..]
                                .iter()
                                .map(String::as_str)
                                .eq(segs.iter().copied())
                    });
                if others.count() > 0 {
                    continue;
                }
                if best
                    .as_ref()
                    .is_none_or(|b| k > b.0 || (k == b.0 && *o > b.2))
                {
                    best = Some((k, run, *o));
                }
            }
        }
        best.map(|(_, r, o)| (r, o))
    }

    pub fn document_by_path(&self, path: &RepoPath) -> Option<&Document> {
        self.idx
            .doc_by_path
            .get(path)
            .and_then(|id| self.documents.get(id))
    }

    pub fn file_by_path(&self, path: &RepoPath) -> Option<&SourceFile> {
        self.idx
            .file_by_path
            .get(path)
            .and_then(|id| self.files.get(id))
    }

    pub fn sections_of(&self, doc: &DocumentId) -> impl Iterator<Item = &Section> {
        self.idx
            .sections_by_doc
            .get(doc)
            .into_iter()
            .flatten()
            .filter_map(|id| self.sections.get(id))
    }

    pub fn claims_of(&self, doc: &DocumentId) -> impl Iterator<Item = &Claim> {
        self.idx
            .claims_by_doc
            .get(doc)
            .into_iter()
            .flatten()
            .filter_map(|id| self.claims.get(id))
    }

    pub fn symbols_of(&self, file: &FileId) -> impl Iterator<Item = &Symbol> {
        self.idx
            .symbols_by_file
            .get(file)
            .into_iter()
            .flatten()
            .filter_map(|id| self.symbols.get(id))
    }

    pub fn symbols_named(&self, name: &str) -> impl Iterator<Item = &Symbol> {
        self.idx
            .symbols_by_name
            .get(name)
            .into_iter()
            .flatten()
            .filter_map(|id| self.symbols.get(id))
    }

    pub fn children_of(&self, sym: &SymbolId) -> impl Iterator<Item = &Symbol> {
        self.idx
            .children
            .get(sym)
            .into_iter()
            .flatten()
            .filter_map(|id| self.symbols.get(id))
    }

    pub fn edges_from(&self, node: &NodeRef) -> impl Iterator<Item = &Edge> {
        self.idx
            .edges_from
            .get(node)
            .into_iter()
            .flatten()
            .filter_map(|id| self.edges.get(id))
    }

    pub fn edges_to(&self, node: &NodeRef) -> impl Iterator<Item = &Edge> {
        self.idx
            .edges_to
            .get(node)
            .into_iter()
            .flatten()
            .filter_map(|id| self.edges.get(id))
    }

    pub fn evidence_for(&self, subject: &NodeRef) -> impl Iterator<Item = &Evidence> {
        self.idx
            .evidence_by_subject
            .get(subject)
            .into_iter()
            .flatten()
            .filter_map(|id| self.evidence.get(id))
    }

    /// Evidence records that list `node` as a dependency.
    pub fn evidence_depending_on(&self, node: &NodeRef) -> Vec<EvidenceId> {
        self.idx
            .evidence_by_dep
            .get(node)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Claims or documents whose references end with `name`.
    pub fn referencers_of_name(&self, name: &str) -> Vec<NodeRef> {
        self.idx
            .refs_by_name
            .get(name)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Claims or documents that link to a section of the document at `path`.
    pub fn referencers_of_document(&self, path: &RepoPath) -> Vec<NodeRef> {
        self.idx
            .refs_by_doc_target
            .get(path)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn findings_for_claim(&self, claim: &ClaimId) -> impl Iterator<Item = &Finding> {
        self.idx
            .findings_by_claim
            .get(claim)
            .into_iter()
            .flatten()
            .filter_map(|id| self.findings.get(id))
    }

    pub fn findings_for_symbol(&self, sym: &SymbolId) -> impl Iterator<Item = &Finding> {
        self.idx
            .findings_by_symbol
            .get(sym)
            .into_iter()
            .flatten()
            .filter_map(|id| self.findings.get(id))
    }

    /// The reconciliation finding of an obligation claim, if any.
    pub fn claim_finding(&self, claim: &ClaimId) -> Option<&Finding> {
        self.findings.get(
            &FindingSubject::Claim {
                claim: claim.clone(),
            }
            .finding_id(),
        )
    }

    /// Status of a claim = status of its open claim finding (`None` for informative claims).
    pub fn claim_status(&self, claim: &ClaimId) -> Option<FindingStatus> {
        self.claim_finding(claim)
            .filter(|f| f.is_open())
            .map(|f| f.status)
    }

    /// Current content hash of any node, `None` if the node no longer exists.
    pub fn content_hash(&self, node: &NodeRef) -> Option<ContentHash> {
        match node {
            NodeRef::Document(id) => self.documents.get(id).map(|d| d.content_hash.clone()),
            NodeRef::Section(id) => self.sections.get(id).map(|s| {
                ContentHash::of_str(&format!(
                    "{}\u{1f}{}",
                    s.heading_path.join("\u{1f}"),
                    s.anchor
                ))
            }),
            NodeRef::Claim(id) => self.claims.get(id).map(|c| c.content_hash.clone()),
            NodeRef::File(id) => self.files.get(id).map(|f| f.content_hash.clone()),
            NodeRef::Symbol(id) => self.symbols.get(id).map(|s| s.content_hash.clone()),
        }
    }

    pub fn exists(&self, node: &NodeRef) -> bool {
        match node {
            NodeRef::Document(id) => self.documents.contains_key(id),
            NodeRef::Section(id) => self.sections.contains_key(id),
            NodeRef::Claim(id) => self.claims.contains_key(id),
            NodeRef::File(id) => self.files.contains_key(id),
            NodeRef::Symbol(id) => self.symbols.contains_key(id),
        }
    }

    /// Short human label for a node (UI, graph view, events).
    pub fn label(&self, node: &NodeRef) -> Option<String> {
        Some(match node {
            NodeRef::Document(id) => self.documents.get(id)?.path.to_string(),
            NodeRef::Section(id) => self.sections.get(id)?.heading.clone(),
            NodeRef::Claim(id) => truncate(&self.claims.get(id)?.text, 120),
            NodeRef::File(id) => self.files.get(id)?.path.to_string(),
            NodeRef::Symbol(id) => self.symbols.get(id)?.qualified_name(),
        })
    }

    /// Ancestor chain of a symbol (nearest first).
    pub fn ancestors(&self, sym: &SymbolId) -> Vec<&Symbol> {
        let mut out = Vec::new();
        let mut cur = self.symbols.get(sym).and_then(|s| s.parent.clone());
        while let Some(id) = cur {
            match self.symbols.get(&id) {
                Some(s) => {
                    cur = s.parent.clone();
                    out.push(s);
                }
                None => break,
            }
        }
        out
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts {
            documents: self.documents.len() as u32,
            claims: self.claims.len() as u32,
            obligation_claims: self.claims.values().filter(|c| c.is_obligation()).count() as u32,
            source_files: self.files.len() as u32,
            symbols: self.symbols.len() as u32,
            edges: self.edges.len() as u32,
            ..Counts::default()
        };
        for f in self.findings.values() {
            if !f.is_open() {
                c.resolved += 1;
                continue;
            }
            match f.status {
                FindingStatus::Verified => c.verified += 1,
                FindingStatus::Violated => c.violated += 1,
                FindingStatus::Unverified => c.unverified += 1,
                FindingStatus::Stale => c.stale += 1,
                FindingStatus::Ambiguous => c.ambiguous += 1,
                FindingStatus::Undocumented => c.undocumented += 1,
            }
        }
        c
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
