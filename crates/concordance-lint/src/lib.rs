//! Strict lint for the requirement format of the spec-driven workflow.
//!
//! The rules (see `docs/spec-lint.md`):
//!
//! | Rule | Checks |
//! |---|---|
//! | R1 | atomic: exactly one modal verb, no semicolon, no "and" joining two modal clauses |
//! | R2 | no vague words in the statement |
//! | R3 | no internals in the statement; stay at the boundary surface |
//! | R4 | provable: a valid `Verify:` method that fits the statement |
//! | R5 | traceable: `Traces:` and `Boundary:` resolve in the white paper; every goal is traced |
//! | R6 | said once: near-duplicate statements |
//! | R7 | normative words only inside requirements |
//! | R8 | IDs: well formed, unique, owned by the file that holds them |
//!
//! [`lint`] is a pure function of the file texts and, for R3, the indexed code. [`lint_repo`]
//! gathers the inputs from an engine model and the repository.

mod rules;
mod text;
mod whitepaper;

pub use whitepaper::{parse_index_table, parse_whitepaper, IndexRow, WhitePaperId};

use concordance_core::config::{Config, LintConfig};
use concordance_core::doc::Authority;
use concordance_graph::Model;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// One violation, at an exact location.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LintFinding {
    /// `R1` .. `R8`, or `R0` when there is nothing to lint.
    pub rule: &'static str,
    pub severity: Severity,
    pub message: String,
    /// Repository-relative, forward slashes.
    pub path: String,
    /// 1-based.
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LintReport {
    pub tool: &'static str,
    pub command: &'static str,
    pub strict: bool,
    pub errors: usize,
    pub warnings: usize,
    pub findings: Vec<LintFinding>,
}

impl LintReport {
    fn new(mut findings: Vec<LintFinding>, strict: bool) -> Self {
        if strict {
            for f in &mut findings {
                f.severity = Severity::Error;
            }
        }
        findings.sort_by(|a, b| {
            (&a.path, a.line, a.rule, &a.message).cmp(&(&b.path, b.line, b.rule, &b.message))
        });
        findings.dedup();
        let errors = findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count();
        LintReport {
            tool: "concordance",
            command: "lint",
            strict,
            errors,
            warnings: findings.len() - errors,
            findings,
        }
    }

    /// Whether the lint failed: any error (with `strict`, any finding).
    pub fn failed(&self) -> bool {
        self.errors > 0
    }

    /// Human-readable lines: `path:line: error[R1] message`.
    pub fn human(&self) -> String {
        let mut out = String::new();
        for f in &self.findings {
            out.push_str(&format!(
                "{}:{}: {}[{}] {}\n",
                f.path,
                f.line,
                f.severity.as_str(),
                f.rule,
                f.message
            ));
        }
        out.push_str(&format!(
            "{} error(s), {} warning(s){}\n",
            self.errors,
            self.warnings,
            if self.strict { " (strict)" } else { "" }
        ));
        out
    }
}

/// A Markdown file to lint.
#[derive(Clone, Debug)]
pub struct SpecFile {
    /// Repository-relative path.
    pub path: String,
    pub text: String,
}

/// Everything the rules look at.
pub struct LintInput<'a> {
    /// The normative specification files, in a stable order.
    pub specs: &'a [SpecFile],
    /// The spec index (`docs/spec/index.md`), if present; its table assigns ID prefixes to files.
    pub index: Option<&'a SpecFile>,
    /// The white paper's repository-relative path and text (`None`: the file is missing).
    pub whitepaper: (&'a str, Option<&'a str>),
    /// The indexed code, for R3.
    pub model: Option<&'a Model>,
    pub settings: &'a LintConfig,
}

/// Run every rule. `strict` turns warnings into errors.
pub fn lint(input: &LintInput<'_>, strict: bool) -> LintReport {
    LintReport::new(rules::run(input), strict)
}

/// Lint the repository an engine model describes: the normative documents under the spec
/// roots, the configured white paper, and the indexed code.
pub fn lint_repo(model: &Model, config: &Config, strict: bool) -> LintReport {
    let root = &config.repo_root;
    let mut docs: Vec<_> = model
        .documents
        .values()
        .filter(|d| d.authority.level == Authority::Normative)
        .collect();
    docs.sort_by(|a, b| a.path.cmp(&b.path));
    let mut specs = Vec::new();
    for d in &docs {
        if let Ok(text) = std::fs::read_to_string(d.path.to_fs(root)) {
            specs.push(SpecFile {
                path: d.path.as_str().to_string(),
                text,
            });
        }
    }
    // The shallowest `index.md` among the specification files.
    let index_at = specs
        .iter()
        .enumerate()
        .filter(|(_, s)| s.path.rsplit('/').next() == Some("index.md"))
        .min_by_key(|(_, s)| s.path.matches('/').count())
        .map(|(i, _)| i);
    let index = index_at.map(|i| specs[i].clone());

    let wp_path = concordance_core::RepoPath::from_fs(root, &config.lint.whitepaper)
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| config.lint.whitepaper.display().to_string());
    let wp_text = std::fs::read_to_string(&config.lint.whitepaper).ok();
    let input = LintInput {
        specs: &specs,
        index: index.as_ref(),
        whitepaper: (&wp_path, wp_text.as_deref()),
        model: Some(model),
        settings: &config.lint,
    };
    lint(&input, strict || config.lint.strict)
}
