//! Whether a Rust source file is compiled at all.
//!
//! A `.rs` file under a crate's directory is only part of the build when a crate target (a
//! library, binary, test, example or bench root) reaches it through `mod x;` declarations. A
//! test in a file nothing declares is never compiled, so no run can have executed it and a
//! result line with the same name belongs to some other test.
//!
//! The check follows `mod x;` the way rustc finds the file (`x.rs` or `x/mod.rs` beside the
//! declaring module, honouring inline `mod a { mod x; }` nesting and `#[path = ".."]`). It does
//! not evaluate `cfg`, so a declaration under a disabled `cfg` still counts as reaching the file.

use crate::Model;
use concordance_core::code::{SourceFile, Symbol, SymbolKind, EXTERNAL_MOD_ATTRIBUTE};
use concordance_core::RepoPath;
use std::collections::BTreeSet;

/// `mod name;` (an out-of-line module declaration), as the Rust indexer marks it.
pub fn is_external_mod(sym: &Symbol) -> bool {
    sym.kind == SymbolKind::Module && sym.attributes.iter().any(|a| a == EXTERNAL_MOD_ATTRIBUTE)
}

/// Whether `rel` (a path relative to a crate's directory) is a file Cargo discovers as a target
/// root without any `[[bin]]`/`[[test]]` entry.
fn is_conventional_target(rel: &str) -> bool {
    let segs: Vec<&str> = rel.split('/').collect();
    match segs.as_slice() {
        ["build.rs"] | ["src", "lib.rs"] | ["src", "main.rs"] => true,
        ["src", "bin", f] | ["tests", f] | ["examples", f] | ["benches", f] => f.ends_with(".rs"),
        ["src", "bin", _, "main.rs"]
        | ["tests", _, "main.rs"]
        | ["examples", _, "main.rs"]
        | ["benches", _, "main.rs"] => true,
        _ => false,
    }
}

fn stem(path: &RepoPath) -> &str {
    let n = path.file_name();
    n.strip_suffix(".rs").unwrap_or(n)
}

fn parent(path: &RepoPath) -> String {
    match path.as_str().rfind('/') {
        Some(i) => path.as_str()[..i].to_string(),
        None => String::new(),
    }
}

fn join(dir: &str, rest: &str) -> String {
    if dir.is_empty() {
        rest.to_string()
    } else {
        format!("{dir}/{rest}")
    }
}

/// The value of a `#[path = "..."]` attribute among `attrs`.
fn path_attribute(attrs: &[String]) -> Option<String> {
    attrs.iter().find_map(|a| {
        let compact: String = a.split_whitespace().collect();
        let rest = compact.strip_prefix("#[path=\"")?;
        Some(rest.split('"').next()?.to_string())
    })
}

impl Model {
    /// Whether the source file `file` is part of a compiled crate target. Files that are not
    /// Rust, or that belong to no known crate, are taken as compiled: nothing says otherwise.
    pub fn file_is_compiled(&self, file: &SourceFile) -> bool {
        let mut visiting = BTreeSet::new();
        self.declared(file, &mut visiting)
    }

    /// Whether test `sym` is in compiled code.
    pub fn test_is_compiled(&self, sym: &Symbol) -> bool {
        self.files
            .get(&sym.file)
            .is_none_or(|f| self.file_is_compiled(f))
    }

    fn declared(&self, file: &SourceFile, visiting: &mut BTreeSet<RepoPath>) -> bool {
        if file.language != "rust" {
            return true;
        }
        let Some(krate) = file.crate_name.as_deref() else {
            return true;
        };
        let info = self.workspace.crates.iter().find(|c| c.name == krate);
        let Some(info) = info else { return true };
        if info.targets.contains(&file.path) {
            return true;
        }
        let rel = match &info.root_dir {
            Some(d) => file
                .path
                .as_str()
                .strip_prefix(d.as_str())
                .map(|r| r.trim_start_matches('/'))
                .unwrap_or(file.path.as_str()),
            None => file.path.as_str(),
        };
        if is_conventional_target(rel) {
            return true;
        }
        if !visiting.insert(file.path.clone()) {
            return false; // a cycle of declarations that no target starts
        }
        let found = self
            .symbols
            .values()
            .filter(|m| is_external_mod(m))
            .filter_map(|m| {
                let host = self.files.get(&m.file)?;
                (host.crate_name.as_deref() == Some(krate) && host.language == "rust")
                    .then_some((m, host))
            })
            .any(|(m, host)| {
                self.declaration_files(m, host).contains(&file.path)
                    && self.declared(host, visiting)
            });
        visiting.remove(&file.path);
        found
    }

    /// The paths a `mod name;` declaration in `host` can name.
    fn declaration_files(&self, m: &Symbol, host: &SourceFile) -> Vec<RepoPath> {
        let dir = parent(&host.path);
        let root_like = stem(&host.path) == "mod"
            || self
                .workspace
                .crates
                .iter()
                .any(|c| c.targets.contains(&host.path))
            || host
                .path
                .as_str()
                .rsplit('/')
                .next()
                .is_some_and(|n| n == "lib.rs" || n == "main.rs");
        let mut base = if root_like {
            dir.clone()
        } else {
            join(&dir, stem(&host.path))
        };
        // Inline modules between the file and the declaration push the directory down.
        let skip = 1 + host.module_path.len();
        if m.path.len() > skip + 1 {
            for seg in &m.path[skip..m.path.len() - 1] {
                base = join(&base, seg);
            }
        }
        let mut out = Vec::new();
        if let Some(p) = path_attribute(&m.attributes) {
            // `#[path]` is relative to the declaring file's directory (inline nesting aside).
            let rel = if base == dir {
                join(&dir, &p)
            } else {
                join(&base, &p)
            };
            out.extend(RepoPath::from_relative_str(&rel));
        } else {
            out.extend(RepoPath::from_relative_str(&join(
                &base,
                &format!("{}.rs", m.name),
            )));
            out.extend(RepoPath::from_relative_str(&join(
                &base,
                &format!("{}/mod.rs", m.name),
            )));
        }
        out
    }
}
