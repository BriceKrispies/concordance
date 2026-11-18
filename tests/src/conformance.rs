//! The table-driven conformance driver.
//!
//! A requirement of `docs/spec` with `Verify: conformance` names its case tables in a `Cases:`
//! attribute. Each table is the `## Cases` section of a fixture's `EXPECTED.md`:
//!
//! | Column | Meaning |
//! |---|---|
//! | `Case` | unique short name |
//! | `Requirement` | the umbrella requirement the row belongs to |
//! | `Results` | `none`, or a results file relative to the fixture, installed as the configured test-results file and recorded (as `concordance record` does) before the final scan |
//! | `Claim contains` | substring identifying the finding: found in its title or in the text of one of its claims (empty: every finding) |
//! | `Status` | the expected status (`violated`, `verified`, ...); `!status` for any other status; `absent` or `absent:status` for "no such finding" |
//! | `Category` | optional category filter (`structural_contradiction`, `test_failure`, ...), `!category` negates |
//! | `Location` | optional `path[:line]` filter: one of the finding's locations is in that file and covers that line; `!path[:line]` negates |
//!
//! A row passes when some open finding matches `Claim contains`, the status, the category and
//! the location. An `absent` row passes when no open finding matches the claim, the (optional)
//! status and the category and location filters.
//!
//! The driver reads the `Cases:` list through the Markdown indexer, copies each fixture, groups
//! its rows by `Results`, installs each results file, scans, checks every row, and fails once
//! with every failing row.

use crate::{fixture_copy, record_and_rescan, repo_root, start, temp_dir};
use concordance_core::config::{Config, ConfigFile, Overrides};
use concordance_core::finding::Finding;
use concordance_graph::Model;
use concordance_lint::{parse_cases, CaseTable};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// `Cases:` of the requirement `id`, read from `docs/spec/*.md` through the Markdown indexer.
pub fn spec_cases(id: &str) -> Vec<String> {
    spec_requirements()
        .into_iter()
        .find(|(rid, _)| rid == id)
        .unwrap_or_else(|| panic!("docs/spec defines no requirement {id}"))
        .1
}

