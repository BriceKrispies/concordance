//! Structured specification sources (Exegra JSONC files) and the virtual documents rendered
//! from them.
//!
//! A source is a `.jsonc` or `.json` file under a spec root, or named in `spec_roots`, for which
//! [`concordance_exegra::is_exegra_spec`] is true. Each of its documents is rendered to Markdown
//! (see `docs/spec-lint.md`, "Structured sources") and indexed under its `path` like a Markdown
//! file. The rendering is cheap and deterministic, so every scan renders every source again and
//! the model's own hash comparison decides which documents changed.

use crate::discover::{FileKind, PathFilter};
use crate::source::MAX_FILE_BYTES;
use concordance_core::config::Config;
use concordance_core::virtual_doc::{SourceDiagnostic, VirtualDoc};
use concordance_core::RepoPath;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

/// What rendering every structured source produced.
#[derive(Clone, Debug, Default)]
pub struct VirtualSet {
    /// Rendered documents by the path they have as files.
    pub docs: BTreeMap<RepoPath, Arc<VirtualDoc>>,
    /// Problems in the sources, located in them.
    pub diagnostics: Vec<SourceDiagnostic>,
    /// Requirement id as written (`KRN-002`) -> id of the entity it was rendered from.
    pub requirements: BTreeMap<String, String>,
    /// The structured source files that were read, sorted.
    pub sources: Vec<RepoPath>,
}

fn diagnostic(
    source: &RepoPath,
    entity: &str,
    line: u32,
    error: bool,
    message: String,
) -> SourceDiagnostic {
    SourceDiagnostic {
        source: source.clone(),
        entity: entity.to_string(),
        line,
        error,
        message,
    }
}

/// Render the structured sources among `files` (the discovered files; other kinds are ignored).
pub fn render_sources(files: &[(RepoPath, FileKind, PathBuf)]) -> VirtualSet {
    let mut set = VirtualSet::default();
    let mut candidates: Vec<&(RepoPath, FileKind, PathBuf)> =
        files.iter().filter(|f| f.1 == FileKind::Exegra).collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    for (rel, _, abs) in candidates {
        let Ok(meta) = std::fs::metadata(abs) else {
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(abs) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        if !concordance_exegra::is_exegra_spec(&text) {
            // A source that stopped being valid JSONC must not vanish without a word.
            if text.contains("\"exegra\"") {
                if let Err(e @ concordance_exegra::ExegraError::Syntax { .. }) =
                    concordance_exegra::expand(rel.as_str(), &text)
                {
                    set.sources.push(rel.clone());
                    set.diagnostics
                        .push(diagnostic(rel, "", 1, true, e.to_string()));
                }
            }
            continue;
        }
        set.sources.push(rel.clone());
        let expansion = match concordance_exegra::expand(rel.as_str(), &text) {
            Ok(x) => x,
            Err(e) => {
                set.diagnostics
                    .push(diagnostic(rel, "", 1, true, e.to_string()));
                continue;
            }
        };
        for d in &expansion.diagnostics {
            set.diagnostics.push(diagnostic(
                rel,
                &d.entity,
                d.line as u32,
                d.severity() == concordance_exegra::Severity::Error,
                format!("{}: {}", d.kind.as_str(), d.message),
            ));
        }
        for (r, e) in &expansion.requirements {
            set.requirements
                .entry(r.clone())
                .or_insert_with(|| e.clone());
        }
        for d in expansion.documents {
            let Some(path) = RepoPath::from_relative_str(&d.path) else {
                set.diagnostics.push(diagnostic(
                    rel,
                    &d.document_id,
                    d.source_line as u32,
                    true,
                    format!(
                        "document path `{}` is not a repository-relative path; the document is not indexed",
                        d.path
                    ),
                ));
                continue;
            };
            if let Some(first) = set.docs.get(&path) {
                set.diagnostics.push(diagnostic(
                    rel,
                    &d.document_id,
                    d.source_line as u32,
                    true,
                    format!(
                        "document path `{path}` is also rendered from {}; this one is not indexed",
                        first.source
                    ),
                ));
                continue;
            }
            let doc = VirtualDoc::new(
                rel.clone(),
                d.text,
                d.origin.entries().map(|(e, l)| (e.to_string(), l as u32)),
            );
            set.docs.insert(path, Arc::new(doc));
        }
    }
    set
}

/// A real Markdown file and a virtual document with the same path: neither is indexed, and each
/// conflict is reported as an error. Returns the conflicting paths.
pub fn resolve_conflicts(set: &mut VirtualSet, markdown: &BTreeSet<RepoPath>) -> Vec<RepoPath> {
    let conflicts: Vec<RepoPath> = set
        .docs
        .keys()
        .filter(|p| markdown.contains(*p))
        .cloned()
        .collect();
    for p in &conflicts {
        if let Some(doc) = set.docs.remove(p) {
            let (entity, line) = doc
                .origin_of(1)
                .map(|o| (o.entity, o.line))
                .unwrap_or_default();
            set.diagnostics.push(diagnostic(
                &doc.source,
                &entity,
                line,
                true,
                format!(
                    "`{p}` is both a Markdown file and a document of this source; neither is indexed (remove one of them)"
                ),
            ));
        }
    }
    conflicts
}

/// Render the structured sources of the configured spec roots, without scanning. Used by
/// commands that read persisted state.
pub fn load_virtual(config: &Config) -> VirtualSet {
    let files = PathFilter::new(config).discover();
    let mut set = render_sources(&files);
    let markdown: BTreeSet<RepoPath> = files
        .iter()
        .filter(|f| f.1 == FileKind::Markdown)
        .map(|f| f.0.clone())
        .collect();
    resolve_conflicts(&mut set, &markdown);
    set
}
