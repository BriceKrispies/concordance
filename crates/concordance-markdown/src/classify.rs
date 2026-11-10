//! Sentence classification: modality, claim kind, bounds, ambiguity and constraint forms.

use crate::text::{has_phrase, has_word, mask_ranges, words_lower};
use concordance_core::doc::{
    AmbiguityKind, AmbiguityNote, Bound, BoundOp, ClaimKind, ConstraintForm, Modality, Reference,
    ReferenceStrength, ReferenceTarget,
};

pub struct Classified {
    pub modality: Modality,
    pub kind: ClaimKind,
    pub bounds: Vec<Bound>,
    pub ambiguity: Vec<AmbiguityNote>,
    pub constraint: Option<ConstraintForm>,
}

/// Inputs describing one sentence.
pub struct Sentence<'a> {
    /// Normalized text, code spans in backticks.
    pub text: &'a str,
    /// Byte ranges of code spans within `text`.
    pub code_ranges: &'a [(usize, usize)],
    pub refs: &'a [Reference],
    /// Contents of the code spans, in order.
    pub codes: &'a [String],
}

const MUSTNOT_STRICT: &[&[&str]] = &[
    &["must", "not"],
    &["shall", "not"],
    &["may", "not"],
    &["must", "never"],
    &["shall", "never"],
    &["cannot"],
    &["can", "not"],
    &["is", "not", "allowed", "to"],
    &["are", "not", "allowed", "to"],
    &["is", "forbidden"],
    &["are", "forbidden"],
    &["is", "prohibited"],
    &["are", "prohibited"],
];

const MUST: &[&[&str]] = &[
    &["must"],
    &["shall"],
    &["is", "required", "to"],
    &["are", "required", "to"],
    &["required"],
    &["has", "to"],
    &["have", "to"],
    &["needs", "to"],
    &["always"],
];

const BEHAVIOR_VERBS: &[&str] = &[
    "returns",
    "return",
    "accepts",
    "takes",
    "rejects",
    "emits",
    "yields",
    "produces",
    "errors",
    "fails",
    "panics",
    "retries",
    "stores",
    "writes",
    "reads",
    "validates",
    "clamps",
    "limits",
    "guarantees",
    "ensures",
];

const VAGUE_WORDS: &[&str] = &[
    "appropriate",
    "appropriately",
    "reasonable",
    "reasonably",
    "fast",
    "quickly",
    "efficient",
    "efficiently",
    "etc",
    "some",
    "usually",
    "generally",
    "normally",
    "typically",
    "sufficient",
    "adequate",
    "robust",
    "various",
    "timely",
    "soon",
];

const VAGUE_PHRASES: &[&[&str]] = &[
    &["as", "needed"],
    &["if", "possible"],
    &["where", "possible"],
    &["as", "appropriate"],
    &["and", "so", "on"],
    &["and", "or"],
    &["user", "friendly"],
];

const INVARIANT_PHRASES: &[&str] = &[
    "at most",
    "at least",
    "no more than",
    "no fewer than",
    "never exceed",
    "never exceeds",
    "does not exceed",
    "at all times",
    "invariant",
    "always remain",
];

fn any_phrase(words: &[String], phrases: &[&[&str]]) -> bool {
    phrases.iter().any(|p| has_phrase(words, p))
}

fn hay(words: &[String]) -> String {
    format!(" {} ", words.join(" "))
}

fn has(h: &str, phrase: &str) -> bool {
    h.contains(&format!(" {phrase} "))
}

/// Modality from lowercase words (code spans already masked).
pub fn modality(words: &[String]) -> Modality {
    if any_phrase(words, MUSTNOT_STRICT) {
        return Modality::MustNot;
    }
    if has_phrase(words, &["should", "not"]) || has_phrase(words, &["should", "never"]) {
        return Modality::ShouldNot;
    }
    if has_word(words, "never") {
        return Modality::MustNot;
    }
    if any_phrase(words, MUST) {
        return Modality::Must;
    }
    if has_word(words, "should") || has_word(words, "recommended") {
        return Modality::Should;
    }
    if has_word(words, "may") || has_word(words, "optional") || has_word(words, "optionally") {
        return Modality::May;
    }
    Modality::Declarative
}

fn is_negative(m: Modality) -> bool {
    matches!(m, Modality::MustNot | Modality::ShouldNot)
}

