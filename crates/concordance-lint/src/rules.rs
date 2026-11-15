//! The lint rules R1..R8.

use crate::text::{find_limit, find_word, mask_code, tokens};
use crate::whitepaper::{parse_index_table, parse_whitepaper, WhitePaperId};
use crate::{LintFinding, LintInput, Severity, SpecFile};
use concordance_core::doc::{ReferenceStrength, ReferenceTarget};
use concordance_core::RepoPath;
use concordance_graph::{Model, Resolved};
use concordance_markdown::{classify_code_span, is_strict_id, structure, RequirementBlock};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Banned in a statement: they are not outcomes (spec-format.md, rule 2).
pub const VAGUE_WORDS: &[&str] = &[
    "correctly",
    "properly",
    "efficient",
    "efficiently",
    "robust",
    "robustly",
    "appropriate",
    "appropriately",
    "as expected",
    "user-friendly",
    "fast",
    "secure",
    "securely",
];

/// Words that mark an implementation detail (rule 3). `[lint] internal_markers` adds to them.
pub const INTERNAL_MARKERS: &[&str] = &[
    "cache",
    "caches",
    "algorithm",
    "hashmap",
    "hash map",
    "btreemap",
    "mutex",
    "thread pool",
    "internal",
    "internals",
];

const VERIFY_METHODS: [&str; 6] = [
    "example",
    "property",
    "scenario",
    "budget",
    "proof",
    "conformance",
];
const MODALS_OUTSIDE: [&str; 5] = ["must", "shall", "never", "always", "required"];

struct Req<'a> {
    file: &'a SpecFile,
    block: RequirementBlock,
    retired: bool,
}

struct Sink(Vec<LintFinding>);

impl Sink {
    fn add(&mut self, rule: &'static str, severity: Severity, path: &str, line: u32, msg: String) {
        self.0.push(LintFinding {
            rule,
            severity,
            message: msg,
            path: path.to_string(),
            line,
        });
    }
    fn err(&mut self, rule: &'static str, path: &str, line: u32, msg: String) {
        self.add(rule, Severity::Error, path, line, msg);
    }
    fn warn(&mut self, rule: &'static str, path: &str, line: u32, msg: String) {
        self.add(rule, Severity::Warning, path, line, msg);
    }
}

pub fn run(input: &LintInput<'_>) -> Vec<LintFinding> {
    let mut sink = Sink(Vec::new());
    let (wp_path, wp_text) = input.whitepaper;
    if input.specs.is_empty() {
        sink.err(
            "R0",
            wp_path,
            1,
            "no normative specification files found under the spec roots; mark them in `[authority] normative`".into(),
        );
    }

    // White paper.
    let wp_ids: Vec<WhitePaperId> = match wp_text {
        Some(t) => parse_whitepaper(t),
        None => {
            sink.err(
                "R5",
                wp_path,
                1,
                "white paper not found; requirements cannot be traced (set `[lint] whitepaper`)"
                    .into(),
            );
            Vec::new()
        }
    };
    let mut seen_wp: HashMap<&str, u32> = HashMap::new();
    for id in &wp_ids {
        if let Some(first) = seen_wp.insert(id.id.as_str(), id.line) {
            sink.err(
                "R8",
                wp_path,
                id.line,
                format!(
                    "white-paper ID `{}` is defined twice (first at line {first})",
                    id.id
                ),
            );
        }
    }
    let wp_known: HashSet<&str> = wp_ids.iter().map(|i| i.id.as_str()).collect();

    // Requirements of every file.
    let mut reqs: Vec<Req> = Vec::new();
    for file in input.specs {
        let s = structure(&file.text);
        for b in s.requirements {
            let retired = b
                .attribute("Status")
                .map(|a| a.value.to_lowercase().starts_with("retired"))
                .unwrap_or(false);
            reqs.push(Req {
                file,
                block: b,
                retired,
            });
        }
        r7_loose(file, &s.loose, &mut sink);
    }
    let defined: HashSet<&str> = reqs.iter().map(|r| r.block.id.as_str()).collect();

    let rows = input
        .index
        .and_then(|i| parse_index_table(&i.text).map(|rows| (i, rows)));

    let mut first_seen: HashMap<String, (String, u32)> = HashMap::new();
    let mut traced: HashSet<String> = HashSet::new();
    let mut listed_checked: HashSet<&str> = HashSet::new();
    for r in &reqs {
        r8_ids(r, &rows, &mut first_seen, &mut listed_checked, &mut sink);
        if r.retired {
            continue;
        }
        r1_atomic(r, &mut sink);
        r2_vague(r, input, &mut sink);
        r3_internals(r, input, &mut sink);
        r4_provable(r, input, &mut sink);
        if input.settings.require_milestone {
            r4_milestone(r, &mut sink);
        }
        r5_traceable(
            r,
            wp_text.is_some(),
            &wp_known,
            &defined,
            wp_path,
            &mut traced,
            &mut sink,
        );
    }
    if wp_text.is_some() {
        // Goals and constraints are both promises: each needs a requirement that demonstrates it.
        for id in wp_ids
            .iter()
            .filter(|i| i.id.starts_with("G-") || i.id.starts_with("K-"))
        {
            if !traced.contains(&id.id) {
                let what = if id.id.starts_with("G-") {
                    "goal"
                } else {
                    "constraint"
                };
                sink.err(
                    "R5",
                    wp_path,
                    id.line,
                    format!("{what} `{}` has no requirement tracing to it", id.id),
                );
            }
        }
    }
    r6_duplicates(&reqs, &mut sink);
    sink.0
}

