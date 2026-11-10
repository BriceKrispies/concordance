//! Layer D (structural part): public implementation behavior with no documented
//! contract. These are proposed specification gaps, never errors.

use crate::derive::{locate, proposals_for};
use crate::text::{bounded, EvidenceBuilder};
use crate::BatchContext;
use concordance_core::code::{FactKind, Symbol, SymbolKind};
use concordance_core::doc::Authority;
use concordance_core::evidence::{
    Citation, CitationRole, Dependency, Evidence, Method, MethodKind, Stance, Strength,
    StructuralCheck,
};
use concordance_core::finding::{
    EvidenceQuality, Finding, FindingCategory, FindingStatus, FindingSubject, Lifecycle, Severity,
};
use concordance_core::graph::{EdgeKind, NodeRef};
use concordance_graph::Model;

fn in_test_code(s: &Symbol) -> bool {
    s.cfg.iter().any(|c| c == "test")
        || s.path
            .iter()
            .any(|p| p == "tests" || p == "benches" || p == "examples")
        || s.attributes.iter().any(|a| a.contains("test"))
}

/// Claims that explicitly reference this symbol (or, for methods, nothing broader).
fn covering_claim_texts(model: &Model, sym: &Symbol) -> Vec<String> {
    model
        .edges_to(&NodeRef::Symbol(sym.id.clone()))
        .filter(|e| e.kind == EdgeKind::ReferencesSymbol)
        .filter_map(|e| match &e.from {
            NodeRef::Claim(c) => model.claims.get(c).map(|c| c.text.to_lowercase()),
            NodeRef::Document(_) => Some(String::new()),
            _ => None,
        })
        .collect()
}

/// Is this symbol a candidate for gap detection at all?
pub fn is_gap_candidate(sym: &Symbol) -> bool {
    sym.is_public_api
        && matches!(
            sym.kind,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::AssociatedFunction
                | SymbolKind::Struct
                | SymbolKind::Enum
                | SymbolKind::Trait
                | SymbolKind::Class
                | SymbolKind::Interface
                | SymbolKind::Constructor
        )
        && !in_test_code(sym)
}

/// The indexer treats file modules as public because a file cannot see its parent's
/// `mod x;` declaration. The model can: if any enclosing module declaration is not
/// `pub`, the symbol is not public API.
pub fn in_private_module(model: &Model, sym: &Symbol) -> bool {
    let Some(file) = model.files.get(&sym.file) else {
        return false;
    };
    let Some(krate) = sym.path.first() else {
        return false;
    };
    let mut path = vec![krate.clone()];
    for seg in &file.module_path {
        path.push(seg.clone());
        let private = model.symbols_named(seg).any(|m| {
            m.kind == SymbolKind::Module
                && m.path == path
                && m.visibility != concordance_core::code::Visibility::Public
        });
        if private {
            return true;
        }
    }
    false
}

