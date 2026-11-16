//! Semantic rules over a structured (Exegra JSONC) specification.
//!
//! The rules R1..R8 read rendered Markdown. These rules read the entities themselves
//! (`claims`, `caseTables`, `terms`, `decisions`) and check the specification against itself:
//!
//! | Code | Config key | Default | Reports |
//! |---|---|---|---|
//! | S1 | `milestone_order` | warning | an active requirement that depends on, or cites in its statement, a requirement (or a case row) of a later milestone |
//! | S2 | `budget_row` | error | an active `budget` requirement with no row in the budget-measurement table |
//! | S3 | `sub_budget` | warning | sub-budgets of a parent budget (by `Depends:`, parent worded as a decomposition) whose sum exceeds the parent |
//! | S4 | `constants` | warning | a configured constant given a value in a rationale, or named in a statement, that no constants table defines |
//! | S5 | `forbidden_terms` | warning | a configured forbidden phrase in active text |
//! | S6 | `duplicate_terms` | error | a glossary term defined more than once |
//! | S7 | `dead_references` | warning | active text citing a retired or nonexistent id |
//! | S8 | `said_once` | info | active statements that are near-duplicates (review hint) |
//! | S9 | `replacement_char` | error | U+FFFD in the source |
//!
//! Every rule is configured under `[lint.rules.<key>]` (see `docs/spec-lint.md`). All of them
//! are pure functions of the source text and the configuration.

use crate::{LintFinding, Severity};
use concordance_core::config::RuleFile;
use concordance_exegra::{parse_source, Source};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

struct Req {
    id: String,
    rf: String,
    statement: String,
    rationale: String,
    active: bool,
    milestone: Option<u32>,
    depends: Vec<String>,
    method: String,
    area: String,
    cases: Vec<String>,
    line: usize,
}

struct Table {
    id: String,
    path: String,
    area: String,
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    line: usize,
}

struct Ent {
    rf: String,
    active: bool,
}

struct Ctx<'a> {
    path: &'a str,
    reqs: Vec<Req>,
    by_id: HashMap<String, usize>,
    tables: Vec<Table>,
    /// Every `ref` of the source (requirements, goals, decisions, ...), upper case.
    refs: HashMap<String, Ent>,
    prefixes: HashSet<String>,
    out: Vec<F>,
}

struct F {
    rule: &'static str,
    sev: Severity,
    line: usize,
    entity: String,
    msg: String,
}

struct Rs {
    on: bool,
    sev: Severity,
    f: RuleFile,
}

fn rs(rules: &BTreeMap<String, RuleFile>, key: &str, on: bool, sev: Severity) -> Rs {
    let f = rules.get(key).cloned().unwrap_or_default();
    let sev = match f.severity.as_deref() {
        Some("error") => Severity::Error,
        Some("warning") => Severity::Warning,
        Some("info") => Severity::Info,
        _ => sev,
    };
    Rs {
        on: f.enabled.unwrap_or(on),
        sev,
        f,
    }
}

