//! `[ALIAS:path:lines]` citations checked against a real git repository at two commits.

use concordance_core::config::{Config, ConfigFile, Overrides, RepoFile};
use concordance_core::finding::{Finding, FindingCategory, FindingStatus};
use concordance_engine::citation::summarize;
use concordance_graph::Model;
use concordance_tests::*;
use std::path::{Path, PathBuf};

fn numbered(prefix: &str, n: usize) -> String {
    (1..=n).map(|i| format!("{prefix} line {i}\n")).collect()
}

/// A repository with two commits; returns the first commit's id.
fn cited_repo(dir: &Path) -> String {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    write(&dir.join("src/a.ts"), &numbered("a", 10));
    write(&dir.join("src/gone.ts"), &numbered("gone", 5));
    write(&dir.join("src/moved.ts"), &numbered("moved", 6));
    write(&dir.join("lib/util.ts"), "export const x = 1;\n");
    write(
        &dir.join("docs/adr/0005-choice.md"),
        "# ADR 5\n\n## Decision\n\nWe chose A.\n",
    );
    write(&dir.join("docs/adr/0007-one.md"), "# 7a\n");
    write(&dir.join("docs/adr/0007-two.md"), "# 7b\n");
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "first"]);
    let first = git(dir, &["rev-parse", "HEAD"]);
    write(
        &dir.join("src/a.ts"),
        &numbered("a", 10).replace(
            "a line 3\na line 4\n",
            "a line 3 changed\na line 4 changed\n",
        ),
    );
    std::fs::remove_file(dir.join("src/gone.ts")).unwrap();
    write(
        &dir.join("src/moved.ts"),
        &format!("inserted\ninserted\n{}", numbered("moved", 6)),
    );
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "second"]);
    first
}

const SPEC: &str = "# Citations