/// Undocumented-behavior findings for one symbol, with the evidence each rests on.
pub fn undocumented_findings(
    model: &Model,
    sym: &Symbol,
    ctx: &BatchContext,
) -> Vec<(Finding, Evidence)> {
    if !is_gap_candidate(sym) || in_private_module(model, sym) {
        return vec![];
    }
    let covering = covering_claim_texts(model, sym);
    let mentions = |words: &[&str]| covering.iter().any(|t| words.iter().any(|w| t.contains(w)));
    let name = sym.qualified_name();
    let mut gaps: Vec<(&str, Severity, String, Vec<&concordance_core::code::Fact>)> = Vec::new();

    let panics: Vec<_> = sym
        .facts
        .iter()
        .filter(|f| f.kind == FactKind::PanicSite)
        .collect();
    if !panics.is_empty() && !mentions(&["panic", "abort", "crash"]) {
        gaps.push((
            "panics",
            Severity::Medium,
            format!("Public `{name}` contains {} direct panic site(s) (e.g. `{}`), but no specification states when it may panic.", panics.len(), panics[0].detail),
            panics,
        ));
    }
    if let Some((_, err)) = sym.returns.as_ref().and_then(|r| r.result.as_ref()) {
        if !mentions(&["error", "err", "fail", "result", "invalid", "reject"]) {
            let errs: Vec<_> = sym
                .facts
                .iter()
                .filter(|f| f.kind == FactKind::ErrorReturn)
                .take(5)
                .collect();
            gaps.push((
                "errors",
                Severity::Low,
                format!("Public `{name}` returns `Result<_, {err}>`, but no specification defines its error conditions."),
                errs,
            ));
        }
    }
    let envs: Vec<_> = sym
        .facts
        .iter()
        .filter(|f| f.kind == FactKind::EnvRead)
        .collect();
    if !envs.is_empty() && !mentions(&["env", "environment", "variable"]) {
        gaps.push((
            "configuration",
            Severity::Low,
            format!("Public `{name}` reads environment configuration (`{}`) that no specification documents.", envs[0].detail),
            envs,
        ));
    }
    if covering.is_empty() && sym.doc.is_none() && gaps.is_empty() && sym.kind.is_callable() {
        gaps.push(("contract", Severity::Info, format!("Public `{name}` has no documented contract: no specification references it and it has no doc comment."), vec![]));
    }

    let symbol_dep = Dependency {
        node: NodeRef::Symbol(sym.id.clone()),
        content_hash: sym.content_hash.clone(),
    };
    gaps.into_iter()
        .map(|(aspect, severity, text, facts)| {
            let subject = FindingSubject::Undocumented { symbol: sym.id.clone(), aspect: aspect.to_string() };
            let mut cites = vec![Citation {
                role: CitationRole::Implementation,
                range: sym.range.clone(),
                excerpt: bounded(sym.signature.as_deref().unwrap_or(&name)),
                node: Some(NodeRef::Symbol(sym.id.clone())),
            }];
            cites.extend(facts.iter().take(8).map(|f| Citation {
                role: CitationRole::Implementation,
                range: f.range.clone(),
                excerpt: bounded(&f.detail),
                node: Some(NodeRef::Symbol(sym.id.clone())),
            }));
            let ev = EvidenceBuilder {
                ctx,
                subject: NodeRef::Symbol(sym.id.clone()),
                method: Method::Structural { check: StructuralCheck::PublicApiCoverage },
            }
            .build(aspect, Stance::Inconclusive, Strength::Strong, text.clone(), cites.clone(), vec![symbol_dep.clone()]);
            let (crate_name, module) = locate(model, std::slice::from_ref(&sym.id));
            let f = Finding {
                id: subject.finding_id(),
                subject,
                status: FindingStatus::Undocumented,
                category: FindingCategory::MissingDocumentation,
                severity,
                title: format!("Undocumented {aspect}: `{}`", concordance_graph::truncate(&name, 80)),
                explanation: format!("{text}\n\nThis is a proposed specification gap, not an error: the behavior may be intended, but it is not defined anywhere it could be checked."),
                claims: vec![],
                symbols: vec![sym.id.clone()],
                documents: vec![],
                locations: cites.iter().map(|c| c.range.clone()).collect(),
                supporting: vec![],
                contradicting: vec![],
                confidence: 0.7,
                quality: EvidenceQuality::Strong,
                methods: vec![MethodKind::Structural],
                limitations: vec!["Public API reachability is approximated syntactically (re-exports are not followed).".into()],
                dependencies: vec![NodeRef::Symbol(sym.id.clone())],
                proposals: proposals_for(FindingStatus::Undocumented, FindingCategory::MissingDocumentation, Authority::Unknown, vec![sym.range.clone()], ctx.semantic_enabled),
                revision: ctx.revision,
                introduced_in: ctx.revision,
                introduced_by: ctx.changed_files.clone(),
                first_seen: ctx.now,
                last_updated: ctx.now,
                lifecycle: Lifecycle::Open,
                crate_name,
                module,
            };
            let mut f = f;
            f.supporting.push(ev.id.clone());
            (f, ev)
        })
        .collect()
}
