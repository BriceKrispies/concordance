//! Exegra sections -> Markdown, as `tools/import-register.js` (`rebuild`, `entityRenderer`,
//! `renderRow`) renders them, with the JSONC origin of every rendered line.

use crate::jsonc::{walk_literal, Node, Value};
use std::collections::HashMap;

/// Where a rendered line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Text copied from a string literal: `at` is the byte offset in the JSONC of the
    /// line's first character, `prefix` the number of rendered characters before it
    /// (`## ` for a heading). `entity` is the byte span of the owning object.
    Text {
        entity: (usize, usize),
        at: usize,
        prefix: u32,
    },
    /// A line rendered from an entity's properties (a table header or row).
    Entity { entity: (usize, usize) },
}

impl Origin {
    pub fn entity(&self) -> (usize, usize) {
        match self {
            Origin::Text { entity, .. } | Origin::Entity { entity } => *entity,
        }
    }
}

/// A row rendered on a line, for requirement tagging.
pub struct RenderedRow<'a> {
    pub line: u32,
    pub row: &'a Node,
    /// The row's kind: its table's `rowKind`, else the list that holds it.
    pub kind: &'a str,
}

pub struct Rendered<'a> {
    pub text: String,
    /// `origins[n]` is the origin of rendered line `n + 1`.
    pub origins: Vec<Origin>,
    pub rows: Vec<RenderedRow<'a>>,
    /// Markers naming an entity that does not render (kept as text).
    pub unresolved: Vec<String>,
}

/// Top-level lists that never hold rows (see `renderIndex`).
const NOT_ROWS: &[&str] = &["tables", "sections", "documents", "views", "relationships"];

/// A value as JavaScript would interpolate it into a template string.
fn js_text<'s>(n: &'s Node, src: &'s str) -> std::borrow::Cow<'s, str> {
    match &n.value {
        Value::Str(s) => s.as_str().into(),
        Value::Null => "null".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        _ => src[n.start..n.end].into(),
    }
}

