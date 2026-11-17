//! Exegra JSONC specifications -> [`DocumentIndex`].
//!
//! An Exegra specification converted from Markdown keeps the Markdown in its sections
//! (verbatim bodies in which each table header and row is a `{{entity:<id>}}` marker).
//! The reader renders those sections to Markdown in memory exactly as Exegra does,
//! indexes the result with `concordance-markdown`, and maps every range back to the
//! JSONC line of the entity that produced it. Authored entities (`claims`, design areas,
//! variation points, seams, assumptions) are indexed from a second, generated text.
//!
//! Like `concordance-markdown`: pure, deterministic, never panics.

mod jsonc;
mod render;

#[cfg(test)]
mod tests;

use concordance_core::doc::{
    is_requirement_id, Authority, AuthorityAssessment, AuthoritySource, ClaimKind, DocumentIndex,
    Modality,
};
use concordance_core::lang::Languages;
use concordance_core::{ContentHash, RepoPath, SourceRange};
use concordance_markdown::{BlockHint, Hints};
use render::Origin;
use std::collections::HashSet;

/// Bump when extraction output changes for identical input (forces re-indexing).
pub const EXTRACTOR_VERSION: &str = "exegra-1";

#[derive(Clone, Debug)]
pub struct Options {
    /// Row kinds whose `ref` defines a requirement ID (singular or plural).
    pub requirement_kinds: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            requirement_kinds: concordance_core::config::ExegraConfig::default().requirement_kinds,
        }
    }
}

impl Options {
    fn defines_requirements(&self, kind: &str) -> bool {
        let k = norm_kind(kind);
        self.requirement_kinds.iter().any(|c| norm_kind(c) == k)
    }
}

/// `boundaryRules` and `boundaryRule` name the same kind.
fn norm_kind(k: &str) -> String {
    let l = k.to_ascii_lowercase();
    l.strip_suffix('s').map(str::to_string).unwrap_or(l)
}

/// Whether `text` is an Exegra specification (a top-level `"exegra"` key).
pub fn is_exegra(text: &str) -> bool {
    jsonc::has_top_level_key(text, "exegra")
}

/// The Markdown Exegra renders from the specification's sections
/// (`import-register.js --render`).
pub fn render_markdown(text: &str) -> Result<String, String> {
    let spec = parse(text)?;
    Ok(render::render(&spec, text).text)
}

fn parse(text: &str) -> Result<jsonc::Node, String> {
    let spec = jsonc::parse(text).map_err(|e| {
        let line = text[..e.offset.min(text.len())].matches('\n').count() + 1;
        format!("JSONC error at line {line}: {}", e.message)
    })?;
    if spec.get("exegra").is_none() {
        return Err("not an Exegra specification (no top-level \"exegra\" key)".into());
    }
    Ok(spec)
}

/// Index one Exegra specification. `configured` is the authority from `[authority]`
/// globs; without one the specification is normative.
pub fn index_exegra(
    path: &RepoPath,
    text: &str,
    configured: Option<AuthorityAssessment>,
    languages: &Languages,
    options: &Options,
) -> Result<DocumentIndex, String> {
    let spec = parse(text)?;
    let authority = configured.unwrap_or_else(|| AuthorityAssessment {
        level: Authority::Normative,
        source: AuthoritySource::SpecFormat,
        rationale: "an Exegra specification is the authority for what it specifies".into(),
    });
    let jl = JsoncLines::new(text);

    // Converted sections.
    let rendered = render::render(&spec, text);
    let mut hints = Hints::default();
    for r in &rendered.rows {
        let Some(id) = r.row.get_str("ref") else {
            continue;
        };
        if options.defines_requirements(r.kind) && is_requirement_id(id) {
            hints.blocks.insert(
                r.line,
                BlockHint {
                    requirement: Some(id.to_string()),
                    whole_claim: true,
                    ..Default::default()
                },
            );
        }
    }
    let mut idx = concordance_markdown::index_document_with(
        path,
        &rendered.text,
        authority.clone(),
        languages,
        &hints,
    );
    let map = LineMap {
        origins: &rendered.origins,
        src: text,
        jl: &jl,
        path,
    };
    map.remap_index(&mut idx);

    // Authored entities.
    let taken: HashSet<String> = idx
        .sections
        .iter()
        .filter(|s| s.heading_path.len() == 1)
        .map(|s| s.heading.clone())
        .collect();
    let authored = authored_text(&spec, text, options, &taken);
    if !authored.text.is_empty() {
        let mut extra = concordance_markdown::index_document_with(
            path,
            &authored.text,
            authority,
            languages,
            &authored.hints,
        );
        let map = LineMap {
            origins: &authored.origins,
            src: text,
            jl: &jl,
            path,
        };
        map.remap_index(&mut extra);
        let root = idx.sections[0].id.clone();
        let old_root = extra.sections[0].id.clone();
        for mut s in extra.sections.into_iter().skip(1) {
            if s.parent.as_ref() == Some(&old_root) {
                s.parent = Some(root.clone());
            }
            idx.sections.push(s);
        }
        // An Exegra claim is an assertion: a "may" inside it bounds what it asserts.
        for c in &mut extra.claims {
            if c.modality == Modality::May
                && c.heading_path.first() == authored.claims_heading.as_ref()
            {
                c.modality = Modality::Declarative;
            }
        }
        idx.claims.extend(extra.claims);
        for mut p in extra.prose_references {
            if p.section == old_root {
                p.section = root.clone();
            }
            idx.prose_references.push(p);
        }
    }

    let total = jl.count();
    idx.sections[0].range = SourceRange::lines(path.clone(), 1, total);
    idx.document.content_hash = ContentHash::of_str(text);
    idx.document.line_count = total;
    Ok(idx)
}

