//! Signature normalisation shared by indexed code and documented code blocks.

use concordance_core::code::SourceIndex;
use concordance_core::lang::DocSignature;
use concordance_core::RepoPath;

/// Canonical spelling of a signature: single spaces, none next to brackets or dots or
/// before `,` `;` `:`; one space after `,` `:` and `;`, around `=` and `=>`, and inside
/// braces.
pub fn normalize(sig: &str) -> String {
    let s = sig.trim().trim_end_matches([';', '{']).trim();
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut pending = false;
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            pending = true;
            i += 1;
            continue;
        }
        let prev = out.chars().last();
        let next = cs.get(i + 1).copied();
        let mut space = pending && prev.is_some();
        pending = false;
        if let Some(p) = prev {
            if matches!(p, '(' | '[' | '.' | '<' | '!') {
                space = false;
            }
        }
        if matches!(c, ')' | ']' | ',' | '.' | ';' | '(' | '>' | '?' | ':' | '<') {
            space = false;
        }
        if c == '}' {
            space = prev != Some('{');
        }
        if c == '{' && prev.is_some() {
            space = true;
        }
        if c == '=' && next == Some('>') {
            if prev.is_some() {
                out.push(' ');
            }
            out.push_str("=>");
            pending = true;
            i += 2;
            continue;
        }
        if c == '=' && !matches!(prev, Some('=' | '!' | '<' | '>')) && next != Some('=') {
            if prev.is_some() {
                out.push(' ');
            }
            out.push('=');
            pending = true;
            i += 1;
            continue;
        }
        if space {
            out.push(' ');
        }
        out.push(c);
        match c {
            ',' | ';' | '{' => pending = true,
            ':' if next != Some(':') && prev != Some(':') => pending = true,
            _ => {}
        }
        i += 1;
    }
    out.trim().to_string()
}

/// Qualifiers documentation usually omits.
const QUALIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "virtual",
    "override",
    "abstract",
    "sealed",
    "async",
    "extern",
    "new",
    "readonly",
    "partial",
    "unsafe",
    "required",
    "file",
];

pub fn strip_modifiers(sig: &str) -> String {
    let mut s = sig.trim();
    loop {
        let word = s.split_whitespace().next().unwrap_or("");
        if QUALIFIERS.contains(&word) && s.len() > word.len() {
            s = s[word.len()..].trim_start();
        } else {
            return s.to_string();
        }
    }
}

fn snippet_path() -> RepoPath {
    RepoPath::from_relative_str("__doc__.cs").expect("valid path")
}

pub const WRAPPER: &str = "__Doc";

fn has_items(idx: &SourceIndex) -> bool {
    idx.symbols
        .iter()
        .any(|s| !s.is_test && s.signature.is_some() && s.name != WRAPPER)
}

/// Signatures of the items declared in a documented code block.
pub fn documented(code: &str) -> Vec<DocSignature> {
    let path = snippet_path();
    let mut idx = crate::index::index_snippet(&path, code);
    if !has_items(&idx) {
        // Members written without their type: wrap them. The prefix stays on line 1 so
        // line numbers are unchanged.
        idx = crate::index::index_snippet(&path, &format!("class {WRAPPER} {{ {code}\n}}"));
    }
    let lines: Vec<&str> = code.lines().collect();
    idx.symbols
        .iter()
        .filter(|s| !s.is_test && s.signature.is_some() && s.name != WRAPPER)
        .map(|s| {
            let start = s.range.start_line.saturating_sub(1) as usize;
            let end = s.range.end_line.saturating_sub(1) as usize;
            let line = lines.get(start).copied().unwrap_or("");
            let name_col = line
                .find(s.name.as_str())
                .map(|b| line[..b].chars().count())
                .unwrap_or(0);
            DocSignature {
                name: s.name.clone(),
                text: s.signature.clone().unwrap_or_default(),
                start,
                end: end.max(start),
                name_col,
            }
        })
        .collect()
}

/// `(name, normalized)` for one documented signature (a declaration, or a member written
/// on its own).
pub fn normalize_documented(text: &str) -> Option<(String, String)> {
    let t = text.trim().trim_end_matches(';').trim();
    let path = snippet_path();
    let first = |code: String| {
        crate::index::index_snippet(&path, &code)
            .symbols
            .into_iter()
            .find(|s| s.signature.is_some() && !s.is_test && s.name != WRAPPER)
            .map(|s| (s.name, s.signature.unwrap_or_default()))
    };
    first(format!("class {WRAPPER} {{ {t}; }}"))
        .or_else(|| first(format!("{t} {{ }}")))
        .or_else(|| first(format!("{t};")))
}
