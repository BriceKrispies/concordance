//! `[ALIAS:path:lines]` citations of files in other git repositories.
//!
//! Each checked citation yields up to three evidence records:
//! * `CitationPath`: the path exists at the cited revision (`Proven` support), resolves only
//!   as a unique prefix (`Strong` support, with a note), is ambiguous (inconclusive) or is
//!   missing (`Proven` contradiction).
//! * `CitationLines`: a line range past the end of the file, a line range on a directory, or
//!   a section that is not a heading of the file (`Proven` contradiction). Absent when fine.
//! * `CitationDrift`: the cited text differs at the `compare` revision. Inconclusive: the
//!   citation is still true at its own revision. Text that only moved is recorded as support.
//!
//! Citations whose alias has no configured repository produce no evidence.

use crate::text::{bounded, EvidenceBuilder};
use concordance_core::doc::{LineSpan, Reference, ReferenceTarget};
use concordance_core::evidence::{
    Citation, Dependency, Evidence, Method, Stance, Strength, StructuralCheck,
};
use concordance_core::graph::NodeRef;
use concordance_core::EvidenceId;
use std::sync::Arc;

/// A configured repository, with its revisions resolved to commits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CitedRepo {
    pub alias: String,
    /// The cited revision as configured, and the commit it resolves to.
    pub cited: String,
    pub cited_commit: String,
    /// The compare revision, when configured and resolvable.
    pub compare: Option<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rev {
    Cited,
    Compare,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    File { blob: String },
    Dir,
    Missing,
}

/// Read-only access to the cited repositories at fixed revisions. The engine implements
/// it over the git CLI; it never reads a working tree.
pub trait CitedRepos: Send + Sync {
    /// The repository a written alias refers to (`APP`, or a configured spelling).
    fn repo(&self, alias: &str) -> Option<CitedRepo>;
    fn entry(&self, repo: &CitedRepo, rev: Rev, path: &str) -> Entry;
    /// Files and directories directly inside `dir` (`""` = the root), as full paths.
    fn entries_in(&self, repo: &CitedRepo, rev: Rev, dir: &str) -> Vec<String>;
    /// Text of a file (lossy UTF-8, line endings as stored).
    fn text(&self, repo: &CitedRepo, rev: Rev, path: &str) -> Option<Arc<str>>;
    /// Files and directories matching a glob (`*` within a segment, `**` across them),
    /// at most `limit`.
    fn glob(&self, repo: &CitedRepo, rev: Rev, pattern: &str, limit: usize) -> Vec<String>;
    /// Where `path` (at the cited revision) went at the compare revision, if git
    /// detected a rename.
    fn renamed(&self, repo: &CitedRepo, path: &str) -> Option<String>;
}

pub fn is_citation_check(e: &Evidence) -> bool {
    matches!(
        e.method,
        Method::Structural {
            check: StructuralCheck::CitationPath
                | StructuralCheck::CitationLines
                | StructuralCheck::CitationDrift
        }
    )
}

fn method(check: StructuralCheck) -> Method {
    Method::Structural { check }
}

fn key(text: &str) -> String {
    format!("cite:{text}")
}

/// Id of the evidence `check` produces for the citation `text` about `subject`.
pub fn evidence_id(subject: &NodeRef, check: StructuralCheck, text: &str) -> EvidenceId {
    let method_key = serde_json::to_string(&method(check)).unwrap_or_default();
    EvidenceId::from_key(&[subject.as_str(), &method_key, &key(text)])
}

fn lines_of(text: &str) -> Vec<&str> {
    text.lines().collect()
}

