//! Filesystem discovery and path classification. The only place that turns absolute
//! paths into `RepoPath`s for analysis.

use concordance_core::config::Config;
use concordance_core::doc::{Authority, AuthorityAssessment, AuthoritySource};
use concordance_core::RepoPath;
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileKind {
    Markdown,
    /// Source code owned by the language plugin with this id.
    Source(&'static str),
}

impl FileKind {
    /// The kind of an indexed source file, from its recorded language.
    pub fn source(language: &str) -> FileKind {
        FileKind::Source(
            crate::languages()
                .by_id(language)
                .map(|p| p.id())
                .unwrap_or("rust"),
        )
    }
}

/// Decides which paths belong to the analysis. Shared by discovery and the watcher
/// so both agree.
pub struct PathFilter {
    root: PathBuf,
    spec_roots: Vec<PathBuf>,
    src_roots: Vec<PathBuf>,
    exclude: GlobSet,
    gitignore: Gitignore,
    state_dir: PathBuf,
}

const SKIP_DIRS: &[&str] = &["target", "node_modules", ".git", ".hg", ".svn"];

fn build_globs(globs: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for g in globs {
        if let Ok(glob) = Glob::new(g) {
            b.add(glob);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

impl PathFilter {
    pub fn new(config: &Config) -> Self {
        let mut gi = GitignoreBuilder::new(&config.repo_root);
        let _ = gi.add(config.repo_root.join(".gitignore"));
        PathFilter {
            root: config.repo_root.clone(),
            spec_roots: config.spec_roots.clone(),
            src_roots: config.src_roots.clone(),
            exclude: build_globs(&config.exclude),
            gitignore: gi.build().unwrap_or_else(|_| Gitignore::empty()),
            state_dir: config.state_dir.clone(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Classify an absolute path. `None` when it is not part of the analysis.
    pub fn classify(&self, abs: &Path) -> Option<(RepoPath, FileKind)> {
        let rel = RepoPath::from_fs(&self.root, abs)?;
        let kind = match rel.extension()? {
            "md" => FileKind::Markdown,
            ext => FileKind::Source(crate::languages().for_extension(ext)?.id()),
        };
        let roots = match kind {
            FileKind::Markdown => &self.spec_roots,
            FileKind::Source(_) => &self.src_roots,
        };
        let under_root = roots.iter().any(|r| {
            RepoPath::from_fs(&self.root, r)
                .map_or(r == &self.root || is_same_dir(r, &self.root), |rr| {
                    rel.starts_with_dir(&rr)
                })
        });
        if !under_root {
            return None;
        }
        let parts: Vec<&str> = rel.as_str().split('/').collect();
        let dirs = &parts[..parts.len() - 1];
        if dirs
            .iter()
            .any(|d| SKIP_DIRS.contains(d) || (d.starts_with('.') && d.len() > 1))
        {
            return None;
        }
        let name = parts[parts.len() - 1];
        if name.starts_with(".#") || name.starts_with('~') || name.ends_with('~') {
            return None;
        }
        if self.exclude.is_match(rel.as_str()) {
            return None;
        }
        if self
            .gitignore
            .matched_path_or_any_parents(rel.as_str(), false)
            .is_ignore()
        {
            return None;
        }
        if RepoPath::from_fs(&self.root, &self.state_dir).is_some_and(|s| rel.starts_with_dir(&s)) {
            return None;
        }
        Some((rel, kind))
    }

    /// Walk every root (respecting .gitignore) and return analysis files, sorted.
    pub fn discover(&self) -> Vec<(RepoPath, FileKind, PathBuf)> {
        let mut out = Vec::new();
        let mut roots: Vec<&PathBuf> = self.spec_roots.iter().chain(&self.src_roots).collect();
        roots.sort();
        roots.dedup();
        for root in roots {
            let walker = ignore::WalkBuilder::new(root)
                .hidden(true)
                .git_ignore(true)
                .git_global(false)
                .git_exclude(false)
                .require_git(false)
                .parents(true)
                .filter_entry(|e| {
                    let name = e.file_name().to_string_lossy();
                    !(e.file_type().is_some_and(|t| t.is_dir())
                        && SKIP_DIRS.contains(&name.as_ref()))
                })
                .build();
            for entry in walker.flatten() {
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    continue;
                }
                if let Some((rel, kind)) = self.classify(entry.path()) {
                    out.push((rel, kind, entry.path().to_path_buf()));
                }
            }
        }
        out.sort();
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }
}

fn is_same_dir(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .eq_ignore_ascii_case(b.to_string_lossy().trim_end_matches(['/', '\\']))
}

/// `[authority]` globs from the configuration file.
pub struct AuthorityRules {
    rules: Vec<(Authority, GlobSet, Vec<String>)>,
}

impl AuthorityRules {
    pub fn new(config: &Config) -> Self {
        let a = &config.authority;
        AuthorityRules {
            rules: vec![
                (
                    Authority::Normative,
                    build_globs(&a.normative),
                    a.normative.clone(),
                ),
                (
                    Authority::Descriptive,
                    build_globs(&a.descriptive),
                    a.descriptive.clone(),
                ),
                (
                    Authority::Informative,
                    build_globs(&a.informative),
                    a.informative.clone(),
                ),
            ],
        }
    }

    pub fn assess(&self, path: &RepoPath) -> Option<AuthorityAssessment> {
        for (level, set, globs) in &self.rules {
            let hits = set.matches(path.as_str());
            if let Some(&i) = hits.first() {
                return Some(AuthorityAssessment {
                    level: *level,
                    source: AuthoritySource::Configured,
                    rationale: format!("matched configured glob `{}`", globs[i]),
                });
            }
        }
        None
    }
}
