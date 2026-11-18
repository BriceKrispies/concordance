//! Reference linking: documentation references and code relationships -> typed edges.

use crate::FsProbe;
use concordance_core::code::{CallTarget, Import, Symbol, SymbolKind};
use concordance_core::doc::{
    Claim, ConstraintForm, ProseReference, Reference, ReferenceStrength, ReferenceTarget,
};
use concordance_core::graph::{Edge, EdgeKind, NodeRef, Resolution};
use concordance_core::{DocumentId, FileId, RepoPath, SourceRange, SymbolId};
use concordance_graph::{lexical_candidates, Model, Resolved};
use std::collections::BTreeSet;

/// Max lexical retrieval candidates linked per claim.
pub const LEXICAL_LIMIT: usize = 5;

fn edge(
    kind: EdgeKind,
    from: &NodeRef,
    text: &str,
    to: Option<NodeRef>,
    resolution: Resolution,
    confidence: f32,
    cfg: Vec<String>,
    origin: Option<SourceRange>,
) -> Edge {
    Edge {
        id: Edge::make_id(kind, from, text, to.as_ref()),
        kind,
        from: from.clone(),
        to,
        target_text: text.to_string(),
        resolution,
        confidence,
        cfg,
        origin,
    }
}

/// The crate a document "belongs" to, if it lives inside a crate directory.
pub fn context_crate(model: &Model, path: &RepoPath) -> Option<String> {
    model
        .workspace
        .crate_for(path)
        .filter(|c| c.root_dir.is_some())
        .map(|c| c.name.clone())
}

fn crate_root_file(model: &Model, name: &str) -> Option<FileId> {
    let krate = model.workspace.crates.iter().find(|c| c.name == name)?;
    krate
        .targets
        .iter()
        .find_map(|t| model.file_by_path(t).map(|f| f.id.clone()))
}

/// Edge for one written reference; `None` for references that are not checkable
/// (external URLs, std paths, dependencies).
pub fn reference_edge(
    model: &Model,
    from: &NodeRef,
    r: &Reference,
    ctx_crate: Option<&str>,
    fs: &dyn FsProbe,
) -> Option<Edge> {
    let origin = r.range.clone();
    match &r.target {
        ReferenceTarget::Symbol { path } => {
            let (to, res, conf) = match model.resolve_path(path, ctx_crate) {
                Resolved::Exact(id) => (Some(id), Resolution::Exact, 1.0),
                Resolved::UniqueSuffix(id) => (Some(id), Resolution::UniqueSuffix, 0.9),
                Resolved::Ambiguous(ids) => {
                    let n = ids.len() as u32;
                    (
                        ids.into_iter().next(),
                        Resolution::Ambiguous { candidates: n },
                        1.0 / n as f32,
                    )
                }
                Resolved::Crate(name) => {
                    let file = crate_root_file(model, &name)?;
                    return Some(edge(
                        EdgeKind::ReferencesFile,
                        from,
                        &r.text,
                        Some(NodeRef::File(file)),
                        Resolution::Exact,
                        1.0,
                        vec![],
                        origin,
                    ));
                }
                Resolved::External(_) => return None,
                Resolved::Unresolved => (None, Resolution::Unresolved, 0.0),
            };
            let cfg = to
                .as_ref()
                .and_then(|id| model.symbols.get(id))
                .map(|s| s.cfg.clone())
                .unwrap_or_default();
            Some(edge(
                EdgeKind::ReferencesSymbol,
                from,
                &r.text,
                to.map(NodeRef::Symbol),
                res,
                conf,
                cfg,
                origin,
            ))
        }
        ReferenceTarget::File { path, .. } => {
            let to = model
                .file_by_path(path)
                .map(|f| NodeRef::File(f.id.clone()))
                .or_else(|| {
                    model
                        .document_by_path(path)
                        .map(|d| NodeRef::Document(d.id.clone()))
                });
            let (to, res, conf) = if to.is_some() || fs.exists(path) {
                (to, Resolution::Exact, 1.0)
            } else {
                // A bare or partial path (`hir_pass.rs`, `src/lib.rs`) written in prose
                // usually names a file somewhere under the source roots: match it as a
                // path suffix over indexed files and documents.
                let written = r.text.trim_matches('`');
                let written = written
                    .split(':')
                    .next()
                    .unwrap_or(written)
                    .trim_start_matches("./");
                let hits = suffix_matches(model, written);
                match hits.len() {
                    0 => (None, Resolution::Unresolved, 0.0),
                    1 => (hits.into_iter().next(), Resolution::UniqueSuffix, 0.9),
                    n => (
                        hits.into_iter().next(),
                        Resolution::Ambiguous {
                            candidates: n as u32,
                        },
                        1.0 / n as f32,
                    ),
                }
            };
            Some(edge(
                EdgeKind::ReferencesFile,
                from,
                &r.text,
                to,
                res,
                conf,
                vec![],
                origin,
            ))
        }
        ReferenceTarget::Section { document, anchor } => {
            let doc = model.document_by_path(document);
            // Exists on disk but outside the spec roots, with an anchor: the anchor cannot
            // be checked against an unindexed document, so the link is not checkable.
            if doc.is_none() && anchor.is_some() && fs.exists(document) {
                return None;
            }
            let to = match (doc, anchor) {
                (Some(d), None) => Some(NodeRef::Document(d.id.clone())),
                (Some(d), Some(a)) => model
                    .sections_of(&d.id)
                    .find(|s| &s.anchor == a)
                    .map(|s| NodeRef::Section(s.id.clone())),
                (None, _) => None,
            };
            let res = match (&to, doc) {
                (Some(_), _) => Resolution::Exact,
                // Exists on disk but not indexed: cannot check anchors; treat as resolved-by-path.
                (None, None) if fs.exists(document) && anchor.is_none() => Resolution::Exact,
                _ => Resolution::Unresolved,
            };
            let conf = if res == Resolution::Exact { 1.0 } else { 0.0 };
            Some(edge(
                EdgeKind::ReferencesSection,
                from,
                &r.text,
                to,
                res,
                conf,
                vec![],
                origin,
            ))
        }
        ReferenceTarget::External { .. } | ReferenceTarget::OutsideRepo { .. } => None,
    }
}

