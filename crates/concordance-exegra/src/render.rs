//! Rendering of one entity into the Markdown form Concordance's front end reads.
//!
//! Only properties are read. The `raw` property (the Markdown an entity was imported from) is
//! never consulted: it is stale as soon as a property is edited.

use crate::{str_of, Diagnostic, DiagnosticKind, Entity};
use serde_json::Value;
use std::collections::HashMap;

/// Lookup of entities by id, for markers and references.
pub(crate) struct Ctx<'a> {
    pub by_id: &'a HashMap<&'a str, Entity<'a>>,
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
        "goals" | "nonGoals" | "constraints" | "openQuestions" | "decisions" => item(e, diags),
        "boundaries" => row(
            e,
            diags,
            "boundaries",
            "| ID | Boundary | Interface | Used by |",
            ["name", "interfaceKind", "usedBy"],
        ),
        "components" => row(
            e,
            diags,
            "components",
            "| ID | Component | Category | Purpose |",
            ["name", "category", "purpose"],
        ),
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
    Some(Rendered {
        block: Block::Lines(lines),
        requirement_ref: Some(r),
    })
}

/// A goal, non-goal, constraint, decision or open question: `- **<ref>** <text>`.
fn item(e: &Entity<'_>, diags: &mut Vec<Diagnostic>) -> Option<Rendered> {
    let Some(r) = text_of(e, "ref") else {
        invalid(
            e,
            diags,
            format!("`{}` has no `ref`; its marker is kept", id_of(e)),
        );
        return None;
    };
    let Some(t) = text_of(e, "statement").or_else(|| text_of(e, "title")) else {
        invalid(
            e,
            diags,
            format!("`{}` has no `statement`; its marker is kept", id_of(e)),
        );
        return None;
    };
    Some(Rendered {
        block: Block::Lines(vec![format!("- **{r}** {t}")]),
        requirement_ref: None,
    })
}

fn cell(s: &str) -> String {
    one_line(s).replace('|', "\\|")
}

/// A boundary or component: a table row `| <ref> | <a> | <b> | <c> |`.
fn row(
    e: &Entity<'_>,
    diags: &mut Vec<Diagnostic>,
    table: &'static str,
    header: &str,
    columns: [&str; 3],
) -> Option<Rendered> {
    let Some(r) = text_of(e, "ref") else {
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

/// Any other entity: a plain paragraph of its main text properties.
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
    .filter_map(|k| text_of(e, k))
    .collect();
    let text = match (label, rest.is_empty()) {
        (Some(l), false) => format!("{l}: {}", rest.join(" ")),
        (Some(l), true) => l,
        (None, false) => rest.join(" "),
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
        block: Block::Lines(vec![text]),
        requirement_ref: None,
    })
}
