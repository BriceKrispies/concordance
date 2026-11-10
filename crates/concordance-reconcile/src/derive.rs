//! Evidence -> finding. This module is the single place where status is decided.
//!
//! Precedence for a claim (first match wins):
//! 1. valid structural contradiction of strength >= Strong -> `violated` (structural)
//! 2. a test declaring `Verifies: <requirement>` failed (one-statement requirement) -> `violated` (test failure)
//! 3. semantic `disagrees` upheld by an adversarial challenge -> `violated` (suspected semantic)
//! 4. semantic evidence invalidated by a change and not re-established -> `stale`
//! 5. semantic `ambiguous` verdict -> `ambiguous`
//! 6. a test declaring `Verifies: <requirement>` passed in a run recorded after the last
//!    source change (one-statement requirement) -> `verified`
//! 7. heuristic vague wording -> `ambiguous`
//! 8. semantic `agrees`, challenged and upheld (or unchallenged for low-impact claims) -> `verified`
//! 9. valid structural support of strength >= Strong covering the claim -> `verified`
//! 10. suggestive contradictions (moderate structural, unchallenged/refuted semantic) -> `unverified` (suspected)
//! 11. otherwise -> `unverified` (missing evidence)

use crate::text::spec_citation;
use crate::BatchContext;
use concordance_core::doc::{Authority, Claim, ClaimKind, Document, Modality};
use concordance_core::evidence::{Evidence, Method, MethodKind, Stance, Strength};
use concordance_core::finding::{
    EvidenceQuality, Finding, FindingCategory, FindingStatus, FindingSubject, Lifecycle, Proposal,
    ResolutionAction, Severity,
};
use concordance_core::graph::{Edge, EdgeKind, NodeRef, Resolution};
use concordance_core::{SourceRange, SymbolId};
use concordance_graph::Model;
use std::collections::BTreeSet;

/// Claims whose agreement verdicts must survive an adversarial challenge before
/// they are reported as verified, and whose contradictions are challenged.
pub fn is_high_impact(claim: &Claim) -> bool {
    claim.authority == Authority::Normative
        && matches!(claim.modality, Modality::Must | Modality::MustNot)
        && !matches!(claim.kind, ClaimKind::Behavior | ClaimKind::ApiContract)
}

/// Does structural analysis alone have anything to say about this claim's content?
fn structurally_checkable(claim: &Claim) -> bool {
    claim.documented_signature.is_some() || claim.constraint.is_some()
}

fn quality_of(s: Strength) -> EvidenceQuality {
    match s {
        Strength::Proven => EvidenceQuality::Proven,
        Strength::Strong => EvidenceQuality::Strong,
        Strength::Moderate => EvidenceQuality::Moderate,
        Strength::Weak => EvidenceQuality::Weak,
    }
}

fn verdict(e: &Evidence) -> Option<&str> {
    match &e.method {
        Method::Semantic { verdict, .. } | Method::Adversarial { verdict, .. } => {
            Some(verdict.as_str())
        }
        Method::Structural { .. } | Method::Test { .. } => None,
    }
}

fn latest<'a>(evs: impl Iterator<Item = &'a &'a Evidence>) -> Option<&'a Evidence> {
    evs.max_by_key(|e| (e.revision, e.created_at)).copied()
}

struct Decision {
    status: FindingStatus,
    category: FindingCategory,
    confidence: f32,
    quality: EvidenceQuality,
    headline: String,
}