/// All edges originating at a claim: explicit references, constraint scopes, and
/// lexical retrieval candidates (obligation claims only).
pub fn claim_edges(
    model: &Model,
    claim: &Claim,
    doc_path: &RepoPath,
    fs: &dyn FsProbe,
) -> Vec<Edge> {
    let from = NodeRef::Claim(claim.id.clone());
    let ctx = context_crate(model, doc_path);
    let mut out: Vec<Edge> = Vec::new();
    for r in &claim.references {
        if let Some(e) = reference_edge(model, &from, r, ctx.as_deref(), fs) {
            out.push(e);
        }
    }
    if let Some(ConstraintForm::NoPanic { scope } | ConstraintForm::NoUnsafe { scope }) =
        &claim.constraint
    {
        if !scope.is_empty() {
            let (target, res, conf) = match model.resolve_path(scope, ctx.as_deref()) {
                Resolved::Crate(c) => {
                    let t = crate_root_file(model, &c).map(NodeRef::File);
                    let r = if t.is_some() {
                        Resolution::Exact
                    } else {
                        Resolution::Unresolved
                    };
                    let conf = if t.is_some() { 1.0 } else { 0.0 };
                    (t, r, conf)
                }
                Resolved::Exact(id) => (Some(NodeRef::Symbol(id)), Resolution::Exact, 1.0),
                Resolved::UniqueSuffix(id) => {
                    (Some(NodeRef::Symbol(id)), Resolution::UniqueSuffix, 0.9)
                }
                Resolved::Ambiguous(ids) => {
                    let n = ids.len() as u32;
                    (
                        ids.into_iter().next().map(NodeRef::Symbol),
                        Resolution::Ambiguous { candidates: n },
                        1.0 / n as f32,
                    )
                }
                _ => (None, Resolution::Unresolved, 0.0),
            };
            let text = scope.join("::");
            out.push(edge(
                EdgeKind::ConstrainsScope,
                &from,
                &text,
                target,
                res,
                conf,
                vec![],
                Some(claim.range.clone()),
            ));
        }
    }
    if let Some(req) = &claim.requirement {
        for t in model.verifiers_of(&req.id).filter(|s| s.is_test) {
            out.push(edge(
                EdgeKind::VerifiedBy,
                &from,
                &req.id,
                Some(NodeRef::Symbol(t.id.clone())),
                Resolution::Exact,
                1.0,
                t.cfg.clone(),
                Some(claim.range.clone()),
            ));
        }
    }
    if claim.is_obligation() {
        let explicit: BTreeSet<NodeRef> = out.iter().filter_map(|e| e.to.clone()).collect();
        for (id, score, name) in lexical_candidates(model, &claim.text, LEXICAL_LIMIT) {
            let to = NodeRef::Symbol(id.clone());
            if explicit.contains(&to) {
                continue;
            }
            let cfg = model
                .symbols
                .get(&id)
                .map(|s| s.cfg.clone())
                .unwrap_or_default();
            out.push(edge(
                EdgeKind::MentionsSymbol,
                &from,
                &name,
                Some(to),
                Resolution::Lexical,
                score,
                cfg,
                None,
            ));
        }
    }
    dedup(out)
}

