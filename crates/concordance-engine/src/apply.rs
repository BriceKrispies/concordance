//! Applying a change batch to the model.
//!
//! A batch has an index phase (documents and source files replaced one by one, each
//! diffed against its previous version) and a reconcile phase (evidence invalidation,
//! re-linking, structural checks and finding derivation for exactly the affected
//! claims, documents and symbols). Every touched entity id is recorded in [`Dirty`] so
//! the store receives one exact delta.

use crate::source::RepoFs;
use concordance_core::code::{SourceIndex, Symbol};
use concordance_core::doc::{Claim, DocumentIndex};
use concordance_core::event::{ClaimSummary, Event, EvidenceSummary, SymbolSummary};
use concordance_core::evidence::{Evidence, MethodKind, StructuralCheck, Validity};
use concordance_core::finding::{Finding, FindingSubject};
use concordance_core::graph::{Edge, EdgeKind, NodeRef};
use concordance_core::{
    ClaimId, DocumentId, EdgeId, EvidenceId, FileId, FindingId, RepoPath, SectionId, SymbolId,
};
use concordance_graph::{truncate, Model};
use concordance_reconcile::lifecycle::{self, Change};
use concordance_reconcile::{derive, gaps, link, structural, BatchContext};
use concordance_storage::{Delta, FindingTransition, Snapshot};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub fn model_from_snapshot(s: Snapshot) -> Model {
    let mut m = Model::default();
    m.revision = s.revision;
    m.workspace = s.workspace.unwrap_or_default();
    for d in s.documents {
        m.upsert_document(d);
    }
    let mut by_doc: BTreeMap<DocumentId, Vec<_>> = BTreeMap::new();
    for sec in s.sections {
        by_doc.entry(sec.document.clone()).or_default().push(sec);
    }
    for (doc, secs) in by_doc {
        m.replace_sections(&doc, secs);
    }
    for c in s.claims {
        m.upsert_claim(c);
    }
    for (doc, refs) in s.prose_references {
        m.set_prose_refs(&doc, refs);
    }
    for f in s.files {
        m.upsert_file(f);
    }
    for sym in s.symbols {
        m.upsert_symbol(sym);
    }
    for (f, imps) in s.imports {
        m.set_imports(&f, imps);
    }
    for e in s.edges {
        m.upsert_edge(e);
    }
    for e in s.evidence {
        m.upsert_evidence(e);
    }
    for f in s.findings {
        m.upsert_finding(f);
    }
    m
}

/// Ids touched since the last commit. Existence in the model at commit time decides
/// upsert vs delete.
#[derive(Default)]
pub struct Dirty {
    pub documents: BTreeSet<DocumentId>,
    pub sections: BTreeSet<SectionId>,
    pub claims: BTreeSet<ClaimId>,
    pub prose_docs: BTreeSet<DocumentId>,
    pub files: BTreeSet<FileId>,
    pub symbols: BTreeSet<SymbolId>,
    pub import_files: BTreeSet<FileId>,
    pub edges: BTreeSet<EdgeId>,
    pub evidence: BTreeSet<EvidenceId>,
    pub findings: BTreeSet<FindingId>,
    pub transitions: Vec<FindingTransition>,
    pub workspace: bool,
}

impl Dirty {
    pub fn to_delta(&self, m: &Model) -> Delta {
        let mut d = Delta {
            revision: m.revision,
            ..Delta::default()
        };
        if self.workspace {
            d.workspace = Some(m.workspace.clone());
        }
        macro_rules! split {
            ($set:expr, $map:expr, $up:ident, $del:ident) => {
                for id in &$set {
                    match $map.get(id) {
                        Some(v) => d.$up.push(v.clone()),
                        None => d.$del.push(id.clone()),
                    }
                }
            };
        }
        split!(
            self.documents,
            m.documents,
            upsert_documents,
            delete_documents
        );
        split!(self.sections, m.sections, upsert_sections, delete_sections);
        split!(self.claims, m.claims, upsert_claims, delete_claims);
        split!(self.files, m.files, upsert_files, delete_files);
        split!(self.symbols, m.symbols, upsert_symbols, delete_symbols);
        split!(self.edges, m.edges, upsert_edges, delete_edges);
        split!(self.evidence, m.evidence, upsert_evidence, delete_evidence);
        for id in &self.findings {
            if let Some(f) = m.findings.get(id) {
                d.upsert_findings.push(f.clone());
            }
        }
        for doc in &self.prose_docs {
            d.prose_references.push((
                doc.clone(),
                m.prose_refs.get(doc).cloned().unwrap_or_default(),
            ));
        }
        for f in &self.import_files {
            d.imports
                .push((f.clone(), m.imports.get(f).cloned().unwrap_or_default()));
        }
        d.finding_transitions = self.transitions.clone();
        d
    }
}

/// What the reconcile phase must revisit.
#[derive(Default)]
pub struct Affected {
    pub claims: BTreeSet<ClaimId>,
    pub docs: BTreeSet<DocumentId>,
    pub symbols: BTreeSet<SymbolId>,
    pub symbol_edges: BTreeSet<SymbolId>,
    pub changed_nodes: Vec<NodeRef>,
    pub conflicts: bool,
    /// Claims whose semantic evidence was invalidated (running analyses get cancelled).
    pub semantic_invalidated: BTreeSet<ClaimId>,
    /// Names of symbols created in this batch: claims that mention them only in prose
    /// (lexical retrieval) must be re-linked too.
    pub new_names: BTreeSet<String>,
}