/// A finding of the semantic rules, with the entity it is about.
pub struct SourceFinding {
    pub finding: LintFinding,
    pub entity: String,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Run the semantic rules on one Exegra source. `path` is the source file's repository-relative
/// path. A source that does not parse yields only S9 (the engine reports the rest as `R0`).
pub fn check_source(
    path: &str,
    text: &str,
    rules: &BTreeMap<String, RuleFile>,
) -> Vec<SourceFinding> {
    let mut cx = Ctx {
        path,
        reqs: Vec::new(),
        by_id: HashMap::new(),
        tables: Vec::new(),
        refs: HashMap::new(),
        prefixes: HashSet::new(),
        out: Vec::new(),
    };
    // S9 reads the raw text, so it runs even when the JSONC does not parse.
    let r = rs(rules, "replacement_char", true, Severity::Error);
    if r.on {
        s9_replacement(&mut cx, text, &r);
    }
    if let Ok(src) = parse_source(text) {
        load(&mut cx, &src);
        let r = rs(rules, "milestone_order", true, Severity::Warning);
        if r.on {
            s1_milestone(&mut cx, &r);
        }
        let r = rs(rules, "budget_row", true, Severity::Error);
        if r.on {
            s2_budget_row(&mut cx, &r);
        }
        let r = rs(rules, "sub_budget", true, Severity::Warning);
        if r.on {
            s3_sub_budget(&mut cx, &r);
        }
        let r = rs(rules, "constants", true, Severity::Warning);
        if r.on {
            s4_constants(&mut cx, &r);
        }
        let r = rs(rules, "forbidden_terms", true, Severity::Warning);
        if r.on {
            s5_forbidden(&mut cx, &src, &r);
        }
        let r = rs(rules, "duplicate_terms", true, Severity::Error);
        if r.on {
            s6_duplicate_terms(&mut cx, &src, &r);
        }
        let r = rs(rules, "dead_references", true, Severity::Warning);
        if r.on {
            s7_dead_refs(&mut cx, &src, &r);
        }
        let r = rs(rules, "said_once", true, Severity::Info);
        if r.on {
            s8_said_once(&mut cx, &r);
        }
    }
    let path = cx.path.to_string();
    cx.out
        .into_iter()
        .map(|f| SourceFinding {
            finding: LintFinding {
                rule: f.rule,
                severity: f.sev,
                message: f.msg,
                path: path.clone(),
                line: f.line as u32,
                origin: None,
            },
            entity: f.entity,
        })
        .collect()
}

fn load(cx: &mut Ctx<'_>, src: &Source) {
    for (v, line) in src.list("claims") {
        if s(v, "kind") != "requirement" {
            continue;
        }
        let id = s(v, "id");
        let req = Req {
            id: id.clone(),
            rf: s(v, "ref"),
            statement: s(v, "statement"),
            rationale: s(v, "rationale"),
            active: s(v, "lifecycle") != "retired",
            milestone: v.get("milestone").and_then(Value::as_u64).map(|m| m as u32),
            depends: str_list(v.get("dependsOn")),
            method: v
                .get("evaluate")
                .and_then(|e| e.get("method"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            area: s(v, "area"),
            cases: str_list(v.get("cases")),
            line,
        };
        cx.by_id.entry(id).or_insert(cx.reqs.len());
        cx.reqs.push(req);
    }
    for (v, line) in src.list("caseTables") {
        let body = s(v, "body");
        let Ok(t) = crate::cases::parse_cases(&body) else {
            continue;
        };
        let path = s(v, "path");
        cx.tables.push(Table {
            id: s(v, "id"),
            area: area_of(&path),
            path,
            columns: t.columns,
            rows: t.rows.into_iter().map(|r| r.cells).collect(),
            line,
        });
    }
    for key in [
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
    ] {
        for (v, _) in src.list(key) {
            let rf = s(v, "ref");
            if rf.is_empty() {
                continue;
            }
            if let Some(p) = rf.split('-').next() {
                cx.prefixes.insert(p.to_string());
            }
            cx.refs.entry(rf.to_uppercase()).or_insert(Ent {
                active: s(v, "lifecycle") != "retired",
                rf,
            });
        }
    }
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The area directory of a document path: the segment before `cases`, else the parent directory.
fn area_of(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if let Some(i) = parts.iter().rposition(|p| *p == "cases") {
        if i > 0 {
            return parts[i - 1].to_string();
        }
    }
    if parts.len() >= 2 {
        parts[parts.len() - 2].to_string()
    } else {
        String::new()
    }
}

impl Ctx<'_> {
    fn add(&mut self, rule: &'static str, r: &Rs, line: usize, entity: &str, msg: String) {
        self.out.push(F {
            rule,
            sev: r.sev,
            line,
            entity: entity.to_string(),
            msg,
        });
    }
}

// ---------------------------------------------------------------------------------------------
// Text helpers.

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// `KRN-002`-style ids in `text`: 1-5 upper-case letters, a dash, 1-3 digits, with no
/// identifier character before or after.
fn find_ids(text: &str) -> Vec<(usize, String)> {
    let b: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let (off, c) = b[i];
        let boundary = i == 0 || !is_id_char(b[i - 1].1);
        if boundary && c.is_ascii_uppercase() {
            let mut j = i;
            while j < b.len() && b[j].1.is_ascii_uppercase() {
                j += 1;
            }
            let letters = j - i;
            if (1..=5).contains(&letters) && j < b.len() && b[j].1 == '-' {
                let mut k = j + 1;
                while k < b.len() && b[k].1.is_ascii_digit() {
                    k += 1;
                }
                let digits = k - (j + 1);
                let after_ok = k >= b.len() || !is_id_char(b[k].1);
                if (1..=3).contains(&digits) && after_ok {
                    let end = if k < b.len() { b[k].0 } else { text.len() };
                    out.push((off, text[off..end].to_string()));
                    i = k;
                    continue;
                }
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }
    out
}

/// Lower case, every non-alphanumeric run as one space, padded with a space at both ends.
fn norm(s: &str) -> String {
    let mut out = String::from(" ");
    let mut gap = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for l in c.to_lowercase() {
                out.push(l);
            }
            gap = false;
        } else if !gap {
            out.push(' ');
            gap = true;
        }
    }
    if !gap {
        out.push(' ');
    }
    out
}

fn contains_phrase(norm_hay: &str, phrase: &str) -> bool {
    let p = norm(phrase);
    !p.trim().is_empty() && norm_hay.contains(&p)
}

/// Lines and sentences of `s`.
fn sentences(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for line in s.lines() {
        let mut start = 0;
        let b = line.as_bytes();
        for i in 0..b.len() {
            if b[i] == b'.' && b.get(i + 1) == Some(&b' ') {
                out.push(&line[start..=i]);
                start = i + 1;
            }
        }
        if start < line.len() {
            out.push(&line[start..]);
        }
    }
    out
}

const TIME_UNITS: [(&str, f64); 14] = [
    ("milliseconds", 1.0),
    ("millisecond", 1.0),
    ("ms", 1.0),
    ("seconds", 1000.0),
    ("second", 1000.0),
    ("minutes", 60_000.0),
    ("minute", 60_000.0),
    ("hours", 3_600_000.0),
    ("hour", 3_600_000.0),
    ("days", 86_400_000.0),
    ("day", 86_400_000.0),
    ("weeks", 604_800_000.0),
    ("week", 604_800_000.0),
    ("s", 1000.0),
];
const OTHER_UNITS: [&str; 14] = [
    "months", "month", "years", "year", "bytes", "byte", "kb", "mb", "gb", "kib", "mib", "percent",
    "entries", "attempts",
];

/// Numbers followed by a unit word: `(byte offset, value, unit)`.
fn numbers_with_unit(text: &str) -> Vec<(usize, f64, String)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit()
            && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'.'))
        {
            let st = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b',' || b[i] == b'.') {
                if (b[i] == b',' || b[i] == b'.') && !b.get(i + 1).is_some_and(u8::is_ascii_digit) {
                    break;
                }
                i += 1;
            }
            let num: String = text[st..i].chars().filter(|c| *c != ',').collect();
            let mut j = i;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            let ws = j;
            while j < b.len() && b[j].is_ascii_alphabetic() {
                j += 1;
            }
            let word = text[ws..j].to_lowercase();
            let known =
                TIME_UNITS.iter().any(|(u, _)| *u == word) || OTHER_UNITS.contains(&word.as_str());
            if known || b.get(i) == Some(&b'%') {
                if let Ok(v) = num.parse::<f64>() {
                    out.push((st, v, if known { word } else { "%".into() }));
                }
            }
            continue;
        }
        i += 1;
    }
    out
}