/// Decide whether `s` is a claim; `None` if it is plain explanatory prose.
pub fn classify(s: &Sentence<'_>) -> Option<Classified> {
    if s.text.trim_end().ends_with('?') {
        return None;
    }
    let masked = mask_ranges(s.text, s.code_ranges);
    let words = words_lower(&masked);
    if words.len() < 3 {
        return None;
    }
    let all_words = words_lower(s.text);
    let modality = modality(&words);
    let h = hay(&words);
    let strong_sym = s.refs.iter().any(|r| {
        matches!(r.target, ReferenceTarget::Symbol { .. })
            && r.strength >= ReferenceStrength::Probable
    });
    let any_sym = s
        .refs
        .iter()
        .any(|r| matches!(r.target, ReferenceTarget::Symbol { .. }));
    let guarantee = ["guarantee", "guarantees", "guaranteed", "ensures"]
        .iter()
        .any(|w| has_word(&words, w));

    let kind;
    let mut constraint = None;
    if modality == Modality::Declarative {
        if !(strong_sym && BEHAVIOR_VERBS.iter().any(|v| has_word(&words, v))) {
            return None;
        }
        kind = if guarantee {
            ClaimKind::Guarantee
        } else {
            ClaimKind::Behavior
        };
    } else {
        constraint = constraint_form(s, &words, modality);
        let arch = [
            "depend",
            "depends",
            "dependency",
            "dependencies",
            "import",
            "imports",
            "layer",
            "layers",
            "crate",
            "crates",
        ]
        .iter()
        .any(|w| has_word(&words, w))
            || has_phrase(&words, &["module", "boundary"]);
        let invariant_phrase = INVARIANT_PHRASES.iter().any(|p| has(&h, p));
        kind = if matches!(constraint, Some(ConstraintForm::NoDependency { .. })) {
            ClaimKind::ArchitecturalConstraint
        } else if is_negative(modality) {
            if has(&h, "exceed")
                || has(&h, "exceeds")
                || has(&h, "no more than")
                || has(&h, "at most")
                || has(&h, "not more than")
            {
                ClaimKind::Invariant
            } else {
                ClaimKind::Prohibition
            }
        } else if invariant_phrase {
            ClaimKind::Invariant
        } else if guarantee {
            ClaimKind::Guarantee
        } else if arch {
            ClaimKind::ArchitecturalConstraint
        } else if any_sym
            && [
                "return",
                "returns",
                "returned",
                "parameter",
                "parameters",
                "argument",
                "arguments",
                "signature",
                "error",
                "errors",
                "result",
                "option",
            ]
            .iter()
            .any(|w| has_word(&all_words, w))
        {
            ClaimKind::ApiContract
        } else {
            ClaimKind::Requirement
        };
    }

    let bounds = extract_bounds(s.text, modality);
    let ambiguity = ambiguity_notes(s.text, &words, modality);
    Some(Classified {
        modality,
        kind,
        bounds,
        ambiguity,
        constraint,
    })
}

// ---------------------------------------------------------------- bounds

struct Tok {
    raw: String,
    lower: String,
}

fn tokens(text: &str) -> Vec<Tok> {
    text.split_whitespace()
        .filter_map(|t| {
            let raw = t
                .trim_end_matches(['.', '!', '?'])
                .trim_matches(|c| matches!(c, '`' | ',' | ';' | ':' | '(' | ')' | '"' | '\''))
                .trim_end_matches('.')
                .to_string();
            if raw.is_empty() {
                None
            } else {
                let lower = raw.to_lowercase();
                Some(Tok { raw, lower })
            }
        })
        .collect()
}

fn parse_number(tok: &str) -> Option<(f64, bool)> {
    let (body, pct) = match tok.strip_suffix('%') {
        Some(b) => (b, true),
        None => (tok, false),
    };
    let clean: String = body.chars().filter(|c| *c != ',' && *c != '_').collect();
    if clean.is_empty() || !clean.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    if !clean.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    clean.parse::<f64>().ok().map(|v| (v, pct))
}

const BOUND_PHRASES: &[(&[&str], BoundOp)] = &[
    (&["must", "never", "exceed"], BoundOp::Le),
    (&["must", "not", "exceed"], BoundOp::Le),
    (&["shall", "not", "exceed"], BoundOp::Le),
    (&["shall", "never", "exceed"], BoundOp::Le),
    (&["should", "not", "exceed"], BoundOp::Le),
    (&["may", "not", "exceed"], BoundOp::Le),
    (&["can", "not", "exceed"], BoundOp::Le),
    (&["does", "not", "exceed"], BoundOp::Le),
    (&["do", "not", "exceed"], BoundOp::Le),
    (&["no", "fewer", "than"], BoundOp::Ge),
    (&["no", "less", "than"], BoundOp::Ge),
    (&["no", "more", "than"], BoundOp::Le),
    (&["not", "more", "than"], BoundOp::Le),
    (&["never", "exceeds"], BoundOp::Le),
    (&["never", "exceed"], BoundOp::Le),
    (&["cannot", "exceed"], BoundOp::Le),
    (&["fewer", "than"], BoundOp::Lt),
    (&["less", "than"], BoundOp::Lt),
    (&["greater", "than"], BoundOp::Gt),
    (&["more", "than"], BoundOp::Gt),
    (&["at", "most"], BoundOp::Le),
    (&["at", "least"], BoundOp::Ge),
    (&["up", "to"], BoundOp::Le),
    (&["exceeds"], BoundOp::Gt),
    (&["exactly"], BoundOp::Eq),
    (&["below"], BoundOp::Lt),
];