fn statement_masked(r: &Req) -> String {
    mask_code(&r.block.statement)
}

fn r1_atomic(r: &Req, sink: &mut Sink) {
    let s = statement_masked(r);
    let mut reasons: Vec<String> = Vec::new();
    let mut at: Option<usize> = None;
    let mut note = |off: usize, why: String, at: &mut Option<usize>| {
        reasons.push(why);
        if at.is_none_or(|a| off < a) {
            *at = Some(off);
        }
    };
    let mut modals: Vec<usize> = find_word(&s, "shall");
    modals.extend(find_word(&s, "must"));
    modals.sort_unstable();
    if modals.is_empty() {
        note(
            0,
            "the statement has no SHALL, SHALL NOT or MUST".into(),
            &mut at,
        );
    } else if modals.len() > 1 {
        note(
            modals[1],
            format!(
                "{} modal verbs in one statement (SHALL/MUST); split it into one requirement each",
                modals.len()
            ),
            &mut at,
        );
    }
    if let Some(i) = s.find(';') {
        note(i, "a semicolon joins clauses; split them".into(), &mut at);
    }
    // "and" followed by a second modal clause.
    let toks = tokens(&s);
    for (k, (off, w)) in toks.iter().enumerate() {
        if w == "and" && modals.len() < 2 {
            if let Some((_, next)) = toks.get(k + 1) {
                if matches!(next.as_str(), "never" | "always" | "will" | "should") {
                    note(
                        *off,
                        "\"and\" joins two modal clauses; split them".into(),
                        &mut at,
                    );
                    break;
                }
            }
        }
    }
    if let Some(off) = at {
        sink.err(
            "R1",
            &r.file.path,
            r.block.line_at(off),
            format!("{}: not atomic: {}", r.block.id, reasons.join("; ")),
        );
    }
}

fn r2_vague(r: &Req, input: &LintInput<'_>, sink: &mut Sink) {
    let s = statement_masked(r);
    let configured: Vec<&str>;
    let words: &[&str] = match &input.settings.vague_words {
        Some(w) => {
            configured = w.iter().map(String::as_str).collect();
            &configured
        }
        None => VAGUE_WORDS,
    };
    for w in words {
        if let Some(&off) = find_word(&s, w).first() {
            sink.err(
                "R2",
                &r.file.path,
                r.block.line_at(off),
                format!(
                    "{}: \"{w}\" is not an outcome; state what a caller can observe (a budget for \"fast\", an adversary for \"secure\")",
                    r.block.id
                ),
            );
        }
    }
}