| Fact | Source |
|---|---|
| Input is read first. | [CIT:src/a.ts:1-2] |
| The limit is applied. | [CIT:src/a.ts:3-4] |
| The tail is long. | [CIT:src/a.ts:8-20] |
| A decision was taken. | [CIT:docs/adr/0005] |
| One of two. | [CIT:docs/adr/0007] |
| The section exists. | [cit:docs/adr/0005-choice.md \"Decision\"] |
| The section is missing. | [CIT:docs/adr/0005-choice.md \"Nope\"] |
| Gone later. | [CIT:src/gone.ts:1-2] |
| Moved later. | [CIT:src/moved.ts:1-3] |
| Never there. | [CIT:src/none.ts] |
| A directory. | [CIT:lib/] |
| A directory with lines. | [CIT:lib:1-2] |
| Elsewhere. | [OTHER:src/x.ts:1] |
| In code. | `[CIT:src/none.ts]` |
";

fn config(ws: &Path, cited_dir: &Path, cited: &str, compare: Option<&str>, state: &Path) -> Config {
    let mut file = ConfigFile::default();
    file.repos.insert(
        "CIT".into(),
        RepoFile {
            path: cited_dir.to_path_buf(),
            cited: cited.into(),
            compare: compare.map(String::from),
            spellings: vec![],
        },
    );
    Config::resolve(
        file,
        None,
        Overrides {
            repo: Some(ws.to_path_buf()),
            spec_roots: vec![PathBuf::from("docs")],
            src_roots: vec![PathBuf::from("docs")],
            state_dir: Some(state.to_path_buf()),
            ..Default::default()
        },
        ws,
        state,
    )
    .unwrap()
}

/// The open finding located at a citation on spec line `line`.
fn at_line(m: &Model, line: u32) -> Option<&Finding> {
    m.findings.values().find(|f| {
        f.is_open()
            && f.locations.iter().any(|l| {
                l.path.as_str() == "docs/spec.md" && l.start_line == line && l.start_col > 1
            })
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn citations_are_checked_against_the_cited_revision_and_drift() {
    let root = temp_dir("citations");
    let cited_dir = root.join("cited");
    let first = cited_repo(&cited_dir);
    let ws = root.join("ws");
    write(&ws.join("docs/spec.md"), SPEC);
    let state = root.join("state");
    let cfg = config(&ws, &cited_dir, &first[..8], Some("main"), &state);
    let repos = cfg.repos.clone();

    let r = start(cfg.clone(), None, 8192).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;
    let s = summarize(&m, &repos).expect("summary");
    let cit = s.aliases.iter().find(|a| a.alias == "CIT").unwrap();
    assert_eq!(cit.citations, 12, "{cit:?}");
    assert_eq!(cit.resolved, 6, "{cit:?}");
    assert_eq!(cit.prefix_resolved, 1, "{cit:?}");
    assert_eq!(cit.ambiguous, 1, "{cit:?}");
    assert_eq!(cit.missing, 1, "{cit:?}");
    assert_eq!(cit.bad_lines, 3, "{cit:?}");
    assert_eq!(cit.drifted, 2, "{cit:?}");
    assert_eq!(cit.moved, 1, "{cit:?}");
    let other = s.aliases.iter().find(|a| a.alias == "OTHER").unwrap();
    assert!(other.cited.is_none());
    assert!(s.note.as_deref().unwrap_or("").contains("OTHER (1)"));

    // Spec line numbers: the table starts at line 3; row k is line 4 + k.
    let unchanged = at_line(&m, 5);
    assert!(
        unchanged
            .is_none_or(|f| f.category != FindingCategory::CitationDrift
                && f.status != FindingStatus::Violated),
        "{unchanged:?}"
    );
    let drift = at_line(&m, 6).expect("drift finding");
    assert_eq!(drift.status, FindingStatus::Stale);
    assert_eq!(drift.category, FindingCategory::CitationDrift);
    assert!(
        drift.explanation.contains("3,4") || drift.explanation.contains("3-4"),
        "{}",
        drift.explanation
    );
    let past = at_line(&m, 7).expect("past-end finding");
    assert_eq!(past.status, FindingStatus::Violated);
    assert_eq!(past.category, FindingCategory::StructuralContradiction);
    assert!(
        past.explanation.contains("has 10 lines"),
        "{}",
        past.explanation
    );
    let loc = past.locations.iter().find(|l| l.start_line == 7).unwrap();
    let col = SPEC.lines().nth(6).unwrap().find("[CIT").unwrap() as u32 + 2;
    assert_eq!(loc.start_col, col, "points at the citation itself");
    assert!(at_line(&m, 8).is_none_or(|f| f.status != FindingStatus::Violated));
    assert_eq!(
        at_line(&m, 9).expect("ambiguous").status,
        FindingStatus::Ambiguous
    );
    assert!(at_line(&m, 10).is_none_or(|f| f.status != FindingStatus::Violated));
    assert_eq!(
        at_line(&m, 11).expect("missing section").status,
        FindingStatus::Violated
    );
    let gone = at_line(&m, 12).expect("deleted file drift");
    assert_eq!(gone.category, FindingCategory::CitationDrift);
    assert!(gone.explanation.contains("deleted"), "{}", gone.explanation);
    assert!(at_line(&m, 13).is_none_or(|f| f.category != FindingCategory::CitationDrift));
    assert_eq!(
        at_line(&m, 14).expect("missing file").status,
        FindingStatus::Violated
    );
    assert!(at_line(&m, 15).is_none_or(|f| f.status != FindingStatus::Violated));
    assert_eq!(
        at_line(&m, 16).expect("lines on a dir").status,
        FindingStatus::Violated
    );
    assert!(
        at_line(&m, 17).is_none(),
        "unconfigured alias is not a finding"
    );
    assert!(at_line(&m, 18).is_none(), "code spans are not citations");

    // Drift never counts as a proven violation.
    let violated = |m: &Model| {
        let mut v: Vec<String> = m
            .findings
            .values()
            .filter(|f| f.is_open() && f.status == FindingStatus::Violated)
            .map(|f| f.id.to_string())
            .collect();
        v.sort();
        v
    };
    let before = violated(&m);
    assert_eq!(
        before.len(),
        4,
        "past end, missing section, missing file, lines on a dir: {before:?}"
    );
    r.stop().await;

    // A restart keeps identities; dropping `compare` resolves the drift findings.
    let r = start(cfg, None, 8192).await;
    r.idle().await;
    let again = r.engine.shared.read(|m| m.clone()).0;
    assert_eq!(violated(&again), before);
    let drift_id = drift.id.clone();
    assert!(again.findings.get(&drift_id).is_some_and(|f| f.is_open()));
    r.stop().await;

    let r = start(config(&ws, &cited_dir, &first, None, &state), None, 8192).await;
    r.idle().await;
    let no_compare = r.engine.shared.read(|m| m.clone()).0;
    assert!(no_compare
        .findings
        .get(&drift_id)
        .is_some_and(|f| !f.is_open()));
    assert_eq!(violated(&no_compare), before);
    r.stop().await;
}

#[test]
fn unusable_repositories_are_configuration_errors() {
    let root = temp_dir("citations-config");
    let cited_dir = root.join("cited");
    let first = cited_repo(&cited_dir);
    let ws = root.join("ws");
    write(&ws.join("docs/spec.md"), SPEC);
    let state = root.join("state");

    let bad_rev = config(&ws, &cited_dir, "no-such-rev", None, &state);
    let err = concordance_engine::git::check(&bad_rev).unwrap_err();
    assert!(err.contains("no-such-rev"), "{err}");

    let warn = config(&ws, &cited_dir, &first, Some("origin/nope"), &state);
    let w = concordance_engine::git::check(&warn).unwrap();
    assert!(w.iter().any(|w| w.contains("origin/nope")), "{w:?}");

    let resolve = |path: PathBuf| {
        let mut file = ConfigFile::default();
        file.repos.insert(
            "XX".into(),
            RepoFile {
                path,
                cited: "HEAD".into(),
                ..Default::default()
            },
        );
        let o = Overrides {
            repo: Some(ws.clone()),
            state_dir: Some(state.clone()),
            ..Default::default()
        };
        Config::resolve(file, None, o, &ws, &state)
    };
    let err = resolve(root.join("missing")).unwrap_err();
    assert!(err.contains("does not exist"), "{err}");
    let not_git = root.join("plain");
    std::fs::create_dir_all(&not_git).unwrap();
    let err = concordance_engine::git::check(&resolve(not_git).unwrap()).unwrap_err();
    assert!(err.contains("not a git repository"), "{err}");
}
