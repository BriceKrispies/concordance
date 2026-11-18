//! Requirement verification by tests.
//!
//! A specification statement tagged with a requirement ID (`**LANG-006.3** ...`, or a
//! `## PSS-003` heading) is linked to every test whose doc comment declares
//! `Verifies: LANG-006.3`. Recorded test outcomes (see [`concordance_core::testrun`])
//! become evidence:
//!
//! * passed, recorded after the last source change -> supports (strong)
//! * failed -> contradicts (strong)
//! * recorded before a source change -> inconclusive ("outdated"); never current
//! * ignored, or no recorded result -> inconclusive
//!
//! Evidence is strong only for a requirement that is one statement: an inline ID, or a
//! heading ID whose section holds exactly one obligation. A test cannot say which of
//! several bundled obligations it exercised, so for bundles it is moderate (never enough
//! to verify or violate on its own).
//!
//! A `Verifies:` declaration naming an ID no specification defines is a broken
//! reference and is reported as a finding on the test.

use crate::derive::{locate, proposals_for};
use crate::structural::symbol_citation;
use crate::text::{spec_citation, EvidenceBuilder};
use crate::BatchContext;
use concordance_core::code::Symbol;
use concordance_core::doc::{Authority, Claim, RequirementForm};
use concordance_core::evidence::{
    Dependency, Evidence, Method, MethodKind, Stance, Strength, StructuralCheck,
};
use concordance_core::finding::{
    EvidenceQuality, Finding, FindingCategory, FindingStatus, FindingSubject, Lifecycle, Severity,
};
use concordance_core::graph::{EdgeKind, NodeRef, Resolution};
use concordance_core::testrun::TestOutcome;
use concordance_core::{ClaimId, DocumentId, SourceRange, SymbolId};
use concordance_graph::Model;
use std::collections::BTreeMap;

/// Number of obligation claims that carry requirement `id`.
pub fn obligations_in(model: &Model, id: &str) -> usize {
    model
        .claims_of_requirement(id)
        .filter(|c| c.is_obligation())
        .count()
}

/// Whether test results about `claim`'s requirement speak for this one statement.
pub fn is_atomic(model: &Model, claim: &Claim) -> bool {
    match &claim.requirement {
        Some(r) if r.form == RequirementForm::Inline => true,
        Some(r) => obligations_in(model, &r.id) <= 1,
        None => false,
    }
}

/// Test evidence for one requirement-tagged claim. Empty when the claim has no
/// requirement or no test declares that it verifies it.
pub fn claim_test_evidence(model: &Model, claim: &Claim, ctx: &BatchContext) -> Vec<Evidence> {
    let Some(req) = &claim.requirement else {
        return vec![];
    };
    let atomic = is_atomic(model, claim);
    let bundle_note = if atomic {
        String::new()
    } else {
        format!(
            " Requirement `{}` bundles {} obligations under one heading; a test cannot show which of them it exercised.",
            req.id,
            obligations_in(model, &req.id)
        )
    };
    let mut verifiers: Vec<&Symbol> = model.verifiers_of(&req.id).filter(|s| s.is_test).collect();
    verifiers.sort_by(|a, b| a.id.cmp(&b.id));
    let mut out = Vec::new();
    for t in verifiers {
        let name = t.qualified_name();
        let (outcome, run, stance, strength, summary) = match model.test_result(t) {
            None => (
                "not_run".to_string(),
                String::new(),
                Stance::Inconclusive,
                Strength::Weak,
                format!("Test `{name}` declares it verifies `{}`, but no ingested test run reports it.", req.id),
            ),
            Some((run, o)) if run.is_outdated() => (
                "outdated".to_string(),
                run.path.clone(),
                Stance::Inconclusive,
                Strength::Moderate,
                format!(
                    "Test `{name}` was recorded as {} in `{}`, but {} indexed source file(s) changed after that run (e.g. `{}`); the result no longer describes the current code. Re-run the tests.",
                    o.as_str(),
                    run.path,
                    run.newer_sources,
                    run.newest_source.as_ref().map(|p| p.to_string()).unwrap_or_default()
                ),
            ),
            Some((run, TestOutcome::Passed)) => (
                "passed".to_string(),
                run.path.clone(),
                Stance::Supports,
                if atomic { Strength::Strong } else { Strength::Moderate },
                format!("Test `{name}`, which declares it verifies `{}`, passed in `{}`.{bundle_note}", req.id, run.path),
            ),
            Some((run, TestOutcome::Failed)) => (
                "failed".to_string(),
                run.path.clone(),
                Stance::Contradicts,
                if atomic { Strength::Strong } else { Strength::Moderate },
                format!("Test `{name}`, which declares it verifies `{}`, FAILED in `{}`.{bundle_note}", req.id, run.path),
            ),
            Some((run, TestOutcome::Ignored)) => (
                "ignored".to_string(),
                run.path.clone(),
                Stance::Inconclusive,
                Strength::Weak,
                format!("Test `{name}` declares it verifies `{}`, but it was ignored in `{}`.", req.id, run.path),
            ),
        };
        let b = EvidenceBuilder {
            ctx,
            subject: NodeRef::Claim(claim.id.clone()),
            method: Method::Test {
                test: name.clone(),
                outcome,
                run,
            },
        };
        let deps = vec![
            Dependency {
                node: NodeRef::Claim(claim.id.clone()),
                content_hash: claim.content_hash.clone(),
            },
            Dependency {
                node: NodeRef::Symbol(t.id.clone()),
                content_hash: t.content_hash.clone(),
            },
        ];
        out.push(b.build(
            &format!("test:{}", t.id),
            stance,
            strength,
            summary,
            vec![spec_citation(claim, None), symbol_citation(t, None)],
            deps,
        ));
    }
    out
}