/// Backticked texts of the Boundary attribute: the exact surface.
fn boundary_surface(r: &Req) -> Vec<String> {
    r.block
        .attribute("Boundary")
        .map(|a| a.codes.iter().map(|c| c.text.to_lowercase()).collect())
        .unwrap_or_default()
}

fn on_surface(span: &str, surface: &[String]) -> bool {
    let span = span.to_lowercase();
    let span = span.trim_end_matches("()");
    surface.iter().any(|s| {
        let s = s.trim_end_matches("()");
        s == span || !find_word(s, span).is_empty() || !find_word(span, s).is_empty()
    })
}

/// How `model` classifies the symbol a reference names: `Some(None)` when it resolves to
/// public API, `Some(Some(why))` when it resolves only to internals, `None` when unknown.
fn internal_reason(model: &Model, path: &[String]) -> Option<String> {
    let describe = |id: &concordance_core::SymbolId| -> Option<(bool, String)> {
        let sym = model.symbols.get(id)?;
        let file = model
            .files
            .get(&sym.file)
            .map(|f| f.path.as_str().to_string())
            .unwrap_or_default();
        Some((
            sym.is_public_api,
            format!(
                "`{}` is {:?} ({}:{})",
                sym.qualified_name(),
                sym.visibility,
                file,
                sym.range.start_line
            ),
        ))
    };
    match model.resolve_path(path, None) {
        Resolved::Exact(id) | Resolved::UniqueSuffix(id) => {
            let (public, what) = describe(&id)?;
            (!public).then_some(what)
        }
        Resolved::Ambiguous(ids) => {
            let all: Vec<(bool, String)> = ids.iter().filter_map(describe).collect();
            if !all.is_empty() && all.iter().all(|(p, _)| !p) {
                Some(all[0].1.clone())
            } else {
                None
            }
        }
        _ => None,
    }
}

fn r3_internals(r: &Req, input: &LintInput<'_>, sink: &mut Sink) {
    let surface = boundary_surface(r);
    let mut done: HashSet<&str> = HashSet::new();
    for span in &r.block.codes {
        if !done.insert(span.text.as_str()) {
            continue;
        }
        let Some((target, strength)) = classify_code_span(&span.text) else {
            continue;
        };
        match target {
            ReferenceTarget::Symbol { path } => {
                if let Some(why) = input.model.and_then(|m| internal_reason(m, &path)) {
                    sink.err(
                        "R3",
                        &r.file.path,
                        span.line,
                        format!(
                            "{}: `{}` is not public API: {why}; state the requirement at the boundary",
                            r.block.id, span.text
                        ),
                    );
                    continue;
                }
                if !surface.is_empty() && !on_surface(&span.text, &surface) {
                    let weak = if strength == ReferenceStrength::Weak {
                        " (identifier)"
                    } else {
                        ""
                    };
                    sink.warn(
                        "R3",
                        &r.file.path,
                        span.line,
                        format!(
                            "{}: `{}`{weak} is not part of the Boundary surface; the statement may name only what is visible there",
                            r.block.id, span.text
                        ),
                    );
                }
            }
            ReferenceTarget::File { path, .. }
                if input.model.is_some_and(|m| m.file_by_path(&path).is_some()) =>
            {
                sink.warn(
                    "R3",
                    &r.file.path,
                    span.line,
                    format!(
                        "{}: `{}` is a source file, an internal; name the boundary surface instead",
                        r.block.id, span.text
                    ),
                );
            }
            _ => {}
        }
    }
    let s = statement_masked(r);
    let mut markers: Vec<String> = INTERNAL_MARKERS.iter().map(|m| m.to_string()).collect();
    markers.extend(input.settings.internal_markers.iter().cloned());
    for m in markers {
        if let Some(&off) = find_word(&s, &m).first() {
            sink.warn(
                "R3",
                &r.file.path,
                r.block.line_at(off),
                format!(
                    "{}: \"{m}\" names an internal detail; those belong in docs/design",
                    r.block.id
                ),
            );
        }
    }
}