const OPERATORS: &[(&str, BoundOp)] = &[
    ("<=", BoundOp::Le),
    (">=", BoundOp::Ge),
    ("\u{2264}", BoundOp::Le),
    ("\u{2265}", BoundOp::Ge),
    ("<", BoundOp::Lt),
    (">", BoundOp::Gt),
];

const SUBJECT_STOP: &[&str] = &[
    "must", "shall", "should", "may", "might", "can", "cannot", "will", "would", "not", "never",
    "always", "does", "do", "is", "are", "be", "been", "to", "the",
];

const UNIT_STOP: &[&str] = &[
    "and", "or", "of", "per", "to", "the", "a", "an", "in", "on", "for", "at", "by", "is", "are",
    "be", "with", "when", "if", "than", "from", "as", "it", "that", "which", "but",
];

fn extract_bounds(text: &str, modality: Modality) -> Vec<Bound> {
    let toks = tokens(text);
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let mut matched: Option<(usize, BoundOp, usize, f64, bool)> = None; // (phrase_len, op, number idx, value, pct)
        for (phrase, op) in BOUND_PHRASES {
            let n = phrase.len();
            if i + n < toks.len() && (0..n).all(|k| toks[i + k].lower == phrase[k]) {
                if *op == BoundOp::Gt && modality != Modality::Must {
                    continue;
                }
                if let Some((v, pct)) = parse_number(&toks[i + n].raw) {
                    matched = Some((n, *op, i + n, v, pct));
                    break;
                }
            }
        }
        if matched.is_none() {
            let t = &toks[i].raw;
            for (sym, op) in OPERATORS {
                if let Some(rest) = t.strip_prefix(sym) {
                    let (idx, numtok) = if rest.is_empty() {
                        match toks.get(i + 1) {
                            Some(n) => (i + 1, n.raw.as_str()),
                            None => break,
                        }
                    } else {
                        (i, rest)
                    };
                    if let Some((v, pct)) = parse_number(numtok) {
                        matched = Some((if rest.is_empty() { 1 } else { 0 }, *op, idx, v, pct));
                    }
                    break;
                }
            }
        }
        let Some((_plen, op, num_idx, value, pct)) = matched else {
            i += 1;
            continue;
        };
        let mut end = num_idx; // inclusive index of last token in the match
        let mut unit = None;
        if pct {
            unit = Some("%".to_string());
        } else if let Some(u) = toks.get(num_idx + 1) {
            let alpha = u.raw.chars().all(|c| c.is_alphabetic());
            if u.raw == "%" {
                unit = Some("%".to_string());
                end = num_idx + 1;
            } else if alpha && !UNIT_STOP.contains(&u.lower.as_str()) {
                unit = Some(u.raw.clone());
                end = num_idx + 1;
            }
        }
        let subject = toks[i.saturating_sub(5)..i]
            .iter()
            .filter(|t| !SUBJECT_STOP.contains(&t.lower.as_str()))
            .map(|t| t.raw.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let phrase_text = toks[i..=end]
            .iter()
            .map(|t| t.raw.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        out.push(Bound {
            subject,
            op,
            value,
            unit,
            text: phrase_text,
        });
        i = end + 1;
    }
    out
}

// ---------------------------------------------------------------- ambiguity

fn ambiguity_notes(text: &str, words: &[String], modality: Modality) -> Vec<AmbiguityNote> {
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut push_vague = |term: String, out: &mut Vec<AmbiguityNote>| {
        if !seen.contains(&term) {
            out.push(AmbiguityNote {
                kind: AmbiguityKind::VagueTerm,
                detail: format!("vague term \"{term}\""),
            });
            seen.push(term);
        }
    };
    for w in words {
        if VAGUE_WORDS.contains(&w.as_str()) {
            push_vague(
                if w == "etc" {
                    "etc.".to_string()
                } else {
                    w.clone()
                },
                &mut out,
            );
        }
    }
    for p in VAGUE_PHRASES {
        if has_phrase(words, p) {
            let term = if p[0] == "user" {
                "user-friendly".to_string()
            } else {
                p.join(" ")
            };
            push_vague(term, &mut out);
        }
    }
    if matches!(modality, Modality::Must | Modality::MustNot) {
        let strong = if modality == Modality::Must {
            "must/shall"
        } else {
            "must not"
        };
        let mut weak: Option<&str> = None;
        if has_phrase(words, &["if", "possible"]) {
            weak = Some("if possible");
        } else if has_phrase(words, &["where", "possible"]) {
            weak = Some("where possible");
        } else if has_word(words, "should") {
            weak = Some("should");
        } else if let Some(i) = words.iter().position(|w| w == "may") {
            if words.get(i + 1).map(String::as_str) != Some("not") {
                weak = Some("may");
            }
        }
        if let Some(w) = weak {
            out.push(AmbiguityNote {
                kind: AmbiguityKind::MixedModality,
                detail: format!("strong modality ({strong}) combined with weak qualifier \"{w}\""),
            });
        }
    }
    let mut words = text.split_whitespace();
    let first = words.next().unwrap_or("");
    let second = words.next().unwrap_or("").to_lowercase();
    // "It"/"They" never carry their antecedent; "This"/"These"/"That" only when used as a
    // bare pronoun ("This must ...") rather than a determiner ("This crate must ...").
    let verbish = [
        "must", "shall", "should", "may", "will", "can", "cannot", "is", "are", "was", "were",
        "has", "have", "never", "always", "returns", "means",
    ];
    let bare_pronoun = matches!(first, "It" | "They")
        || (matches!(first, "This" | "These" | "That") && verbish.contains(&second.as_str()));
    if bare_pronoun {
        out.push(AmbiguityNote {
            kind: AmbiguityKind::UnresolvedSubject,
            detail: format!(
                "sentence starts with \"{first}\" whose antecedent is not in this statement"
            ),
        });
    }
    out
}

// ---------------------------------------------------------------- constraints

const NAME_SKIP: &[&str] = &[
    "the", "a", "an", "it", "they", "this", "that", "these", "those", "which", "and", "or", "code",
    "nothing", "any", "other", "directly", "them",
];
const KIND_WORDS: &[&str] = &[
    "crate", "module", "package", "layer", "library", "lib", "crates", "modules",
];

fn name_like(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '-' | '.' | '/'))
}