/// Findings for `Verifies:` declarations on `sym` that name undefined requirements.
pub fn verification_findings(
    model: &Model,
    sym: &Symbol,
    ctx: &BatchContext,
) -> Vec<(Finding, Evidence)> {
    let name = sym.qualified_name();
    sym.verifies
        .iter()
        .filter(|r| !model.requirement_defined(r))
        .map(|req| {
            let subject = FindingSubject::Verification { symbol: sym.id.clone(), requirement: req.clone() };
            let text = format!(
                "`{name}` declares `Verifies: {req}`, but no specification defines requirement `{req}` (no bold `**{req}**` paragraph opener and no `{req}` heading in the spec roots)."
            );
            let cite = symbol_citation(sym, None);
            let ev = EvidenceBuilder {
                ctx,
                subject: NodeRef::Symbol(sym.id.clone()),
                method: Method::Structural { check: StructuralCheck::RequirementResolution },
            }
            .build(
                req,
                Stance::Contradicts,
                Strength::Proven,
                text.clone(),
                vec![cite.clone()],
                vec![Dependency { node: NodeRef::Symbol(sym.id.clone()), content_hash: sym.content_hash.clone() }],
            );
            let (crate_name, module) = locate(model, std::slice::from_ref(&sym.id));
            let f = Finding {
                id: subject.finding_id(),
                subject,
                status: FindingStatus::Violated,
                category: FindingCategory::StructuralContradiction,
                severity: Severity::Medium,
                title: format!("Test verifies unknown requirement `{req}`: `{}`", concordance_graph::truncate(&name, 80)),
                explanation: format!(
                    "{text}\n\nEither the requirement was renamed or removed from the specification, or the ID is mistyped. Until it resolves, this test's outcome is evidence for nothing."
                ),
                claims: vec![],
                symbols: vec![sym.id.clone()],
                documents: vec![],
                locations: vec![cite.range.clone()],
                supporting: vec![],
                contradicting: vec![ev.id.clone()],
                confidence: 0.95,
                quality: EvidenceQuality::Proven,
                methods: vec![MethodKind::Structural],
                limitations: vec![],
                dependencies: vec![NodeRef::Symbol(sym.id.clone())],
                proposals: proposals_for(
                    FindingStatus::Violated,
                    FindingCategory::StructuralContradiction,
                    Authority::Unknown,
                    vec![sym.range.clone()],
                    ctx.semantic_enabled,
                ),
                revision: ctx.revision,
                introduced_in: ctx.revision,
                introduced_by: ctx.changed_files.clone(),
                first_seen: ctx.now,
                last_updated: ctx.now,
                lifecycle: Lifecycle::Open,
                crate_name,
                module,
            };
            (f, ev)
        })
        .collect()
}

