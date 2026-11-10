//! Proofs of the LINT requirements of `docs/spec/lint.md`. They run the real `concordance`
//! binary (`concordance lint --strict --json`) over `fixtures/spec-lint`.
//!
//! The lint case table is `fixtures/spec-lint/EXPECTED.md`; the requirement's `Cases:` list in
//! the specification names it. For each case stem the fixture is copied, the violation is
//! applied, and the lint must report exactly the table's findings.

use std::collections::BTreeMap;
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

/// A fresh directory per call: tests in this file run in parallel in one process, so the
/// process id alone would let one test delete another's state.
fn temp_dir(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!(
        "concordance-lint-proof-{}-{n}-{tag}",
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

/// `Cases:` files of a requirement of `docs/spec/lint.md`, read through the Markdown indexer.
fn cases_of(id: &str) -> Vec<String> {
    let text = std::fs::read_to_string(repo_root().join("docs/spec/lint.md")).unwrap();
    let block = concordance_markdown::structure(&text)
        .requirements
        .into_iter()
        .find(|b| b.id == id)
        .unwrap_or_else(|| panic!("docs/spec/lint.md defines no {id}"));
    block
        .attribute("Cases")
        .unwrap_or_else(|| panic!("{id} has no Cases"))
        .value
        .split(',')
        .map(|f| f.trim().trim_matches('`').trim().to_string())
        .collect()
}

struct Report {
    code: i32,
    errors: u64,
    warnings: u64,
    /// `(path, rule, line, severity)`, in report order.
    findings: Vec<(String, String, u64, String)>,
}

/// `concordance lint [--strict] --json` over `repo`.
fn lint(repo: &Path, strict: bool) -> Report {
    let state = temp_dir("state");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_concordance"));
    cmd.arg("lint");
    if strict {
        cmd.arg("--strict");
    }
    let out = cmd
        .args(["--json", "--repo"])
        .arg(repo)
        .arg("--state-dir")
        .arg(&state)
        .output()
        .expect("run concordance lint");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let json = stdout
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| {
            panic!(
                "no JSON on stdout\nstdout: {stdout}\nstderr: {}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
    let v: serde_json::Value = serde_json::from_str(json).expect("JSON report");
    let findings = v["findings"]
        .as_array()
        .expect("findings[]")
        .iter()
        .map(|f| {
            assert!(
                !f["message"].as_str().unwrap_or("").is_empty(),
                "finding without a message: {f}"
            );
            (
                f["path"].as_str().unwrap().to_string(),
                f["rule"].as_str().unwrap().to_string(),
                f["line"].as_u64().unwrap(),
                f["severity"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    Report {
        code: out.status.code().unwrap_or(-1),
        errors: v["errors"].as_u64().unwrap(),
        warnings: v["warnings"].as_u64().unwrap(),
        findings,
    }
}

/// A copy of the fixture with the violation of case `stem` applied.
fn with_violation(stem: &str, file: &str) -> PathBuf {
    let repo = temp_dir(stem).join("repo");
    copy_dir(&repo_root().join("fixtures/spec-lint"), &repo);
    let violation = repo.join("violations").join(format!("{stem}.md"));
    if stem == "missing_whitepaper" {
        std::fs::remove_file(repo.join("docs/whitepaper.md")).unwrap();
    } else if file == "docs/whitepaper.md" {
        std::fs::copy(&violation, repo.join("docs/whitepaper.md")).unwrap();
    } else {
        let text = std::fs::read_to_string(&violation).unwrap();
        let prefix: String = text
            .split("**")
            .nth(1)
            .and_then(|id| id.split('-').next())
            .unwrap_or_default()
            .to_uppercase();
        std::fs::copy(
            &violation,
            repo.join("docs/spec").join(format!("{stem}.md")),
        )
        .unwrap();
        let index = repo.join("docs/spec/index.md");
        let mut idx = std::fs::read_to_string(&index).unwrap();
        idx.push_str(&format!("| [{stem}.md]({stem}.md) | {prefix} | x |\n"));
        std::fs::write(index, idx).unwrap();
    }
    repo
}

/// Verifies: LINT-001
#[test]
fn lint_strict_reports_every_listed_finding_with_rule_and_line() {
    let mut failures = Vec::new();
    for file in cases_of("LINT-001") {
        let text = std::fs::read_to_string(repo_root().join(&file)).unwrap();
        let table = concordance_lint::parse_cases(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
        let col = |n: &str| {
            table
                .column(n)
                .unwrap_or_else(|| panic!("{file}: no {n} column"))
        };
        let (c_file, c_rule, c_line, c_sev) =
            (col("File"), col("Rule"), col("Line"), col("Severity"));

        // stem -> expected (path, rule, line, severity), in table order.
        let mut stems: BTreeMap<String, Vec<(String, String, u64, String)>> = BTreeMap::new();
        for r in &table.rows {
            let stem = r.cells[0].split('.').next().unwrap().to_string();
            stems.entry(stem).or_default().push((
                r.cells[c_file].clone(),
                r.cells[c_rule].clone(),
                r.cells[c_line]
                    .parse()
                    .unwrap_or_else(|_| panic!("{file}:{}: bad line", r.line)),
                r.cells[c_sev].clone(),
            ));
        }
        for (stem, expected) in &stems {
            let repo = with_violation(stem, &expected[0].0);
            let mut want: Vec<_> = expected.clone();
            want.sort();

            // `--strict`: every finding is an error and the lint fails.
            let strict = lint(&repo, true);
            let mut got = strict.findings.clone();
            got.sort();
            let want_strict: Vec<_> = want
                .iter()
                .map(|(p, r, l, _)| (p.clone(), r.clone(), *l, "error".to_string()))
                .collect();
            if got != want_strict
                || strict.code != 1
                || strict.errors as usize != want.len()
                || strict.warnings != 0
            {
                failures.push(format!(
                    "case `{stem}` (--strict): expected {want_strict:?} and exit 1, got {got:?} exit {} ({} errors, {} warnings)",
                    strict.code, strict.errors, strict.warnings
                ));
            }

            // Without `--strict`: the table's severities, and the exit status follows the errors.
            let lax = lint(&repo, false);
            let mut got = lax.findings.clone();
            got.sort();
            let errors = want.iter().filter(|w| w.3 == "error").count();
            let expect_code = i32::from(errors > 0);
            if got != want
                || lax.code != expect_code
                || lax.errors as usize != errors
                || lax.warnings as usize != want.len() - errors
            {
                failures.push(format!(
                    "case `{stem}` (no --strict): expected {want:?} exit {expect_code}, got {got:?} exit {} ({} errors, {} warnings)",
                    lax.code, lax.errors, lax.warnings
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} lint case(s) failed:\n\n{}\n",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Verifies: LINT-002
#[test]
fn lint_strict_reports_nothing_for_the_conforming_fixture() {
    let repo = temp_dir("conforming").join("repo");
    copy_dir(&repo_root().join("fixtures/spec-lint"), &repo);
    for strict in [true, false] {
        let r = lint(&repo, strict);
        assert!(
            r.findings.is_empty() && r.code == 0 && r.errors == 0 && r.warnings == 0,
            "strict={strict}: exit {} with {:?}",
            r.code,
            r.findings
        );
    }
}
