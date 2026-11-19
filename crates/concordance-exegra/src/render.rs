//! Rendering of one entity into the Markdown form Concordance's front end reads.
//!
//! Only properties are read. A `raw` property (the Markdown an entity was imported from, which
//! older imports wrote) is never consulted: it is stale as soon as a property is edited.
//!
//! Text properties may span several lines (a decision's sub-points, a milestone's paragraphs);
//! those lines are kept, continuing under a list item's marker.

use crate::{str_of, Diagnostic, DiagnosticKind, Entity};
use serde_json::Value;
use std::collections::HashMap;

/// Lookup of entities by id, for markers and references.
pub(crate) struct Ctx<'a> {
    pub by_id: &'a HashMap<&'a str, Entity<'a>>,
    /// Requirement entity id -> ids of the claims whose `models` names it, in file order.
    pub modelled_by: &'a HashMap<String, Vec<String>>,
}

pub(crate) enum Block {
    /// One or more complete lines.
    Lines(Vec<String>),
    /// A table row. The header and separator lines are emitted when the previous line was not a
    /// row of the same `table`.
    Row {
        table: &'static str,
        header: String,
        separator: String,
        row: String,
    },
}

pub(crate) struct Rendered {
    pub block: Block,
    /// For a requirement: its `ref`.
    pub requirement_ref: Option<String>,
}

/// Render `e`, or report why it cannot be (the caller then keeps the marker line).
pub(crate) fn entity(
    e: &Entity<'_>,
    ctx: &Ctx<'_>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Rendered> {
    match e.list {
        "claims" if str_of(e.value, "kind") == Some("requirement") => requirement(e, ctx, diags),
        "contracts" if is_hole(e) => hole(e, ctx, diags),
        "goals" | "nonGoals" | "constraints" | "openQuestions" | "decisions" => item(e, diags),
        "terms" => term(e, diags),
        "milestones" => match lines_of(e, "detail") {
            Some(lines) => Some(Rendered {
                block: Block::Lines(lines),
                requirement_ref: None,
            }),
            None => paragraph(e, diags),
        },
        "boundaries" => row(
            e,
            diags,
            "boundaries",
            "| ID | Boundary | Interface | Used by |",
            None,
            ["name", "interfaceKind", "usedBy"],
        ),
        "components" => row(
            e,
            diags,
            "components",
            "| ID | Component | Owns | Talks to |",
            None,
            ["name", "purpose", "talksTo"],
        ),
        "releaseStages" => {
            let first = match (text_of(e, "ref"), text_of(e, "name")) {
                (Some(r), Some(n)) if r != n => Some(format!("**{r} {n}**")),
                (_, Some(n)) => Some(format!("**{n}**")),
                (Some(r), None) => Some(format!("**{r}**")),
                (None, None) => None,
            };
            row(
                e,
                diags,
                "releaseStages",
                "| Stage | Purpose | Milestones | Exit evidence |",
                first,
                ["purpose", "milestones", "exitEvidence"],
            )
        }
        _ => paragraph(e, diags),
    }
}

/// `s` on one line: line breaks and the space around them become one space.
pub(crate) fn one_line(s: &str) -> String {
    s.split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The lines of a multi-line text property: trailing space trimmed, blank lines inside kept,
/// leading and trailing blank lines dropped. Indentation (nested list items) is kept.
fn lines_of(e: &Entity<'_>, key: &str) -> Option<Vec<String>> {
    let s = str_of(e.value, key)?;
    let mut lines: Vec<String> = s.split('\n').map(|l| l.trim_end().to_string()).collect();
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    (!lines.is_empty()).then_some(lines)
}

/// `head` followed by the first of `lines`, then the other lines indented by `indent` spaces.
fn under_head(head: &str, lines: &[String], indent: usize) -> Vec<String> {
    let pad = " ".repeat(indent);
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| match (i, l.is_empty()) {
            (0, _) => format!("{head}{l}").trim_end().to_string(),
            (_, true) => String::new(),
            _ => format!("{pad}{l}"),
        })
        .collect()
}

