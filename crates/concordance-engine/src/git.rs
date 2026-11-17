//! Read-only access to the repositories that `[ALIAS:path]` citations point into, through
//! the git CLI at fixed revisions. Nothing here reads or writes a working tree.

use concordance_core::config::{Config, RepoConfig};
use concordance_reconcile::citation::{CitedRepo, CitedRepos, Entry, Rev};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};

/// Blobs larger than this are treated as unreadable (no line checks, no drift).
const MAX_BLOB: usize = 8 * 1024 * 1024;

fn git(repo: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C")
        .arg(repo)
        .arg("-c")
        .arg("core.quotepath=off")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    c
}

fn run(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = git(repo)
        .args(args)
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(out.stdout)
}

/// The commit a revision names, if any.
fn resolve_commit(repo: &Path, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    let out = run(repo, &["rev-parse", "--verify", "--quiet", &spec]).ok()?;
    let s = String::from_utf8_lossy(&out).trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[derive(Default)]
struct Tree {
    /// path -> blob id
    files: HashMap<String, String>,
    dirs: HashSet<String>,
}

/// A long-lived `git cat-file --batch`.
struct CatFile {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl CatFile {
    fn start(repo: &Path) -> Option<CatFile> {
        let mut child = git(repo)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?);
        Some(CatFile {
            child,
            stdin,
            stdout,
        })
    }

    fn read(&mut self, object: &str) -> Option<Vec<u8>> {
        writeln!(self.stdin, "{object}").ok()?;
        self.stdin.flush().ok()?;
        let mut header = String::new();
        self.stdout.read_line(&mut header).ok()?;
        let mut parts = header.split_whitespace();
        let _id = parts.next()?;
        let kind = parts.next()?;
        if kind == "missing" || kind == "ambiguous" {
            return None;
        }
        let size: usize = parts.next()?.parse().ok()?;
        let mut buf = vec![0u8; size + 1]; // content plus the trailing newline
        self.stdout.read_exact(&mut buf).ok()?;
        buf.pop();
        (kind == "blob").then_some(buf)
    }
}

impl Drop for CatFile {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Repo {
    info: CitedRepo,
    config: RepoConfig,
    trees: Mutex<HashMap<Rev, Arc<Tree>>>,
    blobs: Mutex<HashMap<String, Option<Arc<str>>>>,
    cat: Mutex<Option<CatFile>>,
    renames: OnceLock<HashMap<String, String>>,
}

impl Repo {
    fn commit(&self, rev: Rev) -> Option<&str> {
        match rev {
            Rev::Cited => Some(&self.info.cited_commit),
            Rev::Compare => self.info.compare.as_ref().map(|c| c.1.as_str()),
        }
    }

    fn tree(&self, rev: Rev) -> Arc<Tree> {
        let mut trees = self.trees.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = trees.get(&rev) {
            return t.clone();
        }
        let mut tree = Tree::default();
        if let Some(commit) = self.commit(rev) {
            if let Ok(out) = run(&self.config.path, &["ls-tree", "-r", "-t", "-z", commit]) {
                for rec in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
                    let rec = String::from_utf8_lossy(rec);
                    let Some((meta, path)) = rec.split_once('\t') else {
                        continue;
                    };
                    let mut m = meta.split_whitespace();
                    let (_mode, kind, id) = (m.next(), m.next(), m.next());
                    match (kind, id) {
                        (Some("blob"), Some(id)) => {
                            tree.files.insert(path.to_string(), id.to_string());
                        }
                        // Trees and submodules (`commit`) are directories to a reader.
                        (Some(_), Some(_)) => {
                            tree.dirs.insert(path.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
        let tree = Arc::new(tree);
        trees.insert(rev, tree.clone());
        tree
    }

    fn blob_text(&self, blob: &str) -> Option<Arc<str>> {
        if let Some(t) = self
            .blobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(blob)
        {
            return t.clone();
        }
        let bytes = {
            let mut cat = self.cat.lock().unwrap_or_else(|e| e.into_inner());
            if cat.is_none() {
                *cat = CatFile::start(&self.config.path);
            }
            let got = cat.as_mut().and_then(|c| c.read(blob));
            if got.is_none() {
                *cat = None; // the stream may be out of step; restart on next use
            }
            got
        };
        let text: Option<Arc<str>> = bytes
            .filter(|b| b.len() <= MAX_BLOB)
            .map(|b| Arc::from(String::from_utf8_lossy(&b).into_owned()));
        self.blobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(blob.to_string(), text.clone());
        text
    }

    fn renames(&self) -> &HashMap<String, String> {
        self.renames.get_or_init(|| {
            let mut map = HashMap::new();
            let Some((_, compare)) = &self.info.compare else {
                return map;
            };
            let Ok(out) = run(
                &self.config.path,
                &[
                    "diff",
                    "--name-status",
                    "-z",
                    "-M",
                    "--diff-filter=R",
                    &self.info.cited_commit,
                    compare,
                ],
            ) else {
                return map;
            };
            let fields: Vec<String> = out
                .split(|b| *b == 0)
                .map(|f| String::from_utf8_lossy(f).into_owned())
                .collect();
            let mut it = fields.iter();
            while let Some(status) = it.next() {
                if status.starts_with('R') {
                    if let (Some(from), Some(to)) = (it.next(), it.next()) {
                        map.insert(from.clone(), to.clone());
                    }
                }
            }
            map
        })
    }
}

/// The configured cited repositories, with revisions resolved once at startup.
pub struct GitRepos {
    repos: Vec<Repo>,
}

impl GitRepos {
    /// Resolve every `[repos.*]` entry. An unusable repository or cited revision is an
    /// error; an unresolvable `compare` is a warning (drift is then not checked).
    pub fn open(config: &Config) -> Result<(GitRepos, Vec<String>), String> {
        let mut repos = Vec::new();
        let mut warnings = Vec::new();
        for r in &config.repos {
            if run(&r.path, &["rev-parse", "--git-dir"]).is_err() {
                return Err(format!(
                    "[repos.{}]: {} is not a git repository",
                    r.alias,
                    r.path.display()
                ));
            }
            let cited_commit = resolve_commit(&r.path, &r.cited).ok_or_else(|| {
                format!(
                    "[repos.{}]: cited revision `{}` does not exist in {}",
                    r.alias,
                    r.cited,
                    r.path.display()
                )
            })?;
            let compare = match &r.compare {
                Some(c) => match resolve_commit(&r.path, c) {
                    Some(commit) => Some((c.clone(), commit)),
                    None => {
                        warnings.push(format!(
                            "[repos.{}]: compare revision `{c}` does not exist in {}; drift is not checked",
                            r.alias,
                            r.path.display()
                        ));
                        None
                    }
                },
                None => None,
            };
            repos.push(Repo {
                info: CitedRepo {
                    alias: r.alias.clone(),
                    cited: r.cited.clone(),
                    cited_commit,
                    compare,
                },
                config: r.clone(),
                trees: Mutex::new(HashMap::new()),
                blobs: Mutex::new(HashMap::new()),
                cat: Mutex::new(None),
                renames: OnceLock::new(),
            });
        }
        Ok((GitRepos { repos }, warnings))
    }

    pub fn is_empty(&self) -> bool {
        self.repos.is_empty()
    }

    /// Changes whenever a configured repository or a resolved revision changes, so
    /// persisted citation evidence is re-derived (e.g. after `git fetch` moves `compare`).
    pub fn fingerprint(&self) -> String {
        let parts: Vec<String> = self
            .repos
            .iter()
            .map(|r| {
                format!(
                    "{}|{}|{}|{}|{}",
                    r.info.alias,
                    r.config.spellings.join(","),
                    r.config.path.display(),
                    r.info.cited_commit,
                    r.info.compare.as_ref().map(|c| c.1.as_str()).unwrap_or("")
                )
            })
            .collect();
        let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
        concordance_core::short_hash(&refs)
    }

    fn find(&self, repo: &CitedRepo) -> Option<&Repo> {
        self.repos.iter().find(|r| r.info.alias == repo.alias)
    }

    /// Paths of the configured repositories (for messages).
    pub fn paths(&self) -> Vec<PathBuf> {
        self.repos.iter().map(|r| r.config.path.clone()).collect()
    }
}

impl CitedRepos for GitRepos {
    fn repo(&self, alias: &str) -> Option<CitedRepo> {
        self.repos
            .iter()
            .find(|r| r.config.matches(alias))
            .map(|r| r.info.clone())
    }

    fn entry(&self, repo: &CitedRepo, rev: Rev, path: &str) -> Entry {
        let Some(r) = self.find(repo) else {
            return Entry::Missing;
        };
        if rev == Rev::Compare && repo.compare.is_none() {
            return Entry::Missing;
        }
        let t = r.tree(rev);
        if let Some(blob) = t.files.get(path) {
            return Entry::File { blob: blob.clone() };
        }
        if t.dirs.contains(path) {
            return Entry::Dir;
        }
        Entry::Missing
    }

    fn entries_in(&self, repo: &CitedRepo, rev: Rev, dir: &str) -> Vec<String> {
        let Some(r) = self.find(repo) else {
            return Vec::new();
        };
        let t = r.tree(rev);
        let mut out: Vec<String> = t
            .files
            .keys()
            .chain(t.dirs.iter())
            .filter(|p| match p.rsplit_once('/') {
                Some((d, _)) => d == dir,
                None => dir.is_empty(),
            })
            .cloned()
            .collect();
        out.sort();
        out
    }

    fn glob(&self, repo: &CitedRepo, rev: Rev, pattern: &str, limit: usize) -> Vec<String> {
        let Some(r) = self.find(repo) else {
            return Vec::new();
        };
        let Ok(g) = globset::GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map(|g| g.compile_matcher())
        else {
            return Vec::new();
        };
        let t = r.tree(rev);
        let mut out: Vec<String> = t
            .files
            .keys()
            .chain(t.dirs.iter())
            .filter(|p| g.is_match(p.as_str()))
            .take(limit)
            .cloned()
            .collect();
        out.sort();
        out
    }

    fn text(&self, repo: &CitedRepo, rev: Rev, path: &str) -> Option<Arc<str>> {
        let r = self.find(repo)?;
        let Entry::File { blob } = self.entry(repo, rev, path) else {
            return None;
        };
        r.blob_text(&blob)
    }

    fn renamed(&self, repo: &CitedRepo, path: &str) -> Option<String> {
        self.find(repo)?.renames().get(path).cloned()
    }
}

/// Check `[repos]` without starting the engine (configuration errors exit early).
pub fn check(config: &Config) -> Result<Vec<String>, String> {
    GitRepos::open(config).map(|(_, w)| w)
}
