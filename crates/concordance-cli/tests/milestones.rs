//! Milestone gating (`[gate] started_milestones`): requirements of unstarted milestones scan as
//! `planned` and `scan --fail-on` ignores them. Requirements without a `Milestone:` are started.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "concordance-milestones-{}-{tag}",
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

/// A copy of `fixtures/requirements` whose spec is `spec` and whose `[gate]` is `gate`.
/// `LEX-001` is verified by a test that fails in the recorded run; `LEX-002` has no test.
fn repo(tag: &str, spec: &str, gate: &str) -> PathBuf {
    let dir = temp_dir(tag).join("repo");
    copy_dir(&repo_root().join("fixtures/requirements"), &dir);
    std::fs::write(dir.join("docs/spec.md"), spec).unwrap();
    // The fixture's second test verifies an unknown requirement, which is a proven finding of
    // its own and not about milestones: drop it.
    let lib = dir.join("src/lib.rs");
    let text = std::fs::read_to_string(&lib).unwrap().replace("\r\n", "\n");
    let cut = text
        .find("    /// Verifies: LEX-404")
        .expect("LEX-404 test");
    let kept = text[..cut].trim_end_matches('\n');
    std::fs::write(&lib, format!("{kept}\n}}\n")).unwrap();
    std::fs::write(
        dir.join("concordance.toml"),
        format!(
            "spec_roots = [\"docs\"]\nsrc_roots = [\"src\"]\ntest_results = [\"results/failed.txt\"]\n{gate}"
        ),
    )
    .unwrap();
    // Bind the recorded run to the content it was run on, or scan reports it unrecorded.
    let out = Command::new(env!("CARGO_BIN_EXE_concordance"))
        .args(["record", "--repo"])
        .arg(&dir)
        .arg("--state-dir")
        .arg(dir.parent().unwrap().join("state"))
        .output()
        .expect("run concordance record");
    assert!(
        out.status.success(),
        "record failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    dir
}

struct Scan {
    code: i32,
    json: serde_json::Value,
}

impl Scan {
    /// The finding whose claim text contains `needle`.
    fn finding(&self, needle: &str) -> &serde_json::Value {
        self.json["findings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| {
                f["claim_refs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["text"].as_str().unwrap_or("").contains(needle))
            })
            .unwrap_or_else(|| panic!("no finding for {needle}: {}", self.json))
    }
}

fn scan(repo: &Path, fail_on: &str) -> Scan {
    let out = Command::new(env!("CARGO_BIN_EXE_concordance"))
        .args([
            "scan",
            "--json",
            "--no-semantic",
            "--fail-on",
            fail_on,
            "--repo",
        ])
        .arg(repo)
        .arg("--state-dir")
        .arg(repo.parent().unwrap().join("state"))
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
        });
    Scan {
        code: out.status.code().unwrap_or(-1),
        json: serde_json::from_str(line).expect("JSON report"),
    }
}

const SPEC: &str = "# Comment lexer

This specification is normative.

**LEX-001** A block comment SHALL end at the `*/` that balances its opening `/*`.
- Milestone: 2

**LEX-002** The lexer MUST reject a block comment that is never closed.
- Milestone: 2
- Seam: 1
";

/// LEX-001 (unstarted) has a failing test; LEX-002 (started) has none.
const SPEC_UNTESTED: &str = "# Comment lexer

This specification is normative.

**LEX-001** A block comment SHALL end at the `*/` that balances its opening `/*`.
- Milestone: 2

**LEX-002** The lexer MUST reject a block comment that is never closed.
- Milestone: 1
";

#[test]
fn fail_on_unverified_fails_on_a_started_requirement_nobody_proves() {
    let r = repo(
        "unverified",
        SPEC_UNTESTED,
        "[gate]
started_milestones = [1]
",
    );
    // `proven` ignores a requirement that has no test.
    let proven = scan(&r, "proven");
    assert_eq!(proven.code, 0, "{}", proven.json["gate"]);
    assert_eq!(proven.json["gate"]["unverified_requirements"], 1);
    // `unverified` does not.
    let strict = scan(&r, "unverified");
    assert_eq!(strict.code, 1, "{}", strict.json["gate"]);
    assert_eq!(strict.json["gate"]["fail_on"], "unverified");
    assert_eq!(strict.json["gate"]["failed"], true);
}

#[test]
fn fail_on_unverified_leaves_an_unstarted_milestone_alone() {
    let spec = SPEC_UNTESTED.replace("- Milestone: 1", "- Milestone: 2");
    let r = repo(
        "unverified-unstarted",
        &spec,
        "[gate]
started_milestones = [1]
",
    );
    let strict = scan(&r, "unverified");
    assert_eq!(strict.json["gate"]["unverified_requirements"], 0);
    assert_eq!(strict.code, 0, "{}", strict.json["gate"]);
}

#[test]
fn an_unstarted_milestone_is_planned_and_does_not_fail_the_gate() {
    let r = repo("planned", SPEC, "[gate]\nstarted_milestones = [1]\n");
    let s = scan(&r, "proven");
    let f = s.finding("never closed");
    assert_eq!(f["status"], "planned", "{f}");
    assert_eq!(f["milestone"], 2);
    assert_eq!(s.json["counts"]["planned"], 1);
    // LEX-001's test failed in the recorded run, but milestone 2 is not started.
    assert_eq!(s.code, 0, "{}", s.json["gate"]);
    assert_eq!(s.json["gate"]["failed"], false);
    assert_eq!(s.json["gate"]["ignored_unstarted"], 2);
}

#[test]
fn a_started_milestone_is_gated_as_before() {
    let r = repo("started", SPEC, "[gate]\nstarted_milestones = [1, 2]\n");
    let s = scan(&r, "proven");
    assert_eq!(s.finding("never closed")["status"], "unverified");
    assert_eq!(s.finding("balances")["status"], "violated");
    assert_eq!(s.json["counts"]["planned"], 0);
    assert_eq!(s.code, 1);
}
