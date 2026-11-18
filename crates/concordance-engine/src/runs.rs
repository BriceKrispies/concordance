//! Recorded test runs: the manifest `concordance record` writes beside a results file, and how
//! a run read from a results file is bound to the content it describes.
//!
//! Concordance does not run tests, so it cannot know which code a results file was produced
//! from. `record` is the user saying "these results describe the sources and the specification
//! as they are right now": it stores a hash of every indexed source file and of every
//! requirement statement. `scan` compares the manifest with the current content. What stays
//! trusted is that the user recorded immediately after the run and did not edit in between.

use concordance_core::config::Config;
use concordance_core::testrun::{
    normalize_crate, RunBinding, RunManifest, RunScope, MANIFEST_FORMAT,
};
use concordance_core::{ContentHash, RepoPath};
use concordance_graph::Model;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The manifest that belongs to results file `results`: its path plus `.manifest.json`.
pub fn manifest_path(results: &Path) -> PathBuf {
    let mut s = results.as_os_str().to_owned();
    s.push(".manifest.json");
    PathBuf::from(s)
}

/// The results files and their manifests: the files a change to which re-reads the runs.
pub fn watched_files(results: &[PathBuf]) -> Vec<PathBuf> {
    results
        .iter()
        .flat_map(|r| [r.clone(), manifest_path(r)])
        .collect()
}

/// The manifest for `results_text` as the model stands now.
pub fn build_manifest(
    model: &Model,
    results_file: &str,
    results_text: &str,
    recorded_at_ms: u64,
) -> RunManifest {
    RunManifest {
        tool: "concordance".to_string(),
        format: MANIFEST_FORMAT,
        results_file: results_file.to_string(),
        results_hash: ContentHash::of_str(results_text),
        recorded_at_ms,
        sources: model
            .files
            .values()
            .map(|f| (f.path.to_string(), f.content_hash.clone()))
            .collect(),
        statements: model.requirement_statements(),
    }
}

/// One results file that `record` bound to the current content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub results: PathBuf,
    pub manifest: PathBuf,
    pub sources: usize,
    pub statements: usize,
}

/// Write a manifest beside every configured results file, from the model's current content.
/// Nothing is written unless every results file can be read.
pub fn record(
    model: &Model,
    config: &Config,
    recorded_at_ms: u64,
) -> Result<Vec<Recorded>, String> {
    if config.test_results.is_empty() {
        return Err(
            "no test results configured: name them with `test_results` in the configuration or `--test-results`"
                .to_string(),
        );
    }
    let mut manifests = Vec::new();
    for p in &config.test_results {
        let text = std::fs::read_to_string(p).map_err(|e| {
            format!(
                "cannot read the results file {}: {e} (run the tests first, then record)",
                p.display()
            )
        })?;
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        manifests.push((p, build_manifest(model, &name, &text, recorded_at_ms)));
    }
    let mut out = Vec::new();
    for (p, m) in manifests {
        let mp = manifest_path(p);
        let json = serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?;
        std::fs::write(&mp, format!("{json}\n"))
            .map_err(|e| format!("cannot write {}: {e}", mp.display()))?;
        out.push(Recorded {
            results: p.clone(),
            manifest: mp,
            sources: m.sources.len(),
            statements: m.statements.len(),
        });
    }
    Ok(out)
}

/// How a run compares with the content it was recorded against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    pub binding: RunBinding,
    pub changed_sources: usize,
    pub changed_example: Option<RepoPath>,
    pub changed_statements: BTreeSet<String>,
}

fn unrecorded(why: String) -> Bound {
    Bound {
        binding: RunBinding::Unrecorded(why),
        changed_sources: 0,
        changed_example: None,
        changed_statements: BTreeSet::new(),
    }
}

