//! The `ambiguous` fixture. The structural fixtures (`structural-drift`, `consistent`,
//! `undocumented`) are checked by the LANG-001 case tables in their `EXPECTED.md`.

use concordance_core::finding::{Finding, FindingStatus};
use concordance_tests::*;

async fn scan(name: &str) -> (concordance_graph::Model, Running) {
    let repo = fixture_copy(name);
    let state = temp_dir(&format!("{name}-state"));
    let r = start(config_for(&repo, &state, false), None, 8192).await;
    r.idle().await;
    let (m, _) = r.engine.shared.read(|m| m.clone());
    (m, r)
}

fn open(m: &concordance_graph::Model) -> Vec<&Finding> {
    m.findings.values().filter(|f| f.is_open()).collect()
}

fn find<'a>(m: &'a concordance_graph::Model, needle: &str) -> &'a Finding {
    open(m)
        .into_iter()
        .find(|f| f.title.contains(needle))
        .unwrap_or_else(|| {
            panic!(
                "no finding containing {needle:?}; have: {:#?}",
                open(m).iter().map(|f| &f.title).collect::<Vec<_>>()
            )
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ambiguous_fixture_records_uncertainty_instead_of_failures() {
    let (m, r) = scan("ambiguous").await;
    let conflict = find(&m, "Conflicting requirements");
    assert_eq!(conflict.status, FindingStatus::Ambiguous);
    assert_eq!(conflict.claims.len(), 2);
    assert_eq!(find(&m, "must be fast").status, FindingStatus::Ambiguous);
    assert_eq!(find(&m, "timely manner").status, FindingStatus::Ambiguous);
    assert!(open(&m)
        .iter()
        .all(|f| f.status != FindingStatus::Violated && f.status != FindingStatus::Verified));
    r.stop().await;
}

/// What the `structural-drift` case rows cannot say: the evidence behind the findings.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structural_drift_findings_carry_their_evidence() {
    let (m, r) = scan("structural-drift").await;
    let ev = find(&m, "parse_headers()")
        .contradicting
        .iter()
        .filter_map(|id| m.evidence.get(id))
        .next()
        .expect("contradicting evidence");
    assert!(
        ev.summary.contains("parse_header"),
        "rename candidate suggested: {}",
        ev.summary
    );
    for f in open(&m)
        .iter()
        .filter(|f| f.status == FindingStatus::Violated)
    {
        assert!(
            !f.contradicting.is_empty(),
            "violated finding has contradicting evidence: {}",
            f.title
        );
    }
    r.stop().await;
}

/// What the `consistent` case rows cannot say: why a behavioural claim stays unverified.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn behavioural_claims_without_semantic_analysis_say_so() {
    let (m, r) = scan("consistent").await;
    let behavioral = find(&m, "must return `false`");
    assert!(behavioral
        .limitations
        .iter()
        .any(|l| l.contains("Semantic analysis is disabled")));
    // Supporting evidence exists for the claim (linkage), recorded as inconclusive.
    assert!(m
        .evidence_for(&concordance_core::graph::NodeRef::Claim(
            behavioral.claims[0].clone()
        ))
        .any(|e| e.is_valid()));
    r.stop().await;
}