fn spans_text(spans: &[LineSpan]) -> String {
    spans
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// Whether the entry `name` is abbreviated by the written segment `seg`: `0005` for
/// `0005-foo.md`, or an explicit `0003-...` / `0003-…`.
fn abbreviates(seg: &str, name: &str) -> bool {
    let explicit = seg.strip_suffix("...").or_else(|| seg.strip_suffix('…'));
    let prefix = explicit.unwrap_or(seg);
    if prefix.is_empty() || name == prefix || (explicit.is_none() && seg.contains('.')) {
        return false;
    }
    match name.strip_prefix(prefix) {
        Some(rest) => {
            !prefix.ends_with(|c: char| c.is_alphanumeric())
                || rest.starts_with(|c: char| !c.is_alphanumeric())
        }
        None => false,
    }
}

/// Paths at the cited revision that `path` abbreviates, segment by segment
/// (`docs/adr/0005` -> `docs/adr/0005-foo.md`). At most a few are returned.
fn prefix_candidates(repos: &dyn CitedRepos, repo: &CitedRepo, path: &str) -> Vec<String> {
    let mut current: Vec<String> = vec![String::new()];
    for seg in path.split('/') {
        let mut next = Vec::new();
        for dir in &current {
            let full = if dir.is_empty() {
                seg.to_string()
            } else {
                format!("{dir}/{seg}")
            };
            if repos.entry(repo, Rev::Cited, &full) != Entry::Missing {
                next.push(full);
                continue;
            }
            next.extend(
                repos
                    .entries_in(repo, Rev::Cited, dir)
                    .into_iter()
                    .filter(|e| abbreviates(seg, e.rsplit('/').next().unwrap_or(e))),
            );
        }
        next.truncate(8);
        if next.is_empty() {
            return next;
        }
        current = next;
    }
    current
}

fn is_markdown(path: &str) -> bool {
    let l = path.to_ascii_lowercase();
    l.ends_with(".md") || l.ends_with(".mdx") || l.ends_with(".markdown")
}

/// Whether a Markdown text has a heading matching `section` by text or anchor slug.
fn has_heading(text: &str, section: &str) -> bool {
    let want = section.trim().to_lowercase();
    let want_slug = concordance_core::anchor::slugify(&want);
    let mut fence = false;
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            continue;
        }
        if fence || !t.starts_with('#') {
            continue;
        }
        let h = t
            .trim_start_matches('#')
            .trim()
            .trim_end_matches('#')
            .trim();
        let h = h.to_lowercase();
        if h == want || concordance_core::anchor::slugify(&h) == want_slug || h.contains(&want) {
            return true;
        }
    }
    false
}

/// Where the block `needle` occurs in `hay` (0-based line index), preferring `near`.
fn find_block(hay: &[&str], needle: &[&str], near: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    let eq = |a: &str, b: &str| a.trim_end() == b.trim_end();
    let mut best: Option<usize> = None;
    for i in 0..=hay.len() - needle.len() {
        if needle.iter().zip(&hay[i..]).all(|(a, b)| eq(a, b))
            && best.is_none_or(|b| i.abs_diff(near) < b.abs_diff(near))
        {
            best = Some(i);
        }
    }
    best
}

enum Drift {
    Same,
    Moved(String),
    Changed(String),
}

/// Compare the cited spans of `path` (cited) with `now_path` (compare).
fn drift_of(
    repos: &dyn CitedRepos,
    repo: &CitedRepo,
    path: &str,
    now_path: &str,
    spans: &[LineSpan],
    renamed: bool,
) -> Drift {
    let compare = repo.compare.as_ref().map(|c| c.0.as_str()).unwrap_or("");
    let moved_note = if renamed {
        format!(" (renamed to `{now_path}`)")
    } else {
        String::new()
    };
    let (Some(old), Some(new)) = (
        repos.text(repo, Rev::Cited, path),
        repos.text(repo, Rev::Compare, now_path),
    ) else {
        return Drift::Same;
    };
    if spans.is_empty() {
        return if old == new {
            if renamed {
                Drift::Moved(format!("`{path}` is unchanged at {compare}{moved_note}."))
            } else {
                Drift::Same
            }
        } else {
            Drift::Changed(format!(
                "`{path}` changed between {} and {compare}{moved_note}.",
                repo.cited
            ))
        };
    }
    let (o, n) = (lines_of(&old), lines_of(&new));
    let mut changed = Vec::new();
    let mut moved = Vec::new();
    for s in spans {
        let (a, b) = (s.start as usize, s.end as usize);
        if a == 0 || b < a || b > o.len() {
            continue; // reported by CitationLines
        }
        let block = &o[a - 1..b];
        match find_block(&n, block, a - 1) {
            Some(i) if i == a - 1 => {}
            Some(i) => moved.push(format!("{s} -> {}-{}", i + 1, i + block.len())),
            None => changed.push(s.to_string()),
        }
    }
    if !changed.is_empty() {
        Drift::Changed(format!(
            "Cited lines {} of `{path}` changed between {} and {compare}{moved_note}.",
            changed.join(","),
            repo.cited
        ))
    } else if !moved.is_empty() || renamed {
        let at = if moved.is_empty() {
            String::new()
        } else {
            format!(", now at {}", moved.join(", "))
        };
        Drift::Moved(format!(
            "Cited lines of `{path}` are unchanged at {compare}{moved_note}{at}."
        ))
    } else {
        Drift::Same
    }
}