fn milestone_cell(cell: &str) -> Option<u32> {
    let d: String = cell
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    d.parse().ok()
}

// ---------------------------------------------------------------------------------------------
// S1 milestone order.

fn s1_milestone(cx: &mut Ctx<'_>, r: &Rs) {
    let mut found: Vec<(usize, String, String)> = Vec::new();
    for q in cx.reqs.iter().filter(|q| q.active) {
        let Some(m) = q.milestone else { continue };
        let mut seen: HashSet<String> = HashSet::new();
        for d in &q.depends {
            if let Some(&i) = cx.by_id.get(d) {
                let t = &cx.reqs[i];
                if t.active && t.milestone.is_some_and(|tm| tm > m) && seen.insert(t.rf.clone()) {
                    found.push((
                        q.line,
                        q.id.clone(),
                        format!(
                            "{} (milestone {m}) depends on {} (milestone {})",
                            q.rf,
                            t.rf,
                            t.milestone.unwrap_or(0)
                        ),
                    ));
                }
            }
        }
        for (_, id) in find_ids(&q.statement) {
            let Some(&i) = cx.by_id.get(&id.to_lowercase()) else {
                continue;
            };
            let t = &cx.reqs[i];
            if t.rf != q.rf
                && t.active
                && t.milestone.is_some_and(|tm| tm > m)
                && seen.insert(t.rf.clone())
            {
                found.push((
                    q.line,
                    q.id.clone(),
                    format!(
                        "{} (milestone {m}) cites {} (milestone {}) in its statement",
                        q.rf,
                        t.rf,
                        t.milestone.unwrap_or(0)
                    ),
                ));
            }
        }
        // A case row named in backticks in the statement, in one of the requirement's own tables.
        for tid in &q.cases {
            let Some(t) = cx.tables.iter().find(|t| &t.id == tid) else {
                continue;
            };
            let Some(mc) = t
                .columns
                .iter()
                .position(|c| c.to_lowercase().contains("milestone"))
            else {
                continue;
            };
            for row in &t.rows {
                let name = &row[0];
                if q.statement.contains(&format!("`{name}`")) {
                    if let Some(rm) = milestone_cell(&row[mc]).filter(|rm| *rm > m) {
                        found.push((
                            q.line,
                            q.id.clone(),
                            format!(
                                "{} (milestone {m}) cites case row `{name}` of {} (milestone {rm})",
                                q.rf, t.path
                            ),
                        ));
                    }
                }
            }
        }
    }
    for (line, id, msg) in found {
        cx.add("S1", r, line, &id, msg);
    }
}

