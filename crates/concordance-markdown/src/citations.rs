//! `[ALIAS:path:lines]` citations of files in other repositories, found in plain text.
//!
//! Grammar (one bracket may hold several citations separated by `;`, or by `,` when
//! the next part starts with an alias):
//!
//! ```text
//! citation = ALIAS ":" PATH [ ":" RANGES ] [ ("#" ANCHOR | WS? '"' SECTION '"') ] [ WS NOTE ]
//! ALIAS    = letter (letter | digit | "_" | "-"){1,15}
//! RANGES   = RANGE { "," WS? RANGE }      RANGE = ["L"] N [ "-" ["L"] N ]
//! ```
//!
//! The path must contain `/` or `.`, which keeps Jira keys, Confluence page ids and
//! prose like `[Note: ...]` out. Extraction does not know the configured aliases, so
//! the same text always indexes the same way; reconciliation decides which aliases it
//! can check.

use concordance_core::doc::{LineSpan, ReferenceTarget};

/// One citation found in a text, with byte offsets into that text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CitationMatch {
    pub start: usize,
    pub end: usize,
    /// The citation without brackets or separators (`APP:a/b.ts:1-4`).
    pub text: String,
    pub target: ReferenceTarget,
}

const SCHEMES: &[&str] = &[
    "http",
    "https",
    "mailto",
    "file",
    "ftp",
    "data",
    "urn",
    "tel",
    "javascript",
];

/// Every citation in `text`. Text inside backtick code spans is ignored (a code span
/// inside a citation's trailing note is fine). Never panics.
pub fn extract_citations(text: &str) -> Vec<CitationMatch> {
    let masked = mask_code_spans(text);
    let bytes = masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        let Some(close) = masked[i + 1..].find([']', '[']).map(|k| i + 1 + k) else {
            break;
        };
        if bytes[close] == b'[' {
            i = close;
            continue;
        }
        bracket(text, &masked, i + 1, close, &mut out);
        i = close + 1;
    }
    out
}

