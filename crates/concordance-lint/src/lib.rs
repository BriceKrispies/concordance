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

mod cases;
mod rules;
mod text;
mod whitepaper;

pub use cases::{parse_cases, CaseRow, CaseTable};
pub use whitepaper::{parse_index_table, parse_whitepaper, IndexRow, WhitePaperId};

use concordance_core::config::{Config, LintConfig};
use concordance_core::doc::Authority;
use concordance_core::virtual_doc::{Origin, VirtualDoc};
use concordance_core::RepoPath;
use concordance_graph::Model;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;

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
    /// For a virtual document (rendered from a structured source): where `line` was rendered
    /// from in the source file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
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

    /// Human-readable lines: `path:line: error[R1] message`. In a virtual document the location
    /// is followed by its origin: `path:line (source.jsonc:12 entity): error[R1] message`.
    pub fn human(&self) -> String {
        let mut out = String::new();
        for f in &self.findings {
            out.push_str(&format!(
                "{}: {}[{}] {}\n",
                Origin::annotate(&format!("{}:{}", f.path, f.line), f.origin.as_ref()),
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
    /// The repository root, to resolve `Cases:` files (`None`: they are not checked).
    pub repo_root: Option<&'a std::path::Path>,
    /// Documents rendered from structured sources. A `Cases:` file that is one of them is read
    /// from here, not from the file system.
    pub virtual_docs: Option<&'a BTreeMap<RepoPath, Arc<VirtualDoc>>>,
}

impl LintInput<'_> {
    /// The text of the repository-relative file `rel`: a virtual document, else the file.
    pub(crate) fn read_file(&self, rel: &str) -> Option<String> {
        if let (Some(docs), Some(path)) = (self.virtual_docs, RepoPath::from_relative_str(rel)) {
            if let Some(v) = docs.get(&path) {
                return Some(v.text.clone());
            }
        }
        std::fs::read_to_string(self.repo_root?.join(rel)).ok()
    }
}

/// Run every rule. `strict` turns warnings into errors.
pub fn lint(input: &LintInput<'_>, strict: bool) -> LintReport {
    LintReport::new(rules::run(input), strict)
}

/// Lint the repository an engine model describes: the normative documents under the spec
/// roots, the configured white paper, and the indexed code.
pub fn lint_repo(model: &Model, config: &Config, strict: bool) -> LintReport {
    let root = &config.repo_root;
    let globs = if config.lint.spec.is_empty() {
        None
    } else {
        let mut b = globset::GlobSetBuilder::new();
        for g in &config.lint.spec {
            if let Ok(g) = globset::Glob::new(g) {
                b.add(g);
            }
        }
        b.build().ok()
    };
    let mut docs: Vec<_> = model
        .documents
        .values()
        .filter(|d| d.authority.level == Authority::Normative)
        .filter(|d| globs.as_ref().is_none_or(|g| g.is_match(d.path.as_str())))
        .collect();
    docs.sort_by(|a, b| a.path.cmp(&b.path));
    let mut specs = Vec::new();
    for d in &docs {
        let text = match model.virtual_docs.get(&d.path) {
            Some(v) => Some(v.text.clone()),
            None => std::fs::read_to_string(d.path.to_fs(root)).ok(),
        };
        if let Some(text) = text {
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
    let wp_text =
        match RepoPath::from_relative_str(&wp_path).and_then(|p| model.virtual_docs.get(&p)) {
            Some(v) => Some(v.text.clone()),
            None => std::fs::read_to_string(&config.lint.whitepaper).ok(),
        };
    let input = LintInput {
        specs: &specs,
        index: index.as_ref(),
        whitepaper: (&wp_path, wp_text.as_deref()),
        model: Some(model),
        settings: &config.lint,
        repo_root: Some(root),
        virtual_docs: Some(&model.virtual_docs),
    };
    let mut findings = rules::run(&input);
    // Locations in a virtual document also say where they were rendered from.
    for f in &mut findings {
        f.origin = RepoPath::from_relative_str(&f.path).and_then(|p| model.origin_of(&p, f.line));
    }
    // Problems in the structured sources themselves (R0): they are in the source file, not in
    // any rendered document.
    for d in &model.virtual_diagnostics {
        findings.push(LintFinding {
            rule: "R0",
            severity: if d.error {
                Severity::Error
            } else {
                Severity::Warning
            },
            message: format!("{} ({})", d.message, d.entity),
            path: d.source.as_str().to_string(),
            line: d.line,
            origin: None,
        });
    }
    LintReport::new(findings, strict || config.lint.strict)
}