/// Evidence for one citation reference. `spec` is the specification citation (its exact
/// location); `deps` are the subject's dependencies.
pub fn citation_evidence(
    repos: &dyn CitedRepos,
    builder: &EvidenceBuilder<'_>,
    r: &Reference,
    spec: Vec<Citation>,
    deps: &[Dependency],
) -> Vec<Evidence> {
    let ReferenceTarget::Citation {
        alias,
        path,
        lines,
        section,
    } = &r.target
    else {
        return Vec::new();
    };
    let Some(repo) = repos.repo(alias) else {
        return Vec::new();
    };
    let at = format!("{}@{}", repo.alias, repo.cited);
    let k = key(&r.text);
    if path.contains('*') {
        // A glob is checked for at least one match; its lines and drift are not.
        let n = repos.glob(&repo, Rev::Cited, path, 2).len();
        let (stance, summary) = if n == 0 {
            (
                Stance::Contradicts,
                format!("No path at {at} matches `{path}`."),
            )
        } else {
            (Stance::Supports, format!("`{path}` matches paths at {at}."))
        };
        let b = EvidenceBuilder {
            ctx: builder.ctx,
            subject: builder.subject.clone(),
            method: method(StructuralCheck::CitationPath),
        };
        return vec![b.build(&k, stance, Strength::Proven, summary, spec, deps.to_vec())];
    }
    let b = |check| EvidenceBuilder {
        ctx: builder.ctx,
        subject: builder.subject.clone(),
        method: method(check),
    };
    let mut out = Vec::new();
    let build = |check, stance, strength, summary: String| {
        b(check).build(
            &k,
            stance,
            strength,
            bounded(&summary),
            spec.clone(),
            deps.to_vec(),
        )
    };

    // Path at the cited revision.
    let (resolved, kind) = match repos.entry(&repo, Rev::Cited, path) {
        Entry::Missing => {
            let cands = prefix_candidates(repos, &repo, path);
            match cands.len() {
                1 => {
                    let p = cands[0].clone();
                    out.push(build(
                        StructuralCheck::CitationPath,
                        Stance::Supports,
                        Strength::Strong,
                        format!(
                            "`{path}` is not a path at {at}; it resolves by prefix to the only match, `{p}`."
                        ),
                    ));
                    let kind = repos.entry(&repo, Rev::Cited, &p);
                    (p, kind)
                }
                0 => {
                    let later = repo
                        .compare
                        .as_ref()
                        .filter(|_| repos.entry(&repo, Rev::Compare, path) != Entry::Missing)
                        .map(|c| {
                            format!(
                                " It exists at {}, so the citation may name the wrong revision.",
                                c.0
                            )
                        })
                        .unwrap_or_default();
                    out.push(build(
                        StructuralCheck::CitationPath,
                        Stance::Contradicts,
                        Strength::Proven,
                        format!("`{path}` does not exist at {at}.{later}"),
                    ));
                    return out;
                }
                n => {
                    let shown: Vec<String> =
                        cands.iter().take(4).map(|c| format!("`{c}`")).collect();
                    out.push(build(
                        StructuralCheck::CitationPath,
                        Stance::Inconclusive,
                        Strength::Weak,
                        format!(
                            "`{path}` is not a path at {at}, and {n} entries start with it: {}.",
                            shown.join(", ")
                        ),
                    ));
                    return out;
                }
            }
        }
        e => {
            out.push(build(
                StructuralCheck::CitationPath,
                Stance::Supports,
                Strength::Proven,
                format!(
                    "`{path}` exists at {at} ({}).",
                    if e == Entry::Dir { "directory" } else { "file" }
                ),
            ));
            (path.clone(), e)
        }
    };

    // Lines and section against the cited file.
    let text = match kind {
        Entry::File { .. } => repos.text(&repo, Rev::Cited, &resolved),
        _ => None,
    };
    if !lines.is_empty() {
        match (&kind, &text) {
            (Entry::Dir, _) => out.push(build(
                StructuralCheck::CitationLines,
                Stance::Contradicts,
                Strength::Proven,
                format!(
                    "`{resolved}` is a directory at {at}; line ranges {} do not apply.",
                    spans_text(lines)
                ),
            )),
            (_, Some(t)) => {
                let n = lines_of(t).len() as u32;
                let bad: Vec<String> = lines
                    .iter()
                    .filter(|s| s.start == 0 || s.end < s.start || s.end > n)
                    .map(|s| s.to_string())
                    .collect();
                if !bad.is_empty() {
                    out.push(build(
                        StructuralCheck::CitationLines,
                        Stance::Contradicts,
                        Strength::Proven,
                        format!(
                            "`{resolved}` has {n} lines at {at}; cited range {} is outside it.",
                            bad.join(",")
                        ),
                    ));
                }
            }
            _ => {}
        }
    }
    if let (Some(sec), Some(t)) = (section, &text) {
        if is_markdown(&resolved) && !has_heading(t, sec) {
            out.push(build(
                StructuralCheck::CitationLines,
                Stance::Contradicts,
                Strength::Proven,
                format!("`{resolved}` at {at} has no heading \"{sec}\"."),
            ));
        }
    }

    // Drift against the compare revision.
    if let Some((compare, _)) = &repo.compare {
        let drift = match (&kind, repos.entry(&repo, Rev::Compare, &resolved)) {
            (Entry::File { blob: a }, Entry::File { blob: b }) if *a == b => Drift::Same,
            (Entry::File { .. }, Entry::File { .. }) => {
                drift_of(repos, &repo, &resolved, &resolved, lines, false)
            }
            (Entry::File { .. }, Entry::Missing) => match repos.renamed(&repo, &resolved) {
                Some(to) => drift_of(repos, &repo, &resolved, &to, lines, true),
                None => Drift::Changed(format!(
                    "`{resolved}` was deleted between {} and {compare}.",
                    repo.cited
                )),
            },
            (Entry::File { .. }, Entry::Dir) => {
                Drift::Changed(format!("`{resolved}` is a directory at {compare}."))
            }
            (Entry::Dir, Entry::Missing) => Drift::Changed(format!(
                "Directory `{resolved}` no longer exists at {compare}."
            )),
            _ => Drift::Same,
        };
        match drift {
            Drift::Same => {}
            Drift::Moved(s) => out.push(build(
                StructuralCheck::CitationDrift,
                Stance::Supports,
                Strength::Strong,
                s,
            )),
            Drift::Changed(s) => out.push(build(
                StructuralCheck::CitationDrift,
                Stance::Inconclusive,
                Strength::Strong,
                format!("{s} The citation still describes {at}; the cited code may no longer be current."),
            )),
        }
    }
    out
}

