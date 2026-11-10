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
use concordance_core::graph::NodeRef;
use concordance_core::testrun::TestOutcome;
use concordance_graph::Model;

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
