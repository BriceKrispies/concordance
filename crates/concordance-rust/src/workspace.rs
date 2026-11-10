//! Cargo workspace discovery: `cargo metadata`, with a manifest-reading fallback.

use concordance_core::code::{CrateInfo, WorkspaceInfo};
use concordance_core::RepoPath;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn discover(repo_root: &Path, src_roots: &[PathBuf]) -> WorkspaceInfo {
    let root = clean(
        &repo_root
            .canonicalize()
            .unwrap_or_else(|_| repo_root.to_path_buf()),
    );
    let mut ws = WorkspaceInfo::default();
    let dirs = candidate_dirs(&root, src_roots);

    let mut any_ok = false;
    let mut tried: BTreeSet<PathBuf> = BTreeSet::new();
    for dir in &dirs {
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() || !tried.insert(dir.clone()) {
            continue;
        }
        // Already covered by a previous workspace run.
        if let Some(rp) = RepoPath::from_fs(&root, &manifest) {
            if ws.crates.iter().any(|c| c.manifest == rp) {
                continue;
            }
        }
        match run_metadata(&root, dir) {
            Ok(crates) => {
                any_ok = true;
                for c in crates {
                    if !ws.crates.iter().any(|x| x.manifest == c.manifest) {
                        ws.crates.push(c);
                    }
                }
            }
            Err(e) => ws
                .warnings
                .push(format!("cargo metadata failed in {}: {e}", dir.display())),
        }
    }

    if any_ok {
        ws.from_cargo_metadata = true;
    } else if tried.is_empty() {
        // No Cargo.toml anywhere: not a Rust repository (or Rust files without a
        // package). Nothing to warn about.
        ws.crates = fallback(&root, src_roots, &dirs);
    } else {
        ws.warnings.push(
            "cargo metadata unavailable; crates found by reading Cargo.toml directly (dependencies and features are partial)"
                .to_string(),
        );
        ws.crates = fallback(&root, src_roots, &dirs);
    }
    ws.crates.sort_by(|a, b| a.manifest.cmp(&b.manifest));
    ws
}

/// Strip the Windows verbatim prefix (`\\?\`) that `canonicalize` adds.
fn clean(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => p.to_path_buf(),
    }
}

/// repo_root, each source root's ancestors up to the repo root, and the immediate
/// subdirectories of each source root, in a stable order without duplicates.
fn candidate_dirs(root: &Path, src_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![root.to_path_buf()];
    let push = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    for sr in src_roots {
        let abs = if sr.is_absolute() {
            sr.clone()
        } else {
            root.join(sr)
        };
        let abs = clean(&abs.canonicalize().unwrap_or(abs));
        let mut cur = Some(abs.as_path());
        while let Some(d) = cur {
            if RepoPath::from_fs(root, d).is_none() && d != root {
                break;
            }
            push(d.to_path_buf(), &mut out);
            if d == root {
                break;
            }
            cur = d.parent();
        }
        if let Ok(rd) = std::fs::read_dir(&abs) {
            let mut subs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            subs.sort();
            for s in subs {
                if !skip_dir(&s) {
                    push(s, &mut out);
                }
            }
        }
    }
    out
}

fn skip_dir(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_none_or(|n| n == "target" || n == ".git" || n == "node_modules" || n.starts_with('.'))
}

fn to_repo(root: &Path, p: &Path) -> Option<RepoPath> {
    let p = clean(p);
    RepoPath::from_fs(root, &p).or_else(|| {
        let c = clean(&p.canonicalize().ok()?);
        RepoPath::from_fs(root, &c)
    })
}

fn norm(name: &str) -> String {
    name.replace('-', "_")
}