/// Compare the manifest beside `results_path` with the model. `text` is the results file's
/// current text.
pub fn bind(model: &Model, results_path: &Path, text: &str) -> Bound {
    let mp = manifest_path(results_path);
    let raw = match std::fs::read_to_string(&mp) {
        Ok(t) => t,
        Err(_) => {
            return unrecorded(format!(
                "no manifest `{}`: run `concordance record` right after the tests",
                mp.display()
            ))
        }
    };
    let manifest: RunManifest = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            return unrecorded(format!(
                "the manifest `{}` is unreadable ({e}): run `concordance record` again",
                mp.display()
            ))
        }
    };
    if manifest.format != MANIFEST_FORMAT {
        return unrecorded(format!(
            "the manifest `{}` has format {} and this build reads format {MANIFEST_FORMAT}: run `concordance record` again",
            mp.display(),
            manifest.format
        ));
    }
    if manifest.results_hash != ContentHash::of_str(text) {
        return unrecorded(format!(
            "the results file changed after `{}` was written: run the tests and `concordance record` again",
            mp.display()
        ));
    }
    let current: BTreeMap<&str, &ContentHash> = model
        .files
        .values()
        .map(|f| (f.path.as_str(), &f.content_hash))
        .collect();
    let mut changed: BTreeSet<&str> = BTreeSet::new();
    for (p, h) in &current {
        if manifest.sources.get(*p) != Some(*h) {
            changed.insert(*p);
        }
    }
    for p in manifest.sources.keys() {
        if !current.contains_key(p.as_str()) {
            changed.insert(p.as_str());
        }
    }
    let statements = model.requirement_statements();
    let changed_statements = statements
        .iter()
        .filter(|(id, h)| manifest.statements.get(*id) != Some(*h))
        .map(|(id, _)| id.clone())
        .collect();
    Bound {
        binding: RunBinding::Recorded,
        changed_sources: changed.len(),
        changed_example: changed
            .iter()
            .next()
            .and_then(|p| RepoPath::from_relative_str(p)),
        changed_statements,
    }
}

/// The crates a scope names, resolved against the indexed files.
fn crates_of(model: &Model, s: &RunScope) -> BTreeSet<String> {
    if let Some(p) = &s.package {
        return BTreeSet::from([p.clone()]);
    }
    let mut out: BTreeSet<String> = BTreeSet::new();
    if let Some(f) = &s.file {
        // `src/lib.rs` is relative to the directory cargo ran in: the repository root, or the
        // package. Take the file itself, else every indexed file that ends with the path.
        let exact = RepoPath::from_relative_str(f).and_then(|rp| model.file_by_path(&rp));
        match exact {
            Some(file) => out.extend(file.crate_name.clone()),
            None => {
                let tail = format!("/{f}");
                out.extend(
                    model
                        .files
                        .values()
                        .filter(|file| file.path.as_str().ends_with(&tail))
                        .filter_map(|file| file.crate_name.clone()),
                );
            }
        }
        if out.len() > 1 {
            if let Some(b) = &s.binary {
                let b = normalize_crate(b);
                if out.contains(&b) {
                    return BTreeSet::from([b]);
                }
            }
        }
        return out;
    }
    if let Some(b) = &s.binary {
        let b = normalize_crate(b);
        if model.workspace.crates.iter().any(|c| c.name == b) {
            return BTreeSet::from([b]);
        }
        // An integration test binary is named after its file (`tests/<binary>.rs`).
        let want = format!("{}.rs", s.binary.as_deref().unwrap_or(""));
        out.extend(
            model
                .files
                .values()
                .filter(|f| {
                    let p = f.path.as_str();
                    p.ends_with(&format!("/{want}"))
                        && (p.contains("/tests/") || p.starts_with("tests/"))
                })
                .filter_map(|f| f.crate_name.clone()),
        );
    }
    out
}

/// For each result name that every sighting scoped to a crate, the crates that reported it.
/// A name seen anywhere without a scope gets no entry (it may apply to any crate); a scope that
/// names no crate of this repository leaves `"?"`, which matches no test.
pub fn resolve_crates(
    model: &Model,
    scopes: &BTreeMap<String, BTreeSet<Option<RunScope>>>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    let mut known: BTreeMap<&RunScope, BTreeSet<String>> = BTreeMap::new();
    for (name, seen) in scopes {
        if seen.contains(&None) {
            continue;
        }
        let mut crates: BTreeSet<String> = BTreeSet::new();
        for s in seen.iter().flatten() {
            let of = known.entry(s).or_insert_with(|| crates_of(model, s));
            crates.extend(of.iter().cloned());
        }
        if crates.is_empty() {
            crates.insert("?".to_string());
        }
        out.insert(name.clone(), crates);
    }
    out
}
