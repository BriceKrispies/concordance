//! The Exegra results file and the locations of a structured specification, through the real
//! `concordance` binary over `fixtures/exegra/jsonc` (the same specification as
//! `fixtures/exegra/markdown`, kept as one Exegra JSONC file).
//!
//! The fixture's `LIB-002` is proved by a test; `CLI-002` breaks R1 and `CLI-003` breaks R2.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// A fresh directory per call: tests in this file run in parallel in one process.
fn temp_dir(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!(
        "concordance-exegra-results-{}-{n}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

/// A copy of the fixture whose recorded test run is `run`, bound to the content by `concordance record`.
fn repo(tag: &str, run: &str) -> PathBuf {
    let dir = temp_dir(tag).join("repo");
    copy_dir(&repo_root().join("fixtures/exegra/jsonc"), &dir);
    std::fs::write(dir.join("results/passed.txt"), run).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_concordance"))
        .args(["record", "--repo"])
        .arg(&dir)
        .arg("--state-dir")
        .arg(temp_dir(&format!("{tag}-record-state")))
        .output()
        .expect("run concordance record");
    assert!(
        out.status.success(),
        "record failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    dir
}

const PASSED: &str =
    "running 1 test\ntest tests::empty_manifest_has_no_targets ... ok\ntest result: ok.\n";
const FAILED: &str = "test tests::empty_manifest_has_no_targets ... FAILED\n";

fn run(args: &[&str], repo: &Path, state: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_concordance"))
        .args(args)
        .arg("--repo")
        .arg(repo)
        .arg("--state-dir")
        .arg(state)
        .output()
        .expect("run concordance")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// The 1-based line of the entity `id` in the fixture's JSONC.
fn line_of(repo: &Path, id: &str) -> u64 {
    let text = std::fs::read_to_string(repo.join("specs/spec.jsonc")).unwrap();
    let needle = format!("\"id\":\"{id}\"");
    text.lines()
        .position(|l| l.contains(&needle))
        .unwrap_or_else(|| panic!("no entity {id}")) as u64
        + 1
}

fn scan_results(repo: &Path, state: &Path, out: &Path, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["scan", "--exegra-results", out.to_str().unwrap()];
    args.extend_from_slice(extra);
    let o = run(&args, repo, state);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    serde_json::from_str(&std::fs::read_to_string(out).unwrap()).expect("results are JSON")
}

#[test]
fn the_proven_requirement_passes_and_the_unproven_ones_are_absent() {
    let repo = repo("pass", PASSED);
    let state = temp_dir("pass-state");
    let out = state.join("results.json");
    let v = scan_results(&repo, &state, &out, &["--no-generated-at"]);

    assert_eq!(v["tool"], "concordance");
    assert_eq!(v["source"], "specs/spec.jsonc");
    assert!(v.get("generatedAt").is_none());
    let claims = v["claims"].as_object().unwrap();
    let ids: Vec<&str> = claims.keys().map(String::as_str).collect();
    assert_eq!(
        ids,
        ["lib-002"],
        "only the proven requirement is established"
    );
    let c = &claims["lib-002"];
    assert_eq!(c["outcome"], "pass");
    assert_eq!(c["status"], "verified");
    let proofs = c["proofs"].as_array().unwrap();
    assert_eq!(proofs.len(), 1);
    assert!(proofs[0]
        .as_str()
        .unwrap()
        .ends_with("empty_manifest_has_no_targets"));
    assert!(c["detail"]
        .as_str()
        .unwrap()
        .contains("empty_manifest_has_no_targets"));

    // Lint findings are located at their entity in the JSONC.
    let lint = v["lint"].as_array().unwrap();
    let find = |rule: &str, entity: &str| {
        lint.iter()
            .find(|l| l["rule"] == rule && l["entity"] == entity)
            .unwrap_or_else(|| panic!("no {rule} finding at {entity}: {lint:?}"))
    };
    let r1 = find("R1", "cli-002");
    assert_eq!(r1["severity"], "error");
    assert_eq!(r1["claim"], "cli-002");
    assert_eq!(r1["line"].as_u64(), Some(line_of(&repo, "cli-002")));
    let r2 = find("R2", "cli-003");
    assert_eq!(r2["claim"], "cli-003");
    assert_eq!(r2["line"].as_u64(), Some(line_of(&repo, "cli-003")));
    // The retired requirement is only checked by R8, and the others are clean.
    assert!(!lint.iter().any(|l| l["entity"] == "cli-004"));
    assert!(!lint.iter().any(|l| l["entity"] == "lib-001"));
}

#[test]
fn a_failing_proof_is_a_fail() {
    let repo = repo("fail", FAILED);
    let state = temp_dir("fail-state");
    let out = state.join("results.json");
    let v = scan_results(&repo, &state, &out, &["--no-generated-at"]);
    let c = &v["claims"]["lib-002"];
    assert_eq!(c["outcome"], "fail");
    assert_eq!(c["status"], "violated");
    assert_eq!(c["proofs"].as_array().unwrap().len(), 1);
    assert!(c["detail"].as_str().unwrap().contains("failing test"));
    assert_eq!(v["claims"].as_object().unwrap().len(), 1);
}

#[test]
fn report_refuses_results_from_state_older_than_the_source() {
    let repo = repo("behind", PASSED);
    let state = temp_dir("behind-state");
    let out = state.join("results.json");
    scan_results(&repo, &state, &out, &["--no-generated-at"]);
    // Edit a statement without scanning again.
    let spec = repo.join("specs/spec.jsonc");
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(
        &spec,
        text.replace("for an empty manifest", "for a blank manifest"),
    )
    .unwrap();
    let again = state.join("again.json");
    let o = run(
        &["report", "--exegra-results", again.to_str().unwrap()],
        &repo,
        &state,
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("run `concordance scan` first"),
        "{}",
        stderr(&o)
    );
    assert!(!again.exists());
}