/// Derive the reconciliation finding for an obligation claim. Lifecycle fields
/// (first_seen, introduced_in, ...) are filled by [`crate::lifecycle::merge`].
pub fn claim_finding(
    model: &Model,
    claim: &Claim,
    doc: &Document,
    evidence: &[&Evidence],
    edges: &[Edge],
    ctx: &BatchContext,
) -> Finding {
    let valid: Vec<&Evidence> = evidence.iter().copied().filter(|e| e.is_valid()).collect();
    let structural = |stance: Stance, min: Strength| -> Vec<&Evidence> {
        valid
            .iter()
            .copied()
            .filter(|e| {
                e.method.kind() == MethodKind::Structural && e.stance == stance && e.strength >= min
            })
            .collect()
    };
    let strong_contra = structural(Stance::Contradicts, Strength::Strong);
    let moderate_contra = structural(Stance::Contradicts, Strength::Moderate);
    let strong_support = structural(Stance::Supports, Strength::Strong);
    let tests = |stance: Stance| -> Vec<&Evidence> {
        valid
            .iter()
            .copied()
            .filter(|e| {
                e.method.kind() == MethodKind::Test
                    && e.stance == stance
                    && e.strength >= Strength::Strong
            })
            .collect()
    };
    let test_fail = tests(Stance::Contradicts);
    let test_pass = tests(Stance::Supports);
    let test_evidence: Vec<&Evidence> = valid
        .iter()
        .copied()
        .filter(|e| e.method.kind() == MethodKind::Test)
        .collect();
    let semantic = latest(
        valid
            .iter()
            .filter(|e| e.method.kind() == MethodKind::Semantic),
    );
    let challenge = latest(
        valid
            .iter()
            .filter(|e| e.method.kind() == MethodKind::Adversarial),
    );
    let stale_semantic: Vec<&Evidence> = evidence
        .iter()
        .copied()
        .filter(|e| !e.is_valid() && e.method.kind() != MethodKind::Structural)
        .collect();
    let vague = claim.ambiguity.iter().any(|a| {
        use concordance_core::doc::AmbiguityKind::*;
        matches!(a.kind, VagueTerm | MixedModality | UnresolvedSubject)
    });
    let high_impact = is_high_impact(claim);

    let sem_verdict = semantic.and_then(verdict);
    let ch_verdict = challenge.and_then(verdict);

    let d = if let Some(top) = strong_contra.iter().max_by_key(|e| e.strength) {
        Decision {
            status: FindingStatus::Violated,
            category: FindingCategory::StructuralContradiction,
            confidence: if top.strength == Strength::Proven {
                0.95
            } else {
                0.8
            },
            quality: quality_of(top.strength),
            headline: format!(
                "Structural analysis contradicts this claim: {}",
                top.summary
            ),
        }
    } else if let Some(t) = test_fail.first() {
        Decision {
            status: FindingStatus::Violated,
            category: FindingCategory::TestFailure,
            confidence: 0.9,
            quality: EvidenceQuality::Strong,
            headline: format!(
                "A test that verifies this requirement failed: {}",
                t.summary
            ),
        }
    } else if sem_verdict == Some("disagrees") && ch_verdict == Some("upheld") {
        Decision {
            status: FindingStatus::Violated,
            category: FindingCategory::SuspectedSemanticContradiction,
            confidence: 0.65,
            quality: EvidenceQuality::Moderate,
            headline: format!(
                "Semantic analysis found a likely contradiction, and an independent challenge did not refute it: {}",
                semantic.map(|e| e.summary.as_str()).unwrap_or_default()
            ),
        }
    } else if semantic.is_none() && !stale_semantic.is_empty() {
        Decision {
            status: FindingStatus::Stale,
            category: FindingCategory::InvalidatedEvidence,
            confidence: 0.5,
            quality: EvidenceQuality::Weak,
            headline: "Previously established semantic evidence was invalidated by a change and has not been re-established."
                .into(),
        }
    } else if sem_verdict == Some("ambiguous") {
        Decision {
            status: FindingStatus::Ambiguous,
            category: FindingCategory::UnclearDocumentation,
            confidence: 0.6,
            quality: EvidenceQuality::Moderate,
            headline: format!(
                "Semantic analysis found the claim open to multiple interpretations: {}",
                semantic.map(|e| e.summary.as_str()).unwrap_or_default()
            ),
        }
    } else if let Some(t) = test_pass.first() {
        Decision {
            status: FindingStatus::Verified,
            category: FindingCategory::Supported,
            confidence: 0.85,
            quality: EvidenceQuality::Strong,
            headline: if test_pass.len() > 1 {
                format!(
                    "{} tests that verify this requirement passed, e.g.: {}",
                    test_pass.len(),
                    t.summary
                )
            } else {
                format!(
                    "A test that verifies this requirement passed: {}",
                    t.summary
                )
            },
        }
    } else if vague
        && sem_verdict != Some("agrees")
        && sem_verdict != Some("disagrees")
        && !(structurally_checkable(claim) && !strong_support.is_empty())
    {
        Decision {
            status: FindingStatus::Ambiguous,
            category: FindingCategory::UnclearDocumentation,
            confidence: 0.5,
            quality: EvidenceQuality::Weak,
            headline: format!(
                "The claim uses wording that does not define a checkable obligation ({}).",
                claim
                    .ambiguity
                    .iter()
                    .map(|a| a.detail.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        }
    } else if sem_verdict == Some("agrees")
        && (ch_verdict == Some("upheld") || (challenge.is_none() && !high_impact))
    {
        let challenged = challenge.is_some();
        Decision {
            status: FindingStatus::Verified,
            category: FindingCategory::Supported,
            confidence: if challenged { 0.7 } else { 0.55 },
            quality: EvidenceQuality::Moderate,
            headline: format!(
                "Semantic analysis{} found the implementation consistent with this claim: {}",
                if challenged {
                    ", upheld by an independent challenge,"
                } else {
                    " (single pass)"
                },
                semantic.map(|e| e.summary.as_str()).unwrap_or_default()
            ),
        }
    } else if let Some(top) = strong_support
        .iter()
        .filter(|_| structurally_checkable(claim))
        .max_by_key(|e| e.strength)
    {
        Decision {
            status: FindingStatus::Verified,
            category: FindingCategory::Supported,
            confidence: if top.strength == Strength::Proven {
                0.95
            } else {
                0.8
            },
            quality: quality_of(top.strength),
            headline: format!("Structural analysis supports this claim: {}", top.summary),
        }
    } else if !moderate_contra.is_empty()
        || sem_verdict == Some("disagrees")
        || (sem_verdict == Some("agrees") && ch_verdict == Some("refuted"))
    {
        let why = if let Some(e) = moderate_contra.first() {
            e.summary.clone()
        } else if sem_verdict == Some("disagrees") {
            match ch_verdict {
                None => "semantic analysis suspects a contradiction; an adversarial challenge has not confirmed it yet".into(),
                Some(v) => format!("semantic analysis suspects a contradiction, but the adversarial challenge was {v}"),
            }
        } else {
            "an adversarial challenge found counter-evidence to a semantic agreement".into()
        };
        Decision {
            status: FindingStatus::Unverified,
            category: FindingCategory::SuspectedSemanticContradiction,
            confidence: 0.4,
            quality: EvidenceQuality::Moderate,
            headline: format!("Possible contradiction, not established: {why}."),
        }
    } else {
        let linked = edges
            .iter()
            .any(|e| e.kind == EdgeKind::ReferencesSymbol && e.to.is_some());
        let lexical = edges.iter().any(|e| e.kind == EdgeKind::MentionsSymbol);
        let test_note = test_evidence
            .iter()
            .max_by_key(|e| e.strength)
            .map(|e| e.summary.clone());
        let untested = claim
            .requirement
            .as_ref()
            .filter(|_| test_evidence.is_empty())
            .map(|r| {
                format!(
                    "Requirement `{}` has no test that declares `Verifies: {}`.",
                    r.id, r.id
                )
            });
        let headline = match (sem_verdict, linked, lexical) {
            _ if test_note.is_some() => test_note.unwrap_or_default(),
            _ if untested.is_some() => untested.unwrap_or_default(),
            (Some("insufficient_evidence"), _, _) => {
                "Semantic analysis could not establish compliance from the scoped code.".to_string()
            }
            (Some("agrees"), _, _) => {
                "Semantic analysis agrees, but this high-impact claim has not yet passed an adversarial challenge.".to_string()
            }
            (_, true, _) => "The claim is linked to implementation symbols, but no evidence establishes the claimed behavior.".to_string(),
            (_, false, true) => "Only name-similar implementation symbols were found; no evidence establishes the claimed behavior.".to_string(),
            _ => "No implementation evidence for this claim was found.".to_string(),
        };
        Decision {
            status: FindingStatus::Unverified,
            category: FindingCategory::MissingEvidence,
            confidence: 0.5,
            quality: if valid.is_empty() {
                EvidenceQuality::None
            } else {
                EvidenceQuality::Weak
            },
            headline,
        }
    };

    // Supporting / contradicting sets: valid evidence by stance.
    let supporting: Vec<_> = valid
        .iter()
        .filter(|e| e.stance == Stance::Supports)
        .map(|e| e.id.clone())
        .collect();
    let contradicting: Vec<_> = valid
        .iter()
        .filter(|e| e.stance == Stance::Contradicts)
        .map(|e| e.id.clone())
        .collect();

    let mut methods: Vec<MethodKind> = valid
        .iter()
        .map(|e| e.method.kind())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if methods.is_empty() {
        methods.push(MethodKind::Structural);
    }

    let mut limitations = Vec::new();
    if methods.contains(&MethodKind::Test) {
        limitations.push(
            "A passing test shows the cases it exercises; it does not prove the requirement for all inputs.".into(),
        );
    }
    if !methods.contains(&MethodKind::Semantic)
        && !structurally_checkable(claim)
        && !methods.contains(&MethodKind::Test)
    {
        limitations.push(if ctx.semantic_enabled {
            "Behavioral compliance has not been assessed by semantic analysis yet.".to_string()
        } else {
            "Semantic analysis is disabled: structural analysis cannot evaluate behavioral prose."
                .to_string()
        });
    }
    if methods.contains(&MethodKind::Semantic) {
        limitations.push(
            "Semantic results are model-assessed proposals validated against cited source lines, not proofs.".into(),
        );
    }
    if edges
        .iter()
        .all(|e| e.kind != EdgeKind::ReferencesSymbol || e.to.is_none())
        && edges.iter().any(|e| e.kind == EdgeKind::MentionsSymbol)
    {
        limitations.push("Implementation was linked only by name similarity.".into());
    }
    let cfgs: BTreeSet<&String> = edges.iter().flat_map(|e| e.cfg.iter()).collect();
    if !cfgs.is_empty() {
        limitations.push(format!(
            "Linked code is conditionally compiled ({}); the result holds only for configurations that include it.",
            cfgs.into_iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if valid
        .iter()
        .any(|e| matches!(e.method, Method::Structural { .. }) && e.summary.contains("syntactic"))
    {
        limitations.push("Syntactic facts do not account for reachability or data flow.".into());
    }

    // Symbols and locations: explicit links first, then evidence citations.
    let mut symbols: Vec<SymbolId> = Vec::new();
    for e in edges
        .iter()
        .filter(|e| e.kind == EdgeKind::ReferencesSymbol || e.kind == EdgeKind::ConstrainsScope)
    {
        if let Some(NodeRef::Symbol(id)) = &e.to {
            if !symbols.contains(id) {
                symbols.push(id.clone());
            }
        }
    }
    let mut locations: Vec<SourceRange> = vec![claim.range.clone()];
    for ev in valid
        .iter()
        .filter(|e| e.stance != Stance::Inconclusive)
        .chain(valid.iter().filter(|e| e.stance == Stance::Inconclusive))
    {
        for c in &ev.citations {
            if !locations.contains(&c.range) && locations.len() < 12 {
                locations.push(c.range.clone());
            }
            if let Some(NodeRef::Symbol(id)) = &c.node {
                if !symbols.contains(id) {
                    symbols.push(id.clone());
                }
            }
        }
    }
    let dependencies: Vec<NodeRef> = valid
        .iter()
        .flat_map(|e| e.dependencies.iter().map(|d| d.node.clone()))
        .chain(std::iter::once(NodeRef::Claim(claim.id.clone())))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(200)
        .collect();

    let mut explanation = d.headline.clone();
    let detail: Vec<String> = valid
        .iter()
        .filter(|e| e.stance == Stance::Contradicts && !d.headline.contains(e.summary.as_str()))
        .map(|e| format!("- {}", e.summary))
        .collect();
    if !detail.is_empty() {
        explanation.push_str("\n\nContradicting evidence:\n");
        explanation.push_str(&detail.join("\n"));
    }
    explanation.push_str(&format!(
        "\n\nWhy it matters: {}",
        why_it_matters(claim, doc)
    ));

    let severity = severity_for(d.status, d.category, claim.authority);
    let (crate_name, module) = locate(model, &symbols);
    let mut targets = vec![claim.range.clone()];
    targets.extend(
        locations
            .iter()
            .skip(1)
            .filter(|r| r.path != claim.range.path)
            .take(3)
            .cloned(),
    );

    Finding {
        id: FindingSubject::Claim {
            claim: claim.id.clone(),
        }
        .finding_id(),
        subject: FindingSubject::Claim {
            claim: claim.id.clone(),
        },
        status: d.status,
        category: d.category,
        severity,
        title: title_for(d.status, d.category, claim),
        explanation,
        claims: vec![claim.id.clone()],
        symbols,
        documents: vec![doc.id.clone()],
        locations,
        supporting,
        contradicting,
        confidence: d.confidence,
        quality: d.quality,
        methods,
        limitations,
        dependencies,
        proposals: proposals_for(
            d.status,
            d.category,
            claim.authority,
            targets,
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
    }
}

fn why_it_matters(claim: &Claim, doc: &Document) -> String {
    match claim.authority {
        Authority::Normative => format!(
            "`{}` is normative ({}); it defines intended behavior, so disagreement means either the code or the intent must change by explicit decision.",
            doc.path, doc.authority.rationale
        ),
        Authority::Descriptive => format!(
            "`{}` is descriptive ({}); readers rely on it to describe current behavior.",
            doc.path, doc.authority.rationale
        ),
        Authority::Informative => "The passage is informative (example or background) and imposes no obligation.".into(),
        Authority::Unknown => format!(
            "The authority of `{}` could not be established; it is unclear whether the document or the code is meant to be authoritative.",
            doc.path
        ),
    }
}

pub fn locate(model: &Model, symbols: &[SymbolId]) -> (Option<String>, Option<String>) {
    let Some(s) = symbols.iter().find_map(|id| model.symbols.get(id)) else {
        return (None, None);
    };
    let krate = model
        .files
        .get(&s.file)
        .and_then(|f| f.crate_name.clone())
        .or_else(|| s.path.first().cloned());
    let module = model
        .files
        .get(&s.file)
        .map(|f| f.module_path.join("::"))
        .filter(|m| !m.is_empty());
    (krate, module)
}

pub fn severity_for(
    status: FindingStatus,
    category: FindingCategory,
    authority: Authority,
) -> Severity {
    match (status, authority) {
        (FindingStatus::Violated, Authority::Normative) => Severity::High,
        (FindingStatus::Violated, _) => Severity::Medium,
        (FindingStatus::Stale, _) => Severity::Medium,
        (FindingStatus::Ambiguous, Authority::Normative) => Severity::Medium,
        (FindingStatus::Ambiguous, _) => Severity::Low,
        (FindingStatus::Unverified, _)
            if category == FindingCategory::SuspectedSemanticContradiction =>
        {
            Severity::Medium
        }
        (FindingStatus::Undocumented, _) => Severity::Low,
        _ => Severity::Info,
    }
}

fn title_for(status: FindingStatus, category: FindingCategory, claim: &Claim) -> String {
    let short = concordance_graph::truncate(&claim.text, 90);
    let prefix = match (status, category) {
        (FindingStatus::Violated, FindingCategory::StructuralContradiction) => "Contradicted",
        (FindingStatus::Violated, _) => "Likely violated",
        (FindingStatus::Stale, _) => "Evidence stale",
        (FindingStatus::Ambiguous, _) => "Ambiguous",
        (FindingStatus::Verified, _) => "Verified",
        (FindingStatus::Unverified, FindingCategory::SuspectedSemanticContradiction) => {
            "Possible contradiction"
        }
        _ => "Unverified",
    };
    format!("{prefix}: {short}")
}

pub fn proposals_for(
    status: FindingStatus,
    category: FindingCategory,
    authority: Authority,
    targets: Vec<SourceRange>,
    semantic_enabled: bool,
) -> Vec<Proposal> {
    let spec_target: Vec<SourceRange> = targets.iter().take(1).cloned().collect();
    let code_targets: Vec<SourceRange> = targets.iter().skip(1).cloned().collect();
    let p = |action, rationale: &str, requires_decision, t: &Vec<SourceRange>| Proposal {
        action,
        rationale: rationale.to_string(),
        requires_decision,
        targets: t.clone(),
    };
    use ResolutionAction::*;
    match status {
        FindingStatus::Violated => match authority {
            Authority::Normative => vec![
                p(ChangeImplementation, "The specification is normative: change the implementation to satisfy it.", false, &code_targets),
                p(AcceptBehaviorChange, "If the implemented behavior is now intended, change the specification by explicit decision.", true, &spec_target),
            ],
            Authority::Descriptive => vec![
                p(UpdateDescriptiveDocumentation, "The document is descriptive: update it to reflect the verified implementation.", false, &spec_target),
                p(ChangeImplementation, "If the documented behavior is the intended one, change the implementation instead.", true, &code_targets),
            ],
            _ => vec![
                p(ClarifySpecification, "Establish whether this document is normative before choosing which side to change.", true, &spec_target),
                p(ChangeImplementation, "If the document is authoritative, change the implementation.", true, &code_targets),
                p(UpdateDescriptiveDocumentation, "If the code is authoritative, update the document.", true, &spec_target),
            ],
        },
        FindingStatus::Stale => vec![p(
            GatherStrongerEvidence,
            if semantic_enabled { "Re-run semantic analysis against the current revision." } else { "Enable semantic analysis to re-establish the invalidated evidence." },
            false,
            &targets,
        )],
        FindingStatus::Ambiguous => vec![p(ClarifySpecification, "Rewrite the statement as a single checkable obligation (concrete values, one modality).", authority == Authority::Normative, &spec_target)],
        FindingStatus::Unverified if category == FindingCategory::SuspectedSemanticContradiction => vec![
            p(GatherStrongerEvidence, "Confirm or refute the suspected contradiction (adversarial challenge, a test that exercises the behavior).", false, &targets),
        ],
        FindingStatus::Unverified => vec![p(
            GatherStrongerEvidence,
            if semantic_enabled { "Link the claim to its implementation explicitly (code spans naming the symbols) or add a test that demonstrates it." } else { "Enable semantic analysis, or add a test that demonstrates the behavior." },
            false,
            &targets,
        )],
        FindingStatus::Undocumented => vec![p(EstablishSpecification, "Specify the intended behavior so it can be checked.", true, &targets)],
        FindingStatus::Verified => vec![p(NoActionNeeded, "Evidence supports the claim.", false, &[].to_vec())],
    }
}

/// Finding for a dangling reference in non-claim prose.
pub fn prose_finding(
    model: &Model,
    doc: &Document,
    subject: FindingSubject,
    ev: &Evidence,
    ctx: &BatchContext,
) -> Finding {
    let text = match &subject {
        FindingSubject::ProseReference { text, .. } => text.clone(),
        _ => String::new(),
    };
    let severity = match (&ev.method, doc.authority.level) {
        (
            Method::Structural {
                check: concordance_core::evidence::StructuralCheck::LinkResolution,
            },
            _,
        ) => Severity::Low,
        (_, Authority::Informative) => Severity::Low,
        _ => Severity::Medium,
    };
    let targets: Vec<SourceRange> = ev.citations.iter().map(|c| c.range.clone()).collect();
    let mut proposals = vec![Proposal {
        action: ResolutionAction::UpdateDescriptiveDocumentation,
        rationale: "Update or remove the reference so it names something that exists.".into(),
        requires_decision: doc.authority.level == Authority::Normative,
        targets: targets.clone(),
    }];
    if doc.authority.level == Authority::Normative {
        proposals.push(Proposal {
            action: ResolutionAction::ChangeImplementation,
            rationale: "If the normative document is right, restore the referenced item.".into(),
            requires_decision: true,
            targets: vec![],
        });
    }
    let _ = model;
    Finding {
        id: subject.finding_id(),
        subject,
        status: FindingStatus::Violated,
        category: FindingCategory::StructuralContradiction,
        severity,
        title: format!("Broken reference `{}` in {}", concordance_graph::truncate(&text, 60), doc.path),
        explanation: format!("{}\n\nWhy it matters: documentation that names things which do not exist misleads readers and hides drift.", ev.summary),
        claims: vec![],
        symbols: vec![],
        documents: vec![doc.id.clone()],
        locations: targets,
        supporting: vec![],
        contradicting: vec![ev.id.clone()],
        confidence: 0.95,
        quality: quality_of(ev.strength),
        methods: vec![MethodKind::Structural],
        limitations: vec![],
        dependencies: ev.dependencies.iter().map(|d| d.node.clone()).collect(),
        proposals,
        revision: ctx.revision,
        introduced_in: ctx.revision,
        introduced_by: ctx.changed_files.clone(),
        first_seen: ctx.now,
        last_updated: ctx.now,
        lifecycle: Lifecycle::Open,
        crate_name: None,
        module: None,
    }
}

/// Pairs of obligation claims stating incompatible numeric bounds on the same subject.
pub fn conflict_findings(model: &Model, ctx: &BatchContext) -> Vec<(Finding, Evidence)> {
    use concordance_core::doc::BoundOp::*;
    let stop = [
        "the", "must", "shall", "should", "not", "never", "hold", "holds", "have", "has", "be",
        "is", "are", "a", "an", "of", "at", "most", "least",
    ];
    let words = |s: &str| -> BTreeSet<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .map(str::to_lowercase)
            .filter(|w| w.len() >= 3 && !stop.contains(&w.as_str()))
            .collect()
    };
    let bounded: Vec<&Claim> = model
        .claims
        .values()
        .filter(|c| c.is_obligation() && !c.bounds.is_empty())
        .collect();
    let mut out = Vec::new();
    for (i, a) in bounded.iter().enumerate() {
        for b in &bounded[i + 1..] {
            for ba in &a.bounds {
                for bb in &b.bounds {
                    let same_dir = matches!(
                        (ba.op, bb.op),
                        (Le | Lt, Le | Lt) | (Ge | Gt, Ge | Gt) | (Eq, Eq)
                    );
                    if !same_dir || ba.value == bb.value || ba.unit != bb.unit {
                        continue;
                    }
                    let shared_symbol = a.references.iter().any(|r| {
                        b.references.iter().any(|q| {
                            q.target == r.target
                                && matches!(
                                    r.target,
                                    concordance_core::doc::ReferenceTarget::Symbol { .. }
                                )
                        })
                    });
                    let shared_words = !words(&ba.subject).is_disjoint(&words(&bb.subject));
                    if !(shared_symbol || shared_words) {
                        continue;
                    }
                    let subject = FindingSubject::ClaimConflict {
                        claims: vec![a.id.clone(), b.id.clone()],
                    };
                    let ev = crate::text::EvidenceBuilder {
                        ctx,
                        subject: NodeRef::Claim(a.id.clone()),
                        method: Method::Structural {
                            check: concordance_core::evidence::StructuralCheck::ConflictingClaims,
                        },
                    }
                    .build(
                        &format!("conflict:{}", b.id),
                        Stance::Inconclusive,
                        Strength::Moderate,
                        format!("\"{}\" conflicts with \"{}\".", ba.text, bb.text),
                        vec![spec_citation(a, None), spec_citation(b, None)],
                        vec![
                            concordance_core::evidence::Dependency {
                                node: NodeRef::Claim(a.id.clone()),
                                content_hash: a.content_hash.clone(),
                            },
                            concordance_core::evidence::Dependency {
                                node: NodeRef::Claim(b.id.clone()),
                                content_hash: b.content_hash.clone(),
                            },
                        ],
                    );
                    let normative =
                        a.authority == Authority::Normative || b.authority == Authority::Normative;
                    let f = Finding {
                        id: subject.finding_id(),
                        subject,
                        status: FindingStatus::Ambiguous,
                        category: FindingCategory::UnclearDocumentation,
                        severity: if normative { Severity::Medium } else { Severity::Low },
                        title: format!("Conflicting requirements: \"{}\" vs \"{}\"", ba.text, bb.text),
                        explanation: format!(
                            "Two obligations state different bounds for what appears to be the same subject: \"{}\" ({}) and \"{}\" ({}). At most one can be the intended behavior.\n\nWhy it matters: implementation cannot satisfy both, and compliance cannot be judged until the specification is clarified.",
                            a.text, a.range, b.text, b.range
                        ),
                        claims: vec![a.id.clone(), b.id.clone()],
                        symbols: vec![],
                        documents: vec![a.document.clone(), b.document.clone()].into_iter().collect::<BTreeSet<_>>().into_iter().collect(),
                        locations: vec![a.range.clone(), b.range.clone()],
                        supporting: vec![],
                        contradicting: vec![ev.id.clone()],
                        confidence: if shared_symbol { 0.8 } else { 0.55 },
                        quality: EvidenceQuality::Moderate,
                        methods: vec![MethodKind::Structural],
                        limitations: if shared_symbol { vec![] } else { vec!["Subjects were matched by shared wording; they may refer to different things.".into()] },
                        dependencies: vec![NodeRef::Claim(a.id.clone()), NodeRef::Claim(b.id.clone())],
                        proposals: proposals_for(FindingStatus::Ambiguous, FindingCategory::UnclearDocumentation, if normative { Authority::Normative } else { Authority::Unknown }, vec![a.range.clone(), b.range.clone()], ctx.semantic_enabled),
                        revision: ctx.revision,
                        introduced_in: ctx.revision,
                        introduced_by: ctx.changed_files.clone(),
                        first_seen: ctx.now,
                        last_updated: ctx.now,
                        lifecycle: Lifecycle::Open,
                        crate_name: None,
                        module: None,
                    };
                    out.push((f, ev));
                }
            }
        }
    }
    out
}

/// Resolution label helper for UI/explanations.
pub fn resolution_label(r: &Resolution) -> &'static str {
    match r {
        Resolution::Exact => "exact",
        Resolution::UniqueSuffix => "unique suffix",
        Resolution::Ambiguous { .. } => "ambiguous",
        Resolution::Lexical => "lexical",
        Resolution::Unresolved => "unresolved",
    }
}