fn run_metadata(root: &Path, dir: &Path) -> Result<Vec<CrateInfo>, String> {
    let out = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(dir)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().next().unwrap_or("non-zero exit").to_string());
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    let mut crates = vec![];
    for p in v
        .get("packages")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
    {
        let (Some(name), Some(manifest)) = (
            p.get("name").and_then(|n| n.as_str()),
            p.get("manifest_path").and_then(|m| m.as_str()),
        ) else {
            continue;
        };
        let manifest_fs = PathBuf::from(manifest);
        let Some(manifest_rp) = to_repo(root, &manifest_fs) else {
            continue;
        };
        let root_dir = manifest_fs.parent().and_then(|d| to_repo(root, d));
        let mut targets: Vec<RepoPath> = vec![];
        for t in p
            .get("targets")
            .and_then(|t| t.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(rp) = t
                .get("src_path")
                .and_then(|s| s.as_str())
                .and_then(|s| to_repo(root, Path::new(s)))
            {
                if !targets.contains(&rp) {
                    targets.push(rp);
                }
            }
        }
        let mut deps: BTreeSet<String> = BTreeSet::new();
        for d in p
            .get("dependencies")
            .and_then(|d| d.as_array())
            .into_iter()
            .flatten()
        {
            let kind = d.get("kind").and_then(|k| k.as_str());
            if matches!(kind, None | Some("build")) {
                if let Some(n) = d.get("name").and_then(|n| n.as_str()) {
                    deps.insert(norm(n));
                }
            }
        }
        let features: Vec<String> = p
            .get("features")
            .and_then(|f| f.as_object())
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        crates.push(CrateInfo {
            name: norm(name),
            language: "rust".to_string(),
            manifest: manifest_rp,
            root_dir,
            targets,
            dependencies: deps.into_iter().collect(),
            features,
        });
    }
    Ok(crates)
}

// ---- fallback: read Cargo.toml directly ----------------------------------------------

fn fallback(root: &Path, src_roots: &[PathBuf], dirs: &[PathBuf]) -> Vec<CrateInfo> {
    let mut manifests: Vec<PathBuf> = vec![];
    for d in dirs {
        let m = d.join("Cargo.toml");
        if m.is_file() && !manifests.contains(&m) {
            manifests.push(m);
        }
    }
    let walk_roots: Vec<PathBuf> = if src_roots.is_empty() {
        vec![root.to_path_buf()]
    } else {
        src_roots
            .iter()
            .map(|s| {
                if s.is_absolute() {
                    s.clone()
                } else {
                    root.join(s)
                }
            })
            .collect()
    };
    for wr in &walk_roots {
        walk(wr, 0, &mut manifests);
    }
    let mut crates: Vec<CrateInfo> = vec![];
    for m in manifests {
        let Ok(text) = std::fs::read_to_string(&m) else {
            continue;
        };
        let Some(info) = parse_manifest(root, &m, &text) else {
            continue;
        };
        if !crates.iter().any(|c| c.manifest == info.manifest) {
            crates.push(info);
        }
    }
    crates
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 12 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if !skip_dir(&p) {
                walk(&p, depth + 1, out);
            }
        } else if p.file_name().is_some_and(|n| n == "Cargo.toml") && !out.contains(&p) {
            out.push(p);
        }
    }
}

fn parse_manifest(root: &Path, manifest: &Path, text: &str) -> Option<CrateInfo> {
    let mut section = String::new();
    let mut name: Option<String> = None;
    let mut deps: BTreeSet<String> = BTreeSet::new();
    let mut features: Vec<String> = vec![];
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(h) = line.strip_prefix('[') {
            section = h
                .trim_end_matches(']')
                .trim()
                .trim_matches('[')
                .trim()
                .to_string();
            for prefix in ["dependencies.", "build-dependencies."] {
                if let Some(d) = section.strip_prefix(prefix) {
                    deps.insert(norm(d.trim().trim_matches('"')));
                }
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        match section.as_str() {
            "package" if key == "name" => {
                name = Some(value.trim().trim_matches('"').to_string());
            }
            "dependencies" | "build-dependencies" => {
                let k = key
                    .split('.')
                    .next()
                    .unwrap_or(key)
                    .trim()
                    .trim_matches('"');
                deps.insert(norm(k));
            }
            "features" => features.push(key.trim_matches('"').to_string()),
            _ => {}
        }
    }
    let name = name?;
    let dir = manifest.parent()?;
    let mut targets = vec![];
    for rel in ["src/lib.rs", "src/main.rs"] {
        let p = dir.join(rel);
        if p.is_file() {
            targets.extend(to_repo(root, &p));
        }
    }
    if let Ok(rd) = std::fs::read_dir(dir.join("src").join("bin")) {
        let mut bins: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "rs"))
            .collect();
        bins.sort();
        for b in bins {
            targets.extend(to_repo(root, &b));
        }
    }
    Some(CrateInfo {
        name: norm(&name),
        language: "rust".to_string(),
        manifest: to_repo(root, manifest)?,
        root_dir: to_repo(root, dir),
        targets,
        dependencies: deps.into_iter().collect(),
        features,
    })
}