/// `renderRow`: canonical `| a | b |`, or the row's own `layout` with `{}` per cell.
pub fn render_row(cells: &[std::borrow::Cow<'_, str>], layout: Option<&str>) -> String {
    match layout {
        None => {
            let mut out = String::from("|");
            for c in cells {
                if c.is_empty() {
                    out.push(' ');
                } else {
                    out.push(' ');
                    out.push_str(c);
                    out.push(' ');
                }
                out.push('|');
            }
            out
        }
        Some(l) => {
            let mut out = String::new();
            for (k, part) in l.split("{}").enumerate() {
                out.push_str(part);
                if let Some(c) = cells.get(k) {
                    out.push_str(c);
                }
            }
            out
        }
    }
}

/// Byte offsets (in the JSONC) where each `\n`-separated line of a string literal starts.
pub fn literal_line_starts(src: &str, lit: &Node) -> Vec<usize> {
    let mut starts = vec![lit.start + 1];
    let mut after_newline = false;
    let end = walk_literal(src, lit.start + 1, |c, at| {
        if after_newline {
            starts.push(at);
        }
        after_newline = c == '\n';
        true
    });
    if after_newline {
        starts.push(end);
    }
    starts
}

pub fn render<'a>(spec: &'a Node, src: &'a str) -> Rendered<'a> {
    let mut tables: HashMap<&str, &Node> = HashMap::new();
    if let Some(t) = spec.get("tables") {
        for e in t.items() {
            if let Some(id) = e.get_str("id") {
                tables.insert(id, e);
            }
        }
    }
    let mut rows: HashMap<&str, (&Node, &str)> = HashMap::new();
    for (key, list) in spec.members() {
        if NOT_ROWS.contains(&key.as_str()) {
            continue;
        }
        for e in list.items() {
            if let (Some(id), Some(_)) = (e.get_str("id"), e.get_str("table")) {
                rows.insert(id, (e, key.as_str()));
            }
        }
    }

    let mut sections: Vec<&Node> = spec
        .get("sections")
        .map(|s| s.items().iter().collect())
        .unwrap_or_default();
    // Stable, like Array.prototype.sort; a missing order compares equal (NaN in JS).
    sections.sort_by(|a, b| {
        match (
            a.get("order").and_then(Node::num),
            b.get("order").and_then(Node::num),
        ) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            _ => std::cmp::Ordering::Equal,
        }
    });

    let mut out = Rendered {
        text: String::new(),
        origins: Vec::new(),
        rows: Vec::new(),
        unresolved: Vec::new(),
    };
    let push = |out: &mut Rendered<'a>, line: &str, origin: Origin| {
        out.text.push_str(line);
        out.text.push('\n');
        out.origins.push(origin);
    };

    for s in sections {
        let span = (s.start, s.end);
        let level = s.get("level").and_then(Node::num).unwrap_or(0.0);
        if level > 0.0 {
            let n = level as usize;
            let title = s.get("title");
            let line = format!(
                "{} {}",
                "#".repeat(n),
                title.map(|t| js_text(t, src)).unwrap_or_default()
            );
            let origin = match title {
                Some(t) if t.str().is_some() => Origin::Text {
                    entity: span,
                    at: t.start + 1,
                    prefix: n as u32 + 1,
                },
                _ => Origin::Entity { entity: span },
            };
            push(&mut out, &line, origin);
        }
        let Some(body_node) = s
            .get("body")
            .filter(|b| b.str().is_some_and(|b| !b.is_empty()))
        else {
            continue;
        };
        let body = body_node.str().unwrap_or_default();
        // `body.slice(0, -1)`: the body's final character (its newline) is dropped.
        let trimmed = match body.char_indices().last() {
            Some((i, _)) => &body[..i],
            None => body,
        };
        let starts = literal_line_starts(src, body_node);
        for (k, line) in trimmed.split('\n').enumerate() {
            let at = starts.get(k).copied().unwrap_or(body_node.end);
            let text_origin = Origin::Text {
                entity: span,
                at,
                prefix: 0,
            };
            let Some(id) = marker(line) else {
                push(&mut out, line, text_origin);
                continue;
            };
            if let Some(t) = tables.get(id) {
                let cols: Vec<_> = t
                    .get("columns")
                    .map(|c| c.items().iter().map(|x| js_text(x, src)).collect())
                    .unwrap_or_default();
                let origin = Origin::Entity {
                    entity: (t.start, t.end),
                };
                let header = render_row(&cols, None);
                let sep = t
                    .get("separator")
                    .map(|x| js_text(x, src))
                    .unwrap_or_default();
                for l in format!("{header}\n{sep}").split('\n') {
                    push(&mut out, l, origin);
                }
                continue;
            }
            let rendered = rows.get(id).and_then(|&(r, list)| {
                let table = tables.get(r.get_str("table")?)?;
                let cells: Vec<_> = match r.get("cells") {
                    Some(c) if matches!(c.value, Value::Arr(_)) => {
                        c.items().iter().map(|x| js_text(x, src)).collect()
                    }
                    _ => table
                        .get("columnProperties")
                        .map(|ps| {
                            ps.items()
                                .iter()
                                .map(|p| {
                                    p.str()
                                        .and_then(|p| r.get_str(p))
                                        .unwrap_or_default()
                                        .into()
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                let kind = table.get_str("rowKind").unwrap_or(list);
                Some((r, kind, render_row(&cells, r.get_str("layout"))))
            });
            match rendered {
                Some((r, kind, text)) => {
                    out.rows.push(RenderedRow {
                        line: out.origins.len() as u32 + 1,
                        row: r,
                        kind,
                    });
                    let origin = Origin::Entity {
                        entity: (r.start, r.end),
                    };
                    for l in text.split('\n') {
                        push(&mut out, l, origin);
                    }
                }
                None => {
                    out.unresolved.push(id.to_string());
                    push(&mut out, line, text_origin);
                }
            }
        }
    }
    out
}

/// The entity id of a `{{entity:<id>}}` marker line.
fn marker(line: &str) -> Option<&str> {
    let id = line.strip_prefix("{{entity:")?.strip_suffix("}}")?;
    (!id.is_empty() && !id.contains('}')).then_some(id)
}
