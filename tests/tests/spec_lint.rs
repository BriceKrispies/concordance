//! The strict specification lint (`concordance lint`) over `fixtures/spec-lint`, and the
//! proof checks of `scan`: a `Verifies:` naming an unknown requirement, a requirement with
//! several proofs, a proof that calls no public symbol.
//!
//! The fixture holds a white paper, a conforming specification (`docs/spec/cli.md`) and one
//! violating specification per rule under `violations/`. Each case copies the fixture,
//! adds one violating file to `docs/spec` (and a row for it to the spec index) and lints.

use concordance_core::config::{Config, ConfigFile, Overrides};
use concordance_core::finding::{Finding, FindingStatus, FindingSubject};
use concordance_lint::{LintReport, Severity};
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

/// A copy of the fixture with `violations/<name>.md` added as `docs/spec/<name>.md`,
/// owned by `prefix` in the spec index.
fn with_violation(name: &str, prefix: &str) -> PathBuf {
    let repo = fixture_copy("spec-lint");
    std::fs::copy(
        repo.join("violations").join(format!("{name}.md")),
        repo.join("docs/spec").join(format!("{name}.md")),
    )
    .unwrap();
    let index = repo.join("docs/spec/index.md");
    let mut text = std::fs::read_to_string(&index).unwrap();
    text.push_str(&format!("| [{name}.md]({name}.md) | {prefix} | x |\n"));
    std::fs::write(index, text).unwrap();
    repo
}

async fn lint(repo: &Path, strict: bool) -> LintReport {
    let (config, model) = model_of(repo).await;
    concordance_lint::lint_repo(&model, &config, strict)
}

