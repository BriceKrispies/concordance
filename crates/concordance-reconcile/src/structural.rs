//! Layer A: deterministic structural checks. Every result is an [`Evidence`] record
//! with exact citations and the content hashes it depends on.
//!
//! Strength discipline:
//! * `Proven`: the fact is certain from the indexed state (symbol absent, signatures differ,
//!   file missing, heading missing, declared Cargo dependency).
//! * `Strong`: a syntactic fact that directly bears on the claim (an `unwrap()` inside a
//!   no-panic scope, an import that crosses a forbidden boundary, a constant beyond a bound).
//! * `Moderate`: suggestive syntactic facts with known blind spots (a clamp to a larger
//!   value, absence of direct panic sites).

use crate::text::{bounded, similar_name, spec_citation, EvidenceBuilder};
use crate::{BatchContext, FsProbe};
use concordance_core::code::{FactKind, Symbol, SymbolKind};
use concordance_core::doc::{
    BoundOp, Claim, ConstraintForm, Document, ProseReference, Reference, ReferenceStrength,
    ReferenceTarget,
};
use concordance_core::evidence::{
    Citation, CitationRole, Dependency, Evidence, Method, Stance, Strength, StructuralCheck,
};
use concordance_core::finding::FindingSubject;
use concordance_core::graph::{Edge, EdgeKind, NodeRef, Resolution};
use concordance_core::SymbolId;
use concordance_graph::{Model, Resolved};

const MAX_SITE_CITATIONS: usize = 10;

fn dep(model: &Model, node: NodeRef) -> Option<Dependency> {
    let content_hash = model.content_hash(&node)?;
    Some(Dependency { node, content_hash })
}

fn claim_dep(claim: &Claim) -> Dependency {
    Dependency {
        node: NodeRef::Claim(claim.id.clone()),
        content_hash: claim.content_hash.clone(),
    }
}

pub fn symbol_citation(sym: &Symbol, excerpt: Option<&str>) -> Citation {
    Citation {
        role: CitationRole::Implementation,
        range: sym.range.clone(),
        excerpt: bounded(
            excerpt
                .or(sym.signature.as_deref())
                .unwrap_or(&sym.qualified_name()),
        ),
        node: Some(NodeRef::Symbol(sym.id.clone())),
    }
}

/// Possible rename targets for a reference that no longer resolves.
pub fn rename_candidates<'a>(model: &'a Model, path: &[String]) -> Vec<&'a Symbol> {
    let Some(last) = path.last() else {
        return vec![];
    };
    let parent = path.len().checked_sub(2).map(|i| &path[i]);
    let mut out: Vec<&Symbol> = model
        .symbols
        .values()
        .filter(|s| s.kind != SymbolKind::Impl)
        .filter(|s| {
            let name = s.path.last().map(String::as_str).unwrap_or("");
            let parent_ok = match (parent, s.path.len().checked_sub(2).map(|i| &s.path[i])) {
                (Some(p), Some(q)) => p == q || similar_name(p, q),
                _ => true,
            };
            parent_ok && similar_name(last, name)
        })
        .collect();
    out.sort_by_key(|s| {
        (
            crate::text::edit_distance(last, s.path.last().map(String::as_str).unwrap_or("")),
            s.path.len(),
        )
    });
    out.truncate(3);
    out
}