/// `RateLimiter` / `rate_limiter` -> "rate limiter".
fn name_phrase(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch == '_' {
            out.push(' ');
            prev_lower = false;
            continue;
        }
        if ch.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = ch.is_lowercase() || ch.is_ascii_digit();
        out.extend(ch.to_lowercase());
    }
    out.trim().to_string()
}

/// One change batch in progress. Survives across index chunks of a scan.
pub struct Pending {
    pub ctx: BatchContext,
    pub dirty: Dirty,
    pub affected: Affected,
}

fn claim_summary(c: &Claim) -> ClaimSummary {
    ClaimSummary {
        id: c.id.clone(),
        document: c.document.clone(),
        text: truncate(&c.text, 200),
        kind: c.kind,
        modality: c.modality,
    }
}

fn symbol_summary(s: &Symbol) -> SymbolSummary {
    SymbolSummary {
        id: s.id.clone(),
        file: s.file.clone(),
        kind: s.kind,
        path: s.qualified_name(),
    }
}

fn tokens(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() > 1)
        .map(str::to_lowercase)
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    a.intersection(b).count() as f32 / a.union(b).count() as f32
}

/// Carry claim identities across edits: a new claim whose content-derived id is new
/// takes the id of a removed claim from the same document when it is the unique best
/// textual match (Jaccard >= 0.5, runner-up at least 0.1 behind).
pub fn carry_claim_ids(old: &[Claim], new: &mut [Claim]) {
    let new_ids: HashSet<ClaimId> = new.iter().map(|c| c.id.clone()).collect();
    let old_ids: HashSet<ClaimId> = old.iter().map(|c| c.id.clone()).collect();
    let candidates: Vec<(&Claim, HashSet<String>)> = old
        .iter()
        .filter(|c| !new_ids.contains(&c.id))
        .map(|c| (c, tokens(&c.text)))
        .collect();
    let mut taken: HashSet<ClaimId> = HashSet::new();
    for n in new.iter_mut().filter(|n| !old_ids.contains(&n.id)) {
        let nt = tokens(&n.text);
        let mut scored: Vec<(f32, &Claim)> = candidates
            .iter()
            .filter(|(o, _)| !taken.contains(&o.id) && o.kind == n.kind)
            .map(|(o, ot)| (jaccard(&nt, ot), *o))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(&(best, o)) = scored.first() {
            let runner_up = scored.get(1).map_or(0.0, |x| x.0);
            if best >= 0.5 && best - runner_up >= 0.1 {
                taken.insert(o.id.clone());
                n.id = o.id.clone();
            }
        }
    }
}

impl Pending {
    pub fn new(ctx: BatchContext) -> Self {
        Pending {
            ctx,
            dirty: Dirty::default(),
            affected: Affected::default(),
        }
    }

    // ---------------------------------------------------------------- findings

    fn put_finding(&mut self, m: &mut Model, f: Finding, events: &mut Vec<Event>) {
        let prev = m.findings.get(&f.id).cloned();
        let (merged, change) = lifecycle::merge(prev.as_ref(), f, &self.ctx);
        let transition = |from, to| FindingTransition {
            finding: merged.id.clone(),
            revision: self.ctx.revision,
            at: self.ctx.now,
            from,
            to,
            title: merged.title.clone(),
        };
        match change {
            Change::Unchanged => return,
            Change::Created => {
                self.dirty
                    .transitions
                    .push(transition(None, Some(merged.status)));
                events.push(Event::FindingCreated {
                    finding: merged.clone(),
                });
            }
            Change::StatusChanged { from } => {
                self.dirty
                    .transitions
                    .push(transition(Some(from), Some(merged.status)));
                events.push(Event::FindingUpdated {
                    finding: merged.clone(),
                    previous_status: from,
                });
            }
            Change::Updated => {
                events.push(Event::FindingUpdated {
                    finding: merged.clone(),
                    previous_status: merged.status,
                });
            }
        }
        self.dirty.findings.insert(merged.id.clone());
        m.upsert_finding(merged);
    }

    fn resolve_finding(&mut self, m: &mut Model, id: &FindingId, events: &mut Vec<Event>) {
        let Some(prev) = m.findings.get(id).cloned() else {
            return;
        };
        if let Some(resolved) = lifecycle::resolve(&prev, &self.ctx) {
            self.dirty.transitions.push(FindingTransition {
                finding: id.clone(),
                revision: self.ctx.revision,
                at: self.ctx.now,
                from: Some(prev.status),
                to: None,
                title: prev.title.clone(),
            });
            events.push(Event::FindingResolved {
                finding: id.clone(),
                previous_status: prev.status,
            });
            self.dirty.findings.insert(id.clone());
            m.upsert_finding(resolved);
        }
    }

    fn remove_evidence_of(
        &mut self,
        m: &mut Model,
        subject: &NodeRef,
        keep: impl Fn(&Evidence) -> bool,
    ) {
        let ids: Vec<EvidenceId> = m
            .evidence_for(subject)
            .filter(|e| !keep(e))
            .map(|e| e.id.clone())
            .collect();
        for id in ids {
            m.remove_evidence(&id);
            self.dirty.evidence.insert(id);
        }
    }

    fn remove_edges_from(&mut self, m: &mut Model, node: &NodeRef) {
        let ids: Vec<EdgeId> = m.edges_from(node).map(|e| e.id.clone()).collect();
        for id in ids {
            m.remove_edge(&id);
            self.dirty.edges.insert(id);
        }
    }

