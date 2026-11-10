//! Citation validation against what was actually sent, and verdict downgrades.

use crate::model::*;
use crate::prompt::BuiltPrompt;
use crate::schema::{CitationSource, RawAssessment, RawCitation};
use concordance_core::evidence::CitationRole;
use concordance_core::SourceRange;

pub const MAX_CITATION_SPAN: usize = 80;
const MAX_EXPLANATION_CHARS: usize = 1200;
const MAX_AMBIGUITY_CHARS: usize = 300;
const MAX_AMBIGUITIES: usize = 5;
const MAX_NOTE_CHARS: usize = 300;

pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Normalize a model quote: undo our tag escaping, drop `  40| ` line-number prefixes.
fn normalize_quote(q: &str) -> String {
    let unescaped = q.replace("<\\/", "</");
    let stripped: Vec<String> = unescaped
        .lines()
        .map(|l| {
            let t = l.trim_start();
            let digits = t.bytes().take_while(|b| b.is_ascii_digit()).count();
            if digits > 0 && t[digits..].starts_with('|') {
                t[digits + 1..].to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    collapse(&stripped.join(" "))
}

/// The text a citation resolves against.
struct Target<'a> {
    start_line: u32,
    lines: &'a [String],
}

fn slice_text(t: &Target, from: usize, to: usize) -> String {
    collapse(&t.lines[from..=to].join(" "))
}

/// Find the smallest line range (indices) whose collapsed text contains `quote`.
fn find_quote(t: &Target, quote: &str) -> Option<(usize, usize)> {
    let n = t.lines.len();
    for len in 1..=MAX_CITATION_SPAN.min(n) {
        for s in 0..=(n - len) {
            if slice_text(t, s, s + len - 1).contains(quote) {
                return Some((s, s + len - 1));
            }
        }
    }
    None
}

struct Checked {
    start: usize,
    end: usize,
}

fn check_range(t: &Target, c: &RawCitation, quote: &str) -> Result<Checked, String> {
    if c.start_line > c.end_line {
        return Err(format!(
            "start_line {} is after end_line {}",
            c.start_line, c.end_line
        ));
    }
    if t.lines.is_empty() {
        return Err("no lines were sent".into());
    }
    let first = t.start_line as i64;
    let last = first + t.lines.len() as i64 - 1;
    if c.start_line < first || c.end_line > last {
        return Err(format!(
            "lines {}-{} are outside the sent lines {}-{}",
            c.start_line, c.end_line, first, last
        ));
    }
    let span = (c.end_line - c.start_line + 1) as usize;
    if span > MAX_CITATION_SPAN {
        return Err(format!(
            "citation spans {span} lines (max {MAX_CITATION_SPAN})"
        ));
    }
    let start = (c.start_line - first) as usize;
    let end = (c.end_line - first) as usize;
    if slice_text(t, start, end).contains(quote) {
        return Ok(Checked { start, end });
    }
    match find_quote(t, quote) {
        Some((s, e)) => Ok(Checked { start: s, end: e }),
        None => Err("quote not found in the sent text".into()),
    }
}

fn validate_one(c: &RawCitation, prompt: &BuiltPrompt) -> Result<ValidatedCitation, String> {
    let quote = normalize_quote(&c.quote);
    if quote.chars().count() < 3 {
        return Err("quote is empty or too short".into());
    }
    let note = truncate_chars(c.note.trim(), MAX_NOTE_CHARS);
    if c.excerpt_id == "C" {
        if c.source != CitationSource::Passage {
            return Err("excerpt_id \"C\" requires source \"passage\"".into());
        }
        let p = &prompt.passage;
        let t = Target {
            start_line: p.start_line,
            lines: &p.lines,
        };
        let r = check_range(&t, c, &quote)?;
        let sl = p.start_line + r.start as u32;
        let el = p.start_line + r.end as u32;
        return Ok(ValidatedCitation {
            role: CitationRole::Specification,
            symbol: None,
            range: SourceRange::lines(p.document.clone(), sl, el),
            excerpt: p.lines[r.start..=r.end].join("\n"),
            note,
        });
    }
    if c.source != CitationSource::Excerpt {
        return Err(format!(
            "source \"passage\" with excerpt_id \"{}\"",
            c.excerpt_id
        ));
    }
    let ex = prompt
        .excerpts
        .iter()
        .find(|e| e.id == c.excerpt_id)
        .ok_or_else(|| format!("excerpt id \"{}\" was not sent", c.excerpt_id))?;
    let t = Target {
        start_line: ex.start_line,
        lines: &ex.lines,
    };
    let r = check_range(&t, c, &quote)?;
    let sl = ex.start_line + r.start as u32;
    let el = ex.start_line + r.end as u32;
    Ok(ValidatedCitation {
        role: CitationRole::Implementation,
        symbol: Some(ex.symbol.clone()),
        range: SourceRange::lines(ex.file.clone(), sl, el),
        excerpt: ex.lines[r.start..=r.end].join("\n"),
        note,
    })
}

fn validate_list(
    list: &[RawCitation],
    prompt: &BuiltPrompt,
    label: &str,
    rejected: &mut Vec<String>,
) -> Vec<ValidatedCitation> {
    let mut out = Vec::new();
    for c in list {
        match validate_one(c, prompt) {
            Ok(v) => {
                if !out
                    .iter()
                    .any(|o: &ValidatedCitation| o.range == v.range && o.symbol == v.symbol)
                {
                    out.push(v);
                }
            }
            Err(why) => rejected.push(format!(
                "{label} citation {}:{}-{} rejected: {why}",
                c.excerpt_id, c.start_line, c.end_line
            )),
        }
    }
    out
}

/// Validate a parsed response against what was sent and apply downgrade rules.
pub fn finalize(raw: &RawAssessment, prompt: &BuiltPrompt) -> Assessment {
    let mut rejected = prompt.omitted.clone();
    let supporting = validate_list(&raw.supporting, prompt, "supporting", &mut rejected);
    let contradicting = validate_list(&raw.contradicting, prompt, "contradicting", &mut rejected);

    let mut verdict = raw.verdict;
    let mut downgraded = None;
    match verdict {
        Verdict::Disagrees
            if !contradicting
                .iter()
                .any(|c| c.role == CitationRole::Implementation) =>
        {
            verdict = Verdict::InsufficientEvidence;
            downgraded = Some(
                "model reported a contradiction without verifiable code citations".to_string(),
            );
        }
        Verdict::Agrees
            if !supporting
                .iter()
                .any(|c| c.role == CitationRole::Implementation) =>
        {
            verdict = Verdict::InsufficientEvidence;
            downgraded = Some(
                "model reported agreement without verifiable supporting code citations".to_string(),
            );
        }
        _ => {}
    }

    let mut challenge = raw.challenge_outcome;
    if challenge == Some(ChallengeOutcome::Refuted)
        && !supporting
            .iter()
            .chain(&contradicting)
            .any(|c| c.role == CitationRole::Implementation)
    {
        challenge = Some(ChallengeOutcome::Inconclusive);
        let note = "challenge reported a refutation without verifiable citations";
        downgraded = Some(match downgraded {
            Some(d) => format!("{d}; {note}"),
            None => note.to_string(),
        });
    }

    Assessment {
        verdict,
        explanation: truncate_chars(raw.explanation.trim(), MAX_EXPLANATION_CHARS),
        supporting,
        contradicting,
        ambiguities: raw
            .ambiguities
            .iter()
            .map(|a| truncate_chars(a.trim(), MAX_AMBIGUITY_CHARS))
            .filter(|a| !a.is_empty())
            .take(MAX_AMBIGUITIES)
            .collect(),
        rejected,
        downgraded,
        challenge,
    }
}