/// Evidence about one written reference whose edge is `edge` (None = not checkable).
fn reference_evidence(
    model: &Model,
    b: &EvidenceBuilder<'_>,
    r: &Reference,
    edge: Option<&Edge>,
    spec: Citation,
    base_deps: &[Dependency],
) -> Option<Evidence> {
    let edge = edge?;
    if edge.resolution != Resolution::Unresolved {
        return None;
    }
    let (check, strength, summary) = match &r.target {
        ReferenceTarget::Symbol { path } => {
            if model.symbols.is_empty() || r.strength == ReferenceStrength::Weak {
                return None;
            }
            let strength = if r.strength == ReferenceStrength::Explicit {
                Strength::Proven
            } else {
                Strength::Moderate
            };
            let cands = rename_candidates(model, path);
            let mut summary = format!(
                "`{}` does not resolve to any symbol in the indexed source roots.",
                r.text
            );
            if !cands.is_empty() {
                let names: Vec<String> = cands
                    .iter()
                    .map(|s| format!("`{}`", s.qualified_name()))
                    .collect();
                summary.push_str(&format!(" Similar existing symbols: {}.", names.join(", ")));
            }
            (StructuralCheck::SymbolResolution, strength, summary)
        }
        ReferenceTarget::File { path, .. } => (
            StructuralCheck::FileResolution,
            Strength::Proven,
            format!("Referenced file `{path}` does not exist in the repository."),
        ),
        ReferenceTarget::Section { document, anchor } => {
            let summary = match (model.document_by_path(document), anchor) {
                (Some(d), Some(a)) => {
                    let mut anchors: Vec<&str> = model
                        .sections_of(&d.id)
                        .map(|s| s.anchor.as_str())
                        .filter(|s| !s.is_empty())
                        .collect();
                    anchors.sort_by_key(|x| crate::text::edit_distance(x, a));
                    anchors.truncate(4);
                    format!(
                        "`{document}` has no heading with anchor `#{a}`. Existing anchors include: {}.",
                        anchors.iter().map(|x| format!("`#{x}`")).collect::<Vec<_>>().join(", ")
                    )
                }
                _ => format!("Linked document `{document}` does not exist."),
            };
            (StructuralCheck::LinkResolution, Strength::Proven, summary)
        }
        _ => return None,
    };
    let mut deps = base_deps.to_vec();
    if let ReferenceTarget::Section { document, .. } = &r.target {
        if let Some(d) = model.document_by_path(document) {
            deps.extend(dep(model, NodeRef::Document(d.id.clone())));
        }
    }
    let b = EvidenceBuilder {
        ctx: b.ctx,
        subject: b.subject.clone(),
        method: Method::Structural { check },
    };
    Some(b.build(
        &format!("ref:{}", r.text),
        Stance::Contradicts,
        strength,
        summary,
        vec![spec],
        deps,
    ))
}

/// All structural evidence for one claim. `edges` are the claim's current edges.
pub fn claim_evidence(
    model: &Model,
    claim: &Claim,
    edges: &[Edge],
    fs: &dyn FsProbe,
    ctx: &BatchContext,
) -> Vec<Evidence> {
    let subject = NodeRef::Claim(claim.id.clone());
    let b = |check| EvidenceBuilder {
        ctx,
        subject: subject.clone(),
        method: Method::Structural { check },
    };
    let mut out = Vec::new();
    let cdep = claim_dep(claim);

    // 1-3. Reference resolution (symbols, files, links).
    let mut linked: Vec<(&Edge, &Symbol)> = Vec::new();
    for r in &claim.references {
        let e = edges
            .iter()
            .find(|e| e.target_text == r.text && e.kind != EdgeKind::MentionsSymbol);
        if let Some(ev) = reference_evidence(
            model,
            &b(StructuralCheck::SymbolResolution),
            r,
            e,
            spec_citation(claim, r.range.as_ref()),
            &[cdep.clone()],
        ) {
            out.push(ev);
        }
        if let Some(e) = e {
            if let Some(NodeRef::Symbol(id)) = &e.to {
                if let Some(s) = model.symbols.get(id) {
                    linked.push((e, s));
                }
            }
        }
    }
    if !linked.is_empty() {
        let strength = linked
            .iter()
            .map(|(e, _)| match e.resolution {
                Resolution::Exact => Strength::Proven,
                Resolution::UniqueSuffix => Strength::Strong,
                _ => Strength::Moderate,
            })
            .min()
            .unwrap_or(Strength::Moderate);
        let names: Vec<String> = linked
            .iter()
            .map(|(e, s)| format!("`{}` -> `{}`", e.target_text, s.qualified_name()))
            .collect();
        let mut deps = vec![cdep.clone()];
        deps.extend(
            linked
                .iter()
                .filter_map(|(_, s)| dep(model, NodeRef::Symbol(s.id.clone()))),
        );
        let mut cites = vec![spec_citation(claim, None)];
        cites.extend(linked.iter().map(|(_, s)| symbol_citation(s, None)));
        out.push(b(StructuralCheck::SymbolResolution).build(
            "linkage",
            Stance::Inconclusive,
            strength,
            format!(
                "References resolve: {}. Linkage only; it does not establish the claimed behavior.",
                names.join("; ")
            ),
            cites,
            deps,
        ));
    }

    // 4. Documented signature vs. actual signature.
    if let Some(doc_sig) = &claim.documented_signature {
        out.extend(signature_evidence(
            model,
            claim,
            doc_sig,
            &linked,
            &b(StructuralCheck::SignatureComparison),
        ));
    }

    // 5. Deterministic constraint forms.
    if let Some(form) = &claim.constraint {
        out.extend(constraint_evidence(model, claim, form, edges, fs, ctx));
    }

    // 6. Numeric bounds vs constants / clamps in referenced symbols.
    if !claim.bounds.is_empty() {
        out.extend(bound_evidence(
            model,
            claim,
            &linked,
            &b(StructuralCheck::ConstantBound),
        ));
    }

    // 7. Vague or self-contradictory language (heuristic, recorded as uncertainty).
    if !claim.ambiguity.is_empty() {
        let notes: Vec<String> = claim.ambiguity.iter().map(|a| a.detail.clone()).collect();
        out.push(b(StructuralCheck::VagueLanguage).build(
            "vague",
            Stance::Inconclusive,
            Strength::Weak,
            format!(
                "Possibly ambiguous wording (heuristic): {}.",
                notes.join("; ")
            ),
            vec![spec_citation(claim, None)],
            vec![cdep],
        ));
    }
    // Requirement verification by tests (deterministic: recorded outcomes).
    out.extend(crate::verification::claim_test_evidence(model, claim, ctx));
    out
}

