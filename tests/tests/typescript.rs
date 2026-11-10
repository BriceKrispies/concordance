//! A TypeScript package through the TypeScript plugin: structural drift, documented
//! signatures, failure-site constraints and requirement verification from a vitest
//! JUnit report, with exact `.ts` locations.

use concordance_core::config::{Config, ConfigFile, Overrides};
use concordance_core::finding::{Finding, FindingCategory, FindingStatus, FindingSubject};
use concordance_graph::Model;
use concordance_tests::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

fn claim_finding<'a>(m: &'a Model, needle: &str) -> &'a Finding {
    let claim = m
        .claims
        .values()
        .find(|c| c.text.contains(needle))
        .unwrap_or_else(|| panic!("no claim containing {needle:?}"));
    m.findings
        .values()
        .find(|f| {
            f.is_open()
                && f.subject
                    == FindingSubject::Claim {
                        claim: claim.id.clone(),
                    }
        })
        .unwrap_or_else(|| panic!("no finding for {needle:?}"))
}

fn junit(passed: bool) -> String {
    let body = if passed {
        ""
    } else {
        "<failure message=\"expected RangeError\">AssertionError</failure>"
    };
    format!(
        "<?xml version=\"1.0\"?>\n<testsuites><testsuite name=\"src/mixer.test.ts\">\n\
         <testcase classname=\"src/mixer.test.ts\" name=\"Mixer &gt; rejects levels above 100\">{body}</testcase>\n\
         <testcase classname=\"src/mixer.test.ts\" name=\"Mixer &gt; keeps an accepted level\"></testcase>\n\
         </testsuite></testsuites>\n"
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typescript_repository_is_reconciled_with_exact_ts_locations() {
    let repo = fixture_copy("typescript");
    let state = temp_dir("typescript-state");
    let results = state.join("vitest-junit.xml");
    let r = start(config(&repo, &state, &results), None, 8192).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;

    // Indexed as TypeScript, package from package.json.
    let file = m
        .files
        .values()
        .find(|f| f.path.as_str() == "src/mixer.ts")
        .expect("mixer.ts indexed");
    assert_eq!(file.language, "typescript");
    assert_eq!(file.crate_name.as_deref(), Some("volume"));
    assert!(m
        .symbols
        .values()
        .any(|s| s.qualified_name() == "volume::mixer::Mixer::setVolume"));

    // Removed function: violated, located in the spec.
    let gone = claim_finding(&m, "resetAll()");
    assert_eq!(gone.status, FindingStatus::Violated, "{}", gone.explanation);
    assert_eq!(gone.category, FindingCategory::StructuralContradiction);
    assert_eq!(gone.locations[0].path.as_str(), "docs/spec.md");

    // Documented signature differs from the code: violated, pointing at mixer.ts.
    let sig = claim_finding(&m, "function parseLevel(text: string)");
    assert_eq!(sig.status, FindingStatus::Violated, "{}", sig.explanation);
    assert!(
        sig.locations
            .iter()
            .any(|l| l.path.as_str() == "src/mixer.ts" && l.start_line == 21),
        "{:?}",
        sig.locations
    );
    assert!(
        sig.explanation.contains("radix = 10"),
        "{}",
        sig.explanation
    );

    // MUST NOT throw, and the function throws.
    let throws = claim_finding(&m, "MUST NOT throw");
    assert_eq!(
        throws.status,
        FindingStatus::Violated,
        "{}",
        throws.explanation
    );
    assert!(
        throws
            .locations
            .iter()
            .any(|l| l.path.as_str() == "src/mixer.ts" && l.start_line == 24),
        "{:?}",
        throws.locations
    );

    // `Mixer.volume` resolves (dotted type path), so it is not reported as dangling.
    let vol = claim_finding(&m, "returns the current level");
    assert_ne!(vol.status, FindingStatus::Violated, "{}", vol.explanation);

    // VOL-001: linked to the vitest test, no run yet.
    let req = claim_finding(&m, "SHALL reject a level above 100");
    assert_eq!(req.status, FindingStatus::Unverified, "{}", req.explanation);
    assert!(
        req.explanation.contains("rejects levels above 100"),
        "{}",
        req.explanation
    );

    // A passing JUnit report verifies it; a failing one violates it.
    tokio::time::sleep(Duration::from_millis(30)).await;
    std::fs::write(&results, junit(true)).unwrap();
    r.engine.request_scan(false).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;
    let req = claim_finding(&m, "SHALL reject a level above 100");
    assert_eq!(req.status, FindingStatus::Verified, "{}", req.explanation);

    std::fs::write(&results, junit(false)).unwrap();
    r.engine.request_scan(false).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;
    let req = claim_finding(&m, "SHALL reject a level above 100");
    assert_eq!(req.status, FindingStatus::Violated, "{}", req.explanation);
    assert_eq!(req.category, FindingCategory::TestFailure);

    r.stop().await;
}