/// What became of one citation, for summaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Outcome {
    Resolved,
    PrefixResolved,
    Ambiguous,
    Missing,
    /// Past the end, a range on a directory, or a missing section.
    BadLines,
}

/// Classify the evidence of one citation (`path`, `lines`, `drift` records). The bool is
/// whether the cited text drifted.
pub fn outcome(
    path: Option<&Evidence>,
    lines: Option<&Evidence>,
    drift: Option<&Evidence>,
) -> Option<(Outcome, bool)> {
    let p = path?;
    let o = match (p.stance, p.strength) {
        (Stance::Contradicts, _) => Outcome::Missing,
        (Stance::Inconclusive, _) => Outcome::Ambiguous,
        _ if lines.is_some_and(|l| l.stance == Stance::Contradicts) => Outcome::BadLines,
        (_, Strength::Proven) => Outcome::Resolved,
        _ => Outcome::PrefixResolved,
    };
    Some((o, drift.is_some_and(|d| d.stance == Stance::Inconclusive)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_match_by_text_or_slug() {
        let t = "# Title\n\n## Data and Migrations\n```\n# not a heading\n```\n";
        assert!(has_heading(t, "Data and Migrations"));
        assert!(has_heading(t, "data-and-migrations"));
        assert!(!has_heading(t, "not a heading"));
    }

    #[test]
    fn abbreviated_segments() {
        assert!(abbreviates("0005", "0005-foo.md"));
        assert!(abbreviates("0003-...", "0003-auth.md"));
        assert!(abbreviates("002", "002-tenancy"));
        assert!(!abbreviates("0005", "00051-x.md"));
        assert!(!abbreviates("a.md", "a.md.bak"));
        assert!(!abbreviates("0005", "0005"));
    }

    #[test]
    fn moved_blocks_are_found_nearest_first() {
        let hay = ["a", "x", "y", "b", "x", "y"];
        assert_eq!(find_block(&hay, &["x", "y"], 3), Some(4));
        assert_eq!(find_block(&hay, &["x", "y"], 0), Some(1));
        assert_eq!(find_block(&hay, &["q"], 0), None);
    }
}

/// Per-alias counts of distinct citation texts, for reports.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct AliasSummary {
    pub alias: String,
    /// `None` when the alias has no configured repository (not checked).
    pub cited: Option<String>,
    pub compare: Option<String>,
    /// Distinct citation texts, and how often they occur.
    pub citations: usize,
    pub occurrences: usize,
    pub resolved: usize,
    pub prefix_resolved: usize,
    pub ambiguous: usize,
    pub missing: usize,
    /// Line ranges outside the file, ranges on a directory, or a missing section.
    pub bad_lines: usize,
    pub drifted: usize,
    /// Cited text unchanged at `compare` but at other lines or under a new path.
    pub moved: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct CitationSummary {
    pub aliases: Vec<AliasSummary>,
    /// One sentence about aliases that were not checked, if any.
    pub note: Option<String>,
}

/// Summarize every citation in the model. `repos` are the configured repositories.
pub fn summarize(
    model: &concordance_graph::Model,
    repos: &[concordance_core::config::RepoConfig],
) -> Option<CitationSummary> {
    use std::collections::BTreeMap;
    if repos.is_empty() {
        return None;
    }
    // text -> (alias, first subject, occurrences)
    let mut seen: BTreeMap<String, (String, NodeRef, usize)> = BTreeMap::new();
    let mut add = |r: &Reference, subject: NodeRef| {
        if let ReferenceTarget::Citation { alias, .. } = &r.target {
            seen.entry(r.text.clone())
                .or_insert_with(|| (alias.clone(), subject, 0))
                .2 += 1;
        }
    };
    for c in model.claims.values() {
        for r in &c.references {
            add(r, NodeRef::Claim(c.id.clone()));
        }
    }
    for (doc, refs) in &model.prose_refs {
        for p in refs {
            add(&p.reference, NodeRef::Document(doc.clone()));
        }
    }
    let mut by_alias: BTreeMap<String, AliasSummary> = BTreeMap::new();
    for (text, (alias, subject, n)) in seen {
        let repo = repos.iter().find(|r| r.matches(&alias));
        let name = repo.map(|r| r.alias.clone()).unwrap_or(alias);
        // Unconfigured spellings that differ only in case are one alias.
        let s = by_alias
            .entry(name.to_lowercase())
            .or_insert_with(|| AliasSummary {
                alias: name,
                cited: repo.map(|r| r.cited.clone()),
                compare: repo.and_then(|r| r.compare.clone()),
                ..Default::default()
            });
        s.citations += 1;
        s.occurrences += n;
        if repo.is_none() {
            continue;
        }
        let ev = |check| model.evidence.get(&evidence_id(&subject, check, &text));
        let path = ev(StructuralCheck::CitationPath);
        let drift = ev(StructuralCheck::CitationDrift);
        let Some((o, drifted)) = outcome(path, ev(StructuralCheck::CitationLines), drift) else {
            continue;
        };
        match o {
            Outcome::Resolved => s.resolved += 1,
            Outcome::PrefixResolved => s.prefix_resolved += 1,
            Outcome::Ambiguous => s.ambiguous += 1,
            Outcome::Missing => s.missing += 1,
            Outcome::BadLines => s.bad_lines += 1,
        }
        if drifted {
            s.drifted += 1;
        } else if drift.is_some_and(|d| d.stance == Stance::Supports) {
            s.moved += 1;
        }
    }
    let unchecked: Vec<String> = by_alias
        .values()
        .filter(|a| a.cited.is_none())
        .map(|a| format!("{} ({})", a.alias, a.occurrences))
        .collect();
    let note = (!unchecked.is_empty()).then(|| {
        format!(
            "Citations with aliases that have no repository under [repos] were not checked: {}.",
            unchecked.join(", ")
        )
    });
    Some(CitationSummary {
        aliases: by_alias.into_values().collect(),
        note,
    })
}
