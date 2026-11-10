//! Headless report: machine-readable JSON and human-readable text from one model.

use concordance_core::finding::{Counts, Finding, FindingCategory, FindingStatus};
use concordance_core::{RepoPath, Revision, SourceRange};
use concordance_engine::SemanticStatus;
use concordance_graph::Model;
use serde::Serialize;
use std::fmt::Write as _;

/// Which findings make `--fail-on` exit non-zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum FailOn {
    /// Never fail on findings (default).
    None,
    /// Deterministic contradictions only: structural facts and failing tests.
    Proven,
    /// Any `violated` finding, including semantic contradictions that survived an adversarial challenge.
    Violated,
}

#[derive(Serialize)]
pub struct ClaimRef {
    pub id: String,
    pub text: String,
    pub document: Option<RepoPath>,
    pub range: SourceRange,
    pub authority: concordance_core::doc::Authority,
}

#[derive(Serialize)]
pub struct SymbolRef {
    pub id: String,
    pub path: String,
    pub range: SourceRange,
}

#[derive(Serialize)]
pub struct ReportFinding {
    #[serde(flatten)]
    pub finding: Finding,
    pub claim_refs: Vec<ClaimRef>,
    pub symbol_refs: Vec<SymbolRef>,
}

#[derive(Serialize)]
pub struct Report {
    pub tool: &'static str,
    pub version: &'static str,
    pub repository: String,
    pub revision: Revision,
    pub semantic: SemanticSummary,
    pub counts: Counts,
    pub gate: Gate,
    pub findings: Vec<ReportFinding>,
}

#[derive(Serialize)]
pub struct SemanticSummary {
    pub enabled: bool,
    pub available: Option<bool>,
    pub model: String,
    pub detail: Option<String>,
    /// Plain statement of what was and was not assessed.
    pub coverage: String,
}

#[derive(Serialize)]
pub struct Gate {
    pub fail_on: String,
    pub failed: bool,
    pub proven_violations: usize,
    pub suspected_violations: usize,
}

pub fn build(
    m: &Model,
    repo: &str,
    semantic: &SemanticStatus,
    fail_on: FailOn,
    include_resolved: bool,
) -> Report {
    let mut findings: Vec<&Finding> = m
        .findings
        .values()
        .filter(|f| include_resolved || f.is_open())
        .collect();
    findings.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(a.status.cmp(&b.status))
            .then(a.id.cmp(&b.id))
    });
    let open_violated = |cat: Option<FindingCategory>| {
        m.findings
            .values()
            .filter(|f| {
                f.is_open()
                    && f.status == FindingStatus::Violated
                    && cat.is_none_or(|c| f.category == c)
            })
            .count()
    };
    // Deterministic contradictions: structural facts and failing tests.
    let proven = open_violated(Some(FindingCategory::StructuralContradiction))
        + open_violated(Some(FindingCategory::TestFailure));
    let suspected = open_violated(Some(FindingCategory::SuspectedSemanticContradiction));
    let failed = match fail_on {
        FailOn::None => false,
        FailOn::Proven => proven > 0,
        FailOn::Violated => proven + suspected > 0,
    };
    let semantic_active = semantic.enabled && semantic.available == Some(true);
    let coverage = if semantic_active {
        "Structural checks ran on every claim; semantic analysis ran on behavioral claims with linked implementation (within the per-scan budget). Semantic results are model-assessed, validated against cited lines, and are not proofs.".to_string()
    } else if semantic.enabled {
        format!("Semantic analysis was requested but unavailable ({}); only structural checks ran. Behavioral claims remain unverified.", semantic.detail.clone().unwrap_or_default())
    } else {
        "Structural mode: only deterministic checks ran (references, signatures, links, constraints). Behavioral claims cannot be verified in this mode and are reported as unverified, never as verified.".to_string()
    };
    Report {
        tool: "concordance",
        version: env!("CARGO_PKG_VERSION"),
        repository: repo.to_string(),
        revision: m.revision,
        semantic: SemanticSummary {
            enabled: semantic.enabled,
            available: semantic.available,
            model: semantic.model.clone(),
            detail: semantic.detail.clone(),
            coverage,
        },
        counts: m.counts(),
        gate: Gate {
            fail_on: format!("{fail_on:?}").to_lowercase(),
            failed,
            proven_violations: proven,
            suspected_violations: suspected,
        },
        findings: findings
            .into_iter()
            .map(|f| ReportFinding {
                claim_refs: f
                    .claims
                    .iter()
                    .filter_map(|c| m.claims.get(c))
                    .map(|c| ClaimRef {
                        id: c.id.to_string(),
                        text: c.text.clone(),
                        document: m.documents.get(&c.document).map(|d| d.path.clone()),
                        range: c.range.clone(),
                        authority: c.authority,
                    })
                    .collect(),
                symbol_refs: f
                    .symbols
                    .iter()
                    .filter_map(|s| m.symbols.get(s))
                    .map(|s| SymbolRef {
                        id: s.id.to_string(),
                        path: s.qualified_name(),
                        range: s.range.clone(),
                    })
                    .collect(),
                finding: f.clone(),
            })
            .collect(),
    }
}

