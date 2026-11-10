//! Mutation primitives. Each keeps the derived indexes exact; the engine composes
//! them into change batches and decides which events to emit.

use crate::Model;
use concordance_core::code::{Import, SourceFile, Symbol};
use concordance_core::doc::{Claim, Document, ProseReference, Reference, ReferenceTarget, Section};
use concordance_core::evidence::Evidence;
use concordance_core::finding::{Finding, FindingSubject};
use concordance_core::graph::{Edge, NodeRef};
use concordance_core::{
    ClaimId, DocumentId, EdgeId, EvidenceId, FileId, FindingId, RepoPath, SymbolId,
};
use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;

fn add<K: Eq + Hash, V: Ord>(map: &mut HashMap<K, BTreeSet<V>>, k: K, v: V) {
    map.entry(k).or_default().insert(v);
}

fn del<K: Eq + Hash, V: Ord>(map: &mut HashMap<K, BTreeSet<V>>, k: &K, v: &V) {
    if let Some(set) = map.get_mut(k) {
        set.remove(v);
        if set.is_empty() {
            map.remove(k);
        }
    }
}

/// Index keys a reference contributes: symbol last segment, or a linked document path.
enum RefKey {
    Name(String),
    Doc(RepoPath),
}

fn ref_keys(r: &Reference) -> Option<RefKey> {
    match &r.target {
        ReferenceTarget::Symbol { path } => path.last().map(|n| RefKey::Name(n.clone())),
        ReferenceTarget::Section { document, .. } => Some(RefKey::Doc(document.clone())),
        _ => None,
    }
}

