//! The persisted index follows discovery: a file the configuration starts excluding, a file
//! that is deleted and a file moved out of the source roots all leave the index (and every
//! finding derived from them) on the next scan with the same state directory.

use std::path::{Path, PathBuf};
use std::process::Command;

fn temp_dir(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!(
        "concordance-stale-index-{}-{n}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const LIB: &str = "pub fn one() -> u32 {
    1
}
";

/// A test whose `Verifies:` names a requirement the specification does not define.
const STRAY: &str = "/// Verifies: ZZZ-404
#[test]
fn stray_proof() {}
";

const SPEC: &str = "# Spec

This specification is normative.

**LEX-001** The function `one` SHALL return one.
";

fn config(extra: &str) -> String {
    format!("spec_roots = [\"docs\"]\nsrc_roots = [\"src\"]\n{extra}")
}

/// A repository with a stray proof at `src/stray/proof.rs`, and its state directory.
fn repo(tag: &str) -> (PathBuf, PathBuf) {
    let base = temp_dir(tag);
    let dir = base.join("repo");
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::create_dir_all(dir.join("src/stray")).unwrap();
    std::fs::write(dir.join("docs/spec.md"), SPEC).unwrap();
    std::fs::write(dir.join("src/lib.rs"), LIB).unwrap();
    std::fs::write(dir.join("src/stray/proof.rs"), STRAY).unwrap();
    std::fs::write(dir.join("concordance.toml"), config("")).unwrap();
    (dir, base.join("state"))
}

/// The full JSON report of a scan of `repo` with `state`.
fn scan(repo: &Path, state: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_concordance"))
        .args(["scan", "--json", "--no-semantic", "--repo"])
        .arg(repo)
        .arg("--state-dir")
        .arg(state)
        .output()
        .expect("run concordance scan");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| {
            panic!(
                "no JSON on stdout\nstdout: {stdout}\nstderr: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        })
        .to_string();
    line
}

fn assert_stray_reported(repo: &Path, state: &Path) {
    let first = scan(repo, state);
    assert!(
        first.contains("ZZZ-404"),
        "the stray proof is reported: {first}"
    );
}

#[test]
fn a_newly_excluded_file_leaves_the_index() {
    let (repo, state) = repo("exclude");
    assert_stray_reported(&repo, &state);
    std::fs::write(
        repo.join("concordance.toml"),
        config("exclude = [\"src/stray/**\"]\n"),
    )
    .unwrap();
    let again = scan(&repo, &state);
    assert!(!again.contains("ZZZ-404"), "excluded file lingers: {again}");
    assert!(!again.contains("stray"), "excluded file lingers: {again}");
}

#[test]
fn a_deleted_file_leaves_the_index() {
    let (repo, state) = repo("delete");
    assert_stray_reported(&repo, &state);
    std::fs::remove_file(repo.join("src/stray/proof.rs")).unwrap();
    let again = scan(&repo, &state);
    assert!(!again.contains("ZZZ-404"), "deleted file lingers: {again}");
}

#[test]
fn a_file_moved_out_of_the_source_roots_leaves_the_index() {
    let (repo, state) = repo("moved");
    assert_stray_reported(&repo, &state);
    std::fs::create_dir_all(repo.join("elsewhere")).unwrap();
    std::fs::rename(
        repo.join("src/stray/proof.rs"),
        repo.join("elsewhere/proof.rs"),
    )
    .unwrap();
    let again = scan(&repo, &state);
    assert!(!again.contains("ZZZ-404"), "moved file lingers: {again}");
}