fn constraint_form(
    s: &Sentence<'_>,
    words: &[String],
    modality: Modality,
) -> Option<ConstraintForm> {
    if modality != Modality::MustNot {
        return None;
    }
    let toks = tokens(s.text);
    if let Some(i) = (0..toks.len().saturating_sub(3)).find(|&i| {
        matches!(toks[i].lower.as_str(), "must" | "shall")
            && matches!(toks[i + 1].lower.as_str(), "not" | "never")
            && (matches!(toks[i + 2].lower.as_str(), "import" | "imports")
                || (toks[i + 2].lower == "depend"
                    && toks.get(i + 3).is_some_and(|t| t.lower == "on")))
    }) {
        let after = if toks[i + 2].lower == "depend" {
            i + 4
        } else {
            i + 3
        };
        let mut from_idx = i.checked_sub(1);
        if let Some(f) = from_idx {
            if KIND_WORDS.contains(&toks[f].lower.as_str()) {
                from_idx = f
                    .checked_sub(1)
                    .filter(|&g| !NAME_SKIP.contains(&toks[g].lower.as_str()));
            }
        }
        let mut to_idx = after;
        while to_idx < toks.len()
            && (NAME_SKIP.contains(&toks[to_idx].lower.as_str())
                || KIND_WORDS.contains(&toks[to_idx].lower.as_str()))
        {
            to_idx += 1;
        }
        if let (Some(f), Some(t)) = (from_idx, toks.get(to_idx)) {
            let from = &toks[f];
            if !NAME_SKIP.contains(&from.lower.as_str())
                && name_like(&from.raw)
                && name_like(&t.raw)
            {
                return Some(ConstraintForm::NoDependency {
                    from: from.raw.clone(),
                    to: t.raw.clone(),
                });
            }
        }
    }
    // Failure sites: Rust panics, exceptions elsewhere (one check, see FactKind::PanicSite).
    if negated_near(words, |w| {
        w.starts_with("panic")
            || matches!(w, "throw" | "throws" | "raise" | "raises")
            || w.starts_with("exception")
    }) {
        return Some(ConstraintForm::NoPanic { scope: scope_of(s) });
    }
    if negated_near(words, |w| w == "unsafe") {
        return Some(ConstraintForm::NoUnsafe { scope: scope_of(s) });
    }
    None
}

