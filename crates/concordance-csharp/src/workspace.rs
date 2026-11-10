//! Project discovery: every `.csproj` under the repository, plus the projects named by
//! `.sln` files. The project name is the assembly name (`<AssemblyName>`, else the file
//! stem).

use concordance_core::code::{CrateInfo, WorkspaceInfo};
use concordance_core::RepoPath;
use std::path::{Path, PathBuf};

const SKIP: &[&str] = &[
    "bin",
    "obj",
    "node_modules",
    ".git",
    "packages",
    "target",
    "dist",
    ".vs",
];

/// `Acme.Core` -> `Acme_Core`: the segment used as the first element of symbol paths.
pub fn normalize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
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
        } else {
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".csproj") || lower.ends_with(".sln") {
                out.push(p);
            }
        }
    }
}

/// Project files named by a solution: `Project("{..}") = "Name", "src\App\App.csproj", "{..}"`.
fn solution_projects(sln: &Path, text: &str) -> Vec<PathBuf> {
    let dir = sln.parent().unwrap_or(Path::new("."));
    text.lines()
        .filter(|l| l.trim_start().starts_with("Project("))
        .filter_map(|l| {
            let rest = l.split_once('=')?.1;
            let parts: Vec<&str> = rest
                .split(',')
                .map(|p| p.trim().trim_matches('"'))
                .collect();
            let rel = parts.get(1)?;
            rel.to_ascii_lowercase()
                .ends_with(".csproj")
                .then(|| dir.join(rel.replace('\\', "/")))
        })
        .collect()
}

/// Values of `attr="..."` on every `<tag ...>` element.
fn tag_attrs(text: &str, tag: &str, attr: &str) -> Vec<String> {
    let open = format!("<{tag}");
    let key = format!("{attr}=\"");
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(&open) {
        rest = &rest[i + open.len()..];
        let Some(end) = rest.find('>') else { break };
        let el = &rest[..end];
        if let Some(k) = el.find(&key) {
            let v = &el[k + key.len()..];
            if let Some(q) = v.find('"') {
                out.push(v[..q].to_string());
            }
        }
    }
    out
}

/// Text of the first `<tag>..</tag>` element.
fn tag_text(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let i = text.find(&open)? + open.len();
    let j = text[i..].find(&close)?;
    let v = text[i..i + j].trim();
    (!v.is_empty()).then(|| v.to_string())
}

pub fn discover(repo_root: &Path, src_roots: &[PathBuf]) -> WorkspaceInfo {
    let mut found: Vec<PathBuf> = Vec::new();
    walk(repo_root, 0, &mut found);
    for r in src_roots {
        if !r.starts_with(repo_root) {
            walk(r, 0, &mut found);
        }
    }
    let mut projects: Vec<PathBuf> = Vec::new();
    for f in &found {
        let is_sln = f.extension().is_some_and(|e| e.eq_ignore_ascii_case("sln"));
        if is_sln {
            if let Ok(text) = std::fs::read_to_string(f) {
                projects.extend(solution_projects(f, &text));
            }
        } else {
            projects.push(f.clone());
        }
    }
    projects.sort();
    projects.dedup();

    let mut info = WorkspaceInfo {
        from_cargo_metadata: false,
        ..Default::default()
    };
    for m in projects {
        let Ok(text) = std::fs::read_to_string(&m) else {
            info.warnings
                .push(format!("{}: project file not readable", m.display()));
            continue;
        };
        let dir = m.parent().unwrap_or(repo_root);
        let stem = m
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = normalize_name(&tag_text(&text, "AssemblyName").unwrap_or(stem));
        let mut deps: Vec<String> = Vec::new();
        for p in tag_attrs(&text, "ProjectReference", "Include") {
            let file = p.replace('\\', "/");
            let stem = file
                .rsplit('/')
                .next()
                .unwrap_or("")
                .trim_end_matches(".csproj");
            deps.push(normalize_name(stem));
        }
        for p in tag_attrs(&text, "PackageReference", "Include") {
            deps.push(normalize_name(&p));
        }
        let Some(manifest) = RepoPath::from_fs(repo_root, &m) else {
            continue;
        };
        info.crates.push(CrateInfo {
            name,
            language: "csharp".into(),
            manifest,
            root_dir: RepoPath::from_fs(repo_root, dir).filter(|d| !d.as_str().is_empty()),
            targets: vec![],
            dependencies: deps,
            features: vec![],
        });
    }
    info
}