/// Line and column arithmetic over the JSONC text.
struct JsoncLines<'a> {
    text: &'a str,
    starts: Vec<usize>,
    count: u32,
}

impl<'a> JsoncLines<'a> {
    fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        let count = (text.lines().count() as u32).max(1);
        JsoncLines {
            text,
            starts,
            count,
        }
    }
    fn count(&self) -> u32 {
        self.count
    }
    fn line(&self, b: usize) -> u32 {
        (self.starts.partition_point(|&s| s <= b) as u32).clamp(1, self.count())
    }
    fn col(&self, b: usize) -> u32 {
        let ls = self.starts[self.line(b) as usize - 1];
        let b = b.min(self.text.len());
        self.text
            .get(ls..b)
            .map_or(1, |s| s.chars().count() as u32 + 1)
    }
}

struct LineMap<'a> {
    origins: &'a [Origin],
    src: &'a str,
    jl: &'a JsoncLines<'a>,
    path: &'a RepoPath,
}

impl LineMap<'_> {
    fn origin(&self, line: u32) -> Option<&Origin> {
        let i = (line.max(1) as usize - 1).min(self.origins.len().checked_sub(1)?);
        self.origins.get(i)
    }

    /// Byte offset in the JSONC of rendered column `col` (1-based, in characters).
    fn offset(&self, o: &Origin, col: u32) -> usize {
        match *o {
            Origin::Text { at, prefix, .. } => {
                let mut skip = col.saturating_sub(1).saturating_sub(prefix);
                let mut found = None;
                let end = jsonc::walk_literal(self.src, at, |c, b| {
                    if skip == 0 || c == '\n' {
                        found = Some(b);
                        return false;
                    }
                    skip -= 1;
                    true
                });
                found.unwrap_or(end)
            }
            Origin::Entity { entity } => entity.0,
        }
    }

    fn remap(&self, r: &SourceRange) -> SourceRange {
        let (Some(a), Some(b)) = (self.origin(r.start_line), self.origin(r.end_line)) else {
            return SourceRange::lines(self.path.clone(), 1, 1);
        };
        let whole_lines = r.start_col <= 1 && r.end_col <= 1;
        let same = a.entity() == b.entity();
        if same && !whole_lines && matches!(a, Origin::Text { .. }) {
            let s = self.offset(a, r.start_col);
            let e = self.offset(b, r.end_col);
            return SourceRange {
                path: self.path.clone(),
                start_line: self.jl.line(s),
                start_col: self.jl.col(s),
                end_line: self.jl.line(e).max(self.jl.line(s)),
                end_col: self.jl.col(e),
            };
        }
        let line_of = |o: &Origin, last: bool| match *o {
            Origin::Text { at, .. } => self.jl.line(at),
            Origin::Entity { entity } => self.jl.line(if last {
                entity.1.saturating_sub(1)
            } else {
                entity.0
            }),
        };
        let start = line_of(a, false);
        let mut end = line_of(b, true);
        if end < start {
            end = line_of(a, true).max(start);
        }
        SourceRange::lines(self.path.clone(), start, end)
    }

    fn remap_index(&self, idx: &mut DocumentIndex) {
        for s in &mut idx.sections {
            s.range = match self.origin(s.range.start_line) {
                Some(o) if s.level > 0 => SourceRange::lines(
                    self.path.clone(),
                    self.jl.line(o.entity().0),
                    self.jl.line(o.entity().1.saturating_sub(1)),
                ),
                _ => s.range.clone(),
            };
        }
        for c in &mut idx.claims {
            c.range = self.remap(&c.range);
            for r in &mut c.references {
                if let Some(rr) = &r.range {
                    r.range = Some(self.remap(rr));
                }
            }
        }
        for p in &mut idx.prose_references {
            if let Some(rr) = &p.reference.range {
                p.reference.range = Some(self.remap(rr));
            }
        }
    }
}

/// Generated Markdown for authored entities, with origins and hints.
struct Authored {
    text: String,
    origins: Vec<Origin>,
    hints: Hints,
    /// Heading of the section that holds the `claims` entries.
    claims_heading: Option<String>,
}