fn signature_evidence(
    model: &Model,
    claim: &Claim,
    doc_sig: &str,
    linked: &[(&Edge, &Symbol)],
    b: &EvidenceBuilder<'_>,
) -> Option<Evidence> {
    let lang = claim.signature_language.as_deref().unwrap_or("rust");
    let plugin = b.ctx.languages.by_id(lang)?;
    let in_lang = |s: &&Symbol| model.files.get(&s.file).is_some_and(|f| f.language == lang);
    let Some((name, normalized)) = plugin.normalize_signature(doc_sig) else {
        return Some(b.build(
            "signature",
            Stance::Inconclusive,
            Strength::Weak,
            format!("The documented signature could not be parsed as a {lang} item header."),
            vec![spec_citation(claim, None)],
            vec![claim_dep(claim)],
        ));
    };
    // Candidates: linked symbols with that name, else every symbol with that name.
    let mut cands: Vec<&Symbol> = linked
        .iter()
        .map(|(_, s)| *s)
        .filter(|s| s.name == name)
        .filter(in_lang)
        .collect();
    if cands.is_empty() {
        cands = model
            .symbols_named(&name)
            .filter(|s| s.signature.is_some() && s.kind != SymbolKind::Impl)
            .filter(in_lang)
            .collect();
    }
    if cands.is_empty() {
        return None; // dangling reference evidence already covers this
    }
    let documented = plugin.strip_visibility(&normalized);
    let mut deps = vec![claim_dep(claim)];
    deps.extend(
        cands
            .iter()
            .filter_map(|s| dep(model, NodeRef::Symbol(s.id.clone()))),
    );
    if let Some(m) = cands.iter().find(|s| {
        s.signature
            .as_deref()
            .map(|x| plugin.strip_visibility(x))
            .as_deref()
            == Some(documented.as_str())
    }) {
        return Some(b.build(
            "signature",
            Stance::Supports,
            Strength::Proven,
            format!(
                "Documented signature matches `{}` exactly (visibility ignored).",
                m.qualified_name()
            ),
            vec![spec_citation(claim, None), symbol_citation(m, None)],
            deps,
        ));
    }
    let closest = cands[0];
    Some(b.build(
        "signature",
        Stance::Contradicts,
        Strength::Proven,
        format!(
                "Documented signature `{}` differs from the actual signature `{}` of `{}`.",
                documented,
                closest
                    .signature
                    .as_deref()
                    .map(|x| plugin.strip_visibility(x))
                    .unwrap_or_default(),
                closest.qualified_name()
            ),
        vec![spec_citation(claim, None), symbol_citation(closest, None)],
        deps,
    ))
}