    // ---------------------------------------------------------------- documents

    fn remove_claim(&mut self, m: &mut Model, id: &ClaimId, events: &mut Vec<Event>) {
        let node = NodeRef::Claim(id.clone());
        let targets: Vec<SymbolId> = m
            .edges_from(&node)
            .filter_map(|e| match &e.to {
                Some(NodeRef::Symbol(s)) => Some(s.clone()),
                _ => None,
            })
            .collect();
        self.affected.symbols.extend(targets);
        self.remove_edges_from(m, &node);
        self.remove_evidence_of(m, &node, |_| false);
        if let Some(c) = m.remove_claim(id) {
            if !c.bounds.is_empty() {
                self.affected.conflicts = true;
            }
        }
        self.dirty.claims.insert(id.clone());
        self.affected.changed_nodes.push(node);
        self.affected.claims.remove(id);
        let fids: Vec<FindingId> = m.findings_for_claim(id).map(|f| f.id.clone()).collect();
        for fid in fids {
            if matches!(
                m.findings.get(&fid).map(|f| &f.subject),
                Some(FindingSubject::ClaimConflict { .. })
            ) {
                self.affected.conflicts = true;
            } else {
                self.resolve_finding(m, &fid, events);
            }
        }
        events.push(Event::ClaimRemoved { claim: id.clone() });
    }

    pub fn apply_document(
        &mut self,
        m: &mut Model,
        mut idx: DocumentIndex,
        events: &mut Vec<Event>,
    ) {
        let doc_id = idx.document.id.clone();
        let old: Vec<Claim> = m.claims_of(&doc_id).cloned().collect();
        carry_claim_ids(&old, &mut idx.claims);
        let new_ids: HashSet<ClaimId> = idx.claims.iter().map(|c| c.id.clone()).collect();
        for o in old.iter().filter(|o| !new_ids.contains(&o.id)) {
            self.remove_claim(m, &o.id, events);
        }
        for c in idx.claims {
            let prev = m.claims.get(&c.id).cloned();
            match &prev {
                None => events.push(Event::ClaimCreated {
                    claim: claim_summary(&c),
                }),
                Some(p) if p.content_hash != c.content_hash => {
                    events.push(Event::ClaimUpdated {
                        claim: claim_summary(&c),
                    });
                    self.affected
                        .changed_nodes
                        .push(NodeRef::Claim(c.id.clone()));
                }
                Some(p) if *p == c => {
                    // Unchanged claim in a changed document: structural checks still rerun
                    // (cheap) because neighboring context or links may have changed.
                    self.affected.claims.insert(c.id.clone());
                    continue;
                }
                Some(_) => {}
            }
            if !c.bounds.is_empty() || prev.as_ref().is_some_and(|p| !p.bounds.is_empty()) {
                self.affected.conflicts = true;
            }
            self.affected.claims.insert(c.id.clone());
            self.dirty.claims.insert(c.id.clone());
            m.upsert_claim(c);
        }
        for s in m.sections_of(&doc_id) {
            self.dirty.sections.insert(s.id.clone());
        }
        for s in &idx.sections {
            self.dirty.sections.insert(s.id.clone());
        }
        m.replace_sections(&doc_id, idx.sections);
        m.set_prose_refs(&doc_id, idx.prose_references);
        self.dirty.prose_docs.insert(doc_id.clone());
        let path = idx.document.path.clone();
        let doc_changed = m
            .documents
            .get(&doc_id)
            .map(|d| d.content_hash != idx.document.content_hash)
            .unwrap_or(true);
        m.upsert_document(idx.document);
        self.dirty.documents.insert(doc_id.clone());
        if doc_changed {
            self.affected
                .changed_nodes
                .push(NodeRef::Document(doc_id.clone()));
        }
        self.affected.docs.insert(doc_id.clone());
        // Links into this document may have gained or lost their anchor.
        for r in m.referencers_of_document(&path) {
            self.mark_referencer(r);
        }
        let claims = m.claims_of(&doc_id).count() as u32;
        events.push(Event::DocumentIndexed {
            document: doc_id,
            path,
            claims,
        });
    }

    pub fn remove_document(&mut self, m: &mut Model, path: &RepoPath, events: &mut Vec<Event>) {
        let Some(doc) = m.document_by_path(path).cloned() else {
            return;
        };
        let claims: Vec<ClaimId> = m.claims_of(&doc.id).map(|c| c.id.clone()).collect();
        for c in claims {
            self.remove_claim(m, &c, events);
        }
        for s in m.sections_of(&doc.id) {
            self.dirty.sections.insert(s.id.clone());
        }
        let dnode = NodeRef::Document(doc.id.clone());
        self.remove_edges_from(m, &dnode);
        self.remove_evidence_of(m, &dnode, |_| false);
        m.remove_document(&doc.id);
        self.dirty.documents.insert(doc.id.clone());
        self.dirty.prose_docs.insert(doc.id.clone());
        self.affected.changed_nodes.push(dnode);
        self.affected.docs.insert(doc.id.clone());
        for r in m.referencers_of_document(path) {
            self.mark_referencer(r);
        }
        events.push(Event::DocumentRemoved {
            document: doc.id,
            path: path.clone(),
        });
    }

