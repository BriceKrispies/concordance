//! Structural reconciliation over the fixture repositories: the right findings, with
//! the right statuses and exact locations, and nothing mislabeled.

use concordance_core::finding::{Finding, FindingCategory, FindingStatus};
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
async fn structural_drift_is_detected_with_exact_locations() {
    let (m, r) = scan("structural-drift").await;

    let renamed = find(&m, "parse_headers()");
    assert_eq!(renamed.status, FindingStatus::Violated);
    assert_eq!(renamed.category, FindingCategory::StructuralContradiction);
    assert_eq!(renamed.locations[0].path.as_str(), "docs/spec.md");
    let ev = renamed
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

    let removed = find(&m, "Header::from_bytes()");
    assert_eq!(removed.status, FindingStatus::Violated);

    // Outdated documented signature: points at both the spec block and the real fn.
    let sig = find(&m, "pub fn encode");
    assert_eq!(sig.status, FindingStatus::Violated);
    assert!(
        sig.locations
            .iter()
            .any(|l| l.path.as_str() == "src/lib.rs"),
        "{:?}",
        sig.locations
    );
    let text =
        std::fs::read_to_string(r.engine.shared.config.repo_root.join("src/lib.rs")).unwrap();
    let encode_line = text
        .lines()
        .position(|l| l.contains("pub fn encode"))
        .unwrap() as u32
        + 1;
    let code_loc = sig
        .locations
        .iter()
        .find(|l| l.path.as_str() == "src/lib.rs")
        .unwrap();
    assert!(
        code_loc.start_line <= encode_line && encode_line <= code_loc.end_line,
        "code location {code_loc} covers line {encode_line}"
    );

    // Broken links and file references in prose.
    assert_eq!(
        find(&m, "wire-format.md#framing").status,
        FindingStatus::Violated
    );
    assert_eq!(find(&m, "notes.md").status, FindingStatus::Violated);
    assert_eq!(find(&m, "src/codec.rs").status, FindingStatus::Violated);

    // Nothing is verified in structural mode except checkable claims, and no behavioral
    // claim is called violated without structural proof.
    for f in open(&m) {
        if f.status == FindingStatus::Violated {
            assert_eq!(
                f.category,
                FindingCategory::StructuralContradiction,
                "{}",
                f.title
            );
            assert!(
                !f.contradicting.is_empty(),
                "violated finding has contradicting evidence: {}",
                f.title
            );
        }
    }
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn consistent_fixture_verifies_checkable_claims_and_never_fakes_behavioral_verification() {
    let (m, r) = scan("consistent").await;
    let open = open(&m);
    assert!(
        open.iter().all(|f| f.status != FindingStatus::Violated),
        "no violations: {:#?}",
        open.iter()
            .filter(|f| f.status == FindingStatus::Violated)
            .map(|f| &f.title)
            .collect::<Vec<_>>()
    );
    let verified: Vec<&&Finding> = open
        .iter()
        .filter(|f| f.status == FindingStatus::Verified)
        .collect();
    assert!(
        verified.iter().any(|f| f.title.contains("try_acquire")),
        "documented signature verified"
    );
    assert!(
        verified.iter().any(|f| f.title.contains("unsafe")),
        "no-unsafe constraint verified structurally"
    );
    // Behavioral claims stay unverified without semantic analysis.
    let behavioral = find(&m, "must return `false`");
    assert_eq!(behavioral.status, FindingStatus::Unverified);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn undocumented_fixture_proposes_specification_gaps() {
    let (m, r) = scan("undocumented").await;
    let gaps: Vec<&Finding> = open(&m)
        .into_iter()
        .filter(|f| f.status == FindingStatus::Undocumented)
        .collect();
    assert!(gaps
        .iter()
        .any(|f| f.title.contains("panics") && f.title.contains("shutdown")));
    assert!(gaps
        .iter()
        .any(|f| f.title.contains("errors") && f.title.contains("load_config")));
    assert!(gaps
        .iter()
        .any(|f| f.title.contains("configuration") && f.title.contains("load_config")));
    for g in &gaps {
        assert_eq!(g.category, FindingCategory::MissingDocumentation);
        assert!(g.locations.iter().all(|l| l.path.as_str() == "src/lib.rs"));
    }
    r.stop().await;
}