fn constraint_evidence(
    model: &Model,
    claim: &Claim,
    form: &ConstraintForm,
    edges: &[Edge],
    fs: &dyn FsProbe,
    ctx: &BatchContext,
) -> Option<Evidence> {
    let subject = NodeRef::Claim(claim.id.clone());
    let cdep = claim_dep(claim);
    match form {
        ConstraintForm::NoUnsafe { scope } | ConstraintForm::NoPanic { scope } => {
            let is_unsafe = matches!(form, ConstraintForm::NoUnsafe { .. });
            let check = if is_unsafe {
                StructuralCheck::UnsafeConstraint
            } else {
                StructuralCheck::PanicConstraint
            };
            let b = EvidenceBuilder {
                ctx,
                subject,
                method: Method::Structural { check },
            };
            let scope = effective_scope(model, claim, scope, edges);
            let Some(symbols) = model.symbols_in_scope(&scope) else {
                return Some(b.build(
                    "constraint",
                    Stance::Inconclusive,
                    Strength::Weak,
                    format!(
                        "Constraint scope `{}` could not be resolved to indexed code.",
                        scope.join("::")
                    ),
                    vec![spec_citation(claim, None)],
                    vec![cdep],
                ));
            };
            let kind = if is_unsafe {
                FactKind::UnsafeCode
            } else {
                FactKind::PanicSite
            };
            let in_tests = |s: &Symbol| {
                s.cfg.iter().any(|c| c == "test") || s.path.iter().any(|p| p == "tests")
            };
            let sites: Vec<(&Symbol, &concordance_core::code::Fact)> = symbols
                .iter()
                .filter(|s| !in_tests(s))
                .flat_map(|s| {
                    s.facts
                        .iter()
                        .filter(|f| f.kind == kind)
                        .map(move |f| (*s, f))
                })
                .collect();
            let scope_text = if scope.is_empty() {
                "the indexed source roots".to_string()
            } else {
                format!("`{}`", scope.join("::"))
            };
            let what = if is_unsafe {
                "unsafe code"
            } else {
                "direct failure sites (panics or throws)"
            };
            if sites.is_empty() {
                let (strength, limitation) = if is_unsafe {
                    (
                        Strength::Strong,
                        "Macro expansions and dependencies are not inspected.",
                    )
                } else {
                    (Strength::Moderate, "Callees, indexing, arithmetic overflow and panics inside macro arguments are not analyzed.")
                };
                let mut deps = vec![cdep];
                deps.extend(
                    symbols
                        .iter()
                        .filter(|s| !in_tests(s))
                        .take(400)
                        .filter_map(|s| dep(model, NodeRef::Symbol(s.id.clone()))),
                );
                return Some(b.build(
                    "constraint",
                    Stance::Supports,
                    strength,
                    format!("No {what} found in {scope_text} ({} symbols scanned, tests excluded). {limitation}", symbols.len()),
                    vec![spec_citation(claim, None)],
                    deps,
                ));
            }
            let mut cites = vec![spec_citation(claim, None)];
            let mut deps = vec![cdep];
            for (s, f) in sites.iter().take(MAX_SITE_CITATIONS) {
                cites.push(Citation {
                    role: CitationRole::Implementation,
                    range: f.range.clone(),
                    excerpt: bounded(&f.detail),
                    node: Some(NodeRef::Symbol(s.id.clone())),
                });
                deps.extend(dep(model, NodeRef::Symbol(s.id.clone())));
            }
            deps.dedup();
            Some(b.build(
                "constraint",
                Stance::Contradicts,
                Strength::Strong,
                format!("{} {what} found in {scope_text}, e.g. `{}` in `{}` (syntactic; reachability not analyzed).", sites.len(), sites[0].1.detail, sites[0].0.qualified_name()),
                cites,
                deps,
            ))
        }
        ConstraintForm::NoDependency { from, to } => {
            let b = EvidenceBuilder {
                ctx,
                subject,
                method: Method::Structural {
                    check: StructuralCheck::DependencyConstraint,
                },
            };
            let from_n = from.replace('-', "_");
            let to_n = to.replace('-', "_");
            let krate = model.workspace.crates.iter().find(|c| c.name == from_n)?;
            let mut cites = vec![spec_citation(claim, None)];
            let mut deps = vec![cdep];
            if krate.dependencies.contains(&to_n) {
                // Cite the manifest line that declares the dependency (either spelling).
                let dash = to_n.replace('_', "-");
                let line = fs
                    .find_line(&krate.manifest, &|l: &str| {
                        let t = l.trim_start();
                        t.starts_with(&format!("{to_n} "))
                            || t.starts_with(&format!("{to_n}="))
                            || t.starts_with(&format!("{to_n}."))
                            || t.starts_with(&format!("{dash} "))
                            || t.starts_with(&format!("{dash}="))
                            || t.starts_with(&format!("{dash}."))
                    })
                    .unwrap_or(1);
                cites.push(Citation {
                    role: CitationRole::Implementation,
                    range: concordance_core::SourceRange::lines(krate.manifest.clone(), line, line),
                    excerpt: format!("{} declares a dependency on {}", krate.manifest, to),
                    node: None,
                });
                return Some(b.build(
                    "constraint",
                    Stance::Contradicts,
                    Strength::Proven,
                    format!("Crate `{from_n}` declares a Cargo dependency on `{to_n}`."),
                    cites,
                    deps,
                ));
            }
            let mut hits = Vec::new();
            for f in model
                .files
                .values()
                .filter(|f| f.crate_name.as_deref() == Some(from_n.as_str()))
            {
                for imp in model.imports.get(&f.id).into_iter().flatten() {
                    if imp.path.first().map(|s| s.replace('-', "_")) == Some(to_n.clone()) {
                        hits.push((f, imp));
                    }
                }
            }
            if let Some((f, imp)) = hits.first() {
                for (f, imp) in hits.iter().take(MAX_SITE_CITATIONS) {
                    cites.push(Citation {
                        role: CitationRole::Implementation,
                        range: imp.range.clone(),
                        excerpt: format!("use {}", imp.path.join("::")),
                        node: Some(NodeRef::File(f.id.clone())),
                    });
                    deps.extend(dep(model, NodeRef::File(f.id.clone())));
                }
                return Some(b.build(
                    "constraint",
                    Stance::Contradicts,
                    Strength::Strong,
                    format!(
                        "`{}` imports `{}` ({} import site(s)).",
                        f.path,
                        imp.path.join("::"),
                        hits.len()
                    ),
                    cites,
                    deps,
                ));
            }
            let strength = if model.workspace.from_cargo_metadata {
                Strength::Strong
            } else {
                Strength::Moderate
            };
            deps.extend(
                model
                    .files
                    .values()
                    .filter(|f| f.crate_name.as_deref() == Some(from_n.as_str()))
                    .take(400)
                    .filter_map(|f| dep(model, NodeRef::File(f.id.clone()))),
            );
            Some(b.build("constraint", Stance::Supports, strength, format!("Crate `{from_n}` neither declares nor imports `{to_n}` (Cargo metadata and `use` items)."), cites, deps))
        }
    }
}

