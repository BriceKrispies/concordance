//! Module path of a file inside its crate.

use concordance_core::code::CrateInfo;
use concordance_core::RepoPath;

/// `src/lib.rs`, `src/main.rs`, target roots -> `[]`; `src/a.rs`, `src/a/mod.rs` -> `["a"]`;
/// `src/a/b.rs` -> `["a","b"]`; `src/bin/x.rs` -> `[]` (APPROXIMATION: the binary's own
/// crate root; its name is not part of the module path); `tests|examples|benches/..` ->
/// `["tests", ..]`. Without a crate, the file stem (or parent dir for `mod.rs`).
pub(crate) fn module_path(path: &RepoPath, krate: Option<&CrateInfo>) -> Vec<String> {
    let Some(c) = krate else {
        return stem_path(path);
    };
    let rel = match &c.root_dir {
        Some(d) => path
            .as_str()
            .strip_prefix(d.as_str())
            .map(|r| r.trim_start_matches('/'))
            .unwrap_or(path.as_str()),
        None => path.as_str(),
    };
    let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    match segs.first().copied() {
        Some(first @ ("tests" | "examples" | "benches")) if segs.len() >= 2 => {
            let mut out = vec![first.to_string()];
            out.extend(mods(&segs[1..]));
            out
        }
        _ if c.targets.iter().any(|t| t == path) => vec![],
        Some("src") => {
            let rest = &segs[1..];
            if rest.first() == Some(&"bin") {
                let after: &[&str] = if rest.len() > 2 { &rest[2..] } else { &[] };
                let m = mods(after);
                return if m == ["main"] { vec![] } else { m };
            }
            let m = mods(rest);
            if m == ["lib"] || m == ["main"] {
                vec![]
            } else {
                m
            }
        }
        _ => mods(&segs),
    }
}

fn mods(segs: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = segs.iter().map(|s| s.to_string()).collect();
    if let Some(last) = v.last_mut() {
        if let Some(stripped) = last.strip_suffix(".rs") {
            *last = stripped.to_string();
        }
    }
    if v.last().map(String::as_str) == Some("mod") {
        v.pop();
    }
    v
}

fn stem_path(path: &RepoPath) -> Vec<String> {
    let segs: Vec<&str> = path.as_str().split('/').collect();
    let stem = path
        .file_name()
        .strip_suffix(".rs")
        .unwrap_or(path.file_name());
    match stem {
        "mod" if segs.len() >= 2 => vec![segs[segs.len() - 2].to_string()],
        "lib" | "main" => vec![],
        s => vec![s.to_string()],
    }
}
