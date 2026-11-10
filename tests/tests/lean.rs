//! A Lake package through the Lean plugin: structural drift, documented signatures and
//! requirement verification from `lake build` logs, with exact `.lean` locations.

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
            src_roots: vec![PathBuf::from("Volume")],
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lean_package_is_reconciled_with_exact_lean_locations() {
    let repo = fixture_copy("lean");
    let state = temp_dir("lean-state");
    let results = state.join("lake-build.log");
    let r = start(config(&repo, &state, &results), None, 8192).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;

    // Indexed as Lean, package from lakefile.lean.
    let file = m
        .files
        .values()
        .find(|f| f.path.as_str() == "Volume/Mixer.lean")
        .expect("Mixer.lean indexed");
    assert_eq!(file.language, "lean");
    assert_eq!(file.crate_name.as_deref(), Some("volume"));
    assert!(m
        .symbols
        .values()
        .any(|s| s.qualified_name() == "volume::Volume::Mixer::Volume::clampLevel"));
    let thm = m
        .symbols
        .values()
        .find(|s| s.name == "clamp_idempotent")
        .expect("theorem indexed");
    assert!(thm.is_test);
    assert!(thm
        .facts
        .iter()
        .any(|f| f.kind == concordance_core::code::FactKind::Unproven));

    // Removed definition: violated, located in the spec.
    let gone = claim_finding(&m, "resetAll()");
    assert_eq!(gone.status, FindingStatus::Violated, "{}", gone.explanation);
    assert_eq!(gone.category, FindingCategory::StructuralContradiction);
    assert_eq!(gone.locations[0].path.as_str(), "docs/spec.md");

    // Documented signature differs from the code: violated, pointing at Mixer.lean.
    let sig = claim_finding(&m, "def clampLevel (level : Nat) (floor : Nat)");
    assert_eq!(sig.status, FindingStatus::Violated, "{}", sig.explanation);
    assert!(
        sig.locations
            .iter()
            .any(|l| l.path.as_str() == "Volume/Mixer.lean" && l.start_line == 7),
        "{:?}",
        sig.locations
    );

    // Both requirements are linked to their theorems; no build log yet.
    for (needle, thm) in [
        ("SHALL never return a level above", "clamp_le_max"),
        ("SHALL be idempotent", "clamp_idempotent"),
    ] {
        let req = claim_finding(&m, needle);
        assert_eq!(req.status, FindingStatus::Unverified, "{}", req.explanation);
        assert!(req.explanation.contains(thm), "{}", req.explanation);
    }

    // A log with a `sorry` warning inside VOL-002's theorem only.
    let log = |name: &str| std::fs::read_to_string(repo.join("logs").join(name)).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    std::fs::write(&results, log("lake-sorry.log")).unwrap();
    r.engine.request_scan(false).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;
    let ok = claim_finding(&m, "SHALL never return a level above");
    assert_eq!(ok.status, FindingStatus::Verified, "{}", ok.explanation);
    let sorry = claim_finding(&m, "SHALL be idempotent");
    assert_eq!(
        sorry.status,
        FindingStatus::Violated,
        "{}",
        sorry.explanation
    );
    assert_eq!(sorry.category, FindingCategory::TestFailure);

    // A module that failed to build violates every theorem in it.
    std::fs::write(&results, log("lake-failed.log")).unwrap();
    r.engine.request_scan(false).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;
    let ok = claim_finding(&m, "SHALL never return a level above");
    assert_eq!(ok.status, FindingStatus::Violated, "{}", ok.explanation);
    assert_eq!(ok.category, FindingCategory::TestFailure);

    r.stop().await;
}
