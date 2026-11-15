//! What `fixtures/spec-lint` shows beyond the lint cases: how attribute bullets are stored on
//! requirements, and the proof checks of `scan` (a `Verifies:` naming an unknown requirement,
//! a requirement with several proofs, a proof that calls no public symbol).
//!
//! The lint findings themselves are the case table in `fixtures/spec-lint/EXPECTED.md`, run
//! through the real binary by `crates/concordance-cli/tests/lint_proofs.rs`.

use concordance_core::config::{Config, ConfigFile, Overrides};
use concordance_core::finding::{Finding, FindingStatus, FindingSubject};
use concordance_tests::*;
use std::path::{Path, PathBuf};

fn config(repo: &Path, state: &Path) -> Config {
    let mut file = ConfigFile::default();
    file.authority.normative = vec!["docs/spec/**".into()];
    Config::resolve(
        file,
        None,
        Overrides {
            repo: Some(repo.to_path_buf()),
            spec_roots: vec![PathBuf::from("docs/spec")],
            src_roots: vec![PathBuf::from("src")],
            state_dir: Some(state.to_path_buf()),
            ..Default::default()
        },
        repo,
        state,
    )
    .unwrap()
}

async fn model_of(repo: &Path) -> (Config, concordance_graph::Model) {
    let state = temp_dir("spec-lint-state");
    let config = config(repo, &state);
    let r = start(config.clone(), None, 8192).await;
    r.idle().await;
    let model = r.engine.shared.read(|m| m.clone()).0;
    r.stop().await;
    (config, model)
}

fn proof_rule<'a>(
    m: &'a concordance_graph::Model,
    requirement: &str,
    rule: &str,
) -> Vec<&'a Finding> {
    m.findings
        .values()
        .filter(|f| {
            f.is_open()
                && matches!(&f.subject, FindingSubject::ProofRule { requirement: r, rule: k, .. }
                    if r == requirement && k == rule)
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scan_reports_unknown_verifies_duplicate_proofs_and_boundary_bypass() {
    let repo = fixture_copy("spec-lint");
    let (_, m) = model_of(&repo).await;

    // A Verifies naming a requirement that does not exist.
    let unknown: Vec<_> = m
        .findings
        .values()
        .filter(|f| {
            f.is_open()
                && matches!(&f.subject, FindingSubject::Verification { requirement, .. }
                    if requirement == "CLI-404")
        })
        .collect();
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].status, FindingStatus::Violated);

    // Two proofs declare CLI-001.
    let many = proof_rule(&m, "CLI-001", "multiple_proofs");
    assert_eq!(many.len(), 1);
    assert_eq!(many[0].symbols.len(), 2);
    assert_eq!(many[0].status, FindingStatus::Ambiguous);
    assert!(proof_rule(&m, "CLI-002", "multiple_proofs").is_empty());

    // The CLI-003 proof calls only a private function.
    let bypass = proof_rule(&m, "CLI-003", "bypasses_boundary");
    assert_eq!(bypass.len(), 1);
    assert_eq!(bypass[0].status, FindingStatus::Violated);
    assert!(
        bypass[0].explanation.contains("hash_manifest"),
        "{}",
        bypass[0].explanation
    );
    assert!(
        bypass[0].title.contains("hashing_is_stable"),
        "{}",
        bypass[0].title
    );
    // The CLI-002 proof calls `build_project`, which is public.
    assert!(proof_rule(&m, "CLI-002", "bypasses_boundary").is_empty());
    // Neither breach is reported for the two CLI-001 proofs, which call public API.
    assert!(proof_rule(&m, "CLI-001", "bypasses_boundary").is_empty());
}