/// Replace the contents of backtick code spans (and the backticks) with spaces, so
/// offsets stay valid.
fn mask_code_spans(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        out.push_str(&rest[..open]);
        let run = rest[open..].bytes().take_while(|b| *b == b'`').count();
        let fence = &rest[open..open + run];
        let after = &rest[open + run..];
        match after.find(fence) {
            Some(k) => {
                let len = run + k + run;
                out.push_str(&" ".repeat(len));
                rest = &rest[open + len..];
            }
            None => {
                out.push_str(fence);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parse the inside of one `[...]` (`from..to` in byte offsets).
fn bracket(text: &str, masked: &str, from: usize, to: usize, out: &mut Vec<CitationMatch>) {
    let inner = &masked[from..to];
    // Split points: `;` always, `,` only before `ALIAS:`.
    let mut parts: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    let b = inner.as_bytes();
    for (k, &c) in b.iter().enumerate() {
        let split = match c {
            b';' => true,
            b',' => alias_prefix(inner[k + 1..].trim_start()).is_some(),
            _ => false,
        };
        if split {
            parts.push((start, k));
            start = k + 1;
        }
    }
    parts.push((start, inner.len()));
    for (s, e) in parts {
        let part = &inner[s..e];
        let lead = part.len() - part.trim_start().len();
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let abs = from + s + lead;
        if let Some(target) = parse_one(trimmed) {
            let end = abs + trimmed.len();
            out.push(CitationMatch {
                start: abs,
                end,
                text: text.get(abs..end).unwrap_or(trimmed).to_string(),
                target,
            });
        }
    }
}

/// `ALIAS:` at the start of `s`: returns the alias.
fn alias_prefix(s: &str) -> Option<&str> {
    let colon = s.find(':')?;
    let alias = &s[..colon];
    (concordance_core::config::is_alias(alias)
        && !SCHEMES.contains(&alias.to_ascii_lowercase().as_str()))
    .then_some(alias)
}

/// Parse one citation (`APP:a/b.ts:1-4 "Section"`).
pub fn parse_citation(s: &str) -> Option<ReferenceTarget> {
    parse_one(s.trim())
}

fn parse_one(s: &str) -> Option<ReferenceTarget> {
    let alias = alias_prefix(s)?;
    let rest = &s[alias.len() + 1..];
    let path_end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, ':' | '#' | '"' | ',' | ']' | '['))
        .unwrap_or(rest.len());
    let written = &rest[..path_end];
    let path = written.trim_end_matches('/');
    let mut rest = &rest[path_end..];
    // A bare name (`lib`) is a path only with a trailing `/` or a line range.
    let path_like = written.contains('/')
        || path.contains('.')
        || (rest.starts_with(':')
            && rest[1..].starts_with(|c: char| c.is_ascii_digit())
            && path.chars().any(|c| c.is_alphabetic()));
    if !path_like || !valid_path(path) {
        return None;
    }
    let mut path = path.to_string();
    // `docs/specifications/002 spec.md:5`: a directory abbreviation, then a file name.
    if let Some(file) = spaced_file_name(rest) {
        if !path.rsplit('/').next().unwrap_or(&path).contains('.') {
            path = format!("{path}/{file}");
            rest = &rest[rest.find(file).map_or(0, |i| i + file.len())..];
        }
    }
    let mut lines = Vec::new();
    if let Some(r) = rest.strip_prefix(':') {
        // `:…` elides the lines: a whole-file citation.
        let elided = r.strip_prefix('…').or_else(|| r.strip_prefix("..."));
        match elided {
            Some(after) => rest = after,
            None => {
                let (spans, used) = ranges(r)?;
                lines = spans;
                rest = &r[used..];
            }
        }
    }
    let mut section = None;
    if let Some(r) = rest.strip_prefix('#') {
        let end = r.find(char::is_whitespace).unwrap_or(r.len());
        let anchor = &r[..end];
        // GitHub line anchors (`#L10-L20`) are line ranges, not headings.
        match anchor.strip_prefix('L').and_then(|_| ranges(anchor)) {
            Some((spans, used)) if used == anchor.len() && lines.is_empty() => lines = spans,
            _ => section = Some(anchor.to_string()).filter(|a| !a.is_empty()),
        }
        rest = &r[end..];
    } else if let Some(r) = rest.trim_start().strip_prefix('"') {
        let end = r.find('"')?;
        section = Some(r[..end].trim().to_string()).filter(|a| !a.is_empty());
        rest = &r[end + 1..];
    }
    // Anything left is a free note (`(search)`, `unnumbered row`), and must be set off.
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some(ReferenceTarget::Citation {
        alias: alias.to_string(),
        path,
        lines,
        section,
    })
}

/// ` spec.md` / ` spec.md:5` at the start of `rest`: the file name.
fn spaced_file_name(rest: &str) -> Option<&str> {
    let t = rest.strip_prefix(' ')?;
    let end = t
        .find(|c: char| c.is_whitespace() || matches!(c, ':' | '"' | '#'))
        .unwrap_or(t.len());
    let name = &t[..end];
    let (stem, ext) = name.rsplit_once('.')?;
    let ok = !stem.is_empty()
        && (1..=8).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'));
    ok.then_some(name)
}

fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('?')
        && !p.split('/').any(|seg| seg == ".." || seg.is_empty())
        && p.chars().any(|c| c.is_alphanumeric())
}