fn first_word(s: &str) -> String {
    s.trim()
        .trim_start_matches('`')
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or("")
        .to_lowercase()
}

fn r4_provable(r: &Req, input: &LintInput<'_>, sink: &mut Sink) {
    let path = &r.file.path;
    let id = &r.block.id;
    let Some(attr) = r.block.attribute("Verify") else {
        sink.err(
            "R4",
            path,
            r.block.line,
            format!(
                "{id}: missing `Verify:` (one of {})",
                VERIFY_METHODS.join(", ")
            ),
        );
        return;
    };
    let method = first_word(&attr.value);
    if !VERIFY_METHODS.contains(&method.as_str()) {
        sink.err(
            "R4",
            path,
            attr.line,
            format!(
                "{id}: `Verify: {}` is not a method; use one of {}",
                attr.value,
                VERIFY_METHODS.join(", ")
            ),
        );
        return;
    }
    r4_cases(r, &method, input, sink);
    let s = statement_masked(r);
    if matches!(method.as_str(), "example" | "scenario") {
        let mut hit: Option<(usize, &str)> = None;
        for w in ["never", "always", "for every", "for all", "for each"] {
            if let Some(&off) = find_word(&s, w).first() {
                if hit.is_none_or(|h| off < h.0) {
                    hit = Some((off, w));
                }
            }
        }
        if let Some((_, w)) = hit {
            sink.err(
                "R4",
                path,
                attr.line,
                format!("{id}: \"{w}\" cannot be shown by {method}; use `property` or `proof`"),
            );
        }
    }
    if method != "budget" {
        if let Some((_, phrase)) = find_limit(&s) {
            sink.err(
                "R4",
                path,
                attr.line,
                format!("{id}: the limit \"{phrase}\" needs `Verify: budget`, not {method}"),
            );
        }
    }
}

/// With `[lint] require_milestone`: `Milestone:` is present and a positive integer; `Seam:`, when
/// present, is a positive integer no greater than the milestone.
fn r4_milestone(r: &Req, sink: &mut Sink) {
    use concordance_core::doc::parse_positive;
    let path = &r.file.path;
    let id = &r.block.id;
    let milestone = match r.block.attribute("Milestone") {
        None => {
            sink.err(
                "R4",
                path,
                r.block.line,
                format!("{id}: missing `Milestone:` (a positive integer)"),
            );
            None
        }
        Some(a) => {
            let n = parse_positive(&a.value);
            if n.is_none() {
                sink.err(
                    "R4",
                    path,
                    a.line,
                    format!("{id}: `Milestone: {}` is not a positive integer", a.value),
                );
            }
            n
        }
    };
    if let Some(a) = r.block.attribute("Seam") {
        match parse_positive(&a.value) {
            None => sink.err(
                "R4",
                path,
                a.line,
                format!("{id}: `Seam: {}` is not a positive integer", a.value),
            ),
            Some(seam) => {
                if milestone.is_some_and(|m| seam > m) {
                    sink.err(
                        "R4",
                        path,
                        a.line,
                        format!(
                            "{id}: `Seam: {seam}` is greater than `Milestone: {}`",
                            milestone.unwrap_or_default()
                        ),
                    );
                }
            }
        }
    }
}