/// Whether `s` is test scaffolding rather than the code under test: a test, an item under
/// `cfg(test)`, or anything in a `tests` directory.
fn is_test_support(model: &Model, s: &Symbol) -> bool {
    s.is_test
        || s.cfg.iter().any(|c| c.contains("test"))
        || model.files.get(&s.file).is_some_and(|f| {
            f.path.as_str().starts_with("tests/") || f.path.as_str().contains("/tests/")
        })
}

/// What a proof reaches through resolved calls. Test scaffolding is followed (a helper that
/// drives the boundary is part of the proof); other non-public callees are internals.
pub struct ProofReach {
    pub reaches_public: bool,
    pub internals: Vec<SymbolId>,
}

pub fn proof_reach(model: &Model, proof: &Symbol) -> ProofReach {
    let mut reach = ProofReach {
        reaches_public: false,
        internals: Vec::new(),
    };
    let mut seen: std::collections::BTreeSet<SymbolId> = std::collections::BTreeSet::new();
    seen.insert(proof.id.clone());
    let mut frontier = vec![proof.id.clone()];
    for _depth in 0..6 {
        let mut next = Vec::new();
        for id in &frontier {
            for e in model.edges_from(&NodeRef::Symbol(id.clone())) {
                if e.kind != EdgeKind::Calls {
                    continue;
                }
                // A method matched by name only is not evidence that the proof calls it.
                if e.resolution == Resolution::NameOnly {
                    continue;
                }
                let Some(NodeRef::Symbol(t)) = &e.to else {
                    continue;
                };
                let Some(target) = model.symbols.get(t) else {
                    continue;
                };
                if !seen.insert(t.clone()) {
                    continue;
                }
                if target.is_public_api {
                    reach.reaches_public = true;
                } else if is_test_support(model, target) {
                    next.push(t.clone());
                } else {
                    reach.internals.push(t.clone());
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    reach
}

/// Proof-rule findings (`docs/spec-lint.md`): a requirement with more than one declaring
/// proof, and a proof that calls only non-public symbols and so bypasses the boundary.
pub fn proof_rule_findings(model: &Model, ctx: &BatchContext) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut ids: Vec<String> = model
        .requirement_claims()
        .filter_map(|c| c.requirement.as_ref().map(|r| r.id.clone()))
        .collect();
    ids.sort();
    ids.dedup();
    for req in ids {
        let mut proofs: Vec<&Symbol> = model.verifiers_of(&req).collect();
        proofs.sort_by(|a, b| a.id.cmp(&b.id));
        let mut claims: Vec<&Claim> = model.claims_of_requirement(&req).collect();
        claims.sort_by(|a, b| a.id.cmp(&b.id));
        let authority = claims
            .first()
            .map(|c| c.authority)
            .unwrap_or(Authority::Unknown);
        let claim_ids: Vec<ClaimId> = claims.iter().map(|c| c.id.clone()).collect();
        let spec_loc: Vec<SourceRange> = claims.iter().take(1).map(|c| c.range.clone()).collect();
        // The same ID defined in two or more documents: the first claim of each.
        let mut first_in_doc: BTreeMap<DocumentId, SourceRange> = BTreeMap::new();
        for c in &claims {
            first_in_doc
                .entry(c.document.clone())
                .or_insert_with(|| c.range.clone());
        }
        if first_in_doc.len() > 1 {
            let paths: Vec<String> = first_in_doc
                .keys()
                .map(|d| {
                    model
                        .documents
                        .get(d)
                        .map(|doc| doc.path.as_str().to_string())
                        .unwrap_or_default()
                })
                .collect();
            let subject = FindingSubject::ProofRule {
                requirement: req.clone(),
                rule: "duplicate_id".into(),
                symbol: None,
            };
            out.push(proof_finding(
                model,
                ctx,
                subject,
                (
                    FindingStatus::Ambiguous,
                    FindingCategory::UnclearDocumentation,
                    Severity::Medium,
                ),
                format!(
                    "Requirement `{req}` is defined in {} documents",
                    paths.len()
                ),
                format!(
                    "`{req}` opens a requirement in each of {}. A requirement ID names one statement and IDs are never reused: rename one of them, and keep the other as the one that verifies.",
                    paths.join(", ")
                ),
                (claims.iter().map(|c| c.id.clone()).collect(), vec![]),
                first_in_doc.values().cloned().collect(),
                authority,
            ));
        }
        if proofs.len() > 1 {
            let subject = FindingSubject::ProofRule {
                requirement: req.clone(),
                rule: "multiple_proofs".into(),
                symbol: None,
            };
            let names: Vec<String> = proofs
                .iter()
                .map(|p| format!("`{}`", p.qualified_name()))
                .collect();
            let mut locations = spec_loc.clone();
            locations.extend(proofs.iter().map(|p| p.range.clone()));
            out.push(proof_finding(
                model,
                ctx,
                subject,
                (
                    FindingStatus::Ambiguous,
                    FindingCategory::UnclearDocumentation,
                    Severity::Low,
                ),
                format!("Requirement `{req}` has {} proofs", proofs.len()),
                format!(
                    "{} items declare `Verifies: {req}`: {}. Each requirement has exactly one proof that declares it; keep the one that drives the boundary and drop the `Verifies:` line from the rest.",
                    proofs.len(),
                    names.join(", ")
                ),
                (
                    claim_ids.clone(),
                    proofs.iter().map(|p| p.id.clone()).collect(),
                ),
                locations,
                authority,
            ));
        }
        for p in proofs {
            let reach = proof_reach(model, p);
            if reach.reaches_public || reach.internals.is_empty() {
                continue;
            }
            let subject = FindingSubject::ProofRule {
                requirement: req.clone(),
                rule: "bypasses_boundary".into(),
                symbol: Some(p.id.clone()),
            };
            let called: Vec<String> = reach
                .internals
                .iter()
                .filter_map(|i| model.symbols.get(i))
                .map(|s| format!("`{}`", s.qualified_name()))
                .take(5)
                .collect();
            let mut locations = spec_loc.clone();
            locations.push(p.range.clone());
            out.push(proof_finding(
                model,
                ctx,
                subject,
                (
                    FindingStatus::Violated,
                    FindingCategory::StructuralContradiction,
                    Severity::Medium,
                ),
                format!(
                    "Proof `{}` of `{req}` calls no public API",
                    concordance_graph::truncate(&p.qualified_name(), 80)
                ),
                format!(
                    "`{}` declares `Verifies: {req}` but every call it makes that resolves to indexed code goes to a non-public symbol ({}). A proof calls what the boundary of the requirement names and nothing private: a correct rewrite of the internals would break it.",
                    p.qualified_name(),
                    called.join(", ")
                ),
                (claim_ids.clone(), vec![p.id.clone()]),
                locations,
                authority,
            ));
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn proof_finding(
    model: &Model,
    ctx: &BatchContext,
    subject: FindingSubject,
    (status, category, severity): (FindingStatus, FindingCategory, Severity),
    title: String,
    explanation: String,
    (claims, symbols): (Vec<ClaimId>, Vec<SymbolId>),
    locations: Vec<SourceRange>,
    authority: Authority,
) -> Finding {
    let (crate_name, module) = locate(model, &symbols);
    Finding {
        id: subject.finding_id(),
        subject,
        status,
        category,
        severity,
        title,
        explanation,
        claims,
        symbols,
        documents: vec![],
        proposals: proposals_for(
            status,
            category,
            authority,
            locations.clone(),
            ctx.semantic_enabled,
        ),
        locations,
        supporting: vec![],
        contradicting: vec![],
        confidence: 0.9,
        quality: EvidenceQuality::Proven,
        methods: vec![MethodKind::Structural],
        limitations: vec![
            "calls are resolved syntactically; calls through trait objects or closures are not followed"
                .into(),
        ],
        dependencies: vec![],
        revision: ctx.revision,
        introduced_in: ctx.revision,
        introduced_by: ctx.changed_files.clone(),
        first_seen: ctx.now,
        last_updated: ctx.now,
        lifecycle: Lifecycle::Open,
        crate_name,
        module,
    }
}