/// Edges for references written in non-claim prose. Weak symbol references are not
/// linked from prose (too noisy to be meaningful).
pub fn prose_edges(
    model: &Model,
    doc: &DocumentId,
    doc_path: &RepoPath,
    refs: &[ProseReference],
    fs: &dyn FsProbe,
) -> Vec<Edge> {
    let from = NodeRef::Document(doc.clone());
    let ctx = context_crate(model, doc_path);
    let edges = refs
        .iter()
        .filter(|p| {
            !(matches!(p.reference.target, ReferenceTarget::Symbol { .. })
                && p.reference.strength == ReferenceStrength::Weak)
        })
        .filter_map(|p| reference_edge(model, &from, &p.reference, ctx.as_deref(), fs))
        .collect();
    dedup(edges)
}

/// Syntactic code relationships of one symbol: calls (name-resolved) and trait impls.
pub fn symbol_edges(model: &Model, sym: &Symbol) -> Vec<Edge> {
    let from = NodeRef::Symbol(sym.id.clone());
    let krate = sym.path.first().map(String::as_str);
    let imports: &[Import] = model
        .imports
        .get(&sym.file)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut out = Vec::new();
    for call in &sym.calls {
        let text = match &call.target {
            CallTarget::Path { segments } => segments.join("::"),
            CallTarget::Method { name } => format!(".{name}()"),
        };
        let Some((to, res, conf)) = resolve_call(model, imports, &call.target, krate) else {
            continue;
        };
        if to == sym.id {
            continue;
        }
        let cfg = model
            .symbols
            .get(&to)
            .map(|s| s.cfg.clone())
            .unwrap_or_default();
        out.push(edge(
            EdgeKind::Calls,
            &from,
            &text,
            Some(NodeRef::Symbol(to)),
            res,
            conf,
            cfg,
            Some(call.range.clone()),
        ));
    }
    if sym.kind == SymbolKind::Impl {
        if let Some(tp) = sym.impl_info.as_ref().and_then(|i| i.trait_path.as_ref()) {
            let segs: Vec<String> = tp
                .split("::")
                .map(|s| s.split('<').next().unwrap_or(s).trim().to_string())
                .collect();
            if let Some(id) = model.resolve_path(&segs, krate).best().cloned() {
                if model
                    .symbols
                    .get(&id)
                    .is_some_and(|t| t.kind == SymbolKind::Trait)
                {
                    out.push(edge(
                        EdgeKind::Implements,
                        &from,
                        tp,
                        Some(NodeRef::Symbol(id)),
                        Resolution::UniqueSuffix,
                        0.9,
                        vec![],
                        Some(sym.range.clone()),
                    ));
                }
            }
        }
    }
    dedup(out)
}

