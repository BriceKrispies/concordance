//! Concordance's own umbrella requirements (`docs/spec`), proved over the fixtures their
//! case tables name. LANG-001 and RES-001 run the table-driven conformance driver; SCAN-001 and
//! SCAN-002 are properties of every scan of those fixtures. The LINT proofs run the real binary
//! and live in `crates/concordance-cli/tests/lint_proofs.rs`.

use concordance_tests::conformance::*;

/// Verifies: LANG-001
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scan_reports_every_finding_of_the_language_cases() {
    run_conformance("LANG-001").await;
}

/// Verifies: RES-001
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scan_with_test_results_reports_what_the_result_cases_list() {
    run_conformance("RES-001").await;
}

/// Every (fixture, results file) combination the case tables name, including no results.
fn scans() -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for file in all_case_files() {
        let fixture = fixture_of(&file);
        for g in results_groups(&case_table(&file)) {
            if !out.contains(&(fixture.clone(), g.clone())) {
                out.push((fixture.clone(), g));
            }
        }
    }
    out
}

/// Verifies: SCAN-001
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scan_leaves_every_file_of_the_repository_byte_identical() {
    let mut failures = Vec::new();
    for (fixture, results) in scans() {
        let s = scan_with(&fixture, results.as_deref()).await;
        if s.before != s.after {
            let changed: Vec<&String> = s
                .after
                .keys()
                .chain(s.before.keys())
                .filter(|k| s.before.get(*k) != s.after.get(*k))
                .collect();
            failures.push(format!(
                "scan of `{fixture}` (results {}) changed {changed:?}",
                results.as_deref().unwrap_or("none")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Verifies: SCAN-002
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_scans_of_the_same_content_report_the_same_findings() {
    let mut failures = Vec::new();
    for (fixture, results) in scans() {
        let a = findings_projection(&scan_with(&fixture, results.as_deref()).await.model);
        let b = findings_projection(&scan_with(&fixture, results.as_deref()).await.model);
        if a != b {
            let only_a: Vec<&String> = a.iter().filter(|x| !b.contains(x)).collect();
            let only_b: Vec<&String> = b.iter().filter(|x| !a.contains(x)).collect();
            failures.push(format!(
                "scans of `{fixture}` (results {}) differ\nonly in the first: {only_a:#?}\nonly in the second: {only_b:#?}",
                results.as_deref().unwrap_or("none")
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
