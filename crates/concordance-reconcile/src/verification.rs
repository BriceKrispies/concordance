//! Requirement verification by tests.
//!
//! A specification statement tagged with a requirement ID (`**LANG-006.3** ...`, or a
//! `## PSS-003` heading) is linked to every test whose doc comment declares
//! `Verifies: LANG-006.3`. Recorded test outcomes (see [`concordance_core::testrun`])
//! become evidence:
//!
//! * passed, in a run recorded against the current sources and requirement text, and its
//!   proof is not reported by a proof rule -> supports (strong)
//! * failed -> contradicts (strong)
//! * recorded against other content (a source or the requirement text changed), or not
//!   recorded at all -> inconclusive ("outdated", "unrecorded"); never current
//! * passed, but the proof is reported (bypasses the boundary, calls no indexed code, proves
//!   `True`, rests on an axiom) -> inconclusive ("disqualified")
//! * the test is in a file no crate target compiles -> inconclusive ("not_compiled")
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
use concordance_core::code::{FactKind, Symbol, SymbolKind};
use concordance_core::doc::{Authority, Claim, RequirementForm};
use concordance_core::evidence::{
    Dependency, Evidence, Method, MethodKind, Stance, Strength, StructuralCheck,
};
use concordance_core::finding::{
    EvidenceQuality, Finding, FindingCategory, FindingStatus, FindingSubject, Lifecycle, Severity,
};
use concordance_core::graph::{EdgeKind, NodeRef, Resolution};
use concordance_core::testrun::{RunBinding, TestOutcome};
use concordance_core::{ClaimId, SourceRange, SymbolId};
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
        let flaws = proof_flaws(model, t);
        let rules: Vec<String> = flaws.iter().map(|f| format!("`{}`", f.rule)).collect();
        let (outcome, run, stance, strength, summary) = if !model.test_is_compiled(t) {
            (
                "not_compiled".to_string(),
                String::new(),
                Stance::Inconclusive,
                Strength::Weak,
                format!(
                    "Test `{name}` declares it verifies `{}`, but its file `{}` is declared by no `mod` reachable from a crate target, so it is never compiled and no run can have executed it. A result line with the same name is not accepted for it.",
                    req.id,
                    t.range.path
                ),
            )
        } else {
            match model.test_result(t) {
                None => (
                    "not_run".to_string(),
                    String::new(),
                    Stance::Inconclusive,
                    Strength::Weak,
                    format!("Test `{name}` declares it verifies `{}`, but no ingested test run reports it.", req.id),
                ),
                Some((run, o)) if run.is_unrecorded() => (
                    "unrecorded".to_string(),
                    run.path.clone(),
                    Stance::Inconclusive,
                    Strength::Moderate,
                    format!(
                        "Test `{name}` was recorded as {} in `{}`, but that results file is not bound to the content it ran on: {} Until it is, the result verifies nothing.",
                        o.as_str(),
                        run.path,
                        match &run.binding {
                            RunBinding::Unrecorded(why) => why.as_str(),
                            RunBinding::Recorded => "",
                        }
                    ),
                ),
                Some((run, o)) if run.is_outdated() => (
                    "outdated".to_string(),
                    run.path.clone(),
                    Stance::Inconclusive,
                    Strength::Moderate,
                    format!(
                        "Test `{name}` was recorded as {} in `{}`, but {} indexed source file(s) changed after that run (e.g. `{}`); the result no longer describes the current code. Re-run the tests and record them again.",
                        o.as_str(),
                        run.path,
                        run.changed_sources,
                        run.changed_example.as_ref().map(|p| p.to_string()).unwrap_or_default()
                    ),
                ),
                Some((run, o)) if run.statement_changed(&req.id) => (
                    "outdated".to_string(),
                    run.path.clone(),
                    Stance::Inconclusive,
                    Strength::Moderate,
                    format!(
                        "Test `{name}` was recorded as {} in `{}`, but the text of requirement `{}` changed after that run; the result describes the old wording. Re-run the tests and record them again.",
                        o.as_str(),
                        run.path,
                        req.id
                    ),
                ),
                Some((run, TestOutcome::Passed)) if !flaws.is_empty() => (
                    "disqualified".to_string(),
                    run.path.clone(),
                    Stance::Inconclusive,
                    Strength::Moderate,
                    format!(
                        "Test `{name}`, which declares it verifies `{}`, passed in `{}`, but its proof is reported by {} ({}); a passing result of such a proof is not accepted as verification.",
                        req.id,
                        run.path,
                        rules.join(", "),
                        flaws.iter().map(|f| f.detail.as_str()).collect::<Vec<_>>().join("; ")
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
            }
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

/// Whether `s` reads `env!("CARGO_BIN_EXE_<name>")`: the path of the package's own binary,
/// which a test only needs in order to run it.
fn runs_own_binary(s: &Symbol) -> bool {
    s.facts
        .iter()
        .any(|f| f.kind == FactKind::EnvRead && f.detail.contains("CARGO_BIN_EXE_"))
}

pub fn proof_reach(model: &Model, proof: &Symbol) -> ProofReach {
    let mut reach = ProofReach {
        reaches_public: false,
        internals: Vec::new(),
    };
    // A proof that runs the package's own binary reaches its command-line boundary, which no
    // call edge can show.
    if runs_own_binary(proof) {
        reach.reaches_public = true;
    }
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
                    if runs_own_binary(target) {
                        reach.reaches_public = true;
                    }
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

/// A defect in a proof that stops its passing result from verifying the requirement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofFlaw {
    /// `bypasses_boundary`, `calls_nothing`, `trivial_statement` or `uses_axiom`.
    pub rule: &'static str,
    pub detail: String,
}

fn language_of<'a>(model: &'a Model, s: &Symbol) -> &'a str {
    model
        .files
        .get(&s.file)
        .map(|f| f.language.as_str())
        .unwrap_or("")
}

/// The statement a Lean declaration header states: the text after the first colon outside any
/// bracket (`theorem t (n : Nat) : n = n` -> `n = n`).
pub fn lean_statement(signature: &str) -> Option<String> {
    let chars: Vec<char> = signature.chars().collect();
    let mut depth = 0i32;
    for (i, c) in chars.iter().enumerate() {
        match c {
            '(' | '[' | '{' | '\u{27e8}' | '\u{2983}' | '\u{27e6}' | '\u{2039}' => depth += 1,
            ')' | ']' | '}' | '\u{27e9}' | '\u{2984}' | '\u{27e7}' | '\u{203a}' => depth -= 1,
            ':' if depth == 0 => {
                let next = chars.get(i + 1);
                let prev = i.checked_sub(1).and_then(|p| chars.get(p));
                if next == Some(&'=') || next == Some(&':') || prev == Some(&':') {
                    continue;
                }
                let rest: String = chars[i + 1..].iter().collect();
                let rest = rest.trim();
                let rest = rest.strip_suffix(":=").unwrap_or(rest).trim();
                return Some(rest.to_string());
            }
            _ => {}
        }
    }
    None
}

/// Whether a Lean statement is the proposition `True` (possibly parenthesized): any proof of it
/// proves nothing.
fn states_true(statement: &str) -> bool {
    let mut s = statement.trim();
    while let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        s = inner.trim();
    }
    s == "True"
}

