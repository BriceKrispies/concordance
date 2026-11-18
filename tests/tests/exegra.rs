//! Structured specification sources: `fixtures/exegra/markdown` and `fixtures/exegra/jsonc` are
//! the same specification, as Markdown files and as one Exegra JSONC file. Concordance must
//! read both the same way, scan the JSONC incrementally, and expand the real Arcmesh file.

use concordance_core::config::{Config, ConfigFile, Overrides};
use concordance_core::doc::RequirementAttributes;
use concordance_core::event::Event;
use concordance_graph::Model;
use concordance_tests::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const PASSED: &str =
    "running 1 test\ntest tests::empty_manifest_has_no_targets ... ok\ntest result: ok.\n";

fn config(repo: &Path, state: &Path, spec_roots: &[&str]) -> Config {
    let mut file = ConfigFile::default();
    file.authority.normative = vec!["docs/spec/**".into()];
    Config::resolve(
        file,
        None,
        Overrides {
            repo: Some(repo.to_path_buf()),
            spec_roots: spec_roots.iter().map(PathBuf::from).collect(),
            src_roots: vec![PathBuf::from("src")],
            state_dir: Some(state.to_path_buf()),
            test_results: vec![repo.join("results/passed.txt")],
            ..Default::default()
        },
        repo,
        state,
    )
    .unwrap()
}

/// A copy of the fixture with its recorded test run in place (bound to the content by `scan`).
fn fixture(name: &str) -> PathBuf {
    let repo = fixture_copy(name);
    std::fs::write(repo.join("results/passed.txt"), PASSED).unwrap();
    repo
}

async fn scan(repo: &Path, roots: &[&str]) -> (Config, Model) {
    let state = temp_dir("exegra-state");
    let config = config(repo, &state, roots);
    let r = start(config.clone(), None, 8192).await;
    r.idle().await;
    record_and_rescan(&r, &config).await;
    let model = r.engine.shared.read(|m| m.clone()).0;
    r.stop().await;
    (config, model)
}

/// Everything a scan established, minus line numbers and origins.
#[derive(Debug, PartialEq)]
struct Summary {
    documents: BTreeSet<String>,
    /// Requirement id -> (document, statement text, attributes).
    requirements: BTreeMap<String, (String, String, RequirementAttributes)>,
    /// (document, reference text, target, strength).
    references: BTreeSet<(String, String, String, String)>,
    /// (path, rule, severity, message).
    lint: Vec<(String, String, String, String)>,
    /// Requirement id -> status.
    statuses: BTreeMap<String, String>,
}

fn summary(config: &Config, m: &Model) -> Summary {
    let path_of = |c: &concordance_core::doc::Claim| m.documents[&c.document].path.to_string();
    let mut requirements = BTreeMap::new();
    let mut statuses = BTreeMap::new();
    let mut references = BTreeSet::new();
    for c in m.claims.values() {
        for r in &c.references {
            references.insert((
                path_of(c),
                r.text.clone(),
                format!("{:?}", r.target),
                format!("{:?}", r.strength),
            ));
        }
        let Some(req) = &c.requirement else { continue };
        requirements.insert(
            req.id.clone(),
            (path_of(c), c.text.clone(), req.attributes.clone()),
        );
        if let Some(f) = m.claim_finding(&c.id).filter(|f| f.is_open()) {
            statuses.insert(req.id.clone(), f.status.as_str().to_string());
        }
    }
    for refs in m.prose_refs.values() {
        for p in refs {
            references.insert((
                "prose".to_string(),
                p.reference.text.clone(),
                format!("{:?}", p.reference.target),
                format!("{:?}", p.reference.strength),
            ));
        }
    }
    let mut lint: Vec<_> = concordance_lint::lint_repo(m, config, false)
        .findings
        .into_iter()
        .map(|f| {
            (
                f.path,
                f.rule.to_string(),
                f.severity.as_str().to_string(),
                f.message,
            )
        })
        .collect();
    lint.sort();
    Summary {
        documents: m.documents.values().map(|d| d.path.to_string()).collect(),
        requirements,
        references,
        lint,
        statuses,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn markdown_and_jsonc_forms_are_analysed_the_same() {
    let md_repo = fixture("exegra/markdown");
    let js_repo = fixture("exegra/jsonc");
    let (md_config, md) = scan(&md_repo, &["docs", "cases"]).await;
    let (js_config, js) = scan(&js_repo, &["specs/spec.jsonc"]).await;
    assert!(
        js.virtual_diagnostics.is_empty(),
        "{:?}",
        js.virtual_diagnostics
    );
    assert!(md.virtual_docs.is_empty());
    assert_eq!(js.virtual_docs.len(), 5);

    let a = summary(&md_config, &md);
    let b = summary(&js_config, &js);

    // What the fixture is meant to show, so that "the same" is not "both empty".
    assert_eq!(a.documents.len(), 5, "{:?}", a.documents);
    assert_eq!(a.requirements.len(), 6);
    assert_eq!(
        a.requirements["CLI-004"].2.status.as_deref(),
        Some("retired")
    );
    assert_eq!(
        a.requirements["LIB-001"].2.cases.as_deref(),
        Some("cases/flows.md")
    );
    assert!(
        a.lint
            .iter()
            .any(|f| f.1 == "R1" && f.3.contains("CLI-002")),
        "{:?}",
        a.lint
    );
    assert!(
        a.lint
            .iter()
            .any(|f| f.1 == "R2" && f.3.contains("CLI-003")),
        "{:?}",
        a.lint
    );
    assert_eq!(a.statuses["LIB-002"], "verified");
    assert_eq!(a.statuses["CLI-001"], "unverified");

    assert_eq!(a.documents, b.documents);
    assert_eq!(a.requirements, b.requirements);
    assert_eq!(a.references, b.references);
    assert_eq!(a.lint, b.lint);
    assert_eq!(a.statuses, b.statuses);
}

fn edit(path: &Path, from: &str, to: &str) {
    let s = std::fs::read_to_string(path).unwrap();
    assert!(s.contains(from), "{} contains {from:?}", path.display());
    std::fs::write(path, s.replacen(from, to, 1)).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn editing_one_statement_reindexes_only_its_document() {
    let repo = fixture("exegra/jsonc");
    let state = temp_dir("exegra-incr-state");
    let mut r = start(config(&repo, &state, &["specs/spec.jsonc"]), None, 8192).await;
    r.idle().await;
    r.drain();

    edit(
        &repo.join("specs/spec.jsonc"),
        "for an empty manifest",
        "for a blank manifest",
    );
    r.engine
        .notify_changed(vec![repo.join("specs/spec.jsonc")])
        .await;
    r.idle().await;
    let events = r.drain();
    let indexed: Vec<String> = events
        .iter()
        .filter_map(|e| match &e.event {
            Event::DocumentIndexed { path, .. } => Some(path.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(indexed, vec!["docs/spec/lib.md".to_string()]);
    assert!(!has(&events, |e| matches!(
        e,
        Event::DocumentRemoved { .. }
    )));
    let (m, _) = r.engine.shared.read(|m| m.clone());
    let claim = m.claims_of_requirement("LIB-002").next().expect("LIB-002");
    assert!(claim.text.contains("blank manifest"), "{}", claim.text);

    // The same edit again changes nothing.
    r.drain();
    r.engine
        .notify_changed(vec![repo.join("specs/spec.jsonc")])
        .await;
    r.idle().await;
    assert!(!has(&r.drain(), |e| matches!(
        e,
        Event::DocumentIndexed { .. }
    )));
    r.stop().await;
}
