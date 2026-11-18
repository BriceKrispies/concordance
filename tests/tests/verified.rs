//! What `verified` establishes (docs/requirements.md, "What verified establishes"): a passing
//! test result counts only when it is bound to the content it was run on, matched with the
//! crate that ran it, and produced by a proof that exercises the code. Every test here has a
//! contrasting half that must verify, so the check cannot pass by refusing everything.

use concordance_core::config::{Config, ConfigFile, Overrides};
use concordance_core::finding::{Finding, FindingStatus, FindingSubject};
use concordance_graph::Model;
use concordance_tests::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

const TEST: &str = "tests::nested_comment_ends_at_the_balancing_close";

fn config(repo: &Path, state: &Path, results: &Path, src_roots: &[&str]) -> Config {
    Config::resolve(
        ConfigFile::default(),
        None,
        Overrides {
            repo: Some(repo.to_path_buf()),
            spec_roots: vec![PathBuf::from("docs")],
            src_roots: src_roots.iter().map(PathBuf::from).collect(),
            state_dir: Some(state.to_path_buf()),
            test_results: vec![results.to_path_buf()],
            ..Default::default()
        },
        repo,
        state,
    )
    .unwrap()
}

fn model(r: &Running) -> Model {
    r.engine.shared.read(|m| m.clone()).0
}

async fn rescan(r: &Running) -> Model {
    r.engine.request_scan(false).await;
    r.idle().await;
    model(r)
}

/// The open finding of the claim tagged `id`.
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

fn proof_rules<'a>(m: &'a Model, requirement: &str, rule: &str) -> Vec<&'a Finding> {
    m.findings
        .values()
        .filter(|f| {
            f.is_open()
                && matches!(&f.subject, FindingSubject::ProofRule { requirement: r, rule: k, .. }
                    if r == requirement && k == rule)
        })
        .collect()
}

