//! Exegra specification front end: one JSONC file -> the Markdown documents Concordance reads.
//!
//! An Exegra specification keeps a whole project's specification as structured data in one
//! JSONC file. `documents` lists the specification's files by path; each `sections` entry is a
//! heading of one document with its prose in `body`, where every separately modelled entity
//! (a requirement, a goal, a boundary, ...) is represented by one marker line
//! `{{entity:<id>}}`. [`expand`] renders one Markdown text per document **from the entities'
//! properties**, in the formats Concordance's Markdown front end already reads (see
//! `docs/spec-lint.md` and `docs/requirements.md`), so the existing analysis runs on them
//! unchanged. Each text comes with an [`Origin`] that maps every line back to the entity and the
//! JSONC line it was rendered from.
//!
//! The `raw` property some entities carry (the Markdown they were first imported from) is never
//! read: it goes stale as soon as a property is edited.
//!
//! CONTRACT:
//! * [`expand`] is pure and deterministic: same input, same output.
//! * It never panics on any input text.
//! * Problems are [`Diagnostic`]s, never swallowed. A marker that cannot be rendered stays in the
//!   text, so nothing silently disappears.
//! * Lines are separated by `\n`; a non-empty text ends with `\n`.

mod jsonc;
mod render;

use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

/// Bump when the rendered text or the origin mapping changes for identical input (forces
/// re-indexing).
pub const EXTRACTOR_VERSION: &str = "exegra-2";

/// Why a whole file could not be expanded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExegraError {
    /// The text is not valid JSONC.
    Syntax { file: String, message: String },
    /// Valid JSON, but not an Exegra specification (no top-level `"exegra"` key).
    NotExegra { file: String, message: String },
}

impl fmt::Display for ExegraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExegraError::Syntax { file, message } => write!(f, "{file}: invalid JSONC: {message}"),
            ExegraError::NotExegra { file, message } => {
                write!(f, "{file}: not an Exegra specification: {message}")
            }
        }
    }
}

impl std::error::Error for ExegraError {}

/// How serious a [`Diagnostic`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Something in the specification is missing or contradictory; text may be incomplete.
    Error,
    /// The text is complete, but a value does not resolve as it should.
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

/// What kind of problem a [`Diagnostic`] reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticKind {
    /// A marker names an entity that does not exist. The marker line stays in the text.
    UnknownEntity,
    /// A section names a document that does not exist. The section is not rendered.
    UnknownDocument,
    /// Two documents have the same `path`; the later one is dropped.
    DuplicatePath,
    /// A document with role `cases` has no case table.
    MissingCaseTable,
    /// Two entities have the same `id`; the first one is used.
    DuplicateEntity,
    /// An entity that cannot be rendered: no `id`, no `path`, a requirement without `ref` or
    /// `statement`, an entity named by a marker without text. The marker line stays.
    InvalidEntity,
    /// A line contains `{{entity:` but is not exactly one marker.
    MalformedMarker,
    /// A `boundary`, `traces`, `dependsOn` or `cases` value names an entity that does not exist
    /// (or has no `ref`/`path`). Its id is written instead.
    DanglingReference,
    /// A requirement's `casesPath` names a path that matched no case table. It is written into
    /// `Cases:` as it is.
    UnmatchedCasesPath,
    /// A requirement that no rendered section places with a marker. It is never rendered, so no
    /// lint and no verification sees it, and it is left out.
    UnplacedEntity,
    /// A claim whose `kind` is a misspelling of `requirement` (`Requirement`, `requirment`). It is
    /// rendered as prose, so no requirement rule checks it.
    MisspeltKind,
}