/// `Verify: conformance` needs `Cases:` files, each holding a `## Cases` table; `Verify: proof`
/// may list them (optional); `Cases:` is meaningless for any other method.
fn r4_cases(r: &Req, method: &str, input: &LintInput<'_>, sink: &mut Sink) {
    let path = &r.file.path;
    let id = &r.block.id;
    let cases = r.block.attribute("Cases");
    if !matches!(method, "conformance" | "proof") {
        if let Some(c) = cases {
            sink.err(
                "R4",
                path,
                c.line,
                format!(
                    "{id}: `Cases:` is only for `Verify: conformance` or `Verify: proof`, not {method}"
                ),
            );
        }
        return;
    }
    let Some(c) = cases else {
        if method == "proof" {
            return;
        }
        sink.err(
            "R4",
            path,
            r.block.line,
            format!("{id}: `Verify: conformance` needs a `Cases:` list of case-table files"),
        );
        return;
    };
    let files: Vec<&str> = c
        .value
        .split(',')
        .map(|f| f.trim().trim_matches('`').trim())
        .filter(|f| !f.is_empty())
        .collect();
    if files.is_empty() {
        sink.err("R4", path, c.line, format!("{id}: `Cases:` names no file"));
        return;
    }
    let Some(root) = input.repo_root else {
        return;
    };
    for f in files {
        match std::fs::read_to_string(root.join(f)) {
            Err(_) => sink.err(
                "R4",
                path,
                c.line,
                format!("{id}: `Cases:` file `{f}` does not exist"),
            ),
            Ok(text) => {
                if let Err(e) = crate::cases::parse_cases(&text) {
                    sink.err(
                        "R4",
                        path,
                        c.line,
                        format!("{id}: `Cases:` file `{f}`: {e}"),
                    );
                }
            }
        }
    }
}

/// Tokens of `value` shaped like `LETTERS-digits`.
fn id_tokens(value: &str) -> Vec<String> {
    value
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|t| {
            t.split_once('-').is_some_and(|(p, n)| {
                !p.is_empty()
                    && p.bytes().all(|b| b.is_ascii_alphabetic())
                    && !n.is_empty()
                    && n.bytes().all(|b| b.is_ascii_digit())
            })
        })
        .map(str::to_string)
        .collect()
}

fn r5_traceable(
    r: &Req,
    have_whitepaper: bool,
    known: &HashSet<&str>,
    defined: &HashSet<&str>,
    wp_path: &str,
    traced: &mut HashSet<String>,
    sink: &mut Sink,
) {
    let path = &r.file.path;
    let id = &r.block.id;
    match r.block.attribute("Traces") {
        None => sink.err(
            "R5",
            path,
            r.block.line,
            format!("{id}: missing `Traces:` (a white-paper goal or constraint)"),
        ),
        Some(a) => {
            let toks = id_tokens(&a.value);
            if toks.is_empty() {
                sink.err(
                    "R5",
                    path,
                    a.line,
                    format!("{id}: `Traces:` names no white-paper ID"),
                );
            }
            for t in toks {
                if have_whitepaper && !known.contains(t.as_str()) {
                    sink.err(
                        "R5",
                        path,
                        a.line,
                        format!("{id}: `Traces:` names `{t}`, which {wp_path} does not define"),
                    );
                }
                traced.insert(t);
            }
        }
    }
    match r.block.attribute("Boundary") {
        None => sink.err(
            "R5",
            path,
            r.block.line,
            format!("{id}: missing `Boundary:` (a B- ID and the exact surface)"),
        ),
        Some(a) => {
            let b: Vec<String> = id_tokens(&a.value)
                .into_iter()
                .filter(|t| t.starts_with("B-"))
                .collect();
            if b.is_empty() {
                sink.err(
                    "R5",
                    path,
                    a.line,
                    format!("{id}: `Boundary:` names no B- ID"),
                );
            }
            for t in &b {
                if have_whitepaper && !known.contains(t.as_str()) {
                    sink.err(
                        "R5",
                        path,
                        a.line,
                        format!("{id}: `Boundary:` names `{t}`, which {wp_path} does not define"),
                    );
                }
            }
            if !b.is_empty() && a.codes.is_empty() {
                sink.warn(
                    "R5",
                    path,
                    a.line,
                    format!("{id}: `Boundary:` gives no exact surface in backticks"),
                );
            }
        }
    }
    if let Some(a) = r.block.attribute("Depends") {
        for t in a
            .value
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.'))
            .filter(|t| concordance_core::doc::is_requirement_id(t))
        {
            if !defined.contains(t) {
                sink.warn(
                    "R5",
                    path,
                    a.line,
                    format!("{id}: `Depends:` names `{t}`, which no specification defines"),
                );
            }
        }
    }
}