impl Model {
    fn index_refs<'a>(
        &mut self,
        writer: &NodeRef,
        refs: impl Iterator<Item = &'a Reference>,
        insert: bool,
    ) {
        for r in refs {
            match ref_keys(r) {
                Some(RefKey::Name(n)) => {
                    if insert {
                        add(&mut self.idx.refs_by_name, n, writer.clone())
                    } else {
                        del(&mut self.idx.refs_by_name, &n, writer)
                    }
                }
                Some(RefKey::Doc(p)) => {
                    if insert {
                        add(&mut self.idx.refs_by_doc_target, p, writer.clone())
                    } else {
                        del(&mut self.idx.refs_by_doc_target, &p, writer)
                    }
                }
                None => {}
            }
        }
    }

    // ---- documents -------------------------------------------------------------

    pub fn upsert_document(&mut self, doc: Document) {
        self.idx
            .doc_by_path
            .insert(doc.path.clone(), doc.id.clone());
        self.documents.insert(doc.id.clone(), doc);
    }

    /// Removes the document, its sections and prose references. Claims must be removed
    /// first with [`Model::remove_claim`] (so the caller can emit events for them).
    pub fn remove_document(&mut self, id: &DocumentId) -> Option<Document> {
        let doc = self.documents.remove(id)?;
        self.idx.doc_by_path.remove(&doc.path);
        self.replace_sections(id, Vec::new());
        self.set_prose_refs(id, Vec::new());
        self.idx.claims_by_doc.remove(id);
        Some(doc)
    }

    pub fn replace_sections(&mut self, doc: &DocumentId, sections: Vec<Section>) {
        if let Some(old) = self.idx.sections_by_doc.remove(doc) {
            for s in old {
                self.sections.remove(&s);
            }
        }
        for s in sections {
            add(&mut self.idx.sections_by_doc, doc.clone(), s.id.clone());
            self.sections.insert(s.id.clone(), s);
        }
    }

    pub fn set_prose_refs(&mut self, doc: &DocumentId, refs: Vec<ProseReference>) {
        let writer = NodeRef::Document(doc.clone());
        if let Some(old) = self.prose_refs.remove(doc) {
            self.index_refs(&writer, old.iter().map(|p| &p.reference), false);
        }
        if !refs.is_empty() {
            self.index_refs(&writer, refs.iter().map(|p| &p.reference), true);
            self.prose_refs.insert(doc.clone(), refs);
        }
    }

    pub fn upsert_claim(&mut self, claim: Claim) -> Option<Claim> {
        let writer = NodeRef::Claim(claim.id.clone());
        let old = self.claims.remove(&claim.id);
        if let Some(o) = &old {
            self.index_refs(&writer, o.references.iter(), false);
            del(&mut self.idx.claims_by_doc, &o.document, &o.id);
            if let Some(r) = &o.requirement {
                del(&mut self.idx.claims_by_requirement, &r.id, &o.id);
            }
        }
        if let Some(r) = &claim.requirement {
            add(
                &mut self.idx.claims_by_requirement,
                r.id.clone(),
                claim.id.clone(),
            );
        }
        self.index_refs(&writer, claim.references.iter(), true);
        add(
            &mut self.idx.claims_by_doc,
            claim.document.clone(),
            claim.id.clone(),
        );
        self.claims.insert(claim.id.clone(), claim);
        old
    }

    pub fn remove_claim(&mut self, id: &ClaimId) -> Option<Claim> {
        let claim = self.claims.remove(id)?;
        let writer = NodeRef::Claim(id.clone());
        self.index_refs(&writer, claim.references.iter(), false);
        del(&mut self.idx.claims_by_doc, &claim.document, id);
        if let Some(r) = &claim.requirement {
            del(&mut self.idx.claims_by_requirement, &r.id, id);
        }
        Some(claim)
    }

    // ---- source ----------------------------------------------------------------

    pub fn upsert_file(&mut self, file: SourceFile) {
        self.idx
            .file_by_path
            .insert(file.path.clone(), file.id.clone());
        self.files.insert(file.id.clone(), file);
    }

    /// Removes the file record and its imports. Symbols must be removed first.
    pub fn remove_file(&mut self, id: &FileId) -> Option<SourceFile> {
        let f = self.files.remove(id)?;
        self.idx.file_by_path.remove(&f.path);
        self.imports.remove(id);
        self.idx.symbols_by_file.remove(id);
        Some(f)
    }

    pub fn set_imports(&mut self, file: &FileId, imports: Vec<Import>) {
        if imports.is_empty() {
            self.imports.remove(file);
        } else {
            self.imports.insert(file.clone(), imports);
        }
    }

    pub fn upsert_symbol(&mut self, sym: Symbol) -> Option<Symbol> {
        let old = self.symbols.remove(&sym.id);
        if let Some(o) = &old {
            self.unindex_symbol(o);
        }
        add(
            &mut self.idx.symbols_by_file,
            sym.file.clone(),
            sym.id.clone(),
        );
        if let Some(name) = sym.path.last() {
            add(&mut self.idx.symbols_by_name, name.clone(), sym.id.clone());
        }
        if let Some(p) = &sym.parent {
            add(&mut self.idx.children, p.clone(), sym.id.clone());
        }
        for r in &sym.verifies {
            add(
                &mut self.idx.symbols_by_requirement,
                r.clone(),
                sym.id.clone(),
            );
        }
        self.symbols.insert(sym.id.clone(), sym);
        old
    }

    pub fn remove_symbol(&mut self, id: &SymbolId) -> Option<Symbol> {
        let sym = self.symbols.remove(id)?;
        self.unindex_symbol(&sym);
        Some(sym)
    }

    fn unindex_symbol(&mut self, s: &Symbol) {
        del(&mut self.idx.symbols_by_file, &s.file, &s.id);
        if let Some(name) = s.path.last() {
            del(&mut self.idx.symbols_by_name, name, &s.id);
        }
        if let Some(p) = &s.parent {
            del(&mut self.idx.children, p, &s.id);
        }
        for r in &s.verifies {
            del(&mut self.idx.symbols_by_requirement, r, &s.id);
        }
    }

    // ---- edges -------------------------------------------------------------------

    pub fn upsert_edge(&mut self, edge: Edge) {
        if let Some(old) = self.edges.remove(&edge.id) {
            self.unindex_edge(&old);
        }
        add(&mut self.idx.edges_from, edge.from.clone(), edge.id.clone());
        if let Some(to) = &edge.to {
            add(&mut self.idx.edges_to, to.clone(), edge.id.clone());
        }
        self.edges.insert(edge.id.clone(), edge);
    }

    pub fn remove_edge(&mut self, id: &EdgeId) -> Option<Edge> {
        let e = self.edges.remove(id)?;
        self.unindex_edge(&e);
        Some(e)
    }

    fn unindex_edge(&mut self, e: &Edge) {
        del(&mut self.idx.edges_from, &e.from, &e.id);
        if let Some(to) = &e.to {
            del(&mut self.idx.edges_to, to, &e.id);
        }
    }

    // ---- evidence ----------------------------------------------------------------

    pub fn upsert_evidence(&mut self, ev: Evidence) -> Option<Evidence> {
        let old = self.evidence.remove(&ev.id);
        if let Some(o) = &old {
            self.unindex_evidence(o);
        }
        add(
            &mut self.idx.evidence_by_subject,
            ev.subject.clone(),
            ev.id.clone(),
        );
        for d in &ev.dependencies {
            add(&mut self.idx.evidence_by_dep, d.node.clone(), ev.id.clone());
        }
        self.evidence.insert(ev.id.clone(), ev);
        old
    }

    pub fn remove_evidence(&mut self, id: &EvidenceId) -> Option<Evidence> {
        let ev = self.evidence.remove(id)?;
        self.unindex_evidence(&ev);
        Some(ev)
    }

    fn unindex_evidence(&mut self, ev: &Evidence) {
        del(&mut self.idx.evidence_by_subject, &ev.subject, &ev.id);
        for d in &ev.dependencies {
            del(&mut self.idx.evidence_by_dep, &d.node, &ev.id);
        }
    }

    // ---- findings ----------------------------------------------------------------

    pub fn upsert_finding(&mut self, f: Finding) -> Option<Finding> {
        let old = self.findings.remove(&f.id);
        if let Some(o) = &old {
            self.unindex_finding(o);
        }
        for c in &f.claims {
            add(&mut self.idx.findings_by_claim, c.clone(), f.id.clone());
        }
        for s in &f.symbols {
            add(&mut self.idx.findings_by_symbol, s.clone(), f.id.clone());
        }
        if let FindingSubject::Undocumented { symbol, .. } = &f.subject {
            add(
                &mut self.idx.findings_by_symbol,
                symbol.clone(),
                f.id.clone(),
            );
        }
        self.findings.insert(f.id.clone(), f);
        old
    }

    fn unindex_finding(&mut self, f: &Finding) {
        for c in &f.claims {
            del(&mut self.idx.findings_by_claim, c, &f.id);
        }
        for s in &f.symbols {
            del(&mut self.idx.findings_by_symbol, s, &f.id);
        }
        if let FindingSubject::Undocumented { symbol, .. } = &f.subject {
            del(&mut self.idx.findings_by_symbol, symbol, &f.id);
        }
    }

    pub fn finding_ids(&self) -> impl Iterator<Item = &FindingId> {
        self.findings.keys()
    }
}