impl DiagnosticKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DiagnosticKind::UnknownEntity => "unknown-entity",
            DiagnosticKind::UnknownDocument => "unknown-document",
            DiagnosticKind::DuplicatePath => "duplicate-path",
            DiagnosticKind::MissingCaseTable => "missing-case-table",
            DiagnosticKind::DuplicateEntity => "duplicate-entity",
            DiagnosticKind::InvalidEntity => "invalid-entity",
            DiagnosticKind::MalformedMarker => "malformed-marker",
            DiagnosticKind::DanglingReference => "dangling-reference",
            DiagnosticKind::UnmatchedCasesPath => "unmatched-cases-path",
            DiagnosticKind::UnplacedEntity => "unplaced-entity",
            DiagnosticKind::MisspeltKind => "misspelt-kind",
        }
    }

    pub fn severity(self) -> Severity {
        match self {
            DiagnosticKind::UnmatchedCasesPath
            | DiagnosticKind::DanglingReference
            | DiagnosticKind::MalformedMarker => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

/// A problem found while expanding, located at the entity that has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    /// Id of the entity the problem is in.
    pub entity: String,
    /// 1-based line of that entity in the JSONC file.
    pub line: usize,
    pub message: String,
}

impl Diagnostic {
    pub fn severity(&self) -> Severity {
        self.kind.severity()
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {} ({}): {}: {}",
            self.line,
            self.entity,
            self.kind.as_str(),
            self.message
        )
    }
}

/// Maps every line of a rendered document back to the entity and the JSONC line it came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Origin {
    entities: Vec<String>,
    /// Per rendered line: (index into `entities`, JSONC line of the entity).
    lines: Vec<(u32, u32)>,
}

impl Origin {
    /// The entity id and the JSONC line that contributed `line` (1-based) of the text.
    pub fn origin_of(&self, line: usize) -> Option<(&str, usize)> {
        let (e, src) = *self.lines.get(line.checked_sub(1)?)?;
        Some((self.entities[e as usize].as_str(), src as usize))
    }

    /// Number of mapped lines (the line count of the text).
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// `(entity id, JSONC line)` for each line, in order.
    pub fn entries(&self) -> impl Iterator<Item = (&str, usize)> + '_ {
        self.lines
            .iter()
            .map(|&(e, l)| (self.entities[e as usize].as_str(), l as usize))
    }

    fn push(&mut self, entity: &str, source_line: usize) {
        let idx = match self.entities.last() {
            Some(last) if last == entity => self.entities.len() - 1,
            _ => match self.entities.iter().rposition(|e| e == entity) {
                Some(i) => i,
                None => {
                    self.entities.push(entity.to_string());
                    self.entities.len() - 1
                }
            },
        };
        self.lines.push((idx as u32, source_line as u32));
    }
}

/// One Markdown file rendered from the JSONC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtualDocument {
    /// The document's `path`: the file's repository-relative name.
    pub path: String,
    /// The rendered Markdown.
    pub text: String,
    pub origin: Origin,
    /// Id of the `documents` entry.
    pub document_id: String,
    /// 1-based line of the `documents` entry in the JSONC file.
    pub source_line: usize,
}

impl VirtualDocument {
    /// The entity id and the JSONC line that contributed `line` (1-based) of [`Self::text`].
    pub fn origin_of(&self, line: usize) -> Option<(&str, usize)> {
        self.origin.origin_of(line)
    }
}

/// Everything [`expand`] produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expansion {
    /// In the order of the `documents` list.
    pub documents: Vec<VirtualDocument>,
    pub diagnostics: Vec<Diagnostic>,
    /// Requirement id as written (`ref`, e.g. `KRN-002`) -> id of the claim entity it was
    /// rendered from. The first claim wins when two share a `ref`.
    pub requirements: BTreeMap<String, String>,
}

/// Cheap detection: the text is a JSONC object with a top-level `"exegra"` key. Does not
/// validate the rest.
pub fn is_exegra_spec(text: &str) -> bool {
    if !text.contains("\"exegra\"") {
        return false;
    }
    let Ok(clean) = jsonc::clean(text) else {
        return false;
    };
    jsonc::scan_top_level(&clean).is_some_and(|t| t.keys.iter().any(|k| k == "exegra"))
}

