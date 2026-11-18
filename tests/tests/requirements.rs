//! Requirement IDs linked to tests: what the case tables of `fixtures/requirements` cannot
//! say. The link and its explanations, and a recorded pass that never outlives a source
//! change. The statuses a run produces (verified, violated, unverified) are rows of
//! `fixtures/requirements/EXPECTED.md`.

use concordance_core::config::{Config, ConfigFile, Overrides};
use concordance_core::finding::{Finding, FindingStatus, FindingSubject};
use concordance_core::graph::{EdgeKind, NodeRef};
use concordance_graph::Model;
use concordance_tests::*;
use std::path::{Path, PathBuf};

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_is_explained_and_a_recorded_pass_goes_stale_on_change() {
    let repo = fixture_copy("requirements");
    let state = temp_dir("requirements-state");
    let results = state.join("cargo-test.txt"); // outside the repository, not yet written
    let cfg = config(&repo, &state, &results);
    let r = start(cfg.clone(), None, 8192).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;

    // Linked, but no run yet.
    let f = requirement(&m, "LEX-001");
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
    assert!(
        f2.explanation
            .contains("no test that declares `Verifies: LEX-002`"),
        "{}",
        f2.explanation
    );

    // The run passes and is recorded against these sources; then the code changes: the old
    // pass no longer describes it.
    std::fs::write(&results, format!("test {TEST} ... ok\n")).unwrap();
    record_and_rescan(&r, &cfg).await;
    let m = r.engine.shared.read(|m| m.clone()).0;
    assert_eq!(requirement(&m, "LEX-001").status, FindingStatus::Verified);
    let lib = repo.join("src/lib.rs");
    let text = std::fs::read_to_string(&lib).unwrap();
    std::fs::write(
        &lib,
        text.replace("let mut depth = 0usize;", "let mut depth: usize = 0;"),
    )
    .unwrap();
    let m = rescan(&r).await;
    let f = requirement(&m, "LEX-001");
    assert_eq!(f.status, FindingStatus::Stale, "{}", f.explanation);
    assert!(
        f.explanation.contains("changed after that run"),
        "{}",
        f.explanation
    );

    r.stop().await;
}