/// `(rule, line, is_error)` of every finding, all in `path`.
fn found(report: &LintReport, path: &str) -> Vec<(&'static str, u32, bool)> {
    for f in &report.findings {
        assert_eq!(f.path, path, "{f:?}");
        assert!(!f.message.is_empty());
    }
    report
        .findings
        .iter()
        .map(|f| (f.rule, f.line, f.severity == Severity::Error))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_conforming_specification_passes_even_strict() {
    let repo = fixture_copy("spec-lint");
    for strict in [false, true] {
        let report = lint(&repo, strict).await;
        assert!(report.findings.is_empty(), "{}", report.human());
        assert!(!report.failed());
    }
}

/// Every rule: the violating file gives exactly these findings (rule, line, error?), the
/// lint fails, and `--strict` makes every finding an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_rule_reports_one_finding_per_violation_at_its_line() {
    // (file, owning prefix, expected findings)
    type Expected = &'static [(&'static str, u32, bool)];
    let cases: &[(&str, &str, Expected)] = &[
        (
            "r1_atomic",
            "ATOM",
            &[
                ("R1", 3, true),
                ("R1", 8, true),
                ("R1", 13, true),
                ("R1", 18, true),
            ],
        ),
        ("r2_vague", "VAGU", &[("R2", 3, true), ("R2", 8, true)]),
        (
            "r3_internals",
            "INTR",
            &[("R3", 3, true), ("R3", 8, false), ("R3", 13, false)],
        ),
        (
            "r4_provable",
            "PROV",
            &[
                ("R4", 3, true),
                ("R4", 8, true),
                ("R4", 13, true),
                ("R4", 18, true),
                ("R4", 23, true),
            ],
        ),
        (
            "r5_traceable",
            "TRAC",
            &[
                ("R5", 3, true),
                ("R5", 10, true),
                ("R5", 12, true),
                ("R5", 18, true),
            ],
        ),
        ("r6_duplicate", "DUPL", &[("R6", 3, false)]),
        (
            "r7_modal",
            "MODL",
            &[("R7", 3, true), ("R7", 5, true), ("R7", 8, true)],
        ),
        (
            "r8_ids",
            "IDNT",
            &[
                ("R8", 3, true),
                ("R8", 8, true),
                ("R8", 18, true),
                ("R8", 23, true),
            ],
        ),
    ];
    for (name, prefix, expected) in cases {
        let repo = with_violation(name, prefix);
        let path = format!("docs/spec/{name}.md");

        let report = lint(&repo, false).await;
        assert_eq!(
            &found(&report, &path),
            expected,
            "{name}\n{}",
            report.human()
        );
        let has_error = expected.iter().any(|e| e.2);
        assert_eq!(report.failed(), has_error, "{name}: exit status");
        assert_eq!(report.errors, expected.iter().filter(|e| e.2).count());
        assert_eq!(report.warnings, expected.iter().filter(|e| !e.2).count());

        let strict = lint(&repo, true).await;
        assert!(strict.failed(), "{name}: strict must fail");
        assert_eq!(strict.errors, expected.len(), "{name}: strict");
        assert_eq!(strict.warnings, 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn warnings_alone_fail_only_in_strict_mode() {
    let repo = with_violation("r6_duplicate", "DUPL");
    let lax = lint(&repo, false).await;
    assert_eq!((lax.errors, lax.warnings), (0, 1), "{}", lax.human());
    assert!(!lax.failed());
    assert!(lint(&repo, true).await.failed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_goal_without_a_requirement_is_reported_at_the_white_paper() {
    let repo = fixture_copy("spec-lint");
    std::fs::copy(
        repo.join("violations/whitepaper_uncovered.md"),
        repo.join("docs/whitepaper.md"),
    )
    .unwrap();
    let report = lint(&repo, false).await;
    assert_eq!(
        found(&report, "docs/whitepaper.md"),
        vec![("R5", 9, true)],
        "{}",
        report.human()
    );
    assert!(report.findings[0].message.contains("G-3"));
    assert!(report.failed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_white_paper_is_one_finding_not_a_cascade() {
    let repo = fixture_copy("spec-lint");
    std::fs::remove_file(repo.join("docs/whitepaper.md")).unwrap();
    let report = lint(&repo, false).await;
    assert_eq!(
        found(&report, "docs/whitepaper.md"),
        vec![("R5", 1, true)],
        "{}",
        report.human()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_json_report_carries_rule_severity_message_and_location() {
    let repo = with_violation("r3_internals", "INTR");
    let report = lint(&repo, false).await;
    let v = serde_json::to_value(&report).unwrap();
    assert_eq!(v["command"], "lint");
    assert_eq!(v["strict"], false);
    assert_eq!(v["errors"], 1);
    assert_eq!(v["warnings"], 2);
    let first = &v["findings"][0];
    assert_eq!(first["rule"], "R3");
    assert_eq!(first["severity"], "error");
    assert_eq!(first["path"], "docs/spec/r3_internals.md");
    assert_eq!(first["line"], 3);
    assert!(first["message"].as_str().unwrap().contains("hash_manifest"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attribute_bullets_belong_to_the_requirement_and_are_not_claims() {
    let repo = fixture_copy("spec-lint");
    let (_, model) = model_of(&repo).await;
    let claim = model
        .claims
        .values()
        .find(|c| c.requirement.as_ref().is_some_and(|r| r.id == "CLI-001"))
        .expect("a claim tagged CLI-001");
    let attrs = &claim.requirement.as_ref().unwrap().attributes;
    assert_eq!(attrs.verify.as_deref(), Some("scenario"));
    assert_eq!(attrs.boundary.as_deref(), Some("B-1 `tool build`"));
    assert_eq!(attrs.traces.as_deref(), Some("G-2"));
    assert!(attrs
        .rationale
        .as_deref()
        .unwrap()
        .starts_with("A partial build"));
    let depends = model
        .claims
        .values()
        .find(|c| c.requirement.as_ref().is_some_and(|r| r.id == "CLI-003"))
        .unwrap();
    let r = depends.requirement.as_ref().unwrap();
    assert_eq!(r.attributes.depends.as_deref(), Some("CLI-002"));
    let retired = model
        .claims
        .values()
        .find(|c| c.requirement.as_ref().is_some_and(|r| r.id == "CLI-005"))
        .unwrap();
    assert_eq!(
        retired
            .requirement
            .as_ref()
            .unwrap()
            .attributes
            .status
            .as_deref(),
        Some("retired")
    );
    // No claim's text is an attribute bullet.
    for c in model
        .claims
        .values()
        .filter(|c| c.document == claim.document)
    {
        for label in [
            "Verify:",
            "Boundary:",
            "Traces:",
            "Rationale:",
            "Depends:",
            "Status:",
        ] {
            assert!(
                !c.text.starts_with(label),
                "attribute extracted as a claim: {}",
                c.text
            );
        }
    }
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