fn is_callable_symbol(model: &Model, id: &SymbolId) -> bool {
    model.symbols.get(id).is_some_and(|s| s.kind.is_callable())
}

/// Expands a call's first segment through the file's `use` items: with `use a::b::c;` the
/// call `c(..)` means `a::b::c`, and `use a::b as d;` makes `d::x()` mean `a::b::x`. Glob
/// imports name no segment, so they are not expanded.
fn expand_import(imports: &[Import], segments: &[String]) -> Option<Vec<String>> {
    let first = segments.first()?;
    let imp = imports.iter().find(|i| {
        let bound = match (&i.alias, i.path.last()) {
            (Some(alias), _) => alias.as_str(),
            (None, Some(last)) => last.as_str(),
            (None, None) => return false,
        };
        !i.glob && bound == first
    })?;
    let mut out = imp.path.clone();
    out.extend(segments[1..].iter().cloned());
    Some(out)
}

/// Resolves one call site to a callable symbol and labels how sure the link is. Only
/// callables are candidates: a module or field that shares the name is never a call target.
fn resolve_call(
    model: &Model,
    imports: &[Import],
    target: &CallTarget,
    krate: Option<&str>,
) -> Option<(SymbolId, Resolution, f32)> {
    match target {
        // The receiver's type is not known, so the method name is all there is to match. The
        // edge is labelled `NameOnly`: it does not say which method was called.
        CallTarget::Method { name } => {
            let mut ids: Vec<SymbolId> = match model.resolve_path(std::slice::from_ref(name), krate)
            {
                Resolved::Exact(id) | Resolved::UniqueSuffix(id) => vec![id],
                Resolved::Ambiguous(ids) => ids,
                _ => return None,
            };
            ids.retain(|id| is_callable_symbol(model, id));
            // Method calls with many candidates are noise; keep small ambiguity sets only.
            let n = ids.len();
            if n == 0 || n > 3 {
                return None;
            }
            Some((ids.swap_remove(0), Resolution::NameOnly, 0.5 / n as f32))
        }
        CallTarget::Path { segments } => {
            let expanded = expand_import(imports, segments);
            let path = expanded.as_deref().unwrap_or(segments.as_slice());
            let single = |id: SymbolId, res: Resolution, conf: f32| {
                is_callable_symbol(model, &id).then_some((id, res, conf))
            };
            match model.resolve_path(path, krate) {
                Resolved::Exact(id) => single(id, Resolution::Exact, 1.0),
                Resolved::UniqueSuffix(id) => single(id, Resolution::UniqueSuffix, 0.8),
                Resolved::Ambiguous(mut ids) => {
                    ids.retain(|id| is_callable_symbol(model, id));
                    match ids.len() {
                        0 => None,
                        1 => Some((ids.swap_remove(0), Resolution::UniqueSuffix, 0.8)),
                        n if n <= 3 => {
                            let n = n as u32;
                            Some((
                                ids.swap_remove(0),
                                Resolution::Ambiguous { candidates: n },
                                1.0 / n as f32,
                            ))
                        }
                        _ => None,
                    }
                }
                _ => None,
            }
        }
    }
}

fn dedup(mut edges: Vec<Edge>) -> Vec<Edge> {
    let mut seen = BTreeSet::new();
    edges.retain(|e| seen.insert(e.id.clone()));
    edges
}

/// Indexed files and documents whose repository path ends with `/<written>`.
fn suffix_matches(model: &Model, written: &str) -> Vec<NodeRef> {
    if written.is_empty() || written.contains("..") {
        return Vec::new();
    }
    let tail = format!("/{}", written.replace('\\', "/"));
    let ends = |p: &str| p.ends_with(&tail) || p.eq_ignore_ascii_case(written);
    let mut out: Vec<NodeRef> = model
        .files
        .values()
        .filter(|f| ends(f.path.as_str()))
        .map(|f| NodeRef::File(f.id.clone()))
        .collect();
    out.extend(
        model
            .documents
            .values()
            .filter(|d| ends(d.path.as_str()))
            .map(|d| NodeRef::Document(d.id.clone())),
    );
    out
}