/// `20,374-384` / `L10-L20` / `1, 59-61`. Returns the spans and the bytes consumed.
fn ranges(s: &str) -> Option<(Vec<LineSpan>, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    let num = |i: &mut usize| -> Option<u32> {
        if b.get(*i) == Some(&b'L') {
            *i += 1;
        }
        let st = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        s[st..*i].parse().ok()
    };
    loop {
        let start = num(&mut i)?;
        let mut end = start;
        if b.get(i) == Some(&b'-')
            && b.get(i + 1)
                .is_some_and(|c| c.is_ascii_digit() || *c == b'L')
        {
            i += 1;
            end = num(&mut i)?;
        }
        out.push(LineSpan { start, end });
        // Another range follows only after `,` and optional spaces, then a digit.
        let mut j = i;
        if b.get(j) != Some(&b',') {
            break;
        }
        j += 1;
        while b.get(j) == Some(&b' ') {
            j += 1;
        }
        if !b.get(j).is_some_and(|c| c.is_ascii_digit() || *c == b'L') {
            break;
        }
        i = j;
    }
    Some((out, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> (String, String, Vec<(u32, u32)>, Option<String>) {
        let all = extract_citations(s);
        assert_eq!(all.len(), 1, "{s}: {all:?}");
        match &all[0].target {
            ReferenceTarget::Citation {
                alias,
                path,
                lines,
                section,
            } => (
                alias.clone(),
                path.clone(),
                lines.iter().map(|l| (l.start, l.end)).collect(),
                section.clone(),
            ),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn citation_forms() {
        assert_eq!(
            one("see [APP:apps/api/src/catalog.ts:67-132] here"),
            (
                "APP".into(),
                "apps/api/src/catalog.ts".into(),
                vec![(67, 132)],
                None
            )
        );
        assert_eq!(one("[DB:a/b.md:7]").2, vec![(7, 7)]);
        assert_eq!(
            one("[APP:docs/adr/0002:20,374-384]").2,
            vec![(20, 20), (374, 384)]
        );
        assert_eq!(one("[aas:docs/x.md:1, 59-61]").2, vec![(1, 1), (59, 61)]);
        assert_eq!(one("[APP:docs/adr/0005]").1, "docs/adr/0005");
        assert_eq!(one("[WEB:services/worker]").1, "services/worker");
        assert_eq!(one("[WEB:services/]").1, "services");
        assert_eq!(one("[WEB:lib:1-2]").1, "lib");
        assert_eq!(
            one("[APP:AGENTS.md \"Data and Migrations\"]").3.as_deref(),
            Some("Data and Migrations")
        );
        assert_eq!(one("[APP:README.md#setup]").3.as_deref(), Some("setup"));
        assert_eq!(one("[APP:apps/api/src (search)]").1, "apps/api/src");
        let (_, path, lines, _) = one("[APP:docs/specifications/002 spec.md:205-209]");
        assert_eq!(
            (path.as_str(), lines),
            ("docs/specifications/002/spec.md", vec![(205, 209)])
        );
        assert_eq!(one("[APP:docs/adr/0003-...:21]").1, "docs/adr/0003-...");
        assert_eq!(one("[APP:docs/a.md:…]").2, vec![]);
        assert_eq!(one("[WEB:services/*/src]").1, "services/*/src");
        assert_eq!(one("[APP:src/a.ts#L10-L20]").2, vec![(10, 20)]);
        assert_eq!(one("[APP:src/a.ts:L10-L20]").2, vec![(10, 20)]);
    }

    #[test]
    fn several_citations_in_one_bracket() {
        let all = extract_citations(
            "[app:a/b.ts:378-385; app:docs/c.md:24-36] and [docs:docs/x.md:29-35, docs:AGENTS.md:101-110]",
        );
        let texts: Vec<&str> = all.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "app:a/b.ts:378-385",
                "app:docs/c.md:24-36",
                "docs:docs/x.md:29-35",
                "docs:AGENTS.md:101-110"
            ]
        );
        let s = "x [app:a/b.ts:3; app:c.md:4]";
        for c in extract_citations(s) {
            assert_eq!(&s[c.start..c.end], c.text);
        }
    }

    #[test]
    fn non_citations_are_ignored() {
        for s in [
            "[PROJ-351]",
            "[Wiki:1031]",
            "[PLAN:57; TA3:422]",
            "[Note: see the guide.md]",
            "[http://example.com/a.md]",
            "[APP:../outside.md]",
            "`[APP:a/b.ts:1]`",
            "[link](APP:a/b.ts)",
            "[x:a.md]",
            "[WEB:services]",
            "[PRD-506:12]",
        ] {
            assert!(extract_citations(s).is_empty(), "{s}");
        }
    }

    #[test]
    fn code_span_inside_a_note_is_fine() {
        let (_, path, _, _) = one("[APP:infra/terraform/app (variable `topic_arn`)]");
        assert_eq!(path, "infra/terraform/app");
    }
}