/// Axioms declared in the repository that Lean theorem `proof` rests on, found by following the
/// names its text mentions through the definitions and theorems they name.
fn lean_axioms_used(model: &Model, proof: &Symbol) -> Vec<String> {
    let is_axiom = |s: &Symbol| {
        s.kind == SymbolKind::Const
            && s.signature
                .as_deref()
                .is_some_and(|g| g.starts_with("axiom"))
            && language_of(model, s) == "lean"
    };
    if !model.symbols.values().any(is_axiom) {
        return Vec::new();
    }
    let mut found: Vec<String> = Vec::new();
    let mut seen: std::collections::BTreeSet<SymbolId> = std::collections::BTreeSet::new();
    seen.insert(proof.id.clone());
    let mut queue = vec![proof.id.clone()];
    while let Some(id) = queue.pop() {
        let Some(sym) = model.symbols.get(&id) else {
            continue;
        };
        for fact in sym.facts.iter().filter(|f| f.kind == FactKind::Mention) {
            let name = fact.detail.as_str();
            let last = name.rsplit('.').next().unwrap_or(name);
            for cand in model.symbols_named(last) {
                if language_of(model, cand) != "lean" {
                    continue;
                }
                let full = cand.path.join(".");
                if full != name && !full.ends_with(&format!(".{name}")) {
                    continue;
                }
                if is_axiom(cand) {
                    let q = cand.qualified_name();
                    if !found.contains(&q) {
                        found.push(q);
                    }
                } else if seen.len() < 4000 && seen.insert(cand.id.clone()) {
                    queue.push(cand.id.clone());
                }
            }
        }
    }
    found.sort();
    found
}