fn text_of(e: &Entity<'_>, key: &str) -> Option<String> {
    str_of(e.value, key).map(one_line).filter(|s| !s.is_empty())
}

fn id_of<'a>(e: &Entity<'a>) -> &'a str {
    str_of(e.value, "id").unwrap_or("")
}

fn invalid(e: &Entity<'_>, diags: &mut Vec<Diagnostic>, message: String) {
    diags.push(Diagnostic {
        kind: DiagnosticKind::InvalidEntity,
        entity: id_of(e).to_string(),
        line: e.line,
        message,
    });
}

fn strings<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a str> {
    v.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

/// A number or string property as text (`3`, not `3.0`).
fn number_text(e: &Entity<'_>, key: &str) -> Option<String> {
    match e.value.get(key)? {
        Value::Number(n) => Some(
            n.as_u64()
                .map(|x| x.to_string())
                .or_else(|| n.as_i64().map(|x| x.to_string()))
                .unwrap_or_else(|| n.to_string()),
        ),
        Value::String(_) => text_of(e, key),
        _ => None,
    }
}

/// The `ref` of the entity `id`, or `id` itself (with a diagnostic) when it has none.
fn reference(
    e: &Entity<'_>,
    ctx: &Ctx<'_>,
    diags: &mut Vec<Diagnostic>,
    property: &str,
    id: &str,
) -> String {
    match ctx.by_id.get(id).and_then(|t| text_of(t, "ref")) {
        Some(r) => r,
        None => {
            diags.push(Diagnostic {
                kind: DiagnosticKind::DanglingReference,
                entity: id_of(e).to_string(),
                line: e.line,
                message: format!(
                    "`{property}` of `{}` names `{id}`, which is not an entity with a `ref`",
                    id_of(e)
                ),
            });
            id.to_string()
        }
    }
}

/// A requirement: `**<ref>** <statement>` and its attribute bullets, in a fixed order.
fn requirement(e: &Entity<'_>, ctx: &Ctx<'_>, diags: &mut Vec<Diagnostic>) -> Option<Rendered> {
    let (Some(r), Some(statement)) = (text_of(e, "ref"), text_of(e, "statement")) else {
        invalid(
            e,
            diags,
            format!(
                "requirement `{}` has no `ref` or no `statement`; its marker is kept",
                id_of(e)
            ),
        );
        return None;
    };
    let v = e.value;
    let mut lines = vec![format!("**{r}** {statement}")];
    if str_of(v, "lifecycle") == Some("retired") {
        lines.push("- Status: retired".to_string());
    }
    if let Some(m) = v
        .get("evaluate")
        .and_then(|x| str_of(x, "method"))
        .map(one_line)
        .filter(|m| !m.is_empty())
    {
        lines.push(format!("- Verify: {m}"));
    }
    let mut cases: Vec<String> = Vec::new();
    for cid in strings(v, "cases") {
        match ctx.by_id.get(cid).and_then(|c| text_of(c, "path")) {
            Some(p) => cases.push(p),
            None => {
                diags.push(Diagnostic {
                    kind: DiagnosticKind::DanglingReference,
                    entity: id_of(e).to_string(),
                    line: e.line,
                    message: format!(
                        "`cases` of `{}` names `{cid}`, which is not an entity with a `path`",
                        id_of(e)
                    ),
                });
                cases.push(cid.to_string());
            }
        }
    }
    if let Some(p) = text_of(e, "casesPath") {
        diags.push(Diagnostic {
            kind: DiagnosticKind::UnmatchedCasesPath,
            entity: id_of(e).to_string(),
            line: e.line,
            message: format!(
                "`casesPath` of `{}` is `{p}`, which matched no case table",
                id_of(e)
            ),
        });
        cases.push(p);
    }
    if !cases.is_empty() {
        lines.push(format!("- Cases: {}", cases.join(", ")));
    }
    let detail = text_of(e, "boundaryDetail");
    let boundary = text_of(e, "boundary").map(|b| reference(e, ctx, diags, "boundary", &b));
    match (boundary, detail) {
        (Some(b), Some(d)) => lines.push(format!("- Boundary: {b} {d}")),
        (Some(b), None) => lines.push(format!("- Boundary: {b}")),
        (None, Some(d)) => lines.push(format!("- Boundary: {d}")),
        (None, None) => {}
    }
    for (key, label) in [("traces", "Traces"), ("dependsOn", "Depends")] {
        let refs: Vec<String> = strings(v, key)
            .map(|id| reference(e, ctx, diags, key, id))
            .collect();
        if !refs.is_empty() {
            lines.push(format!("- {label}: {}", refs.join(", ")));
        }
    }
    if let Some(r) = text_of(e, "rationale") {
        lines.push(format!("- Rationale: {r}"));
    }
    if let Some(m) = number_text(e, "milestone") {
        lines.push(format!("- Milestone: {m}"));
    }
    if let Some(s) = number_text(e, "seam") {
        lines.push(format!("- Seam: {s}"));
    }
    if let Some(x) = text_of(e, "realisation") {
        lines.push(format!("- Realisation: {x}"));
    }
    let mut modelling: Vec<&str> = Vec::new();
    for key in [id_of(e), r.as_str()] {
        for c in ctx.modelled_by.get(key).into_iter().flatten() {
            if !modelling.contains(&c.as_str()) {
                modelling.push(c);
            }
        }
    }
    if !modelling.is_empty() {
        lines.push(format!("- Modelled by: {}", modelling.join(", ")));
    }
    Some(Rendered {
        block: Block::Lines(lines),
        requirement_ref: Some(r),
    })
}

/// A contract that is a hole: `realisation` is `hand` and it has a `ref`.
pub(crate) fn is_hole(e: &Entity<'_>) -> bool {
    e.list == "contracts"
        && str_of(e.value, "realisation") == Some("hand")
        && text_of(e, "ref").is_some()
}

/// The text of one postcondition: a string (the id of an entity with a `statement`, or text),
/// or an object with `statement`, `text` or `description`.
fn postcondition(ctx: &Ctx<'_>, v: &Value) -> Option<String> {
    let text = match v {
        Value::String(s) => match ctx
            .by_id
            .get(s.as_str())
            .and_then(|t| text_of(t, "statement"))
        {
            Some(t) => t,
            None => one_line(s),
        },
        Value::Object(_) => ["statement", "text", "description"]
            .iter()
            .find_map(|k| v.get(*k).and_then(Value::as_str))
            .map(one_line)?,
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

/// A hole: a hand-written contract, rendered as a requirement `**<ref>** ...` that its
/// postconditions state, with `Realisation: hole` and the component that implements it.
fn hole(e: &Entity<'_>, ctx: &Ctx<'_>, diags: &mut Vec<Diagnostic>) -> Option<Rendered> {
    let r = text_of(e, "ref")?;
    let posts: Vec<String> = e
        .value
        .get("postconditions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| postcondition(ctx, v))
        .collect();
    let name = text_of(e, "name").unwrap_or_else(|| id_of(e).to_string());
    let statement = if posts.is_empty() {
        format!("The item implementing {name} SHALL satisfy its contract.")
    } else {
        format!(
            "The item implementing {name} SHALL satisfy its postconditions: {}",
            posts.join(" ")
        )
    };
    let mut lines = vec![
        format!("**{r}** {statement}"),
        "- Realisation: hole".to_string(),
    ];
    if let Some(c) = text_of(e, "implementedBy") {
        let c = reference(e, ctx, diags, "implementedBy", &c);
        lines.push(format!("- Component: {c}"));
    }
    if let Some(m) = number_text(e, "milestone") {
        lines.push(format!("- Milestone: {m}"));
    }
    Some(Rendered {
        block: Block::Lines(lines),
        requirement_ref: None,
    })
}

/// A goal, non-goal, constraint, decision or open question: `- **<ref>** <text>`. Text over
/// several lines (sub-points, paragraphs) continues indented under the item. A goal's release
/// stage follows its text as `*(<stage>)*`; a non-goal's horizon leads it as `*<horizon>*`.
fn item(e: &Entity<'_>, diags: &mut Vec<Diagnostic>) -> Option<Rendered> {
    let Some(r) = text_of(e, "ref") else {
        invalid(
            e,
            diags,
            format!("`{}` has no `ref`; its marker is kept", id_of(e)),
        );
        return None;
    };
    let Some(mut t) = lines_of(e, "statement").or_else(|| lines_of(e, "title")) else {
        invalid(
            e,
            diags,
            format!("`{}` has no `statement`; its marker is kept", id_of(e)),
        );
        return None;
    };
    Some(Rendered {
        block: Block::Lines({
            if let Some(stage) = text_of(e, "stage").filter(|_| e.list == "goals") {
                if let Some(last) = t.last_mut() {
                    *last = format!("{last} *({stage})*");
                }
            }
            let head = match text_of(e, "horizon").filter(|_| e.list == "nonGoals") {
                Some(h) => format!("- **{r}** *{h}* "),
                None => format!("- **{r}** "),
            };
            under_head(&head, &t, 2)
        }),
        requirement_ref: None,
    })
}

/// A glossary term: `- **<term>**: <definition>`.
fn term(e: &Entity<'_>, diags: &mut Vec<Diagnostic>) -> Option<Rendered> {
    let Some(name) = text_of(e, "term") else {
        return paragraph(e, diags);
    };
    let lines = lines_of(e, "definition").unwrap_or_default();
    Some(Rendered {
        block: Block::Lines(if lines.is_empty() {
            vec![format!("- **{name}**")]
        } else {
            under_head(&format!("- **{name}**: "), &lines, 2)
        }),
        requirement_ref: None,
    })
}

fn cell(s: &str) -> String {
    let mut out = String::new();
    let mut code = false;
    for ch in one_line(s).chars() {
        if ch == '`' {
            code = !code;
        }
        if ch == '|' && !code {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A boundary, component or release stage: a table row `| <first> | <a> | <b> | <c> |`, where
/// `<first>` is `first` or else the entity's `ref`.
fn row(
    e: &Entity<'_>,
    diags: &mut Vec<Diagnostic>,
    table: &'static str,
    header: &str,
    first: Option<String>,
    columns: [&str; 3],
) -> Option<Rendered> {
    let Some(r) = first.or_else(|| text_of(e, "ref")) else {
        invalid(
            e,
            diags,
            format!("`{}` has no `ref`; its marker is kept", id_of(e)),
        );
        return None;
    };
    let cells: Vec<String> = columns
        .iter()
        .map(|k| cell(str_of(e.value, k).unwrap_or("")))
        .collect();
    Some(Rendered {
        block: Block::Row {
            table,
            header: header.to_string(),
            separator: "|---|---|---|---|".to_string(),
            row: format!(
                "| {} | {} | {} | {} |",
                cell(&r),
                cells[0],
                cells[1],
                cells[2]
            ),
        },
        requirement_ref: None,
    })
}

/// Any other entity: its label and main text properties, each property's lines kept.
fn paragraph(e: &Entity<'_>, diags: &mut Vec<Diagnostic>) -> Option<Rendered> {
    let label = ["name", "title", "term"].iter().find_map(|k| text_of(e, k));
    let rest: Vec<String> = [
        "purpose",
        "statement",
        "definition",
        "proves",
        "detail",
        "exitEvidence",
        "description",
    ]
    .iter()
    .filter_map(|k| lines_of(e, k))
    .flatten()
    .collect();
    let lines = match (label, rest.is_empty()) {
        (Some(l), false) => under_head(&format!("{l}: "), &rest, 0),
        (Some(l), true) => vec![l],
        (None, false) => rest,
        (None, true) => {
            invalid(
                e,
                diags,
                format!("`{}` has no text properties; its marker is kept", id_of(e)),
            );
            return None;
        }
    };
    Some(Rendered {
        block: Block::Lines(lines),
        requirement_ref: None,
    })
}
