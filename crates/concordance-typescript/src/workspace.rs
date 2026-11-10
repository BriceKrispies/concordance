//! npm package discovery: every `package.json` under the source roots (and the
//! repository root), skipping `node_modules` and build output.

use concordance_core::code::{CrateInfo, WorkspaceInfo};
use concordance_core::RepoPath;
use std::path::{Path, PathBuf};

const SKIP: &[&str] = &[
    "node_modules",
    ".git",
    "dist",
    "build",
    "out",
    "coverage",
    "target",
    ".next",
];

/// `@scope/my-pkg` -> `my_pkg`: the segment used as the first element of symbol paths.
pub fn normalize_name(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 8 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !SKIP.contains(&name.as_str()) && !name.starts_with('.') {
                walk(&p, depth + 1, out);
            }
        } else if name == "package.json" {
            out.push(p);
        }
    }
}

pub fn discover(repo_root: &Path, src_roots: &[PathBuf]) -> WorkspaceInfo {
    let mut manifests: Vec<PathBuf> = Vec::new();
    let root_manifest = repo_root.join("package.json");
    if root_manifest.is_file() {
        manifests.push(root_manifest);
    }
    for r in src_roots {
        // Package manifests above a source root (e.g. root `src` inside a package).
        let mut up = r.parent();
        while let Some(d) = up {
            if !d.starts_with(repo_root) {
                break;
            }
            let m = d.join("package.json");
            if m.is_file() {
                manifests.push(m);
            }
            up = d.parent();
        }
        walk(r, 0, &mut manifests);
    }
    manifests.sort();
    manifests.dedup();

    let mut info = WorkspaceInfo {
        from_cargo_metadata: false,
        ..Default::default()
    };
    for m in manifests {
        let Ok(text) = std::fs::read_to_string(&m) else {
            continue;
        };
        let json: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                info.warnings
                    .push(format!("{}: invalid package.json: {e}", m.display()));
                continue;
            }
        };
        let dir = m.parent().unwrap_or(repo_root);
        let name = json
            .get("name")
            .and_then(|n| n.as_str())
            .map(normalize_name)
            .unwrap_or_else(|| {
                normalize_name(
                    &dir.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
            });
        let deps: Vec<String> = ["dependencies", "peerDependencies"]
            .iter()
            .filter_map(|k| json.get(*k).and_then(|d| d.as_object()))
            .flat_map(|o| o.keys().map(|k| normalize_name(k)))
            .collect();
        let Some(manifest) = RepoPath::from_fs(repo_root, &m) else {
            continue;
        };
        info.crates.push(CrateInfo {
            name,
            language: "typescript".into(),
            manifest,
            root_dir: RepoPath::from_fs(repo_root, dir).filter(|d| !d.as_str().is_empty()),
            targets: vec![],
            dependencies: deps,
            features: vec![],
        });
    }
    info
}
