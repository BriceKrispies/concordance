//! A register-style specification (tables quoting other sources) over TypeScript:
//! structural checks must not turn table wording into whole-codebase contradictions.

use concordance_core::finding::{Finding, FindingCategory, FindingStatus, FindingSubject};
use concordance_graph::Model;
use concordance_tests::*;

fn finding<'a>(m: &'a Model, needle: &str) -> Option<&'a Finding> {
    let claim = m
        .claims
        .values()
        .find(|c| c.text.contains(needle))
        .unwrap_or_else(|| panic!("no claim containing {needle:?}"));
    m.findings.values().find(|f| {
        f.is_open()
            && f.subject
                == FindingSubject::Claim {
                    claim: claim.id.clone(),
                }
    })
}

fn status(m: &Model, needle: &str) -> Option<(FindingStatus, FindingCategory)> {
    finding(m, needle).map(|f| (f.status, f.category))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_tables_do_not_misfire() {
    let repo = fixture_copy("table-registers");
    let state = temp_dir("table-registers-state");
    let r = start(config_for(&repo, &state, false), None, 8192).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;

    let never_throws = status(&m, "The client never throws");
    assert_ne!(
        never_throws.map(|s| s.0),
        Some(FindingStatus::Violated),
        "an unscoped prose row is not a whole-codebase no-throw constraint"
    );
    assert_ne!(
        never_throws.map(|s| s.0),
        Some(FindingStatus::Ambiguous),
        "the quoted Summary cell is not graded for vagueness"
    );
    assert_eq!(
        status(&m, "load `placeholder()` output quickly").map(|s| s.0),
        Some(FindingStatus::Ambiguous),
        "vague wording in the author's own column is still reported"
    );
    assert_ne!(
        status(&m, "cannot connect to the control database").map(|s| s.0),
        Some(FindingStatus::Ambiguous),
        "the `May do` column label is not a weak qualifier"
    );
    assert_ne!(
        status(&m, "find_widget_key").map(|s| s.0),
        Some(FindingStatus::Violated),
        "a call on a value cannot prove the routine is missing"
    );
    assert!(
        !m.claims
            .values()
            .flat_map(|c| &c.references)
            .any(|r| r.text == "rate(1 hour)"),
        "a schedule expression is not a code reference"
    );
    assert_eq!(
        status(&m, "`RemoteWidgetTable`"),
        Some((
            FindingStatus::Unverified,
            FindingCategory::SuspectedStructuralContradiction
        )),
        "with semantic analysis off, a suspicion is labelled structural"
    );

    // Exports of test files are not public API.
    let undocumented: Vec<&str> = m
        .findings
        .values()
        .filter(|f| f.is_open() && f.status == FindingStatus::Undocumented)
        .map(|f| f.title.as_str())
        .collect();
    assert!(
        undocumented.iter().any(|t| t.contains("loadWidget")),
        "production exports are still reported: {undocumented:?}"
    );
    assert!(
        !undocumented
            .iter()
            .any(|t| t.contains("failingSource") || t.contains("makeWidget")),
        "test helpers are not public API: {undocumented:?}"
    );

    // True positives stay violated.
    assert_eq!(
        status(&m, "`loadWidget()` MUST NOT throw").map(|s| s.0),
        Some(FindingStatus::Violated)
    );
    assert_eq!(
        status(&m, "`renderAll()` MUST").map(|s| s.0),
        Some(FindingStatus::Violated)
    );
    r.stop().await;
}
