//! Response types (see docs/api.md). Built from a model snapshot.

use concordance_core::code::{SourceFile, Symbol, SymbolKind, Visibility};
use concordance_core::doc::{AuthorityAssessment, Claim, Document};
use concordance_core::finding::{FindingStatus, FindingSubject};
use concordance_core::graph::{Edge, EdgeKind, NodeRef};
use concordance_core::{DocumentId, RepoPath, SourceRange, SymbolId};
use concordance_graph::Model;
use serde::Serialize;

#[derive(Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub seq: u64,
}

pub fn page<T>(all: Vec<T>, offset: usize, limit: usize, seq: u64) -> Page<T> {
    let total = all.len();
    let items = all.into_iter().skip(offset).take(limit).collect();
    Page {
        items,
        total,
        offset,
        limit,
        seq,
    }
}

#[derive(Serialize, Default)]
pub struct StatusCounts {
    pub verified: u32,
    pub violated: u32,
    pub unverified: u32,
    pub stale: u32,
    pub ambiguous: u32,
}

impl StatusCounts {
    fn add(&mut self, s: FindingStatus) {
        match s {
            FindingStatus::Verified => self.verified += 1,
            FindingStatus::Violated => self.violated += 1,
            FindingStatus::Unverified => self.unverified += 1,
            FindingStatus::Stale => self.stale += 1,
            FindingStatus::Ambiguous => self.ambiguous += 1,
            FindingStatus::Undocumented => {}
        }
    }
}

#[derive(Serialize)]
pub struct DocumentSummary {
    pub id: DocumentId,
    pub path: RepoPath,
    pub title: Option<String>,
    pub authority: AuthorityAssessment,
    pub sections: usize,
    pub claims: usize,
    pub obligations: usize,
    pub status_counts: StatusCounts,
}

pub fn document_summary(m: &Model, d: &Document) -> DocumentSummary {
    let mut counts = StatusCounts::default();
    let mut claims = 0;
    let mut obligations = 0;
    for c in m.claims_of(&d.id) {
        claims += 1;
        if c.is_obligation() {
            obligations += 1;
        }
        if let Some(s) = m.claim_status(&c.id) {
            counts.add(s);
        }
    }
    DocumentSummary {
        id: d.id.clone(),
        path: d.path.clone(),
        title: d.title.clone(),
        authority: d.authority.clone(),
        sections: m.sections_of(&d.id).count(),
        claims,
        obligations,
        status_counts: counts,
    }
}

#[derive(Serialize)]
pub struct ClaimView {
    pub claim: Claim,
    pub status: Option<FindingStatus>,
    pub finding: Option<String>,
    pub document_path: Option<RepoPath>,
}

pub fn claim_view(m: &Model, c: &Claim) -> ClaimView {
    let f = m.claim_finding(&c.id).filter(|f| f.is_open());
    ClaimView {
        claim: c.clone(),
        status: f.map(|f| f.status),
        finding: f.map(|f| f.id.to_string()),
        document_path: m.documents.get(&c.document).map(|d| d.path.clone()),
    }
}

#[derive(Serialize)]
pub struct EdgeView {
    pub edge: Edge,
    pub target_label: Option<String>,
    pub target_kind: Option<&'static str>,
}

pub fn edge_view(m: &Model, e: &Edge) -> EdgeView {
    EdgeView {
        edge: e.clone(),
        target_label: e.to.as_ref().and_then(|t| m.label(t)),
        target_kind: e.to.as_ref().map(|t| t.kind()),
    }
}

#[derive(Serialize)]
pub struct SymbolView {
    pub id: SymbolId,
    pub name: String,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub is_public_api: bool,
    pub has_doc: bool,
    pub file_path: Option<RepoPath>,
    pub range: SourceRange,
    pub constrained_by: usize,
    pub finding_status: Option<FindingStatus>,
}

pub fn constraining_edges<'a>(m: &'a Model, id: &SymbolId) -> impl Iterator<Item = &'a Edge> {
    m.edges_to(&NodeRef::Symbol(id.clone())).filter(|e| {
        matches!(
            e.kind,
            EdgeKind::ReferencesSymbol | EdgeKind::ConstrainsScope
        ) && matches!(e.from, NodeRef::Claim(_))
    })
}

pub fn symbol_view(m: &Model, s: &Symbol) -> SymbolView {
    SymbolView {
        id: s.id.clone(),
        name: s.name.clone(),
        qualified_name: s.qualified_name(),
        kind: s.kind,
        visibility: s.visibility,
        is_public_api: s.is_public_api,
        has_doc: s.doc.is_some(),
        file_path: m.files.get(&s.file).map(|f| f.path.clone()),
        range: s.range.clone(),
        constrained_by: constraining_edges(m, &s.id).count(),
        finding_status: m
            .findings_for_symbol(&s.id)
            .filter(|f| f.is_open())
            .map(|f| f.status)
            .min(),
    }
}

#[derive(Serialize)]
pub struct CrateSummary {
    pub name: String,
    pub root_dir: Option<RepoPath>,
    pub files: usize,
    pub symbols: usize,
    pub public_api: usize,
    pub unconstrained_public_api: usize,
    pub modules: Vec<String>,
}

pub fn crate_summaries(m: &Model) -> Vec<CrateSummary> {
    let mut names: Vec<String> = m.workspace.crates.iter().map(|c| c.name.clone()).collect();
    for f in m.files.values() {
        if let Some(c) = &f.crate_name {
            if !names.contains(c) {
                names.push(c.clone());
            }
        }
    }
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let files: Vec<&SourceFile> = m
                .files
                .values()
                .filter(|f| f.crate_name.as_deref() == Some(name.as_str()))
                .collect();
            let mut modules: Vec<String> = files
                .iter()
                .map(|f| f.module_path.join("::"))
                .filter(|s| !s.is_empty())
                .collect();
            modules.sort();
            modules.dedup();
            let syms: Vec<&Symbol> = files.iter().flat_map(|f| m.symbols_of(&f.id)).collect();
            let public: Vec<&&Symbol> = syms
                .iter()
                .filter(|s| {
                    s.is_public_api && s.kind != SymbolKind::Impl && s.kind != SymbolKind::Module
                })
                .collect();
            CrateSummary {
                root_dir: m
                    .workspace
                    .crates
                    .iter()
                    .find(|c| c.name == name)
                    .and_then(|c| c.root_dir.clone()),
                files: files.len(),
                symbols: syms.len(),
                public_api: public.len(),
                unconstrained_public_api: public
                    .iter()
                    .filter(|s| constraining_edges(m, &s.id).next().is_none())
                    .count(),
                modules,
                name,
            }
        })
        .collect()
}

/// Every claim a finding is about (claim findings, conflicts).
pub fn finding_claims(subject: &FindingSubject) -> Vec<concordance_core::ClaimId> {
    match subject {
        FindingSubject::Claim { claim } => vec![claim.clone()],
        FindingSubject::ClaimConflict { claims } => claims.clone(),
        _ => vec![],
    }
}
