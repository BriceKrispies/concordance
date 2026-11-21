//! Specification size limits. A specification over 8 MiB, the old cap for every file, is read and
//! linted; one over `MAX_SPEC_BYTES` is a hard error for `scan`, `lint` and `record`, never a skip.
//! The large files are generated inside each test, not committed.

use concordance_engine::source::MAX_SPEC_BYTES;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const OLD_CAP: u64 = 8 * 1024 * 1024;

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
        "concordance-spec-size-{}-{n}-{tag}",
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

/// A copy of `fixtures/spec-lint` that `record` can run against (it needs a results file).
fn fixture_copy(tag: &str) -> PathBuf {
    let repo = temp_dir(tag).join("repo");
    copy_dir(&repo_root().join("fixtures/spec-lint"), &repo);
    let config = repo.join("concordance.toml");
    // A top-level key, so it goes before the first table header.
    let text = std::fs::read_to_string(&config).unwrap();
    let text = format!("test_results = [\"target/test-results/cargo-test.txt\"]\n{text}");
    std::fs::write(config, text).unwrap();
    repo
}

/// Run `concordance <args>` over `repo` with a fresh state directory.
fn run(repo: &Path, args: &[&str]) -> Output {
    let state = temp_dir("state");
    Command::new(env!("CARGO_BIN_EXE_concordance"))
        .args(args)
        .arg("--repo")
        .arg(repo)
        .arg("--state-dir")
        .arg(&state)
        .output()
        .expect("run concordance")
}

/// A specification over the old 8 MiB cap is read, and each of its requirements is linted.
#[test]
fn spec_over_8_mib_is_read_and_linted() {
    let repo = fixture_copy("read");
    let spec = repo.join("docs/spec/cli.md");

    // Padding: prose with no normative words, no identifiers and no digits, so it raises no finding.
    // The requirement at the end breaks R2 (`correctly` is a banned word), and that finding must
    // come from the lint reading the file's last line.
    let mut text = std::fs::read_to_string(&spec).unwrap();
    let filler = "Background: the build keeps its notes in the state directory.\n";
    let lines_needed = (OLD_CAP as usize / filler.len()) + 1;
    text.reserve(lines_needed * filler.len() + 256);
    for _ in 0..lines_needed {
        text.push_str(filler);
    }
    text.push_str(
        "\n**CLI-901** The build command SHALL finish correctly.\n\
         - Verify: scenario\n\
         - Boundary: B-1 `tool build`\n\
         - Traces: G-1\n",
    );
    let line = text
        .lines()
        .position(|l| l.contains("**CLI-901**"))
        .expect("the appended requirement")
        + 1;
    std::fs::write(&spec, &text).unwrap();
    assert!(
        std::fs::metadata(&spec).unwrap().len() > OLD_CAP,
        "the generated spec must be over 8 MiB"
    );

    let out = run(&repo, &["lint", "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_ne!(
        out.status.code(),
        Some(2),
        "a spec over 8 MiB must not be refused\nstderr: {stderr}"
    );
    let json = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("no JSON on stdout\nstdout: {stdout}\nstderr: {stderr}"));
    let report: serde_json::Value = serde_json::from_str(json).expect("JSON report");
    let found = report["findings"]
        .as_array()
        .expect("findings[]")
        .iter()
        .any(|f| {
            f["path"] == "docs/spec/cli.md"
                && f["rule"] == "R2"
                && f["line"].as_u64() == Some(line as u64)
        });
    assert!(
        found,
        "the lint must report R2 at docs/spec/cli.md:{line}; report: {json}"
    );
}

/// A specification over `MAX_SPEC_BYTES` makes `scan`, `lint` and `record` exit 2, naming
/// the file, its size and the limit, and nothing is linted.
#[test]
fn spec_over_the_read_limit_is_a_hard_error() {
    let repo = fixture_copy("refuse");
    let huge = repo.join("docs/spec/huge.md");
    // A sparse-sized file: the refusal is decided from its length, before any read.
    let file = std::fs::File::create(&huge).unwrap();
    file.set_len(MAX_SPEC_BYTES + 1).unwrap();
    drop(file);
    let expected = format!("docs/spec/huge.md is {} bytes", MAX_SPEC_BYTES + 1);

    for cmd in [
        &["lint", "--json"][..],
        &["scan", "--json"][..],
        &["record"][..],
    ] {
        let out = run(&repo, cmd);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(2),
            "`concordance {}` must exit 2 for an oversize spec\nstderr: {stderr}",
            cmd[0]
        );
        assert!(
            stderr.contains(&expected),
            "`concordance {}` must name the file and its size\nstderr: {stderr}",
            cmd[0]
        );
        assert!(
            stderr.contains("over the 64 MiB limit"),
            "`concordance {}` must name the limit\nstderr: {stderr}",
            cmd[0]
        );
        assert!(
            !String::from_utf8_lossy(&out.stdout).contains("\"findings\""),
            "`concordance {}` must not print a report for the refused spec",
            cmd[0]
        );
    }
}