// ---------------------------------------------------------------------------------------------
// S2 budget rows, S3 sub-budgets.

fn s2_budget_row(cx: &mut Ctx<'_>, r: &Rs) {
    let pats = if r.f.tables.is_empty() {
        vec!["budget-measurement".to_string()]
    } else {
        r.f.tables.clone()
    };
    let mut rows: HashSet<String> = HashSet::new();
    let mut any = false;
    for t in cx
        .tables
        .iter()
        .filter(|t| pats.iter().any(|p| t.path.contains(p.as_str())))
    {
        any = true;
        let col = t
            .columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case("requirement"))
            .unwrap_or(0);
        for row in &t.rows {
            rows.insert(row[col].trim().to_uppercase());
        }
    }
    if !any {
        return;
    }
    let mut found = Vec::new();
    for q in cx.reqs.iter().filter(|q| q.active && q.method == "budget") {
        if !rows.contains(&q.rf.to_uppercase()) {
            found.push((
                q.line,
                q.id.clone(),
                format!(
                    "{} is verified as a budget but has no row in the budget-measurement table",
                    q.rf
                ),
            ));
        }
    }
    for (line, id, msg) in found {
        cx.add("S2", r, line, &id, msg);
    }
}

/// The single upper bound of a statement in milliseconds with its percentile, when the statement
/// holds exactly one `within|at most|under|no more than|less than <number> <time unit>` and no
/// other number with a time unit.
fn single_bound(statement: &str) -> Option<(f64, Option<u32>)> {
    let nums: Vec<_> = numbers_with_unit(statement)
        .into_iter()
        .filter(|(_, _, u)| TIME_UNITS.iter().any(|(t, _)| t == u))
        .collect();
    if nums.len() != 1 {
        return None;
    }
    let (at, v, unit) = &nums[0];
    let before = statement[..*at].trim_end().to_lowercase();
    let bounded = [
        "within",
        "at most",
        "under",
        "no more than",
        "less than",
        "no later than",
    ]
    .iter()
    .any(|p| before.ends_with(p));
    if !bounded {
        return None;
    }
    let mult = TIME_UNITS.iter().find(|(t, _)| t == unit)?.1;
    // A percentile: `99th percentile`. Two different percentiles are not comparable.
    let low = statement.to_lowercase();
    let mut pct = None;
    if let Some(i) = low.find("th percentile") {
        let d: String = low[..i]
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        pct = d.parse().ok();
    }
    Some((v * mult, pct))
}