/// Authored lists read as informative text, with the string fields that hold their prose.
const INFORMATIVE: &[(&str, &str, &str, &[&str])] = &[
    (
        "designAreas",
        "Design areas",
        "Design area",
        &["title", "intent", "missing"],
    ),
    (
        "variationPoints",
        "Variation points",
        "Variation point",
        &["description", "tenantNeed", "provisionalDefault"],
    ),
    (
        "seams",
        "Seams",
        "Seam",
        &["question", "provisionalChoice", "interface", "revisitWhen"],
    ),
    ("assumptions", "Assumptions", "Assumption", &["statement"]),
];

/// The claim kind an Exegra claim `kind` names, if it names one.
fn claim_kind(kind: &str) -> Option<ClaimKind> {
    Some(match kind {
        "invariant" => ClaimKind::Invariant,
        "structural" | "calls" | "reads" | "writes" | "data-flow" | "dependency" => {
            ClaimKind::ArchitecturalConstraint
        }
        "interface" => ClaimKind::ApiContract,
        "guarantee" => ClaimKind::Guarantee,
        "prohibition" => ClaimKind::Prohibition,
        "behavior" | "behaviour" => ClaimKind::Behavior,
        "requirement" => ClaimKind::Requirement,
        _ => return None,
    })
}

impl Authored {
    fn line(&mut self, s: &str, origin: Origin) {
        self.text.push_str(s);
        self.text.push('\n');
        self.origins.push(origin);
    }

    fn heading(&mut self, level: usize, title: &str, entity: (usize, usize)) {
        self.line(
            &format!("{} {title}", "#".repeat(level)),
            Origin::Entity { entity },
        );
        self.line("", Origin::Entity { entity });
    }

    /// A string literal as one paragraph; returns the line it starts on.
    fn paragraph(&mut self, src: &str, lit: &jsonc::Node, entity: (usize, usize)) -> u32 {
        let first = self.origins.len() as u32 + 1;
        let value = lit.str().unwrap_or_default();
        let starts = render::literal_line_starts(src, lit);
        for (k, l) in value.split('\n').enumerate() {
            let at = starts.get(k).copied().unwrap_or(lit.end);
            // A leading block marker would turn the statement into a heading, list or table.
            let escape = k == 0
                && l.chars()
                    .next()
                    .is_some_and(|c| "#>-+*=|`~<".contains(c) || c.is_ascii_digit());
            let (line, prefix) = if escape {
                (format!("\\{l}"), 1)
            } else {
                (l.to_string(), 0)
            };
            self.line(&line, Origin::Text { entity, at, prefix });
        }
        self.line("", Origin::Entity { entity });
        first
    }
}

fn authored_text(
    spec: &jsonc::Node,
    src: &str,
    options: &Options,
    taken: &HashSet<String>,
) -> Authored {
    let mut a = Authored {
        text: String::new(),
        origins: Vec::new(),
        hints: Hints::default(),
        claims_heading: None,
    };
    // A converted top-level heading of the same name would share section ids.
    let title = |t: &str| {
        if taken.contains(t) {
            format!("{t} (Exegra)")
        } else {
            t.to_string()
        }
    };
    let claims_define = options.defines_requirements("claims");
    if let Some(list) = spec.get("claims").filter(|l| !l.items().is_empty()) {
        let heading = title("Claims");
        a.heading(1, &heading, (list.start, list.end));
        a.claims_heading = Some(heading);
        for c in list.items() {
            let Some(statement) = c.get("statement").filter(|s| s.str().is_some()) else {
                continue;
            };
            let line = a.paragraph(src, statement, (c.start, c.end));
            let requirement = c
                .get_str("ref")
                .filter(|r| claims_define && is_requirement_id(r))
                .map(str::to_string);
            a.hints.blocks.insert(
                line,
                BlockHint {
                    requirement,
                    whole_claim: true,
                    kind: c.get_str("kind").and_then(claim_kind),
                    authority: None,
                },
            );
        }
    }
    for (key, plural, singular, fields) in INFORMATIVE {
        let Some(list) = spec.get(key).filter(|l| !l.items().is_empty()) else {
            continue;
        };
        a.heading(1, &title(plural), (list.start, list.end));
        for e in list.items() {
            let name = ["ref", "key", "id"]
                .iter()
                .find_map(|k| e.get_str(k))
                .unwrap_or("?");
            a.heading(2, &format!("{singular} {name}"), (e.start, e.end));
            for f in *fields {
                let Some(lit) = e
                    .get(f)
                    .filter(|v| v.str().is_some_and(|s| !s.trim().is_empty()))
                else {
                    continue;
                };
                let line = a.paragraph(src, lit, (e.start, e.end));
                a.hints.blocks.insert(
                    line,
                    BlockHint {
                        authority: Some(Authority::Informative),
                        ..Default::default()
                    },
                );
            }
        }
    }
    a
}
