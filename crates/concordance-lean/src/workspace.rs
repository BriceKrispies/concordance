//! Lake project discovery: every directory holding a `lakefile.lean` or `lakefile.toml`
//! (and/or a `lean-toolchain`) under the source roots, the repository root, and the
//! directories above a source root. `.lake` (dependencies, build output) is skipped.

use concordance_core::code::{CrateInfo, WorkspaceInfo};
use concordance_core::RepoPath;
use std::path::{Path, PathBuf};

const SKIP: &[&str] = &[
    ".lake",
    ".git",
    "build",
    "node_modules",
    "target",
    "out",
    "dist",
];

fn is_manifest(name: &str) -> bool {
    matches!(name, "lakefile.lean" | "lakefile.toml" | "lean-toolchain")
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
        } else if is_manifest(&name) {
            out.push(dir.to_path_buf());
        }
    }
}

/// `«my-pkg»` / `My.Pkg` -> `my_pkg` / `My_Pkg`: the first segment of symbol paths.
pub fn normalize_name(name: &str) -> String {
    name.trim_matches(|c| c == '«' || c == '»' || c == '"')
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

/// What `lakefile.lean` declares: package name, `require`d packages, `lean_lib` names.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LakeFile {
    pub name: Option<String>,
    pub requires: Vec<String>,
    pub libs: Vec<String>,
}

/// Read the declarations out of a `lakefile.lean` (line-start `package`, `require`,
/// `lean_lib` commands).
pub fn parse_lakefile_lean(text: &str) -> LakeFile {
    let mut out = LakeFile::default();
    for line in text.lines() {
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let mut words = line.split_whitespace();
        let (Some(kw), Some(arg)) = (words.next(), words.next()) else {
            continue;
        };
        let arg = arg.trim_end_matches([':', ',']);
        match kw {
            "package" if out.name.is_none() => out.name = Some(normalize_name(arg)),
            "require" => out.requires.push(normalize_name(arg)),
            "lean_lib" => out.libs.push(normalize_name(arg)),
            _ => {}
        }
    }
    out
}

pub fn parse_lakefile_toml(text: &str) -> Result<LakeFile, String> {
    let v: toml::Value = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut out = LakeFile {
        name: v.get("name").and_then(|n| n.as_str()).map(normalize_name),
        ..Default::default()
    };
    let names = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
                    .map(normalize_name)
                    .collect()
            })
            .unwrap_or_default()
    };
    out.requires = names("require");
    out.libs = names("lean_lib");
    Ok(out)
}

pub fn discover(repo_root: &Path, src_roots: &[PathBuf]) -> WorkspaceInfo {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if ["lakefile.lean", "lakefile.toml", "lean-toolchain"]
        .iter()
        .any(|m| repo_root.join(m).is_file())
    {
        dirs.push(repo_root.to_path_buf());
    }
    for r in src_roots {
        let mut up = r.parent();
        while let Some(d) = up {
            if !d.starts_with(repo_root) {
                break;
            }
            if ["lakefile.lean", "lakefile.toml", "lean-toolchain"]
                .iter()
                .any(|m| d.join(m).is_file())
            {
                dirs.push(d.to_path_buf());
            }
            up = d.parent();
        }
        walk(r, 0, &mut dirs);
    }
    dirs.sort();
    dirs.dedup();

    let mut info = WorkspaceInfo {
        from_cargo_metadata: false,
        ..Default::default()
    };
    for dir in dirs {
        let mut lake = LakeFile::default();
        let mut manifest: Option<PathBuf> = None;
        let lean = dir.join("lakefile.lean");
        let toml_path = dir.join("lakefile.toml");
        if let Ok(text) = std::fs::read_to_string(&lean) {
            lake = parse_lakefile_lean(&text);
            manifest = Some(lean);
        } else if let Ok(text) = std::fs::read_to_string(&toml_path) {
            match parse_lakefile_toml(&text) {
                Ok(l) => lake = l,
                Err(e) => info.warnings.push(format!(
                    "{}: invalid lakefile.toml: {e}",
                    toml_path.display()
                )),
            }
            manifest = Some(toml_path);
        }
        let toolchain = std::fs::read_to_string(dir.join("lean-toolchain"))
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        let manifest = match manifest {
            Some(m) => m,
            None => dir.join("lean-toolchain"),
        };
        let name = lake.name.clone().unwrap_or_else(|| {
            normalize_name(
                &dir.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            )
        });
        let Some(manifest_rel) = RepoPath::from_fs(repo_root, &manifest) else {
            continue;
        };
        // Library root files: `lean_lib Foo` -> `Foo.lean` when it exists.
        let targets: Vec<RepoPath> = lake
            .libs
            .iter()
            .map(|l| dir.join(format!("{l}.lean")))
            .filter(|p| p.is_file())
            .filter_map(|p| RepoPath::from_fs(repo_root, &p))
            .collect();
        info.crates.push(CrateInfo {
            name,
            language: "lean".into(),
            manifest: manifest_rel,
            root_dir: RepoPath::from_fs(repo_root, &dir).filter(|d| !d.as_str().is_empty()),
            targets,
            dependencies: lake.requires,
            features: toolchain
                .into_iter()
                .map(|t| format!("toolchain={t}"))
                .collect(),
        });
    }
    info
}