fn s3_sub_budget(cx: &mut Ctx<'_>, r: &Rs) {
    let markers: Vec<String> = if r.f.markers.is_empty() {
        [
            "together",
            "in total",
            "end to end",
            "end-to-end",
            "altogether",
            "combined",
            "included",
            "includes",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    } else {
        r.f.markers.clone()
    };
    let mut found = Vec::new();
    for q in cx.reqs.iter().filter(|q| q.active && q.method == "budget") {
        let low = q.statement.to_lowercase();
        if !markers.iter().any(|m| low.contains(&m.to_lowercase())) {
            continue;
        }
        let Some((parent, ppct)) = single_bound(&q.statement) else {
            continue;
        };
        let mut sum = 0.0;
        let mut names = Vec::new();
        let mut ok = true;
        for d in &q.depends {
            let Some(&i) = cx.by_id.get(d) else { continue };
            let c = &cx.reqs[i];
            if !c.active || c.method != "budget" {
                continue;
            }
            match single_bound(&c.statement) {
                Some((v, cpct)) if cpct == ppct => {
                    sum += v;
                    names.push(c.rf.clone());
                }
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && !names.is_empty() && sum > parent {
            found.push((
                q.line,
                q.id.clone(),
                format!(
                    "the sub-budgets of {} ({}) add up to {} ms, over its own bound of {} ms",
                    q.rf,
                    names.join(", "),
                    sum,
                    parent
                ),
            ));
        }
    }
    for (line, id, msg) in found {
        cx.add("S3", r, line, &id, msg);
    }
}

// ---------------------------------------------------------------------------------------------
// S4 constants.

fn s4_constants(cx: &mut Ctx<'_>, r: &Rs) {
    if r.f.names.is_empty() {
        return;
    }
    let pats = if r.f.tables.is_empty() {
        vec!["constants".to_string()]
    } else {
        r.f.tables.clone()
    };
    // Per table row: (area, normalised case name and Constant cell).
    let mut defined: Vec<(String, String)> = Vec::new();
    for t in cx
        .tables
        .iter()
        .filter(|t| pats.iter().any(|p| t.path.contains(p.as_str())))
    {
        let col = t
            .columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case("constant"))
            .unwrap_or(1.min(t.columns.len() - 1));
        for row in &t.rows {
            defined.push((t.area.clone(), norm(&format!("{} {}", row[0], row[col]))));
        }
    }
    let names = r.f.names.clone();
    let statements = r.f.statements.unwrap_or(true);
    let mut found = Vec::new();
    for q in cx.reqs.iter().filter(|q| q.active) {
        for name in &names {
            let has_row = |any_area: bool| {
                defined
                    .iter()
                    .any(|(a, c)| (any_area || *a == q.area) && contains_phrase(c, name))
            };
            // A value in the rationale needs a row in the area's own table.
            for sent in sentences(&q.rationale) {
                if contains_phrase(&norm(sent), name)
                    && !numbers_with_unit(sent).is_empty()
                    && !has_row(false)
                {
                    found.push((
                        q.line,
                        q.id.clone(),
                        format!(
                            "the rationale of {} states a value for `{name}`, which is not in the constants table of area {}",
                            q.rf, q.area
                        ),
                    ));
                    break;
                }
            }
            if statements && contains_phrase(&norm(&q.statement), name) && !has_row(true) {
                found.push((
                    q.line,
                    q.id.clone(),
                    format!(
                        "{} names the constant `{name}`, which no constants table defines",
                        q.rf
                    ),
                ));
            }
        }
    }
    for (line, id, msg) in found {
        cx.add("S4", r, line, &id, msg);
    }
}

// ---------------------------------------------------------------------------------------------
// S5 forbidden terms, S6 duplicate terms.

/// Lower case with every whitespace run as one space. Unlike [`norm`] it keeps punctuation, so
/// `sign-in session` and `sign in session` stay different phrases.
fn literal_norm(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether the phrase occurs in `hay` (from [`literal_norm`]) as whole words.
fn literal_contains(hay: &str, phrase: &str) -> bool {
    let p = literal_norm(phrase);
    if p.is_empty() {
        return false;
    }
    let word = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    let mut from = 0;
    while let Some(i) = hay[from..].find(&p) {
        let at = from + i;
        let end = at + p.len();
        let before = hay[..at].chars().next_back();
        let after = hay[end..].chars().next();
        if !before.is_some_and(word) && !after.is_some_and(word) {
            return true;
        }
        from = at + p.len().max(1);
    }
    false
}

fn s5_forbidden(cx: &mut Ctx<'_>, src: &Source, r: &Rs) {
    if r.f.forbidden.is_empty() {
        return;
    }
    let mut texts: Vec<(usize, String, String)> = Vec::new(); // line, entity, text
    for q in cx.reqs.iter().filter(|q| q.active) {
        texts.push((q.line, q.id.clone(), q.statement.clone()));
        texts.push((q.line, q.id.clone(), q.rationale.clone()));
    }
    for (v, line) in src.list("decisions") {
        texts.push((line, s(v, "id"), s(v, "statement")));
    }
    for (v, line) in src.list("terms") {
        texts.push((line, s(v, "id"), s(v, "definition")));
    }
    for t in &cx.tables {
        let body = t
            .rows
            .iter()
            .map(|r| r.join(" | "))
            .collect::<Vec<_>>()
            .join("\n");
        texts.push((t.line, t.id.clone(), body));
    }
    for (v, line) in src.list("sections") {
        texts.push((line, s(v, "id"), s(v, "body")));
    }
    let mut found = Vec::new();
    for (line, id, text) in texts {
        for sent in sentences(&text) {
            let n = literal_norm(sent);
            for f in &r.f.forbidden {
                if !literal_contains(&n, &f.phrase) {
                    continue;
                }
                // A sentence that names the replacement beside the phrase explains the ban.
                let low = sent.to_lowercase();
                if [
                    "formerly",
                    "replaces",
                    "replaced",
                    "instead of",
                    "deprecated",
                ]
                .iter()
                .any(|m| low.contains(m))
                    && literal_contains(&n, &f.replacement)
                {
                    continue;
                }
                found.push((
                    line,
                    id.clone(),
                    format!("`{}` is not used here; write `{}`", f.phrase, f.replacement),
                ));
            }
        }
    }
    found.sort();
    found.dedup();
    for (line, id, msg) in found {
        cx.add("S5", r, line, &id, msg);
    }
}

fn s6_duplicate_terms(cx: &mut Ctx<'_>, src: &Source, r: &Rs) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut found = Vec::new();
    for (v, line) in src.list("terms") {
        let term = s(v, "term");
        let mut key = norm(&term).trim().to_string();
        if key.len() > 3 {
            if let Some(k) = key.strip_suffix('s') {
                key = k.to_string();
            }
        }
        if key.is_empty() {
            continue;
        }
        match seen.get(&key) {
            Some(first) => found.push((
                line,
                s(v, "id"),
                format!("the term `{term}` is defined twice (first at line {first})"),
            )),
            None => {
                seen.insert(key, line);
            }
        }
    }
    for (line, id, msg) in found {
        cx.add("S6", r, line, &id, msg);
    }
}

// ---------------------------------------------------------------------------------------------
// S7 dead references.

fn s7_dead_refs(cx: &mut Ctx<'_>, src: &Source, r: &Rs) {
    let markers: Vec<String> = if r.f.markers.is_empty() {
        [
            "retired",
            "removed",
            "deleted",
            "replace",
            "supersed",
            "merged",
            "dropped",
            "withdrawn",
            "formerly",
            "no longer",
            "renumbered",
            "relaxes",
            "obsolete",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    } else {
        r.f.markers.clone()
    };
    let with_rationales = r.f.rationales.unwrap_or(false);
    let mut texts: Vec<(usize, String, String, String)> = Vec::new(); // line, entity, own ref, text
    for q in cx.reqs.iter().filter(|q| q.active) {
        texts.push((q.line, q.id.clone(), q.rf.clone(), q.statement.clone()));
        if with_rationales {
            texts.push((q.line, q.id.clone(), q.rf.clone(), q.rationale.clone()));
        }
    }
    for (v, line) in src.list("decisions") {
        texts.push((line, s(v, "id"), s(v, "ref"), s(v, "statement")));
    }
    // Active case rows: the rows of a table some active requirement cites.
    let cited: HashSet<&String> = cx
        .reqs
        .iter()
        .filter(|q| q.active)
        .flat_map(|q| q.cases.iter())
        .collect();
    for t in cx.tables.iter().filter(|t| cited.contains(&t.id)) {
        for row in &t.rows {
            texts.push((
                t.line,
                t.id.clone(),
                String::new(),
                format!("{} (row `{}` of {})", row.join(" | "), row[0], t.path),
            ));
        }
    }
    let mut found = Vec::new();
    for (line, id, own, text) in &texts {
        for sent in sentences(text) {
            let low = sent.to_lowercase();
            let history = markers.iter().any(|m| low.contains(&m.to_lowercase()));
            if history {
                continue;
            }
            for (_, tok) in find_ids(sent) {
                let key = tok.to_uppercase();
                if key == own.to_uppercase() {
                    continue;
                }
                let prefix = tok.split('-').next().unwrap_or("");
                match cx.refs.get(&key) {
                    Some(e) if !e.active => found.push((
                        *line,
                        id.clone(),
                        format!("{} cites {}, which is retired", name_of(id, own), e.rf),
                    )),
                    Some(_) => {}
                    None => {
                        if cx.prefixes.contains(prefix) {
                            found.push((
                                *line,
                                id.clone(),
                                format!("{} cites {tok}, which does not exist", name_of(id, own)),
                            ));
                        }
                    }
                }
            }
        }
    }
    found.sort();
    found.dedup();
    for (line, id, msg) in found {
        cx.add("S7", r, line, &id, msg);
    }
}

fn name_of(id: &str, own: &str) -> String {
    if own.is_empty() {
        id.to_string()
    } else {
        own.to_string()
    }
}

// ---------------------------------------------------------------------------------------------
// S8 said once.

const STOP: [&str; 38] = [
    "the", "a", "an", "of", "to", "be", "is", "are", "shall", "must", "not", "and", "or", "in",
    "on", "for", "that", "it", "its", "with", "by", "as", "at", "from", "every", "each", "any",
    "which", "this", "than", "when", "while", "then", "no", "same", "one", "only", "within",
];

fn stem(w: &str) -> String {
    for suf in ["ing", "ed", "es", "s"] {
        if w.len() > suf.len() + 3 {
            if let Some(b) = w.strip_suffix(suf) {
                return b.to_string();
            }
        }
    }
    w.to_string()
}

fn jaccard(a: &[u32], b: &[u32]) -> f64 {
    let (mut i, mut j, mut inter) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                inter += 1;
                i += 1;
                j += 1;
            }
        }
    }
    let union = a.len() + b.len() - inter;
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

/// Two statements that differ by one substituted word, or by different numbers, are parallel
/// rules (revenue / retention, milestone 1 / milestone 8), not one rule said twice.
fn is_parallel(a: &[u32], b: &[u32], names: &[String]) -> bool {
    let only = |x: &[u32], y: &[u32]| -> Vec<u32> {
        x.iter().copied().filter(|t| !y.contains(t)).collect()
    };
    let (ua, ub) = (only(a, b), only(b, a));
    let num = |u: &[u32]| {
        u.iter()
            .any(|t| names[*t as usize].chars().all(|c| c.is_ascii_digit()))
    };
    (ua.len() == 1 && ub.len() == 1) || (num(&ua) && num(&ub))
}

fn s8_said_once(cx: &mut Ctx<'_>, r: &Rs) {
    let threshold = r.f.threshold_percent.unwrap_or(75).min(100) as f64 / 100.0;
    let min_tokens = r.f.min_tokens.unwrap_or(6) as usize;
    let mut vocab: HashMap<String, u32> = HashMap::new();
    let mut sets: Vec<(usize, Vec<u32>)> = Vec::new(); // (req index, sorted token ids)
                                                       // R6's own measure, so a pair it already reports is not reported again.
    let mut r6: HashMap<usize, Vec<u32>> = HashMap::new();
    for (i, q) in cx.reqs.iter().enumerate().filter(|(_, q)| q.active) {
        let mut ids: Vec<u32> = Vec::new();
        let mut ids6: Vec<u32> = Vec::new();
        for (_, w) in crate::text::tokens(&q.statement) {
            if ![
                "the", "a", "an", "of", "to", "be", "is", "are", "shall", "must",
            ]
            .contains(&w.as_str())
            {
                let n = vocab.len() as u32;
                ids6.push(*vocab.entry(format!("r6:{w}")).or_insert(n));
            }
            if STOP.contains(&w.as_str()) {
                continue;
            }
            let n = vocab.len() as u32;
            ids.push(*vocab.entry(stem(&w)).or_insert(n));
        }
        ids.sort_unstable();
        ids.dedup();
        ids6.sort_unstable();
        ids6.dedup();
        if ids.len() >= min_tokens {
            sets.push((i, ids));
            r6.insert(i, ids6);
        }
    }
    let mut names = vec![String::new(); vocab.len()];
    for (w, i) in &vocab {
        names[*i as usize] = w.clone();
    }
    let mut found = Vec::new();
    for b in 1..sets.len() {
        let mut best: Option<(f64, usize)> = None;
        for a in 0..b {
            let sim = jaccard(&sets[a].1, &sets[b].1);
            if sim >= threshold && best.is_none_or(|(s, _)| sim > s) {
                let (ia, ib) = (sets[a].0, sets[b].0);
                if is_parallel(&sets[a].1, &sets[b].1, &names) {
                    continue;
                }
                if r6
                    .get(&ia)
                    .zip(r6.get(&ib))
                    .is_some_and(|(x, y)| y.len() >= 3 && jaccard(x, y) >= 0.8)
                {
                    continue;
                }
                best = Some((sim, a));
            }
        }
        if let Some((sim, a)) = best {
            let (ia, ib) = (sets[a].0, sets[b].0);
            found.push((
                cx.reqs[ib].line,
                cx.reqs[ib].id.clone(),
                format!(
                    "{} reads {:.0}% like {} (line {}); check that the rule is stated once",
                    cx.reqs[ib].rf,
                    sim * 100.0,
                    cx.reqs[ia].rf,
                    cx.reqs[ia].line
                ),
            ));
        }
    }
    for (line, id, msg) in found {
        cx.add("S8", r, line, &id, msg);
    }
}

// ---------------------------------------------------------------------------------------------
// S9 replacement character.

fn s9_replacement(cx: &mut Ctx<'_>, text: &str, r: &Rs) {
    let mut found = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let n = line.matches('\u{FFFD}').count() + line.to_lowercase().matches("\\ufffd").count();
        if n > 0 {
            found.push((i + 1, n));
        }
    }
    for (line, n) in found {
        cx.add(
            "S9",
            r,
            line,
            "",
            format!("{n} replacement character(s) U+FFFD: text was lost to a bad encoding"),
        );
    }
}

#[cfg(test)]
mod tests;
