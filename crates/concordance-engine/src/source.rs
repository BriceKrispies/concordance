//! Safe, hash-checked access to repository files. Only paths known to the model (or
//! constructed from validated `RepoPath`s) are ever read; nothing here accepts
//! arbitrary filesystem paths.

use crate::discover::FileKind;
use concordance_core::graph::NodeRef;
use concordance_core::virtual_doc::{Origin, VirtualDoc};
use concordance_core::{ContentHash, RepoPath};
use concordance_graph::Model;
use concordance_reconcile::FsProbe;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Largest ordinary source file read for scanning code.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Largest specification file read and indexed: a Markdown specification or a structured
/// (`.json`/`.jsonc`) source. A larger one is refused by `scan`, `lint` and `record` with an
/// error naming it; it is never skipped silently.
pub const MAX_SPEC_BYTES: u64 = 64 * 1024 * 1024;

/// Whether `path` is a specification file, which gets [`MAX_SPEC_BYTES`] rather than
/// [`MAX_FILE_BYTES`].
pub fn is_spec_path(path: &RepoPath) -> bool {
    matches!(path.extension(), Some("md" | "json" | "jsonc"))
}

/// The read limit for `path`.
pub fn read_limit(path: &RepoPath) -> u64 {
    if is_spec_path(path) {
        MAX_SPEC_BYTES
    } else {
        MAX_FILE_BYTES
    }
}

/// A specification file over [`MAX_SPEC_BYTES`]: it is not read, so it cannot be linted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OversizeSpec {
    pub path: RepoPath,
    pub bytes: u64,
    pub limit: u64,
}

impl std::fmt::Display for OversizeSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const MIB: f64 = 1024.0 * 1024.0;
        write!(
            f,
            "specification file {} is {} bytes ({:.1} MiB), over the {} MiB limit, so it cannot be read or linted; split it into smaller files",
            self.path,
            self.bytes,
            self.bytes as f64 / MIB,
            self.limit / (1024 * 1024)
        )
    }
}

/// The file at `abs` (repository path `path`, discovered as `kind`) if it is a specification over
/// [`MAX_SPEC_BYTES`]. A Markdown file always is one. A `.json`/`.jsonc` file is one only when its
/// content is a structured source, so an unrelated large data file is not refused. A file that
/// cannot be stat'ed or read is reported as a specification rather than skipped.
pub fn oversize_spec(path: &RepoPath, kind: &FileKind, abs: &Path) -> Option<OversizeSpec> {
    let bytes = std::fs::metadata(abs).ok()?.len();
    if bytes <= MAX_SPEC_BYTES {
        return None;
    }
    let is_spec = match kind {
        FileKind::Markdown => true,
        FileKind::Exegra => match std::fs::read(abs) {
            Ok(b) => concordance_exegra::is_exegra_spec(&String::from_utf8_lossy(&b)),
            Err(_) => true,
        },
        FileKind::Source(_) => false,
    };
    is_spec.then(|| OversizeSpec {
        path: path.clone(),
        bytes,
        limit: MAX_SPEC_BYTES,
    })
}

/// The specification files among `files` that are over [`MAX_SPEC_BYTES`] (see [`oversize_spec`]).
pub fn oversize_specs(files: &[(RepoPath, FileKind, PathBuf)]) -> Vec<OversizeSpec> {
    files
        .iter()
        .filter_map(|(path, kind, abs)| oversize_spec(path, kind, abs))
        .collect()
}

/// Read a repository file as text (lossy UTF-8). Refuses files over [`read_limit`] (the
/// specification limit for specification files, [`MAX_FILE_BYTES`] otherwise) and anything that
/// resolves outside the repository root.
pub fn read_text(root: &Path, path: &RepoPath) -> std::io::Result<String> {
    let abs = path.to_fs(root);
    let meta = std::fs::metadata(&abs)?;
    let limit = read_limit(path);
    if meta.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "{path} is {} bytes, over the {limit}-byte read limit",
                meta.len()
            ),
        ));
    }
    let canon = std::fs::canonicalize(&abs).map(concordance_core::config::strip_verbatim)?;
    if RepoPath::from_fs(root, &canon).is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "path escapes repository",
        ));
    }
    let bytes = std::fs::read(&abs)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Read a document as text: the rendered text of a virtual document, else the file.