/// One element of a top-level array.
#[derive(Clone, Copy)]
pub(crate) struct Entity<'a> {
    pub value: &'a Value,
    /// 1-based JSONC line.
    pub line: usize,
    /// The top-level key of the array (`claims`, `goals`, ...).
    pub list: &'a str,
}

/// Whether `kind` is a misspelling of `requirement`: not the exact word, but the same letters in
/// another case, or at most two edits away (`Requirement`, `requirment`, `requirments`).
pub(crate) fn is_misspelling_of_requirement(kind: &str) -> bool {
    const WORD: &str = "requirement";
    kind != WORD && edit_distance(&kind.to_ascii_lowercase(), WORD) <= 2
}

/// Levenshtein distance between `a` and `b`.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != *cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

pub(crate) fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Top-level arrays whose entries are entities a marker can name.
const ENTITY_LISTS: [&str; 15] = [
    "documents",
    "sections",
    "caseTables",
    "claims",
    "goals",
    "nonGoals",
    "boundaries",
    "decisions",
    "constraints",
    "openQuestions",
    "components",
    "releaseStages",
    "milestones",
    "terms",
    "contracts",
];

const MARKER_PREFIX: &str = "{{entity:";

/// `{{entity:<id>}}` -> `<id>`. The whole (trimmed) line must be the marker.
fn marker_id(line: &str) -> Option<&str> {
    let id = line
        .trim()
        .strip_prefix(MARKER_PREFIX)?
        .strip_suffix("}}")?
        .trim();
    (!id.is_empty() && !id.contains('}') && !id.contains('{')).then_some(id)
}

