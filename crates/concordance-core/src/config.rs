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
//! context = "explore"              # "explore": the model searches the repository read-only;
//!                                  # "excerpts": it sees only linked symbol excerpts
//! max_turns = 20                   # explore: tool-use turns per analysis
//!
//! [lint]                          # `concordance lint` (see docs/spec-lint.md)
//! whitepaper = "docs/whitepaper.md"
//! require_milestone = false        # every requirement needs a valid Milestone:/Seam:
//! strict = false                   # warnings fail the lint
//! spec = ["docs/spec/**"]           # lint only these normative files (default: all)
//! vague_words = ["fast"]           # replaces the built-in banned-word list
//! internal_markers = ["shard"]     # added to the built-in internal-detail words
//!
//! [gate]                          # milestone gating (see docs/requirements.md)
//! started_milestones = [1, 2]      # requirements of other milestones scan as `planned`
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
    #[serde(default)]
    pub lint: LintFile,
    #[serde(default)]
    pub gate: GateFile,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LintFile {
    /// White paper path (default `docs/whitepaper.md`), relative to the config file's repository.
    pub whitepaper: Option<PathBuf>,
    pub strict: Option<bool>,
    /// Every requirement needs a valid `Milestone:` (and `Seam:` no greater than it).
    pub require_milestone: Option<bool>,
    pub vague_words: Option<Vec<String>>,
    #[serde(default)]
    pub internal_markers: Vec<String>,
    /// Globs (repository-relative) restricting which normative files are linted.
    #[serde(default)]
    pub spec: Vec<String>,
    /// Semantic rules over structured sources, `[lint.rules.<name>]` (see `RULE_NAMES`).
    #[serde(default)]
    pub rules: std::collections::BTreeMap<String, RuleFile>,
}

/// The semantic lint rules that can be configured under `[lint.rules.<name>]`.
pub const RULE_NAMES: [&str; 9] = [
    "milestone_order",
    "budget_row",
    "sub_budget",
    "constants",
    "forbidden_terms",
    "duplicate_terms",
    "dead_references",
    "said_once",
    "replacement_char",
];

/// One `[lint.rules.<name>]` table. Every key is optional; a rule reads the keys it knows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleFile {
    pub enabled: Option<bool>,
    /// `error`, `warning` or `info`.
    pub severity: Option<String>,
    /// Similarity threshold in percent (`said_once`).
    pub threshold_percent: Option<u32>,
    /// Fewest tokens a statement needs to be compared (`said_once`).
    pub min_tokens: Option<u32>,
    /// Constant names (`constants`).
    #[serde(default)]
    pub names: Vec<String>,
    /// Path substrings that select the table documents the rule reads (`budget_row`, `constants`).
    #[serde(default)]
    pub tables: Vec<String>,
    /// Words that mark a decomposition (`sub_budget`) or a history note (`dead_references`).
    #[serde(default)]
    pub markers: Vec<String>,
    /// Also read rationales (`dead_references`).
    pub rationales: Option<bool>,
    /// Also check statements that name a constant (`constants`; default true).
    pub statements: Option<bool>,
    /// Forbidden phrases with their replacement (`forbidden_terms`).
    #[serde(default)]
    pub forbidden: Vec<ForbiddenTerm>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForbiddenTerm {
    pub phrase: String,
    pub replacement: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateFile {
    /// Milestones that are started. Unset: every requirement counts as started.
    pub started_milestones: Option<Vec<u32>>,
}

/// Resolved `[gate]` settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GateConfig {
    /// `None`: no gating. `Some`: a requirement whose `Milestone:` is not listed is `planned`;
    /// a requirement without a `Milestone:` is started.
    pub started_milestones: Option<Vec<u32>>,
}

impl GateConfig {
    /// Whether a requirement with this milestone is started.
    pub fn is_started(&self, milestone: Option<u32>) -> bool {
        match (&self.started_milestones, milestone) {
            (Some(list), Some(m)) => list.contains(&m),
            _ => true,
        }
    }
}

/// Resolved `[lint]` settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LintConfig {
    /// Absolute path of the white paper (may not exist).
    pub whitepaper: PathBuf,
    pub strict: bool,
    pub require_milestone: bool,
    /// Replacement for the built-in banned words, if configured.
    pub vague_words: Option<Vec<String>>,
    /// Extra words that mark an internal detail, added to the built-in ones.
    pub internal_markers: Vec<String>,
    /// Globs restricting which normative files are linted; empty means all of them.
    pub spec: Vec<String>,
    /// `[lint.rules.<name>]` tables, as written.
    pub rules: std::collections::BTreeMap<String, RuleFile>,
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
    pub context: Option<String>,
    pub max_turns: Option<u32>,
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
    /// The model searches the repository with read-only tools (Read, Grep, Glob) instead
    /// of seeing only the excerpts of linked symbols.
    pub explore: bool,
    /// Explore mode: maximum tool-use turns per analysis.
    pub max_turns: u32,
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
    pub lint: LintConfig,
    pub gate: GateConfig,
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
            explore: match s.context.as_deref() {
                None | Some("explore") => true,
                Some("excerpts") => false,
                Some(other) => {
                    return Err(format!(
                        "semantic.context must be \"explore\" or \"excerpts\", not \"{other}\""
                    ))
                }
            },
            max_turns: s.max_turns.unwrap_or(20).clamp(1, 60),
        };

        let lint = LintConfig {
            whitepaper: absolutize(
                &repo_root,
                file.lint
                    .whitepaper
                    .as_deref()
                    .unwrap_or(Path::new("docs/whitepaper.md")),
            ),
            strict: file.lint.strict.unwrap_or(false),
            require_milestone: file.lint.require_milestone.unwrap_or(false),
            vague_words: file.lint.vague_words,
            internal_markers: file.lint.internal_markers,
            spec: file.lint.spec,
            rules: file.lint.rules,
        };
        if let Some(bad) = lint
            .rules
            .keys()
            .find(|k| !RULE_NAMES.contains(&k.as_str()))
        {
            return Err(format!(
                "[lint.rules.{bad}] is not a rule; the rules are {}",
                RULE_NAMES.join(", ")
            ));
        }

        Ok(Config {
            repo_root,
            spec_roots,
            src_roots,
            exclude: file.exclude,
            state_dir,
            test_results,
            authority: file.authority,
            semantic,
            lint,
            gate: GateConfig {
                started_milestones: file.gate.started_milestones,
            },
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