    fn mark_referencer(&mut self, r: NodeRef) {
        match r {
            NodeRef::Claim(c) => {
                self.affected.claims.insert(c);
            }
            NodeRef::Document(d) => {
                self.affected.docs.insert(d);
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- source

    fn on_symbol_gone_or_renamed(&mut self, m: &Model, name: &str) {
        for r in m.referencers_of_name(name) {
            self.mark_referencer(r);
        }
    }

    fn mark_symbol_referencers(&mut self, m: &Model, id: &SymbolId) {
        let refs: Vec<NodeRef> = m
            .edges_to(&NodeRef::Symbol(id.clone()))
            .map(|e| e.from.clone())
            .collect();
        for r in refs {
            match r {
                NodeRef::Symbol(caller) => {
                    self.affected.symbol_edges.insert(caller);
                }
                other => self.mark_referencer(other),
            }
        }
    }

    /// Re-map citations into a symbol whose tokens did not change but whose layout did
    /// (moved, or comments/blank lines inserted). Citations of the whole symbol or of a
    /// fact site are re-derived from the new symbol. Other citations are shifted only when
    /// the symbol moved rigidly; otherwise the line mapping is unknown and the evidence is
    /// invalidated rather than left pointing at the wrong lines.
    fn remap_citations(
        &mut self,
        m: &mut Model,
        old: &Symbol,
        new: &Symbol,
        events: &mut Vec<Event>,
    ) {
        let node = NodeRef::Symbol(new.id.clone());
        let delta = new.range.start_line as i64 - old.range.start_line as i64;
        let rigid = (new.range.end_line as i64 - new.range.start_line as i64)
            == (old.range.end_line as i64 - old.range.start_line as i64);
        for id in m.evidence_depending_on(&node) {
            let Some(mut ev) = m.evidence.get(&id).cloned() else {
                continue;
            };
            let mut touched = false;
            let mut unmappable = false;
            for c in ev
                .citations
                .iter_mut()
                .filter(|c| c.node.as_ref() == Some(&node))
            {
                if c.range == old.range {
                    c.range = new.range.clone();
                } else if let Some(pos) = old.facts.iter().position(|f| f.range == c.range) {
                    let f = &old.facts[pos];
                    // nth occurrence of the same (kind, detail) in the old symbol -> same in the new one.
                    let nth = old.facts[..pos]
                        .iter()
                        .filter(|x| x.kind == f.kind && x.detail == f.detail)
                        .count();
                    match new
                        .facts
                        .iter()
                        .filter(|x| x.kind == f.kind && x.detail == f.detail)
                        .nth(nth)
                    {
                        Some(nf) => c.range = nf.range.clone(),
                        None => unmappable = true,
                    }
                } else if rigid {
                    c.range.start_line = (c.range.start_line as i64 + delta).max(1) as u32;
                    c.range.end_line = (c.range.end_line as i64 + delta).max(1) as u32;
                } else {
                    unmappable = true;
                }
                touched = true;
            }
            if !touched {
                continue;
            }
            if unmappable && ev.is_valid() {
                let reason = format!("cited lines inside `{}` moved non-uniformly; citation positions can no longer be trusted", new.qualified_name());
                ev.validity = concordance_core::evidence::Validity::Invalidated {
                    revision: self.ctx.revision,
                    reason: reason.clone(),
                    changed: vec![node.clone()],
                };
                events.push(Event::EvidenceInvalidated {
                    evidence: ev.id.clone(),
                    subject: ev.subject.clone(),
                    reason,
                });
                if let NodeRef::Claim(c) = &ev.subject {
                    if ev.method.kind() != MethodKind::Structural {
                        self.affected.semantic_invalidated.insert(c.clone());
                    }
                }
            }
            match &ev.subject {
                NodeRef::Claim(c) => {
                    self.affected.claims.insert(c.clone());
                }
                NodeRef::Document(d) => {
                    self.affected.docs.insert(d.clone());
                }
                _ => {}
            }
            self.dirty.evidence.insert(id.clone());
            m.upsert_evidence(ev);
        }
    }

    fn remove_symbol(&mut self, m: &mut Model, id: &SymbolId, events: &mut Vec<Event>) {
        let node = NodeRef::Symbol(id.clone());
        self.mark_symbol_referencers(m, id);
        // Call edges into the removed symbol are dropped; callers get re-linked.
        let incoming: Vec<EdgeId> = m
            .edges_to(&node)
            .filter(|e| matches!(e.from, NodeRef::Symbol(_)))
            .map(|e| e.id.clone())
            .collect();
        for e in incoming {
            m.remove_edge(&e);
            self.dirty.edges.insert(e);
        }
        self.remove_edges_from(m, &node);
        self.remove_evidence_of(m, &node, |_| false);
        let fids: Vec<FindingId> = m
            .findings_for_symbol(id)
            .filter(|f| matches!(f.subject, FindingSubject::Undocumented { .. }))
            .map(|f| f.id.clone())
            .collect();
        for f in fids {
            self.resolve_finding(m, &f, events);
        }
        if let Some(s) = m.remove_symbol(id) {
            if let Some(n) = s.path.last() {
                self.on_symbol_gone_or_renamed(m, n);
            }
        }
        self.dirty.symbols.insert(id.clone());
        self.affected.changed_nodes.push(node);
        self.affected.symbols.remove(id);
        self.affected.symbol_edges.remove(id);
        events.push(Event::SymbolRemoved { symbol: id.clone() });
    }

    pub fn apply_source(&mut self, m: &mut Model, idx: SourceIndex, events: &mut Vec<Event>) {
        let file_id = idx.file.id.clone();
        let old: BTreeMap<SymbolId, Symbol> = m
            .symbols_of(&file_id)
            .map(|s| (s.id.clone(), s.clone()))
            .collect();
        let new_ids: HashSet<SymbolId> = idx.symbols.iter().map(|s| s.id.clone()).collect();
        let mut structure_changed = false;
        for id in old.keys().filter(|id| !new_ids.contains(*id)) {
            self.remove_symbol(m, id, events);
            structure_changed = true;
        }
        for s in idx.symbols {
            match old.get(&s.id) {
                None => {
                    events.push(Event::SymbolCreated {
                        symbol: symbol_summary(&s),
                    });
                    if let Some(n) = s.path.last() {
                        self.on_symbol_gone_or_renamed(m, n);
                        self.affected.new_names.insert(n.clone());
                    }
                    structure_changed = true;
                }
                Some(p) if p.content_hash != s.content_hash => {
                    let signature_changed = p.signature_hash != s.signature_hash;
                    events.push(Event::SymbolUpdated {
                        symbol: symbol_summary(&s),
                        signature_changed,
                    });
                    self.affected
                        .changed_nodes
                        .push(NodeRef::Symbol(s.id.clone()));
                    self.mark_symbol_referencers(m, &s.id);
                    structure_changed = true;
                }
                Some(p) if p.range != s.range || p.facts != s.facts => {
                    let old = p.clone();
                    let id = s.id.clone();
                    m.upsert_symbol(s.clone());
                    self.dirty.symbols.insert(id.clone());
                    self.remap_citations(m, &old, &s, events);
                    self.affected.symbols.insert(id);
                    continue;
                }
                Some(p) if *p == s => continue,
                Some(_) => {}
            }
            self.affected.symbols.insert(s.id.clone());
            self.affected.symbol_edges.insert(s.id.clone());
            self.dirty.symbols.insert(s.id.clone());
            m.upsert_symbol(s);
        }
        if structure_changed {
            // Constraint claims (no-panic, no-unsafe, no-dependency) may cover anything.
            let constrained: Vec<ClaimId> = m
                .claims
                .values()
                .filter(|c| c.constraint.is_some())
                .map(|c| c.id.clone())
                .collect();
            self.affected.claims.extend(constrained);
        }
        let prev_imports = m.imports.get(&file_id).cloned().unwrap_or_default();
        if prev_imports != idx.imports {
            let constrained: Vec<ClaimId> = m
                .claims
                .values()
                .filter(|c| c.constraint.is_some())
                .map(|c| c.id.clone())
                .collect();
            self.affected.claims.extend(constrained);
        }
        m.set_imports(&file_id, idx.imports);
        self.dirty.import_files.insert(file_id.clone());
        let changed = m
            .files
            .get(&file_id)
            .map(|f| f.content_hash != idx.file.content_hash)
            .unwrap_or(true);
        if changed {
            self.affected
                .changed_nodes
                .push(NodeRef::File(file_id.clone()));
        }
        let path = idx.file.path.clone();
        let parse_error = idx.file.parse_error.clone();
        m.upsert_file(idx.file);
        self.dirty.files.insert(file_id.clone());
        let symbols = m.symbols_of(&file_id).count() as u32;
        events.push(Event::SourceIndexed {
            file: file_id,
            path,
            symbols,
            parse_error,
        });
    }

    pub fn remove_source(&mut self, m: &mut Model, path: &RepoPath, events: &mut Vec<Event>) {
        let Some(file) = m.file_by_path(path).cloned() else {
            return;
        };
        let ids: Vec<SymbolId> = m.symbols_of(&file.id).map(|s| s.id.clone()).collect();
        for id in ids {
            self.remove_symbol(m, &id, events);
        }
        let fnode = NodeRef::File(file.id.clone());
        let refs: Vec<NodeRef> = m.edges_to(&fnode).map(|e| e.from.clone()).collect();
        for r in refs {
            self.mark_referencer(r);
        }
        m.remove_file(&file.id);
        self.dirty.files.insert(file.id.clone());
        self.dirty.import_files.insert(file.id.clone());
        self.affected.changed_nodes.push(fnode);
        let constrained: Vec<ClaimId> = m
            .claims
            .values()
            .filter(|c| c.constraint.is_some())
            .map(|c| c.id.clone())
            .collect();
        self.affected.claims.extend(constrained);
        events.push(Event::SourceRemoved {
            file: file.id,
            path: path.clone(),
        });
    }

    // ---------------------------------------------------------------- reconcile

    /// Recompute everything affected by this batch. Returns obligation claims whose
    /// reconciliation changed (semantic scheduling candidates).
    pub fn reconcile(
        &mut self,
        m: &mut Model,
        fs: &RepoFs,
        events: &mut Vec<Event>,
    ) -> Vec<ClaimId> {
        self.relink_lexical_mentions(m);
        self.invalidate(m, events);
        self.relink_symbols(m);
        let claims: Vec<ClaimId> = std::mem::take(&mut self.affected.claims)
            .into_iter()
            .filter(|c| m.claims.contains_key(c))
            .collect();
        for c in &claims {
            self.reconcile_claim(m, c, fs, events);
        }
        let docs: Vec<DocumentId> = std::mem::take(&mut self.affected.docs)
            .into_iter()
            .collect();
        for d in &docs {
            self.reconcile_prose(m, d, fs, events);
        }
        let syms: Vec<SymbolId> = std::mem::take(&mut self.affected.symbols)
            .into_iter()
            .collect();
        for s in &syms {
            self.reconcile_gaps(m, s, events);
        }
        if std::mem::take(&mut self.affected.conflicts)
            || claims
                .iter()
                .any(|c| m.claims.get(c).is_some_and(|c| !c.bounds.is_empty()))
        {
            self.reconcile_conflicts(m, events);
        }
        claims
            .into_iter()
            .filter(|c| m.claims.get(c).is_some_and(|c| c.is_obligation()))
            .collect()
    }

    /// Obligation claims whose prose mentions a newly created symbol's name get re-linked.
    fn relink_lexical_mentions(&mut self, m: &Model) {
        let names = std::mem::take(&mut self.affected.new_names);
        if names.is_empty() {
            return;
        }
        let obligations = m.claims.values().filter(|c| c.is_obligation());
        if names.len() > 300 {
            let all: Vec<ClaimId> = obligations.map(|c| c.id.clone()).collect();
            self.affected.claims.extend(all);
            return;
        }
        let phrases: Vec<String> = names
            .iter()
            .map(|n| name_phrase(n))
            .filter(|p| p.len() >= 4)
            .collect();
        let hits: Vec<ClaimId> = obligations
            .filter(|c| {
                let t = c.text.to_lowercase();
                phrases.iter().any(|p| t.contains(p.as_str()))
            })
            .map(|c| c.id.clone())
            .collect();
        self.affected.claims.extend(hits);
    }

    /// Invalidate valid evidence whose dependencies changed.
    fn invalidate(&mut self, m: &mut Model, events: &mut Vec<Event>) {
        let changed = std::mem::take(&mut self.affected.changed_nodes);
        let mut seen: HashSet<EvidenceId> = HashSet::new();
        for node in changed {
            for id in m.evidence_depending_on(&node) {
                if !seen.insert(id.clone()) {
                    continue;
                }
                let Some(ev) = m.evidence.get(&id) else {
                    continue;
                };
                if !ev.is_valid() {
                    continue;
                }
                let stale: Vec<NodeRef> = ev
                    .dependencies
                    .iter()
                    .filter(|d| m.content_hash(&d.node).as_ref() != Some(&d.content_hash))
                    .map(|d| d.node.clone())
                    .collect();
                if stale.is_empty() {
                    continue;
                }
                let mut ev = ev.clone();
                let labels: Vec<String> = stale
                    .iter()
                    .map(|n| m.label(n).unwrap_or_else(|| format!("{n} (removed)")))
                    .take(3)
                    .collect();
                let reason = format!("dependency changed: {}", labels.join(", "));
                ev.validity = Validity::Invalidated {
                    revision: self.ctx.revision,
                    reason: reason.clone(),
                    changed: stale,
                };
                events.push(Event::EvidenceInvalidated {
                    evidence: ev.id.clone(),
                    subject: ev.subject.clone(),
                    reason,
                });
                match &ev.subject {
                    NodeRef::Claim(c) => {
                        self.affected.claims.insert(c.clone());
                        if ev.method.kind() != MethodKind::Structural {
                            self.affected.semantic_invalidated.insert(c.clone());
                        }
                    }
                    NodeRef::Document(d) => {
                        self.affected.docs.insert(d.clone());
                    }
                    NodeRef::Symbol(s) => {
                        self.affected.symbols.insert(s.clone());
                    }
                    _ => {}
                }
                self.dirty.evidence.insert(ev.id.clone());
                m.upsert_evidence(ev);
            }
        }
    }

    fn relink_symbols(&mut self, m: &mut Model) {
        let syms: Vec<SymbolId> = std::mem::take(&mut self.affected.symbol_edges)
            .into_iter()
            .collect();
        for id in syms {
            let Some(sym) = m.symbols.get(&id) else {
                continue;
            };
            let new = link::symbol_edges(m, sym);
            let node = NodeRef::Symbol(id.clone());
            let old: Vec<EdgeId> = m.edges_from(&node).map(|e| e.id.clone()).collect();
            let new_ids: HashSet<EdgeId> = new.iter().map(|e| e.id.clone()).collect();
            for o in old.iter().filter(|o| !new_ids.contains(*o)) {
                m.remove_edge(o);
                self.dirty.edges.insert(o.clone());
            }
            for e in new {
                if m.edges.get(&e.id) != Some(&e) {
                    self.dirty.edges.insert(e.id.clone());
                    m.upsert_edge(e);
                }
            }
        }
    }

    /// Replace the edges from `node` with `new`, emitting relationship events
    /// (lexical retrieval edges are not announced individually).
    fn replace_doc_edges(
        &mut self,
        m: &mut Model,
        node: &NodeRef,
        new: Vec<Edge>,
        events: &mut Vec<Event>,
    ) {
        let old: Vec<Edge> = m.edges_from(node).cloned().collect();
        let new_ids: HashSet<EdgeId> = new.iter().map(|e| e.id.clone()).collect();
        let old_ids: HashSet<EdgeId> = old.iter().map(|e| e.id.clone()).collect();
        for o in old.iter().filter(|o| !new_ids.contains(&o.id)) {
            m.remove_edge(&o.id);
            self.dirty.edges.insert(o.id.clone());
            if let Some(NodeRef::Symbol(s)) = &o.to {
                self.affected.symbols.insert(s.clone());
            }
            if o.to.is_some() && o.kind != EdgeKind::MentionsSymbol {
                let still_written = new.iter().any(|n| n.target_text == o.target_text);
                let reason = if still_written {
                    format!(
                        "`{}` no longer resolves to {}",
                        o.target_text,
                        o.to.as_ref()
                            .and_then(|t| m.label(t))
                            .unwrap_or_else(|| "its previous target".into())
                    )
                } else {
                    format!(
                        "reference `{}` was removed from the documentation",
                        o.target_text
                    )
                };
                events.push(Event::RelationshipInvalidated {
                    edge: o.id.clone(),
                    from: o.from.clone(),
                    reason,
                });
            }
        }
        for e in new {
            if let Some(NodeRef::Symbol(s)) = &e.to {
                if !old_ids.contains(&e.id) {
                    self.affected.symbols.insert(s.clone());
                }
            }
            if m.edges.get(&e.id) == Some(&e) {
                continue;
            }
            if !old_ids.contains(&e.id) && e.kind != EdgeKind::MentionsSymbol {
                events.push(Event::RelationshipCreated { edge: e.clone() });
            }
            self.dirty.edges.insert(e.id.clone());
            m.upsert_edge(e);
        }
    }

    /// Replace structural evidence of `subject` (optionally only of one check) with `new`.
    fn replace_structural_evidence(
        &mut self,
        m: &mut Model,
        subject: &NodeRef,
        new: Vec<Evidence>,
        only: Option<StructuralCheck>,
        events: &mut Vec<Event>,
    ) {
        let new_ids: HashSet<EvidenceId> = new.iter().map(|e| e.id.clone()).collect();
        let is_target = |e: &Evidence| match (&e.method, only) {
            (concordance_core::evidence::Method::Structural { check }, Some(o)) => *check == o,
            (concordance_core::evidence::Method::Structural { .. }, None) => true,
            // Test evidence is recomputed with the claim's structural evidence.
            (concordance_core::evidence::Method::Test { .. }, None) => true,
            _ => false,
        };
        let stale: Vec<EvidenceId> = m
            .evidence_for(subject)
            .filter(|e| is_target(e) && !new_ids.contains(&e.id))
            .map(|e| e.id.clone())
            .collect();
        for id in stale {
            m.remove_evidence(&id);
            self.dirty.evidence.insert(id);
        }
        for mut ev in new {
            if let Some(p) = m.evidence.get(&ev.id) {
                let same = p.is_valid()
                    && p.stance == ev.stance
                    && p.strength == ev.strength
                    && p.summary == ev.summary
                    && p.citations == ev.citations
                    && p.dependencies == ev.dependencies;
                if same {
                    continue;
                }
                if p.is_valid() && p.stance == ev.stance && p.summary == ev.summary {
                    ev.created_at = p.created_at;
                } else {
                    events.push(Event::EvidenceCreated {
                        evidence: EvidenceSummary::from(&ev),
                    });
                }
            } else {
                events.push(Event::EvidenceCreated {
                    evidence: EvidenceSummary::from(&ev),
                });
            }
            self.dirty.evidence.insert(ev.id.clone());
            m.upsert_evidence(ev);
        }
    }

    pub fn reconcile_claim(
        &mut self,
        m: &mut Model,
        id: &ClaimId,
        fs: &RepoFs,
        events: &mut Vec<Event>,
    ) {
        let Some(claim) = m.claims.get(id).cloned() else {
            return;
        };
        let Some(doc) = m.documents.get(&claim.document).cloned() else {
            return;
        };
        let node = NodeRef::Claim(id.clone());
        let edges = link::claim_edges(m, &claim, &doc.path, fs);
        self.replace_doc_edges(m, &node, edges, events);
        let edges: Vec<Edge> = m.edges_from(&node).cloned().collect();
        let ev = structural::claim_evidence(m, &claim, &edges, fs, &self.ctx);
        self.replace_structural_evidence(m, &node, ev, None, events);
        self.derive_claim(m, id, events);
    }

    /// Recompute only the finding of a claim from its current evidence (used after
    /// semantic results are applied).
    pub fn derive_claim(&mut self, m: &mut Model, id: &ClaimId, events: &mut Vec<Event>) {
        let Some(claim) = m.claims.get(id).cloned() else {
            return;
        };
        let Some(doc) = m.documents.get(&claim.document).cloned() else {
            return;
        };
        let node = NodeRef::Claim(id.clone());
        let fid = FindingSubject::Claim { claim: id.clone() }.finding_id();
        if claim.is_obligation() {
            let edges: Vec<Edge> = m.edges_from(&node).cloned().collect();
            let evs: Vec<&Evidence> = m.evidence_for(&node).collect();
            let f = derive::claim_finding(m, &claim, &doc, &evs, &edges, &self.ctx);
            self.put_finding(m, f, events);
        } else if m.findings.get(&fid).is_some_and(|f| f.is_open()) {
            self.resolve_finding(m, &fid, events);
        }
    }

    fn reconcile_prose(
        &mut self,
        m: &mut Model,
        doc_id: &DocumentId,
        fs: &RepoFs,
        events: &mut Vec<Event>,
    ) {
        let node = NodeRef::Document(doc_id.clone());
        let old_open: Vec<FindingId> = m
            .findings
            .values()
            .filter(|f| f.is_open() && matches!(&f.subject, FindingSubject::ProseReference { document, .. } if document == doc_id))
            .map(|f| f.id.clone())
            .collect();
        let Some(doc) = m.documents.get(doc_id).cloned() else {
            for f in old_open {
                self.resolve_finding(m, &f, events);
            }
            return;
        };
        let refs = m.prose_refs.get(doc_id).cloned().unwrap_or_default();
        let edges = link::prose_edges(m, doc_id, &doc.path, &refs, fs);
        self.replace_doc_edges(m, &node, edges, events);
        let edges: Vec<Edge> = m.edges_from(&node).cloned().collect();
        let found = structural::prose_evidence(m, &doc, &refs, &edges, &self.ctx);
        self.replace_structural_evidence(
            m,
            &node,
            found.iter().map(|(_, e)| e.clone()).collect(),
            None,
            events,
        );
        let mut keep: HashSet<FindingId> = HashSet::new();
        for (subject, ev) in found {
            let f = derive::prose_finding(m, &doc, subject, &ev, &self.ctx);
            keep.insert(f.id.clone());
            self.put_finding(m, f, events);
        }
        for f in old_open.into_iter().filter(|f| !keep.contains(f)) {
            self.resolve_finding(m, &f, events);
        }
    }

    fn reconcile_gaps(&mut self, m: &mut Model, sym_id: &SymbolId, events: &mut Vec<Event>) {
        let old_open: Vec<FindingId> = m
            .findings_for_symbol(sym_id)
            .filter(|f| {
                f.is_open()
                    && matches!(
                        f.subject,
                        FindingSubject::Undocumented { .. } | FindingSubject::Verification { .. }
                    )
            })
            .map(|f| f.id.clone())
            .collect();
        let Some(sym) = m.symbols.get(sym_id).cloned() else {
            for f in old_open {
                self.resolve_finding(m, &f, events);
            }
            return;
        };
        let found = gaps::undocumented_findings(m, &sym, &self.ctx);
        let unknown =
            concordance_reconcile::verification::verification_findings(m, &sym, &self.ctx);
        let node = NodeRef::Symbol(sym_id.clone());
        self.replace_structural_evidence(
            m,
            &node,
            found.iter().map(|(_, e)| e.clone()).collect(),
            Some(StructuralCheck::PublicApiCoverage),
            events,
        );
        self.replace_structural_evidence(
            m,
            &node,
            unknown.iter().map(|(_, e)| e.clone()).collect(),
            Some(StructuralCheck::RequirementResolution),
            events,
        );
        let found: Vec<_> = found.into_iter().chain(unknown).collect();
        let mut keep: HashSet<FindingId> = HashSet::new();
        for (f, _) in found {
            keep.insert(f.id.clone());
            self.put_finding(m, f, events);
        }
        for f in old_open.into_iter().filter(|f| !keep.contains(f)) {
            self.resolve_finding(m, &f, events);
        }
    }

    fn reconcile_conflicts(&mut self, m: &mut Model, events: &mut Vec<Event>) {
        let found = derive::conflict_findings(m, &self.ctx);
        let old_ev: Vec<EvidenceId> = m
            .evidence
            .values()
            .filter(|e| {
                matches!(
                    e.method,
                    concordance_core::evidence::Method::Structural {
                        check: StructuralCheck::ConflictingClaims
                    }
                )
            })
            .map(|e| e.id.clone())
            .collect();
        let new_ev: HashSet<EvidenceId> = found.iter().map(|(_, e)| e.id.clone()).collect();
        for id in old_ev.into_iter().filter(|i| !new_ev.contains(i)) {
            m.remove_evidence(&id);
            self.dirty.evidence.insert(id);
        }
        let old_open: Vec<FindingId> = m
            .findings
            .values()
            .filter(|f| f.is_open() && matches!(f.subject, FindingSubject::ClaimConflict { .. }))
            .map(|f| f.id.clone())
            .collect();
        let mut keep = HashSet::new();
        for (f, ev) in found {
            if m.evidence
                .get(&ev.id)
                .map(|p| p.summary != ev.summary || !p.is_valid())
                .unwrap_or(true)
            {
                events.push(Event::EvidenceCreated {
                    evidence: EvidenceSummary::from(&ev),
                });
                self.dirty.evidence.insert(ev.id.clone());
                m.upsert_evidence(ev);
            }
            keep.insert(f.id.clone());
            self.put_finding(m, f, events);
        }
        for f in old_open.into_iter().filter(|f| !keep.contains(f)) {
            self.resolve_finding(m, &f, events);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edited_claim_keeps_identity_when_unambiguous() {
        let mk = |id: &str, text: &str| -> Claim {
            serde_json::from_value(serde_json::json!({
                "id": ClaimId::from_key(&[id]).as_str(),
                "document": DocumentId::from_key(&["d"]).as_str(),
                "section": concordance_core::SectionId::from_key(&["s"]).as_str(),
                "heading_path": [], "text": text, "kind": "requirement", "modality": "must",
                "authority": "normative", "block": "paragraph",
                "range": {"path": "d.md", "start_line": 1, "start_col": 1, "end_line": 1, "end_col": 1},
                "content_hash": concordance_core::ContentHash::of_str(text).as_str(),
                "references": [], "bounds": [], "ambiguity": [],
                "documented_signature": null, "constraint": null
            }))
            .unwrap()
        };
        let old = vec![
            mk(
                "a",
                "The limiter must reject requests when no tokens remain",
            ),
            mk("b", "Volumes must be persisted to disk"),
        ];
        let mut new = vec![
            mk(
                "a2",
                "The limiter must reject new requests when no tokens remain",
            ),
            mk("c", "Something entirely different must happen"),
        ];
        carry_claim_ids(&old, &mut new);
        assert_eq!(new[0].id, old[0].id, "lightly edited claim keeps its id");
        assert_ne!(new[1].id, old[1].id, "unrelated claim does not steal an id");
    }
}