/// Scope for a constraint: explicit scope, else the crate the claim's constraint edge
/// points at, else whole roots.
fn effective_scope(model: &Model, claim: &Claim, scope: &[String], edges: &[Edge]) -> Vec<String> {
    if !scope.is_empty() {
        if let Resolved::Unresolved = model.resolve_path(scope, None) {
            if let Some(e) = edges.iter().find(|e| e.kind == EdgeKind::ConstrainsScope) {
                if let Some(NodeRef::File(f)) = &e.to {
                    if let Some(c) = model.files.get(f).and_then(|f| f.crate_name.clone()) {
                        return vec![c];
                    }
                }
            }
        }
        return scope.to_vec();
    }
    let _ = claim;
    Vec::new()
}

/// Parse the numeric literal in a fact detail like `MAX = 100`, `v.min(150)`, `x > 1_000`.
fn literal_numbers(s: &str) -> Vec<f64> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_alpha = false;
    for ch in s.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_digit() || (ch == '.' && !cur.is_empty()) || (ch == '_' && !cur.is_empty()) {
            if cur.is_empty() && prev_alpha {
                continue;
            }
            cur.push(ch);
        } else {
            if !cur.is_empty() && !prev_alpha {
                let clean: String = cur.chars().filter(|c| *c != '_').collect();
                if let Ok(v) = clean.trim_end_matches('.').parse::<f64>() {
                    out.push(v);
                }
            }
            cur.clear();
            prev_alpha = ch.is_alphanumeric() || ch == '_';
        }
    }
    out
}

