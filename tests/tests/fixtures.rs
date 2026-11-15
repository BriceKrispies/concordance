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