/// A copy of `fixtures/requirements` with `results` as its recorded run, indexed but not recorded.
async fn lexer(results: &str) -> (PathBuf, Config, Running) {
    let repo = fixture_copy("requirements");
    let state = temp_dir("verified-state");
    let file = state.join("cargo-test.txt");
    std::fs::write(&file, results).unwrap();
    let cfg = config(&repo, &state, &file, &["src"]);
    let r = start(cfg.clone(), None, 8192).await;
    r.idle().await;
    (repo, cfg, r)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_results_file_with_no_manifest_verifies_nothing() {
    let (_repo, cfg, r) = lexer(&format!("test {TEST} ... ok\n")).await;
    let m = model(&r);
    let f = requirement(&m, "LEX-001");
    assert_eq!(f.status, FindingStatus::Stale, "{}", f.explanation);
    assert!(f.explanation.contains("no manifest"), "{}", f.explanation);

    // Recording binds the same results to the sources they were run on.
    record_and_rescan(&r, &cfg).await;
    let m = model(&r);
    let f = requirement(&m, "LEX-001");
    assert_eq!(f.status, FindingStatus::Verified, "{}", f.explanation);
    assert!(cfg.test_results[0]
        .with_file_name("cargo-test.txt.manifest.json")
        .exists());
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn editing_the_requirement_text_after_the_run_makes_it_stale() {
    let (repo, cfg, r) = lexer(&format!("test {TEST} ... ok\n")).await;
    record_and_rescan(&r, &cfg).await;
    assert_eq!(
        requirement(&model(&r), "LEX-001").status,
        FindingStatus::Verified
    );

    let spec = repo.join("docs/spec.md");
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(&spec, text.replace("SHALL end at", "SHALL end exactly at")).unwrap();
    let m = rescan(&r).await;
    let f = requirement(&m, "LEX-001");
    assert_eq!(f.status, FindingStatus::Stale, "{}", f.explanation);
    assert!(
        f.explanation
            .contains("text of requirement `LEX-001` changed"),
        "{}",
        f.explanation
    );

    // Recording the same results against the new wording is the user saying the run covers it.
    record_and_rescan(&r, &cfg).await;
    assert_eq!(
        requirement(&model(&r), "LEX-001").status,
        FindingStatus::Verified
    );
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_source_touched_without_a_change_keeps_the_recorded_result() {
    let (repo, cfg, r) = lexer(&format!("test {TEST} ... ok\n")).await;
    record_and_rescan(&r, &cfg).await;

    // A newer modification time with the same bytes is not a change.
    tokio::time::sleep(Duration::from_millis(40)).await;
    let lib = repo.join("src/lib.rs");
    let text = std::fs::read(&lib).unwrap();
    std::fs::write(&lib, text).unwrap();
    let m = rescan(&r).await;
    let f = requirement(&m, "LEX-001");
    assert_eq!(f.status, FindingStatus::Verified, "{}", f.explanation);
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_results_file_rewritten_after_recording_is_not_trusted() {
    let (_repo, cfg, r) = lexer(&format!("test {TEST} ... ok\n")).await;
    record_and_rescan(&r, &cfg).await;

    // Another run overwrites the file; its manifest still describes the first.
    let mut text = std::fs::read_to_string(&cfg.test_results[0]).unwrap();
    text.push_str("test result: ok. 1 passed\n");
    std::fs::write(&cfg.test_results[0], text).unwrap();
    let m = rescan(&r).await;
    let f = requirement(&m, "LEX-001");
    assert_eq!(f.status, FindingStatus::Stale, "{}", f.explanation);
    assert!(
        f.explanation.contains("results file changed"),
        "{}",
        f.explanation
    );
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_test_in_a_file_no_module_declares_verifies_nothing() {
    let repo = fixture_copy("requirements");
    write(
        &repo.join("src/orphan.rs"),
        "/// Verifies: LEX-002\n#[test]\nfn orphan_test() {\n    assert_eq!(crate::block_comment_len(\"/**/\"), Some(4));\n}\n",
    );
    let state = temp_dir("verified-state");
    let file = state.join("cargo-test.txt");
    std::fs::write(&file, "test orphan_test ... ok\n").unwrap();
    let cfg = config(&repo, &state, &file, &["src"]);
    let r = start(cfg.clone(), None, 8192).await;
    r.idle().await;
    record_and_rescan(&r, &cfg).await;
    let m = model(&r);
    let f = requirement(&m, "LEX-002");
    assert_ne!(f.status, FindingStatus::Verified, "{}", f.explanation);
    assert!(
        f.explanation.contains("never compiled"),
        "{}",
        f.explanation
    );

    // Declaring the module compiles the file, and the same result line now applies.
    let lib = repo.join("src/lib.rs");
    let text = std::fs::read_to_string(&lib).unwrap();
    std::fs::write(&lib, format!("{text}\nmod orphan;\n")).unwrap();
    record_and_rescan(&r, &cfg).await;
    let m = model(&r);
    let f = requirement(&m, "LEX-002");
    assert_eq!(f.status, FindingStatus::Verified, "{}", f.explanation);
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_result_line_of_another_crate_does_not_verify_a_test() {
    let status = |results: String| async move {
        let (_repo, cfg, r) = lexer(&results).await;
        record_and_rescan(&r, &cfg).await;
        let m = model(&r);
        let f = requirement(&m, "LEX-001").clone();
        r.stop().await;
        f
    };
    // libtest: the `Running` line names the target the lines below it came from.
    let own = status(format!(
        "Running unittests src/lib.rs (target/debug/deps/lexer-0123456789abcdef)\ntest {TEST} ... ok\n"
    ))
    .await;
    assert_eq!(own.status, FindingStatus::Verified, "{}", own.explanation);
    let other = status(format!(
        "Running unittests crates/other/src/lib.rs (target/debug/deps/other-0123456789abcdef)\ntest {TEST} ... ok\n"
    ))
    .await;
    assert_ne!(
        other.status,
        FindingStatus::Verified,
        "{}",
        other.explanation
    );
    assert!(
        other
            .explanation
            .contains("no ingested test run reports it"),
        "{}",
        other.explanation
    );
    // nextest: the first segment of the binary id is the package.
    let own = status(format!("        PASS [   0.004s] lexer {TEST}\n")).await;
    assert_eq!(own.status, FindingStatus::Verified, "{}", own.explanation);
    let other = status(format!("        PASS [   0.004s] other {TEST}\n")).await;
    assert_ne!(
        other.status,
        FindingStatus::Verified,
        "{}",
        other.explanation
    );
}

/// A crate with one public function and three tests of it.
fn demo_repo() -> PathBuf {
    let repo = temp_dir("verified-demo").join("repo");
    write(
        &repo.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    );
    write(
        &repo.join("docs/spec.md"),
        "# Demo\n\nThis specification is normative.\n\n**DEM-001** The demo SHALL add one to its input.\n\n**DEM-002** The demo SHALL never overflow on zero.\n\n**DEM-003** The demo SHALL add one twice.\n",
    );
    write(
        &repo.join("src/lib.rs"),
        r#"pub fn api(x: u32) -> u32 {
    helper(x)
}

fn helper(x: u32) -> u32 {
    x + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies: DEM-001
    #[test]
    fn bypasses() {
        assert_eq!(helper(1), 2);
    }

    /// Verifies: DEM-002
    #[test]
    fn calls_nothing() {
        assert!(true);
    }

    /// Verifies: DEM-003
    #[test]
    fn drives_the_boundary() {
        assert_eq!(api(api(1)), 3);
    }
}
"#,
    );
    repo
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_passing_proof_that_exercises_no_public_code_does_not_verify() {
    let repo = demo_repo();
    let state = temp_dir("verified-state");
    let file = state.join("cargo-test.txt");
    std::fs::write(
        &file,
        "test tests::bypasses ... ok\ntest tests::calls_nothing ... ok\ntest tests::drives_the_boundary ... ok\n",
    )
    .unwrap();
    let cfg = config(&repo, &state, &file, &["src"]);
    let r = start(cfg.clone(), None, 8192).await;
    r.idle().await;
    record_and_rescan(&r, &cfg).await;
    let m = model(&r);

    // The proof that calls only a private function is reported, and its pass is not accepted.
    let f = requirement(&m, "DEM-001");
    assert_eq!(f.status, FindingStatus::Unverified, "{}", f.explanation);
    assert!(
        f.explanation.contains("bypasses_boundary"),
        "{}",
        f.explanation
    );
    assert_eq!(proof_rules(&m, "DEM-001", "bypasses_boundary").len(), 1);

    // A proof with no call that resolves to indexed code is reported too.
    let f = requirement(&m, "DEM-002");
    assert_eq!(f.status, FindingStatus::Unverified, "{}", f.explanation);
    assert!(f.explanation.contains("calls_nothing"), "{}", f.explanation);
    let rules = proof_rules(&m, "DEM-002", "calls_nothing");
    assert_eq!(rules.len(), 1);
    assert_ne!(rules[0].status, FindingStatus::Verified);

    // The proof that calls the public function is verified, and has no proof-rule finding.
    let f = requirement(&m, "DEM-003");
    assert_eq!(f.status, FindingStatus::Verified, "{}", f.explanation);
    assert!(proof_rules(&m, "DEM-003", "calls_nothing").is_empty());
    assert!(proof_rules(&m, "DEM-003", "bypasses_boundary").is_empty());
    r.stop().await;
}

/// A copy of the Lean fixture with a module of two theorems that prove nothing, and `log` as
/// the Lake build log.
async fn lean(log: &str) -> (Config, Running) {
    let repo = fixture_copy("lean");
    let spec = repo.join("docs/spec.md");
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(
        &spec,
        format!("{text}\n**VOL-003** The mixer SHALL keep a trivial promise.\n\n**VOL-004** The mixer SHALL rest on its axioms.\n"),
    )
    .unwrap();
    write(
        &repo.join("Volume/Extra.lean"),
        "namespace Volume\n\naxiom bogus_axiom : 1 = 2\n\n/-- Verifies: VOL-003 -/\ntheorem trivial_claim : True := trivial\n\n/-- Verifies: VOL-004 -/\ntheorem axiom_claim : 1 = 2 := bogus_axiom\n\nend Volume\n",
    );
    let state = temp_dir("verified-lean-state");
    let file = state.join("lake-build.log");
    std::fs::write(&file, log).unwrap();
    let cfg = config(&repo, &state, &file, &["Volume"]);
    let r = start(cfg.clone(), None, 8192).await;
    r.idle().await;
    (cfg, r)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lean_log_verifies_only_the_modules_it_shows_built() {
    // The build completed, but the log never shows `Volume.Mixer` built.
    let (cfg, r) = lean("✔ [1/2] Built Volume\nBuild completed successfully.\n").await;
    record_and_rescan(&r, &cfg).await;
    let m = model(&r);
    let f = requirement(&m, "VOL-001");
    assert_ne!(f.status, FindingStatus::Verified, "{}", f.explanation);
    r.stop().await;

    // Naming the module verifies its theorem.
    let (cfg, r) = lean("✔ [1/2] Built Volume.Mixer\nBuild completed successfully.\n").await;
    record_and_rescan(&r, &cfg).await;
    let m = model(&r);
    let f = requirement(&m, "VOL-001");
    assert_eq!(f.status, FindingStatus::Verified, "{}", f.explanation);
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lean_theorem_stating_true_or_resting_on_an_axiom_is_reported() {
    let (cfg, r) = lean(
        "✔ [1/3] Built Volume.Extra\n✔ [2/3] Built Volume.Mixer\nBuild completed successfully.\n",
    )
    .await;
    record_and_rescan(&r, &cfg).await;
    let m = model(&r);

    // The theorem that is built and sound still verifies.
    assert_eq!(requirement(&m, "VOL-001").status, FindingStatus::Verified);

    let f = requirement(&m, "VOL-003");
    assert_eq!(f.status, FindingStatus::Unverified, "{}", f.explanation);
    assert!(
        f.explanation.contains("trivial_statement"),
        "{}",
        f.explanation
    );
    assert_eq!(proof_rules(&m, "VOL-003", "trivial_statement").len(), 1);

    let f = requirement(&m, "VOL-004");
    assert_eq!(f.status, FindingStatus::Unverified, "{}", f.explanation);
    assert!(f.explanation.contains("bogus_axiom"), "{}", f.explanation);
    assert_eq!(proof_rules(&m, "VOL-004", "uses_axiom").len(), 1);
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bounds_that_only_share_a_unit_are_not_in_conflict() {
    let repo = temp_dir("verified-conflicts").join("repo");
    write(&repo.join("src/lib.rs"), "pub fn x() {}\n");
    write(
        &repo.join("docs/spec.md"),
        "# Limits\n\nThis specification is normative.\n\n## Retention\n\nThe source database of a moved tenant SHALL be kept for at least 7 days after its placement switch.\n\nA delivery version SHALL remain served for at least 90 days after its replacement is published.\n\n## Latency\n\nA directory entry SHALL be served for at most 15 minutes after the control plane was last reached.\n\nAn access token SHALL be valid for at most 5 minutes.\n\n## Availability\n\nThe delivery of published content SHALL answer at least 99.95 percent of its requests over each window of 30 days.\n\nThe authoring application and the sign-in pages SHALL answer at least 99.9 percent of their requests over each window of 30 days.\n\n## Capacity\n\nThe upload queue MUST hold at most 100 jobs.\n\nThe upload queue MUST hold at most 500 jobs.\n",
    );
    let state = temp_dir("verified-conflicts-state");
    let r = start(config_for(&repo, &state, false), None, 8192).await;
    r.idle().await;
    let m = model(&r);
    let conflicts: Vec<&Finding> = m
        .findings
        .values()
        .filter(|f| f.is_open() && matches!(f.subject, FindingSubject::ClaimConflict { .. }))
        .collect();
    // Only the two bounds on the upload queue are about the same thing.
    assert_eq!(
        conflicts.len(),
        1,
        "{:#?}",
        conflicts.iter().map(|f| &f.title).collect::<Vec<_>>()
    );
    assert!(
        conflicts[0].explanation.contains("upload queue"),
        "{}",
        conflicts[0].explanation
    );
    r.stop().await;
}