/// Every case file named by any requirement of `docs/spec`, without duplicates.
pub fn all_case_files() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, files) in spec_requirements() {
        for f in files {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

fn spec_requirements() -> Vec<(String, Vec<String>)> {
    let dir = repo_root().join("docs/spec");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let text = std::fs::read_to_string(&p).unwrap();
        for b in concordance_markdown::structure(&text).requirements {
            let files = b
                .attribute("Cases")
                .map(|a| {
                    a.value
                        .split(',')
                        .map(|f| f.trim().trim_matches('`').trim().to_string())
                        .filter(|f| !f.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            out.push((b.id.clone(), files));
        }
    }
    out
}

/// The fixture a case file belongs to: the name of its directory under `fixtures/`.
pub fn fixture_of(case_file: &str) -> String {
    Path::new(case_file)
        .parent()
        .and_then(|p| p.file_name())
        .unwrap_or_else(|| panic!("{case_file} is not inside a fixture"))
        .to_string_lossy()
        .into_owned()
}

/// The parsed `## Cases` table of a repository-relative case file.
pub fn case_table(case_file: &str) -> CaseTable {
    let text = std::fs::read_to_string(repo_root().join(case_file))
        .unwrap_or_else(|e| panic!("{case_file}: {e}"));
    parse_cases(&text).unwrap_or_else(|e| panic!("{case_file}: {e}"))
}

/// The distinct results files of a case table, always starting with `None` (no results).
pub fn results_groups(table: &CaseTable) -> Vec<Option<String>> {
    let mut groups = vec![None];
    if let Some(c) = table.column("Results") {
        for r in &table.rows {
            let v = r.cells[c].trim_matches('`').trim();
            if !v.is_empty() && v != "none" && !groups.contains(&Some(v.to_string())) {
                groups.push(Some(v.to_string()));
            }
        }
    }
    groups
}

/// A scan of one fixture copy.
pub struct Scanned {
    pub repo: PathBuf,
    pub model: Model,
    /// Content hash of every file of the copy before the scan.
    pub before: BTreeMap<String, u64>,
    /// ... and after it.
    pub after: BTreeMap<String, u64>,
}

/// Hash every file under `dir`, keyed by its relative path.
pub fn tree_hash(dir: &Path) -> BTreeMap<String, u64> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, u64>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let bytes = std::fs::read(&p).unwrap();
                let mut h = std::collections::hash_map::DefaultHasher::new();
                bytes.hash(&mut h);
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, h.finish());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn src_roots(repo: &Path) -> Vec<PathBuf> {
    ["src", "Volume"]
        .iter()
        .filter(|d| repo.join(d).is_dir())
        .map(PathBuf::from)
        .collect()
}

/// Copy `fixtures/<fixture>`, install `results` (a path relative to the fixture) as the
/// configured test-results file in a fresh state directory, and scan once.
pub async fn scan_with(fixture: &str, results: Option<&str>) -> Scanned {
    let repo = fixture_copy(fixture);
    let state = temp_dir(&format!("{fixture}-state"));
    let installed = match results {
        Some(rel) => {
            let text =
                std::fs::read(repo.join(rel)).unwrap_or_else(|e| panic!("{fixture}/{rel}: {e}"));
            let name = Path::new(rel).file_name().unwrap();
            let dst = state.join(name);
            std::fs::write(&dst, text).unwrap();
            dst
        }
        None => state.join("no-results"),
    };
    let config = Config::resolve(
        ConfigFile::default(),
        None,
        Overrides {
            repo: Some(repo.clone()),
            spec_roots: vec![PathBuf::from("docs")],
            src_roots: src_roots(&repo),
            state_dir: Some(state.clone()),
            test_results: vec![installed],
            ..Default::default()
        },
        &repo,
        &state,
    )
    .unwrap();
    let before = tree_hash(&repo);
    let r = start(config.clone(), None, 8192).await;
    r.idle().await;
    if results.is_some() {
        // The run was made on these sources: bind it, as `concordance record` does.
        record_and_rescan(&r, &config).await;
    }
    let model = r.engine.shared.read(|m| m.clone()).0;
    r.stop().await;
    let after = tree_hash(&repo);
    Scanned {
        repo,
        model,
        before,
        after,
    }
}

/// The open findings of a scan as comparable strings: status, category, claim texts and
/// locations. Independent of identifiers and timestamps.
pub fn findings_projection(model: &Model) -> Vec<String> {
    let mut out: Vec<String> = model
        .findings
        .values()
        .filter(|f| f.is_open())
        .map(|f| {
            let mut claims: Vec<String> = f
                .claims
                .iter()
                .filter_map(|c| model.claims.get(c).map(|c| c.text.clone()))
                .collect();
            claims.sort();
            let mut locs: Vec<String> = f.locations.iter().map(|l| l.to_string()).collect();
            locs.sort();
            format!(
                "{} | {} | {:?} | {:?} | {:?}",
                json_name(&f.status),
                json_name(&f.category),
                f.title,
                claims,
                locs
            )
        })
        .collect();
    out.sort();
    out
}

fn json_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Rows

struct Row<'a> {
    case: &'a str,
    results: Option<String>,
    claim: &'a str,
    status: &'a str,
    category: &'a str,
    location: &'a str,
    line: u32,
}

fn cell<'a>(table: &CaseTable, cells: &'a [String], name: &str) -> Result<&'a str, String> {
    table
        .column(name)
        .map(|i| cells[i].trim())
        .ok_or_else(|| format!("the case table has no `{name}` column"))
}

fn negated(cell: &str) -> (bool, &str) {
    match cell.strip_prefix('!') {
        Some(rest) => (true, rest.trim()),
        None => (false, cell),
    }
}

fn clean(cell: &str) -> &str {
    cell.trim().trim_matches('`').trim()
}

fn claim_matches(model: &Model, f: &Finding, needle: &str) -> bool {
    needle.is_empty()
        || f.title.contains(needle)
        || f.claims
            .iter()
            .filter_map(|c| model.claims.get(c))
            .any(|c| c.text.contains(needle))
}

fn category_matches(f: &Finding, cell: &str) -> bool {
    let (neg, want) = negated(clean(cell));
    want.is_empty() || ((json_name(&f.category) == want) != neg)
}

fn location_matches(f: &Finding, cell: &str) -> bool {
    let (neg, want) = negated(clean(cell));
    if want.is_empty() {
        return true;
    }
    let (path, line) = match want.rsplit_once(':') {
        Some((p, l)) if l.parse::<u32>().is_ok() => (p, l.parse::<u32>().ok()),
        _ => (want, None),
    };
    let hit = f.locations.iter().any(|l| {
        l.path.as_str() == path && line.is_none_or(|n| l.start_line <= n && n <= l.end_line)
    });
    hit != neg
}

