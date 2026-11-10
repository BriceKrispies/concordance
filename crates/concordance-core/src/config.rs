//! Configuration: the small TOML file schema, CLI overrides, and the resolved form
//! every subsystem receives. Parsing TOML text happens in the CLI; this module only
//! defines the schema and the merge rules.
//!
//! ```toml
//! repo = "/path/to/repo"          # optional; --repo overrides
//! spec_roots = ["docs"]            # repeatable --spec-root overrides the whole list
//! src_roots = ["crates", "src"]    # repeatable --src-root overrides the whole list
//! exclude = ["docs/archive/**"]    # extra globs, relative to repo
//! state_dir = "C:/somewhere"       # optional; default is outside the repo
//!
//! [authority]                      # globs relative to repo; first match wins in this order
//! normative = ["docs/spec/**"]
//! descriptive = ["README.md", "docs/guide/**"]
//! informative = ["docs/history/**"]
//!
//! [semantic]
//! enabled = false
//! model = "sonnet"
//! challenger_model = "opus"
//! max_concurrent = 2
//! max_analyses_per_scan = 40
//! claude_path = "claude"
//!
//! [server]
//! host = "127.0.0.1"
//! port = 4173
//!
//! test_results = ["ci/test-output.txt"]  # libtest output (text or JSON lines); --test-results overrides
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub repo: Option<PathBuf>,
    #[serde(default)]
    pub spec_roots: Vec<PathBuf>,
    #[serde(default)]
    pub src_roots: Vec<PathBuf>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub state_dir: Option<PathBuf>,
    /// Test-run outputs (libtest text or JSON lines) to ingest as evidence. Relative
    /// paths resolve against the config file's directory. They may live outside the repo.
    #[serde(default)]
    pub test_results: Vec<PathBuf>,
    #[serde(default)]
    pub authority: AuthorityRules,
    #[serde(default)]
    pub semantic: SemanticFile,
    #[serde(default)]
    pub server: ServerFile,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityRules {
    #[serde(default)]
    pub normative: Vec<String>,
    #[serde(default)]
    pub descriptive: Vec<String>,
    #[serde(default)]
    pub informative: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticFile {
    pub enabled: Option<bool>,
    pub model: Option<String>,
    pub challenger_model: Option<String>,
    pub max_concurrent: Option<usize>,
    pub max_analyses_per_scan: Option<usize>,
    pub claude_path: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerFile {
    pub host: Option<String>,
    pub port: Option<u16>,
}

/// Values given on the command line. `Some`/non-empty wins over the file.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub repo: Option<PathBuf>,
    pub spec_roots: Vec<PathBuf>,
    pub src_roots: Vec<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub test_results: Vec<PathBuf>,
    pub semantic: Option<bool>,
    pub model: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SemanticConfig {
    pub enabled: bool,
    pub model: String,
    pub challenger_model: String,
    pub max_concurrent: usize,
    pub max_analyses_per_scan: usize,
    pub claude_path: String,
}

/// Fully resolved configuration. All paths absolute.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Config {
    pub repo_root: PathBuf,
    pub spec_roots: Vec<PathBuf>,
    pub src_roots: Vec<PathBuf>,
    pub exclude: Vec<String>,
    pub state_dir: PathBuf,
    /// Absolute paths of test-run outputs to ingest (see [`crate::testrun`]).
    pub test_results: Vec<PathBuf>,
    pub authority: AuthorityRules,
    pub semantic: SemanticConfig,
    pub host: String,
    pub port: u16,
}

impl Config {
    /// Merge file + overrides. `base_dir` resolves a relative `repo` from the file
    /// (the config file's directory) or from the CLI (the current directory).
    /// `default_state_base` is the per-user data directory (e.g. `%LOCALAPPDATA%\concordance`).
    pub fn resolve(
        file: ConfigFile,
        file_dir: Option<&Path>,
        cli: Overrides,
        cwd: &Path,
        default_state_base: &Path,
    ) -> Result<Config, String> {
        let repo = match (cli.repo, file.repo) {
            (Some(r), _) => absolutize(cwd, &r),
            (None, Some(r)) => absolutize(file_dir.unwrap_or(cwd), &r),
            (None, None) => cwd.to_path_buf(),
        };
        let repo_root = std::fs::canonicalize(&repo)
            .map(strip_verbatim)
            .map_err(|e| format!("repository root {} is not accessible: {e}", repo.display()))?;
        if !repo_root.is_dir() {
            return Err(format!(
                "repository root {} is not a directory",
                repo_root.display()
            ));
        }

        let pick = |cli: Vec<PathBuf>, file: Vec<PathBuf>, default: &str| -> Vec<PathBuf> {
            let list = if !cli.is_empty() {
                cli
            } else if !file.is_empty() {
                file
            } else {
                vec![PathBuf::from(default)]
            };
            let mut out: Vec<PathBuf> = list.iter().map(|p| absolutize(&repo_root, p)).collect();
            out.dedup();
            out
        };
        let spec_roots = pick(cli.spec_roots, file.spec_roots, ".");
        let src_roots = pick(cli.src_roots, file.src_roots, ".");
        for r in spec_roots.iter().chain(&src_roots) {
            if !r.exists() {
                return Err(format!("root {} does not exist", r.display()));
            }
        }

        let state_dir = match cli.state_dir.or(file.state_dir) {
            Some(d) => absolutize(cwd, &d),
            None => {
                let name = repo_root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let key = crate::short_hash(&[&repo_root.to_string_lossy().to_lowercase()]);
                default_state_base.join(format!("{}-{}", sanitize(&name), &key[..8]))
            }
        };

        let test_results: Vec<PathBuf> = if !cli.test_results.is_empty() {
            cli.test_results
                .iter()
                .map(|p| absolutize(cwd, p))
                .collect()
        } else {
            file.test_results
                .iter()
                .map(|p| absolutize(file_dir.unwrap_or(cwd), p))
                .collect()
        };

        let s = file.semantic;
        let model = cli
            .model
            .or(s.model)
            .unwrap_or_else(|| "sonnet".to_string());
        let semantic = SemanticConfig {
            enabled: cli.semantic.or(s.enabled).unwrap_or(false),
            challenger_model: s.challenger_model.unwrap_or_else(|| model.clone()),
            model,
            max_concurrent: s.max_concurrent.unwrap_or(2).clamp(1, 8),
            max_analyses_per_scan: s.max_analyses_per_scan.unwrap_or(40),
            claude_path: s.claude_path.unwrap_or_else(|| "claude".to_string()),
        };

        Ok(Config {
            repo_root,
            spec_roots,
            src_roots,
            exclude: file.exclude,
            state_dir,
            test_results,
            authority: file.authority,
            semantic,
            host: cli
                .host
                .or(file.server.host)
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            port: cli.port.or(file.server.port).unwrap_or(4173),
        })
    }
}

fn absolutize(base: &Path, p: &Path) -> PathBuf {
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    };
    // Normalize `.`/`..` lexically (roots may not exist yet when reported as errors).
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `canonicalize` on Windows returns `\\?\C:\...`; strip it so paths stay printable and
/// prefix comparisons with user-supplied paths work.
pub fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if !rest.starts_with("UNC\\") {
            return PathBuf::from(rest);
        }
    }
    p
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