/// The defects of `proof` (a symbol that declares `Verifies:`), judged from what the index can
/// see: which indexed code it calls, and for Lean what the theorem states and rests on.
pub fn proof_flaws(model: &Model, proof: &Symbol) -> Vec<ProofFlaw> {
    let mut out = Vec::new();
    match language_of(model, proof) {
        "lean" => {
            if proof
                .signature
                .as_deref()
                .and_then(lean_statement)
                .is_some_and(|s| states_true(&s))
            {
                out.push(ProofFlaw {
                    rule: "trivial_statement",
                    detail: "the theorem states `True`, which every proof establishes".to_string(),
                });
            }
            let axioms = lean_axioms_used(model, proof);
            if !axioms.is_empty() {
                out.push(ProofFlaw {
                    rule: "uses_axiom",
                    detail: format!(
                        "the theorem rests on the declared axiom{} {}",
                        if axioms.len() == 1 { "" } else { "s" },
                        axioms
                            .iter()
                            .take(5)
                            .map(|a| format!("`{a}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                });
            }
        }
        // Calls are matched reliably only in Rust: TypeScript and C# constructors and method
        // calls are not resolved, so a proof in those languages is not judged by its calls.
        "rust" => {
            let reach = proof_reach(model, proof);
            if reach.reaches_public {
                return out;
            }
            if reach.internals.is_empty() {
                out.push(ProofFlaw {
                    rule: "calls_nothing",
                    detail: "none of its calls resolves to indexed code".to_string(),
                });
            } else {
                let called: Vec<String> = reach
                    .internals
                    .iter()
                    .filter_map(|i| model.symbols.get(i))
                    .map(|s| format!("`{}`", s.qualified_name()))
                    .take(5)
                    .collect();
                out.push(ProofFlaw {
                    rule: "bypasses_boundary",
                    detail: format!(
                        "every call it makes that resolves to indexed code goes to a non-public symbol ({})",
                        called.join(", ")
                    ),
                });
            }
        }
        _ => {}
    }
    out
}

/// Proof-rule findings (`docs/spec-lint.md`): a requirement with more than one declaring
/// proof, and every flaw [`proof_flaws`] finds in a proof.
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
            for flaw in proof_flaws(model, p) {
                let subject = FindingSubject::ProofRule {
                    requirement: req.clone(),
                    rule: flaw.rule.into(),
                    symbol: Some(p.id.clone()),
                };
                let mut locations = spec_loc.clone();
                locations.push(p.range.clone());
                let pname = concordance_graph::truncate(&p.qualified_name(), 80);
                let (class, title, explanation) = match flaw.rule {
                    "bypasses_boundary" => (
                        (
                            FindingStatus::Violated,
                            FindingCategory::StructuralContradiction,
                            Severity::Medium,
                        ),
                        format!("Proof `{pname}` of `{req}` calls no public API"),
                        format!(
                            "`{}` declares `Verifies: {req}` but {}. A proof calls what the boundary of the requirement names and nothing private: a correct rewrite of the internals would break it. Its passing result is not accepted as verification.",
                            p.qualified_name(),
                            flaw.detail
                        ),
                    ),
                    "calls_nothing" => (
                        (
                            FindingStatus::Ambiguous,
                            FindingCategory::UnclearDocumentation,
                            Severity::Low,
                        ),
                        format!("Proof `{pname}` of `{req}` calls no indexed code"),
                        format!(
                            "`{}` declares `Verifies: {req}` but {}: it calls nothing the index can resolve, so nothing shows that it exercises the code the requirement is about (an empty test, or one that only calls other crates or runs a program Concordance cannot see). Its passing result is not accepted as verification.",
                            p.qualified_name(),
                            flaw.detail
                        ),
                    ),
                    "trivial_statement" => (
                        (
                            FindingStatus::Violated,
                            FindingCategory::StructuralContradiction,
                            Severity::Medium,
                        ),
                        format!("Proof `{pname}` of `{req}` proves `True`"),
                        format!(
                            "`{}` declares `Verifies: {req}` but {}: a proof of it says nothing about the requirement. Its passing result is not accepted as verification.",
                            p.qualified_name(),
                            flaw.detail
                        ),
                    ),
                    _ => (
                        (
                            FindingStatus::Ambiguous,
                            FindingCategory::UnclearDocumentation,
                            Severity::Low,
                        ),
                        format!("Proof `{pname}` of `{req}` rests on an axiom"),
                        format!(
                            "`{}` declares `Verifies: {req}` but {}. What it proves holds only if the axiom does; a build that completes does not show that. Its passing result is not accepted as verification.",
                            p.qualified_name(),
                            flaw.detail
                        ),
                    ),
                };
                out.push(proof_finding(
                    model,
                    ctx,
                    subject,
                    class,
                    title,
                    explanation,
                    (claim_ids.clone(), vec![p.id.clone()]),
                    locations,
                    authority,
                ));
            }
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