fn r6_duplicates(reqs: &[Req], sink: &mut Sink) {
    const STOP: [&str; 10] = [
        "the", "a", "an", "of", "to", "be", "is", "are", "shall", "must",
    ];
    let sets: Vec<Option<BTreeSet<String>>> = reqs
        .iter()
        .map(|r| {
            if r.retired {
                return None;
            }
            let set: BTreeSet<String> = tokens(&r.block.statement)
                .into_iter()
                .map(|(_, w)| w)
                .filter(|w| !STOP.contains(&w.as_str()))
                .collect();
            (set.len() >= 3).then_some(set)
        })
        .collect();
    for j in 0..reqs.len() {
        let Some(b) = &sets[j] else { continue };
        for i in 0..j {
            let Some(a) = &sets[i] else { continue };
            let inter = a.intersection(b).count();
            let union = a.union(b).count();
            if union > 0 && inter as f64 / union as f64 >= 0.8 {
                sink.warn(
                    "R6",
                    &reqs[j].file.path,
                    reqs[j].block.line,
                    format!(
                        "{} restates {} ({}:{}); state a rule once and refer to it by ID",
                        reqs[j].block.id, reqs[i].block.id, reqs[i].file.path, reqs[i].block.line
                    ),
                );
                break;
            }
        }
    }
}

fn r7_loose(file: &SpecFile, loose: &[concordance_markdown::LooseText], sink: &mut Sink) {
    for l in loose {
        let mut hits: Vec<(usize, &str)> = Vec::new();
        for w in MODALS_OUTSIDE {
            if let Some(&off) = find_word(&l.text, w).first() {
                hits.push((off, w));
            }
        }
        hits.sort_unstable();
        if let Some(&(off, _)) = hits.first() {
            let words: Vec<&str> = hits.iter().map(|h| h.1).collect();
            sink.err(
                "R7",
                &file.path,
                l.line_at(off),
                format!(
                    "\"{}\" outside a requirement; make it a requirement or reword it as context",
                    words.join("\", \"")
                ),
            );
        }
    }
}