/// Render every document of the Exegra specification in `text`.
///
/// `rel_path` is the JSONC file's own path, used only in error messages.
pub fn expand(rel_path: &str, text: &str) -> Result<Expansion, ExegraError> {
    let clean = jsonc::clean(text).map_err(|message| ExegraError::Syntax {
        file: rel_path.to_string(),
        message,
    })?;
    let root: Value = serde_json::from_str(&clean).map_err(|e| ExegraError::Syntax {
        file: rel_path.to_string(),
        message: e.to_string(),
    })?;
    let not_exegra = |message: &str| ExegraError::NotExegra {
        file: rel_path.to_string(),
        message: message.to_string(),
    };
    let obj = root
        .as_object()
        .ok_or_else(|| not_exegra("the top level is not an object"))?;
    if !obj.contains_key("exegra") {
        return Err(not_exegra("no top-level \"exegra\" key"));
    }
    let top = jsonc::scan_top_level(&clean).ok_or_else(|| not_exegra("unreadable top level"))?;
    let table = jsonc::LineTable::new(&clean);

    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Every element of every entity list, with its line.
    let mut lists: BTreeMap<&str, Vec<Entity<'_>>> = BTreeMap::new();
    for key in ENTITY_LISTS {
        let Some(items) = obj.get(key).and_then(Value::as_array) else {
            continue;
        };
        let starts = top.elements.get(key);
        let list = items
            .iter()
            .enumerate()
            .map(|(i, v)| Entity {
                value: v,
                line: starts
                    .and_then(|s| s.get(i))
                    .map_or(1, |&off| table.line(off)),
                list: key,
            })
            .collect();
        lists.insert(key, list);
    }

    // Entities by id, for marker and reference lookup. The first one of an id wins.
    let mut by_id: HashMap<&str, Entity<'_>> = HashMap::new();
    for (key, list) in &lists {
        for e in list {
            let Some(id) = str_of(e.value, "id").filter(|i| !i.is_empty()) else {
                diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::InvalidEntity,
                    entity: format!("{key}[]"),
                    line: e.line,
                    message: format!("an entry of `{key}` has no string `id`"),
                });
                continue;
            };
            if let Some(first) = by_id.get(id) {
                diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::DuplicateEntity,
                    entity: id.to_string(),
                    line: e.line,
                    message: format!(
                        "id `{id}` is already used by the entity at line {}; that one is used",
                        first.line
                    ),
                });
                continue;
            }
            by_id.insert(id, *e);
        }
    }

    let empty: Vec<Entity<'_>> = Vec::new();
    let documents = lists.get("documents").unwrap_or(&empty);
    let sections = lists.get("sections").unwrap_or(&empty);
    let case_tables = lists.get("caseTables").unwrap_or(&empty);

    // Sections by document id, in `order` (stable for equal or missing orders).
    let known_docs: HashMap<&str, Entity<'_>> = documents
        .iter()
        .filter_map(|d| str_of(d.value, "id").map(|id| (id, *d)))
        .collect();
    let mut secs_by_doc: HashMap<&str, Vec<Entity<'_>>> = HashMap::new();
    for s in sections {
        let Some(sid) = str_of(s.value, "id") else {
            continue;
        };
        match str_of(s.value, "document").filter(|d| known_docs.contains_key(d)) {
            Some(d) => secs_by_doc.entry(d).or_default().push(*s),
            None => diagnostics.push(Diagnostic {
                kind: DiagnosticKind::UnknownDocument,
                entity: sid.to_string(),
                line: s.line,
                message: format!(
                    "section `{sid}` names document `{}`, which does not exist",
                    str_of(s.value, "document").unwrap_or("")
                ),
            }),
        }
    }
    for list in secs_by_doc.values_mut() {
        list.sort_by(|a, b| {
            let key = |e: &Entity<'_>| e.value.get("order").and_then(Value::as_f64).unwrap_or(0.0);
            key(a)
                .partial_cmp(&key(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    let mut case_by_doc: HashMap<&str, Entity<'_>> = HashMap::new();
    for c in case_tables {
        if let Some(d) = str_of(c.value, "document") {
            case_by_doc.entry(d).or_insert(*c);
        }
    }

    // Requirement entity id -> claims whose `models` names it (a string or a list of ids).
    let mut modelled_by: HashMap<String, Vec<String>> = HashMap::new();
    for e in lists.get("claims").unwrap_or(&empty) {
        let Some(cid) = str_of(e.value, "id").filter(|i| !i.is_empty()) else {
            continue;
        };
        let targets: Vec<&str> = match e.value.get("models") {
            Some(Value::String(s)) => vec![s.as_str()],
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        for t in targets {
            let v = modelled_by.entry(t.to_string()).or_default();
            if !v.iter().any(|x| x == cid) {
                v.push(cid.to_string());
            }
        }
    }
    let ctx = render::Ctx {
        by_id: &by_id,
        modelled_by: &modelled_by,
    };
    let mut out = Expansion::default();
    let mut seen_paths: HashMap<String, String> = HashMap::new();
    // Ids that a marker in a rendered section names.
    let mut placed: HashSet<String> = HashSet::new();
    for d in documents {
        let Some(id) = str_of(d.value, "id") else {
            continue;
        };
        let Some(path) = str_of(d.value, "path").filter(|p| !p.is_empty()) else {
            diagnostics.push(Diagnostic {
                kind: DiagnosticKind::InvalidEntity,
                entity: id.to_string(),
                line: d.line,
                message: format!("document `{id}` has no `path`"),
            });
            continue;
        };
        if let Some(first) = seen_paths.get(path) {
            diagnostics.push(Diagnostic {
                kind: DiagnosticKind::DuplicatePath,
                entity: id.to_string(),
                line: d.line,
                message: format!(
                    "document `{id}` has the path `{path}`, which document `{first}` already has; `{id}` is dropped"
                ),
            });
            continue;
        }
        seen_paths.insert(path.to_string(), id.to_string());

        let mut b = Builder::default();
        let sections_of_doc = secs_by_doc.get(id).map(Vec::as_slice).unwrap_or(&[]);
        if let Some(ct) = case_by_doc.get(id) {
            let cid = str_of(ct.value, "id").unwrap_or(id);
            let body = str_of(ct.value, "body").unwrap_or("").replace("\r\n", "\n");
            if !body.is_empty() {
                for line in body.strip_suffix('\n').unwrap_or(&body).split('\n') {
                    b.line(line, cid, ct.line);
                }
            }
            if !sections_of_doc.is_empty() {
                diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::InvalidEntity,
                    entity: id.to_string(),
                    line: d.line,
                    message: format!(
                        "document `{id}` has a case table and sections; only the case table is rendered"
                    ),
                });
            }
        } else {
            if str_of(d.value, "role") == Some("cases") {
                diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::MissingCaseTable,
                    entity: id.to_string(),
                    line: d.line,
                    message: format!("document `{id}` has role `cases` but no case table"),
                });
            }
            for s in sections_of_doc {
                if let Some(body) = str_of(s.value, "body") {
                    placed.extend(body.lines().filter_map(marker_id).map(str::to_string));
                }
                b.section(s, &ctx, &mut diagnostics, &mut out.requirements);
            }
        }
        out.documents.push(VirtualDocument {
            path: path.to_string(),
            text: b.out,
            origin: b.origin,
            document_id: id.to_string(),
            source_line: d.line,
        });
    }
    // A misspelt requirement kind is prose to every rule: say so.
    for e in lists.get("claims").unwrap_or(&empty) {
        let Some(kind) = str_of(e.value, "kind") else {
            continue;
        };
        if is_misspelling_of_requirement(kind) {
            diagnostics.push(Diagnostic {
                kind: DiagnosticKind::MisspeltKind,
                entity: str_of(e.value, "id").unwrap_or("").to_string(),
                line: e.line,
                message: format!(
                    "claim kind `{kind}` is not `requirement`, so the entity is rendered as prose and no requirement rule checks it; write `requirement`"
                ),
            });
        }
    }
    // A requirement that no rendered section places is in no document: say so.
    for e in lists.get("claims").unwrap_or(&empty) {
        if str_of(e.value, "kind") != Some("requirement") {
            continue;
        }
        let Some(id) = str_of(e.value, "id").filter(|i| !i.is_empty()) else {
            continue;
        };
        if !placed.contains(id) {
            diagnostics.push(Diagnostic {
                kind: DiagnosticKind::UnplacedEntity,
                entity: id.to_string(),
                line: e.line,
                message: format!(
                    "requirement `{id}` is in no section body, so it is not rendered and nothing checks it; place its marker `{{{{entity:{id}}}}}` in a section"
                ),
            });
        }
    }
    // A hole that no rendered section places is in no document: nothing checks it.
    for e in lists.get("contracts").unwrap_or(&empty) {
        if !render::is_hole(e) {
            continue;
        }
        let Some(id) = str_of(e.value, "id").filter(|i| !i.is_empty()) else {
            continue;
        };
        if !placed.contains(id) {
            diagnostics.push(Diagnostic {
                kind: DiagnosticKind::UnplacedEntity,
                entity: id.to_string(),
                line: e.line,
                message: format!(
                    "hole `{id}` is in no section body, so it is not rendered and nothing checks that it is implemented; place its marker `{{{{entity:{id}}}}}` in a section"
                ),
            });
        }
    }
    out.diagnostics = diagnostics;
    Ok(out)
}

/// Accumulates the lines of one document.
#[derive(Default)]
struct Builder {
    out: String,
    origin: Origin,
    /// The table the previous line belongs to, when it was a rendered table row.
    table: Option<&'static str>,
}

impl Builder {
    /// Append one line (no terminator) contributed by `entity`.
    fn line(&mut self, text: &str, entity: &str, source_line: usize) {
        self.out.push_str(text);
        self.out.push('\n');
        self.origin.push(entity, source_line);
        self.table = None;
    }

    /// Whether the previous line is a table line.
    fn after_table_line(&self) -> bool {
        self.out
            .strip_suffix('\n')
            .and_then(|o| o.rsplit('\n').next())
            .is_some_and(|l| l.trim_start().starts_with('|'))
    }

    /// The heading line and the body of one section, with markers rendered.
    fn section(
        &mut self,
        s: &Entity<'_>,
        ctx: &render::Ctx<'_>,
        diagnostics: &mut Vec<Diagnostic>,
        requirements: &mut BTreeMap<String, String>,
    ) {
        let sid = str_of(s.value, "id").unwrap_or("");
        let level = s.value.get("level").and_then(Value::as_u64).unwrap_or(0) as usize;
        if level > 0 {
            let title = str_of(s.value, "title").unwrap_or("").trim();
            let heading = if title.is_empty() {
                "#".repeat(level)
            } else {
                format!("{} {}", "#".repeat(level), title)
            };
            self.line(&heading, sid, s.line);
        }
        let body = str_of(s.value, "body").unwrap_or("").replace("\r\n", "\n");
        if body.is_empty() {
            return;
        }
        // The body's last `\n` terminates its last line; it is not an empty line of its own.
        for l in body.strip_suffix('\n').unwrap_or(&body).split('\n') {
            let Some(id) = marker_id(l) else {
                if l.contains(MARKER_PREFIX) {
                    diagnostics.push(Diagnostic {
                        kind: DiagnosticKind::MalformedMarker,
                        entity: sid.to_string(),
                        line: s.line,
                        message: format!(
                            "section `{sid}` has a line with `{MARKER_PREFIX}` that is not exactly one marker; it is kept as text"
                        ),
                    });
                }
                self.line(l, sid, s.line);
                continue;
            };
            let Some(e) = ctx.by_id.get(id) else {
                diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::UnknownEntity,
                    entity: sid.to_string(),
                    line: s.line,
                    message: format!("section `{sid}` names entity `{id}`, which does not exist"),
                });
                self.line(l, sid, s.line);
                continue;
            };
            match render::entity(e, ctx, diagnostics) {
                None => self.line(l, sid, s.line),
                Some(rendered) => {
                    if let Some(r) = rendered.requirement_ref {
                        requirements.entry(r).or_insert_with(|| id.to_string());
                    }
                    match rendered.block {
                        render::Block::Lines(lines) => {
                            for rl in &lines {
                                self.line(rl, id, e.line);
                            }
                        }
                        render::Block::Row {
                            table,
                            header,
                            separator,
                            row,
                        } => {
                            // A header is written only when the row does not continue a table
                            // already in the document (a rendered one, or one whose header and
                            // separator lines are text of the section body).
                            if self.table != Some(table) && !self.after_table_line() {
                                self.line(&header, id, e.line);
                                self.line(&separator, id, e.line);
                            }
                            self.line(&row, id, e.line);
                            self.table = Some(table);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

/// The parsed JSONC of an Exegra specification: the root value and the 1-based line of each
/// element of its top-level arrays. Used by checks that read the entities' properties directly.
#[derive(Clone, Debug)]
pub struct Source {
    pub root: Value,
    /// Top-level array key -> line of each element, in order.
    pub lines: BTreeMap<String, Vec<usize>>,
}

impl Source {
    /// The elements of the top-level array `key` with their JSONC lines (empty when absent).
    pub fn list<'a>(&'a self, key: &str) -> Vec<(&'a Value, usize)> {
        let Some(items) = self.root.get(key).and_then(Value::as_array) else {
            return Vec::new();
        };
        let lines = self.lines.get(key);
        items
            .iter()
            .enumerate()
            .map(|(i, v)| (v, lines.and_then(|l| l.get(i)).copied().unwrap_or(1)))
            .collect()
    }
}

/// Parse an Exegra JSONC file. The error is a message; it never panics.
pub fn parse_source(text: &str) -> Result<Source, String> {
    let clean = jsonc::clean(text)?;
    let root: Value = serde_json::from_str(&clean).map_err(|e| e.to_string())?;
    if !root.is_object() {
        return Err("the top level is not an object".into());
    }
    let top = jsonc::scan_top_level(&clean).ok_or("unreadable top level")?;
    let table = jsonc::LineTable::new(&clean);
    let lines = top
        .elements
        .iter()
        .map(|(k, offs)| (k.clone(), offs.iter().map(|&o| table.line(o)).collect()))
        .collect();
    Ok(Source { root, lines })
}