/// Whether a word matching `target` is itself the object of a negation: a negating
/// word ("not", "never", "no", "without", "forbid…") within the four words before it.
/// "must not use unsafe code" qualifies; "…unsafe reinvestment … never a different
/// winner" does not, because the negation governs a different phrase.
fn negated_near(words: &[String], target: impl Fn(&str) -> bool) -> bool {
    const NEG: &[&str] = &[
        "not",
        "never",
        "no",
        "without",
        "nor",
        "forbid",
        "forbids",
        "forbidden",
        "free",
    ];
    words.iter().enumerate().any(|(i, w)| {
        if !target(w) {
            return false;
        }
        // "panic-free" / "unsafe-free" style: negation right after.
        if words.get(i + 1).is_some_and(|n| n == "free") {
            return true;
        }
        // "unsafe code is forbidden", "panics are not allowed": a prohibition right after.
        const AFTER: &[&str] = &[
            "forbidden",
            "prohibited",
            "disallowed",
            "banned",
            "not",
            "never",
        ];
        words[i.saturating_sub(4)..i]
            .iter()
            .any(|p| NEG.contains(&p.as_str()) && p != "free")
            || words
                .iter()
                .skip(i + 1)
                .take(3)
                .any(|p| AFTER.contains(&p.as_str()))
    })
}

fn scope_of(s: &Sentence<'_>) -> Vec<String> {
    for r in s.refs {
        if let ReferenceTarget::Symbol { path } = &r.target {
            if r.strength >= ReferenceStrength::Probable {
                return path.clone();
            }
        }
    }
    for c in s.codes {
        let segs: Vec<&str> = c.split("::").collect();
        if !segs.is_empty()
            && segs
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|ch| ch.is_alphanumeric() || ch == '_'))
        {
            return segs.into_iter().map(str::to_string).collect();
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> Option<Classified> {
        // Code ranges: locate backtick pairs.
        let mut ranges = Vec::new();
        let mut start = None;
        for (i, c) in text.char_indices() {
            if c == '`' {
                match start.take() {
                    Some(s) => ranges.push((s, i + 1)),
                    None => start = Some(i),
                }
            }
        }
        let codes: Vec<String> = ranges
            .iter()
            .map(|&(s, e)| text[s + 1..e - 1].to_string())
            .collect();
        classify(&Sentence {
            text,
            code_ranges: &ranges,
            refs: &[],
            codes: &codes,
        })
    }

    #[test]
    fn modality_and_kind() {
        let c = run("The server MUST reject empty requests.").unwrap();
        assert_eq!(
            (c.modality, c.kind),
            (Modality::Must, ClaimKind::Requirement)
        );
        let c = run("Clients should never retry on 4xx responses.").unwrap();
        assert_eq!(
            (c.modality, c.kind),
            (Modality::ShouldNot, ClaimKind::Prohibition)
        );
        assert!(run("The weather is nice today.").is_none());
    }

    #[test]
    fn bound_extraction() {
        let c = run("A client must never exceed 100 requests per window.").unwrap();
        assert_eq!(c.kind, ClaimKind::Invariant);
        let b = &c.bounds[0];
        assert_eq!((b.op, b.value), (BoundOp::Le, 100.0));
        assert_eq!(b.unit.as_deref(), Some("requests"));
    }

    #[test]
    fn dependency_constraint() {
        let c = run("`domain` must not depend on `infra`.").unwrap();
        assert_eq!(
            c.constraint,
            Some(ConstraintForm::NoDependency {
                from: "domain".into(),
                to: "infra".into()
            })
        );
        assert_eq!(c.kind, ClaimKind::ArchitecturalConstraint);
    }
}

#[cfg(test)]
mod negation_tests {
    use super::negated_near;
    use crate::text::words_lower;

    fn unsafe_neg(s: &str) -> bool {
        negated_near(&words_lower(s), |w| w == "unsafe")
    }

    #[test]
    fn unsafe_constraint_requires_negation_of_unsafe_itself() {
        assert!(unsafe_neg("This crate must not use unsafe code."));
        assert!(unsafe_neg("The parser SHALL contain no unsafe blocks."));
        assert!(unsafe_neg("Unsafe code is forbidden in the core."));
        assert!(!unsafe_neg(
            "Corrupted witness, zero allowance, unsafe reinvestment and budget exhaustion produce the owning refusal, never an opportunistic different winner."
        ));
    }
}