pub fn human(r: &Report, verbose: bool) -> String {
    let mut out = String::new();
    let c = &r.counts;
    let _ = writeln!(
        out,
        "Concordance {} — {} (revision {})",
        r.version, r.repository, r.revision.0
    );
    let _ = writeln!(
        out,
        "{} documents, {} claims ({} obligations), {} source files, {} symbols, {} relationships",
        c.documents, c.claims, c.obligation_claims, c.source_files, c.symbols, c.edges
    );
    let _ = writeln!(out, "\n  violated {:>5}   stale {:>5}   ambiguous {:>5}   unverified {:>5}   verified {:>5}   undocumented {:>5}", c.violated, c.stale, c.ambiguous, c.unverified, c.verified, c.undocumented);
    let _ = writeln!(out, "\n{}\n", r.semantic.coverage);
    let order = [
        FindingStatus::Violated,
        FindingStatus::Stale,
        FindingStatus::Ambiguous,
        FindingStatus::Undocumented,
        FindingStatus::Unverified,
        FindingStatus::Verified,
    ];
    for status in order {
        let items: Vec<&ReportFinding> = r
            .findings
            .iter()
            .filter(|f| f.finding.status == status)
            .collect();
        if items.is_empty() {
            continue;
        }
        let show_all = verbose
            || matches!(
                status,
                FindingStatus::Violated | FindingStatus::Stale | FindingStatus::Ambiguous
            );
        let _ = writeln!(out, "{} ({})", status.as_str().to_uppercase(), items.len());
        let limit = if show_all { usize::MAX } else { 10 };
        for f in items.iter().take(limit) {
            let loc = f
                .finding
                .locations
                .first()
                .map(|l| l.to_string())
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "  [{}] {}",
                f.finding.severity.as_str(),
                f.finding.title
            );
            let _ = writeln!(
                out,
                "        at {loc}  ({:?}, confidence {:.2})",
                f.finding.category, f.finding.confidence
            );
            if show_all && matches!(status, FindingStatus::Violated | FindingStatus::Stale) {
                for line in f.finding.explanation.lines().take(4) {
                    if !line.trim().is_empty() {
                        let _ = writeln!(out, "        {line}");
                    }
                }
                for l in f.finding.locations.iter().skip(1).take(3) {
                    let _ = writeln!(out, "        see {l}");
                }
            }
        }
        if items.len() > limit {
            let _ = writeln!(
                out,
                "  … {} more (use --verbose or --json)",
                items.len() - limit
            );
        }
        let _ = writeln!(out);
    }
    if r.gate.fail_on != "none" {
        let _ = writeln!(
            out,
            "Gate --fail-on {}: {} ({} proven, {} suspected violations)",
            r.gate.fail_on,
            if r.gate.failed { "FAILED" } else { "passed" },
            r.gate.proven_violations,
            r.gate.suspected_violations
        );
    }
    out
}