fn bound_evidence(
    model: &Model,
    claim: &Claim,
    linked: &[(&Edge, &Symbol)],
    b: &EvidenceBuilder<'_>,
) -> Vec<Evidence> {
    let mut out = Vec::new();
    let mut syms: Vec<&Symbol> = linked.iter().map(|(_, s)| *s).collect();
    // Constants named in the claim text by code span count as linked too.
    syms.dedup_by_key(|s| s.id.clone());
    for bound in &claim.bounds {
        for s in &syms {
            let relevant: Vec<&concordance_core::code::Fact> = s
                .facts
                .iter()
                .filter(|f| matches!(f.kind, FactKind::ConstValue | FactKind::Clamp))
                .collect();
            for f in relevant {
                let values = literal_numbers(&f.detail);
                let Some(&v) = values
                    .iter()
                    .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                else {
                    continue;
                };
                let upper = matches!(bound.op, BoundOp::Le | BoundOp::Lt);
                let lower = matches!(bound.op, BoundOp::Ge | BoundOp::Gt);
                if !(upper || lower) {
                    continue;
                }
                let violates = !bound.op.admits(v, bound.value);
                let (stance, strength, summary) = match (f.kind, violates) {
                    (FactKind::ConstValue, true) => (
                        Stance::Contradicts,
                        Strength::Strong,
                        format!("Constant `{}` is {v}, outside the documented bound \"{}\".", f.detail, bound.text),
                    ),
                    (FactKind::ConstValue, false) => (
                        Stance::Inconclusive,
                        Strength::Moderate,
                        format!("Constant `{}` is within the documented bound \"{}\"; whether it is enforced is not established structurally.", f.detail, bound.text),
                    ),
                    (_, true) if upper => (
                        Stance::Contradicts,
                        Strength::Moderate,
                        format!("`{}` in `{}` clamps to {v}, which exceeds the documented bound \"{}\" (syntactic; data flow not analyzed).", f.detail, s.qualified_name(), bound.text),
                    ),
                    _ => continue,
                };
                let mut deps = vec![claim_dep(claim)];
                deps.extend(dep(model, NodeRef::Symbol(s.id.clone())));
                out.push(b.build(
                    &format!("bound:{}:{}:{}", bound.text, s.id, f.detail),
                    stance,
                    strength,
                    summary,
                    vec![
                        spec_citation(claim, None),
                        Citation {
                            role: CitationRole::Implementation,
                            range: f.range.clone(),
                            excerpt: bounded(&f.detail),
                            node: Some(NodeRef::Symbol(s.id.clone())),
                        },
                    ],
                    deps,
                ));
            }
        }
    }
    out
}

/// Structural evidence for references in non-claim prose. Only contradictions are
/// returned; each is keyed by the finding subject it belongs to.
pub fn prose_evidence(
    model: &Model,
    doc: &Document,
    refs: &[ProseReference],
    edges: &[Edge],
    ctx: &BatchContext,
) -> Vec<(FindingSubject, Evidence)> {
    let subject = NodeRef::Document(doc.id.clone());
    let b = EvidenceBuilder {
        ctx,
        subject: subject.clone(),
        method: Method::Structural {
            check: StructuralCheck::SymbolResolution,
        },
    };
    let base = vec![Dependency {
        node: subject.clone(),
        content_hash: doc.content_hash.clone(),
    }];
    let mut out = Vec::new();
    for p in refs {
        let r = &p.reference;
        // Prose only produces findings for Explicit references.
        if r.strength != ReferenceStrength::Explicit {
            continue;
        }
        let e = edges.iter().find(|e| e.target_text == r.text);
        let spec = Citation {
            role: CitationRole::Specification,
            range: r
                .range
                .clone()
                .unwrap_or_else(|| concordance_core::SourceRange::lines(doc.path.clone(), 1, 1)),
            excerpt: bounded(&r.text),
            node: Some(NodeRef::Section(p.section.clone())),
        };
        if let Some(ev) = reference_evidence(model, &b, r, e, spec, &base) {
            out.push((
                FindingSubject::ProseReference {
                    document: doc.id.clone(),
                    text: r.text.clone(),
                },
                ev,
            ));
        }
    }
    out
}

/// Symbols a claim is linked to explicitly (resolved reference edges), best first.
pub fn explicit_symbols<'a>(model: &'a Model, edges: &[Edge]) -> Vec<&'a Symbol> {
    let mut out: Vec<&Symbol> = edges
        .iter()
        .filter(|e| e.kind == EdgeKind::ReferencesSymbol)
        .filter_map(|e| match &e.to {
            Some(NodeRef::Symbol(id)) => model.symbols.get(id),
            _ => None,
        })
        .collect();
    out.dedup_by_key(|s| s.id.clone());
    out
}

pub fn symbol_ids(syms: &[&Symbol]) -> Vec<SymbolId> {
    syms.iter().map(|s| s.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::literal_numbers;

    #[test]
    fn literal_numbers_ignore_identifier_digits() {
        assert_eq!(literal_numbers("MAX_VOLUME = 100"), vec![100.0]);
        assert_eq!(literal_numbers("v.min(150)"), vec![150.0]);
        assert_eq!(literal_numbers("x2 > 1_000"), vec![1000.0]);
    }
}