pub fn read_doc_text(model: &Model, root: &Path, path: &RepoPath) -> std::io::Result<String> {
    match model.virtual_docs.get(path) {
        Some(v) => Ok(v.text.clone()),
        None => read_text(root, path),
    }
}

/// Existence and line probes over the repository, where a virtual document exists and has
/// lines like a file.
pub struct RepoFs {
    pub root: PathBuf,
    pub virtuals: BTreeMap<RepoPath, Arc<VirtualDoc>>,
    pub cited: Option<std::sync::Arc<crate::git::GitRepos>>,
}

impl FsProbe for RepoFs {
    fn exists(&self, path: &RepoPath) -> bool {
        self.virtuals.contains_key(path) || path.to_fs(&self.root).exists()
    }

    fn find_line(&self, path: &RepoPath, pred: &dyn Fn(&str) -> bool) -> Option<u32> {
        let text = match self.virtuals.get(path) {
            Some(v) => v.text.clone(),
            None => read_text(&self.root, path).ok()?,
        };
        text.lines().position(pred).map(|i| i as u32 + 1)
    }

    fn cited_repos(&self) -> Option<&dyn concordance_reconcile::citation::CitedRepos> {
        self.cited
            .as_deref()
            .map(|c| c as &dyn concordance_reconcile::citation::CitedRepos)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Highlight {
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceExcerpt {
    pub path: RepoPath,
    pub file_id: String,
    pub language: &'static str,
    pub start_line: u32,
    pub lines: Vec<String>,
    pub highlight: Option<Highlight>,
    /// For a virtual document: where the first highlighted line was rendered from in its source
    /// file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// False when the file changed on disk since it was indexed: lines may not match
    /// the recorded ranges.
    pub current: bool,
}

/// Indexed content hash and node id for a repository path.
fn indexed(model: &Model, path: &RepoPath) -> Option<(ContentHash, NodeRef, &'static str)> {
    if let Some(d) = model.document_by_path(path) {
        let language = match path.extension() {
            Some("json" | "jsonc") => "jsonc",
            _ => "markdown",
        };
        return Some((
            d.content_hash.clone(),
            NodeRef::Document(d.id.clone()),
            language,
        ));
    }
    model
        .file_by_path(path)
        .map(|f| (f.content_hash.clone(), NodeRef::File(f.id.clone()), "rust"))
}

/// Lines `[start-context, end+context]` of an indexed file, with the `[start, end]` range highlighted.
pub fn excerpt(
    model: &Model,
    root: &Path,
    path: &RepoPath,
    start: u32,
    end: u32,
    context: u32,
) -> Option<SourceExcerpt> {
    let (hash, node, language) = indexed(model, path)?;
    let text = read_doc_text(model, root, path).ok()?;
    let current = ContentHash::of_str(&text) == hash;
    let all: Vec<&str> = text.lines().collect();
    let total = all.len() as u32;
    if total == 0 {
        return None;
    }
    let start = start.clamp(1, total);
    let end = end.clamp(start, total);
    let from = start.saturating_sub(context).max(1);
    let to = (end + context).min(total).min(from + 400);
    Some(SourceExcerpt {
        path: path.clone(),
        file_id: node.to_string(),
        language,
        start_line: from,
        lines: all[(from - 1) as usize..to as usize]
            .iter()
            .map(|l| l.to_string())
            .collect(),
        highlight: Some(Highlight {
            start_line: start,
            end_line: end,
        }),
        origin: model.origin_of(path, start),
        current,
    })
}