fn r8_ids<'a>(
    r: &Req<'a>,
    rows: &Option<(&SpecFile, Vec<crate::whitepaper::IndexRow>)>,
    first_seen: &mut HashMap<String, (String, u32)>,
    listed_checked: &mut HashSet<&'a str>,
    sink: &mut Sink,
) {
    let path = &r.file.path;
    let id = r.block.id.as_str();
    let well_formed = is_strict_id(id);
    if !well_formed {
        sink.err(
            "R8",
            path,
            r.block.line,
            format!("malformed ID `{id}`: use an upper-case prefix, a dash and three digits (`CLI-004`)"),
        );
    }
    match first_seen.get(id) {
        Some((p, l)) => sink.err(
            "R8",
            path,
            r.block.line,
            format!("duplicate ID `{id}` (first defined at {p}:{l}); IDs are never reused"),
        ),
        None => {
            first_seen.insert(id.to_string(), (path.clone(), r.block.line));
        }
    }
    let Some((index, rows)) = rows else { return };
    if !well_formed || index.path == r.file.path {
        return;
    }
    let dir_base = RepoPath::from_relative_str(&index.path);
    let row = rows.iter().find(|row| {
        let resolved = dir_base
            .as_ref()
            .and_then(|b| b.join_relative(&row.file))
            .map(|p| p.as_str().to_string());
        resolved.as_deref() == Some(path.as_str())
            || row.file.rsplit('/').next() == path.rsplit('/').next()
    });
    let Some(row) = row else {
        if listed_checked.insert(r.file.path.as_str()) {
            sink.err(
                "R8",
                path,
                r.block.line,
                format!(
                    "this file holds requirements but {} does not list it",
                    index.path
                ),
            );
        }
        return;
    };
    let prefix = id.rsplit_once('-').map(|p| p.0).unwrap_or(id);
    if !row.prefixes.iter().any(|p| p == prefix) {
        let owns = if row.prefixes.is_empty() {
            "no prefix".to_string()
        } else {
            format!("`{}`", row.prefixes.join("`, `"))
        };
        sink.err(
            "R8",
            path,
            r.block.line,
            format!(
                "prefix `{prefix}` of `{id}` is not owned by this file ({} gives it {owns})",
                index.path
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concordance_core::config::LintConfig;

    fn settings(require_milestone: bool) -> LintConfig {
        LintConfig {
            whitepaper: "docs/whitepaper.md".into(),
            strict: false,
            require_milestone,
            vague_words: None,
            internal_markers: Vec::new(),
            spec: Vec::new(),
        }
    }

    /// The `(line, message)` of every R4 finding for a one-requirement spec whose attributes
    /// are `attrs`.
    fn r4(require_milestone: bool, attrs: &str) -> Vec<(u32, String)> {
        let text = format!(
            "**CLI-004** `tool build` SHALL exit with status 2.\n- Verify: scenario\n{attrs}"
        );
        let specs = [SpecFile {
            path: "docs/spec/cli.md".into(),
            text,
        }];
        let settings = settings(require_milestone);
        let input = LintInput {
            specs: &specs,
            index: None,
            whitepaper: ("docs/whitepaper.md", None),
            model: None,
            settings: &settings,
            repo_root: None,
        };
        run(&input)
            .into_iter()
            .filter(|f| f.rule == "R4")
            .map(|f| (f.line, f.message))
            .collect()
    }

    #[test]
    fn a_seam_greater_than_the_milestone_is_an_error() {
        let f = r4(true, "- Milestone: 2\n- Seam: 3\n");
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].0, 4);
        assert!(f[0].1.contains("greater than"), "{f:?}");
    }

    /// R4 findings for a requirement with `Verify: {verify}` and a `Cases:` line, with the case
    /// files resolved under `root`.
    fn r4_cases_of(verify: &str, cases: &str, root: Option<&std::path::Path>) -> Vec<String> {
        let text = format!(
            "**CLI-004** `tool build` SHALL exit with status 2.
- Verify: {verify}
{cases}"
        );
        let specs = [SpecFile {
            path: "docs/spec/cli.md".into(),
            text,
        }];
        let settings = settings(false);
        let input = LintInput {
            specs: &specs,
            index: None,
            whitepaper: ("docs/whitepaper.md", None),
            model: None,
            settings: &settings,
            repo_root: root,
        };
        run(&input)
            .into_iter()
            .filter(|f| f.rule == "R4")
            .map(|f| f.message)
            .collect()
    }

    #[test]
    fn cases_are_accepted_on_a_proof_and_checked_like_conformance() {
        let dir = std::env::temp_dir().join(format!("concordance-r4-proof-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("good.md"),
            "## Cases

| Case | Theorem |
|---|---|
| a | t1 |
",
        )
        .unwrap();
        std::fs::write(
            dir.join("bad.md"),
            "no table here
",
        )
        .unwrap();
        assert!(r4_cases_of(
            "proof",
            "- Cases: good.md
",
            Some(&dir)
        )
        .is_empty());
        // Optional on a proof.
        assert!(r4_cases_of("proof", "", Some(&dir)).is_empty());
        let missing = r4_cases_of(
            "proof",
            "- Cases: nope.md
",
            Some(&dir),
        );
        assert!(missing[0].contains("does not exist"), "{missing:?}");
        let bad = r4_cases_of(
            "proof",
            "- Cases: bad.md
",
            Some(&dir),
        );
        assert_eq!(bad.len(), 1, "{bad:?}");
        // Still required for conformance.
        let req = r4_cases_of("conformance", "", Some(&dir));
        assert!(req[0].contains("needs a `Cases:`"), "{req:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