fn describe(f: &Finding) -> String {
    let locs: Vec<String> = f.locations.iter().map(|l| l.to_string()).collect();
    format!(
        "{} / {} / {:?} / [{}]",
        json_name(&f.status),
        json_name(&f.category),
        f.title,
        locs.join(", ")
    )
}

/// Check one row against a scan. `Err` says what was found instead.
fn check_row(model: &Model, row: &Row) -> Result<(), String> {
    let open: Vec<&Finding> = model.findings.values().filter(|f| f.is_open()).collect();
    let (absent, status_filter) = match row.status.strip_prefix("absent") {
        Some("") => (true, ""),
        Some(rest) => match rest.strip_prefix(':') {
            Some(s) => (true, s.trim()),
            None => (false, row.status),
        },
        None => (false, row.status),
    };
    let status_ok = |f: &Finding| {
        let (neg, want) = negated(status_filter);
        want.is_empty() || ((json_name(&f.status) == want) != neg)
    };
    if absent {
        let hits: Vec<&&Finding> = open
            .iter()
            .filter(|f| {
                claim_matches(model, f, row.claim)
                    && status_ok(f)
                    && category_matches(f, row.category)
                    && location_matches(f, row.location)
            })
            .collect();
        if hits.is_empty() {
            return Ok(());
        }
        return Err(format!(
            "expected no such finding, found {}: {}",
            hits.len(),
            hits.iter()
                .take(5)
                .map(|f| describe(f))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let candidates: Vec<&&Finding> = open
        .iter()
        .filter(|f| claim_matches(model, f, row.claim))
        .collect();
    if candidates.is_empty() {
        return Err(format!(
            "no open finding contains {:?}; findings: {}",
            row.claim,
            open.iter()
                .take(12)
                .map(|f| format!("{:?}", f.title))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if candidates.iter().any(|f| {
        status_ok(f) && category_matches(f, row.category) && location_matches(f, row.location)
    }) {
        return Ok(());
    }
    Err(format!(
        "expected status {:?} category {:?} location {:?}; found {}",
        row.status,
        row.category,
        row.location,
        candidates
            .iter()
            .take(5)
            .map(|f| describe(f))
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

/// Run every row of the case tables that `requirement` names in `docs/spec`, for the rows
/// that belong to `requirement`. Panics once, listing every failing row.
pub async fn run_conformance(requirement: &str) {
    let files = spec_cases(requirement);
    assert!(!files.is_empty(), "{requirement} names no Cases files");
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for file in &files {
        let fixture = fixture_of(file);
        let table = case_table(file);
        let mut rows: Vec<Row> = Vec::new();
        for r in &table.rows {
            let get = |name: &str| cell(&table, &r.cells, name);
            let built = (|| -> Result<Option<Row>, String> {
                if get("Requirement")? != requirement {
                    return Ok(None);
                }
                let results = clean(get("Results")?);
                Ok(Some(Row {
                    case: &r.cells[0],
                    results: (!results.is_empty() && results != "none")
                        .then(|| results.to_string()),
                    claim: get("Claim contains")?,
                    status: clean(get("Status")?),
                    category: get("Category").unwrap_or(""),
                    location: get("Location").unwrap_or(""),
                    line: r.line,
                }))
            })();
            match built {
                Ok(Some(row)) => rows.push(row),
                Ok(None) => {}
                Err(e) => failures.push(format!("{file}:{}: {e}", r.line)),
            }
        }
        if rows.is_empty() {
            failures.push(format!("{file}: no row belongs to {requirement}"));
            continue;
        }
        let mut groups: Vec<Option<String>> = Vec::new();
        for r in &rows {
            if !groups.contains(&r.results) {
                groups.push(r.results.clone());
            }
        }
        for group in groups {
            let scanned = scan_with(&fixture, group.as_deref()).await;
            for row in rows.iter().filter(|r| r.results == group) {
                checked += 1;
                if let Err(why) = check_row(&scanned.model, row) {
                    failures.push(format!(
                        "case `{}` ({file}:{}, fixture `{fixture}`, results {}): {why}",
                        row.case,
                        row.line,
                        group.as_deref().unwrap_or("none"),
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {checked} conformance row(s) for {requirement} failed:\n\n{}\n",
        failures.len(),
        failures.join("\n\n")
    );
}
