//! Requirement IDs linked to tests, and recorded test outcomes as evidence: the status
//! follows the run (passed, failed), never outlives a source change, and a test that
//! verifies an undefined requirement is reported.

use concordance_core::config::{Config, ConfigFile, Overrides};
use concordance_core::evidence::MethodKind;
use concordance_core::finding::{Finding, FindingCategory, FindingStatus, FindingSubject};
use concordance_core::graph::{EdgeKind, NodeRef};
use concordance_graph::Model;
use concordance_tests::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

const TEST: &str = "tests::nested_comment_ends_at_the_balancing_close";

fn config(repo: &Path, state: &Path, results: &Path) -> Config {
    Config::resolve(
        ConfigFile::default(),
        None,
        Overrides {
            repo: Some(repo.to_path_buf()),
            spec_roots: vec![PathBuf::from("docs")],
            src_roots: vec![PathBuf::from("src")],
            state_dir: Some(state.to_path_buf()),
            test_results: vec![results.to_path_buf()],
            ..Default::default()
        },
        repo,
        state,
    )
    .unwrap()
}

fn requirement<'a>(m: &'a Model, id: &str) -> &'a Finding {
    let claim = m
        .claims
        .values()
        .find(|c| c.requirement.as_ref().is_some_and(|r| r.id == id))
        .unwrap_or_else(|| panic!("no claim tagged {id}"));
    m.findings
        .values()
        .find(|f| {
            f.is_open()
                && f.subject
                    == FindingSubject::Claim {
                        claim: claim.id.clone(),
                    }
        })
        .unwrap_or_else(|| panic!("no finding for {id}"))
}

async fn rescan(r: &Running) -> Model {
    r.engine.request_scan(false).await;
    r.idle().await;
    r.engine.shared.read(|m| m.clone()).0
}

/// Write `text` as the run output, strictly after every source modification.
async fn record(results: &Path, text: &str) {
    tokio::time::sleep(Duration::from_millis(30)).await;
    std::fs::write(results, text).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_outcomes_drive_requirement_status_and_go_stale_on_change() {
    let repo = fixture_copy("requirements");
    let state = temp_dir("requirements-state");
    let results = state.join("cargo-test.txt"); // outside the repository, not yet written
    let r = start(config(&repo, &state, &results), None, 8192).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;

    // Linked, but no run yet.
    let f = requirement(&m, "LEX-001");
    assert_eq!(f.status, FindingStatus::Unverified, "{}", f.explanation);
    assert!(
        f.explanation.contains("no ingested test run reports it"),
        "{}",
        f.explanation
    );
    let claim = m
        .claims
        .values()
        .find(|c| c.text.contains("balances"))
        .unwrap();
    let edge = m
        .edges_from(&NodeRef::Claim(claim.id.clone()))
        .find(|e| e.kind == EdgeKind::VerifiedBy)
        .expect("LEX-001 is linked to its test");
    let NodeRef::Symbol(t) = edge.to.clone().unwrap() else {
        panic!()
    };
    assert!(m.symbols[&t].qualified_name().ends_with(TEST));

    // A requirement nothing verifies says so.
    let f2 = requirement(&m, "LEX-002");
    assert_eq!(f2.status, FindingStatus::Unverified);
    assert!(
        f2.explanation
            .contains("no test that declares `Verifies: LEX-002`"),
        "{}",
        f2.explanation
    );

    // A test verifying an undefined requirement is a broken reference.
    let bad = m
        .findings
        .values()
        .find(|f| matches!(&f.subject, FindingSubject::Verification { requirement, .. } if requirement == "LEX-404"))
        .expect("finding for Verifies: LEX-404");
    assert_eq!(bad.status, FindingStatus::Violated);

    // Passed in a run recorded after the sources: verified by the test.
    record(
        &results,
        &format!("running 2 tests\ntest {TEST} ... ok\ntest result: ok.\n"),
    )
    .await;
    let m = rescan(&r).await;
    let f = requirement(&m, "LEX-001");
    assert_eq!(f.status, FindingStatus::Verified, "{}", f.explanation);
    assert_eq!(f.category, FindingCategory::Supported);
    assert!(f.methods.contains(&MethodKind::Test));
    assert_eq!(
        requirement(&m, "LEX-002").status,
        FindingStatus::Unverified,
        "an unrelated pass verifies nothing else"
    );

    // Failed: violated, as a test failure.
    record(&results, &format!("test {TEST} ... FAILED\n")).await;
    let m = rescan(&r).await;
    let f = requirement(&m, "LEX-001");
    assert_eq!(f.status, FindingStatus::Violated, "{}", f.explanation);
    assert_eq!(f.category, FindingCategory::TestFailure);

    // Passing again, then the code changes: the old pass no longer describes it.
    record(&results, &format!("test {TEST} ... ok\n")).await;
    assert_eq!(
        requirement(&rescan(&r).await, "LEX-001").status,
        FindingStatus::Verified
    );
    tokio::time::sleep(Duration::from_millis(30)).await;
    let lib = repo.join("src/lib.rs");
    let text = std::fs::read_to_string(&lib).unwrap();
    std::fs::write(
        &lib,
        text.replace("let mut depth = 0usize;", "let mut depth: usize = 0;"),
    )
    .unwrap();
    let m = rescan(&r).await;
    let f = requirement(&m, "LEX-001");
    assert_eq!(f.status, FindingStatus::Unverified, "{}", f.explanation);
    assert!(
        f.explanation.contains("changed after that run"),
        "{}",
        f.explanation
    );

    r.stop().await;
}
