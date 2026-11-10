//! Signature normalisation shared by indexed code and documented code blocks.

use concordance_core::lang::DocSignature;
use concordance_core::RepoPath;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// Canonical spelling of a signature: whitespace only where two words meet, trailing
/// commas and `;` dropped, then one space after `,` and `:` and around `=>` / `|` / `&`.
pub fn normalize(sig: &str) -> String {
    let chars: Vec<char> = sig
        .trim()
        .trim_end_matches([';', '{'])
        .trim()
        .chars()
        .collect();
    let mut tight = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            let prev = tight.chars().last();
            let next = chars[i..].iter().find(|c| !c.is_whitespace()).copied();
            if prev.is_some_and(is_word) && next.is_some_and(is_word) {
                tight.push(' ');
            }
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            continue;
        }
        tight.push(c);
        i += 1;
    }
    // Trailing commas before closers.
    let mut cleaned = String::new();
    let tc: Vec<char> = tight.chars().collect();
    for (k, c) in tc.iter().enumerate() {
        if *c == ','
            && tc
                .get(k + 1)
                .is_some_and(|n| matches!(n, ')' | ']' | '>' | '}'))
        {
            continue;
        }
        cleaned.push(*c);
    }
    let mut out = String::new();
    let cc: Vec<char> = cleaned.chars().collect();
    let mut k = 0;
    while k < cc.len() {
        let c = cc[k];
        if c == '=' && cc.get(k + 1) == Some(&'>') {
            out.push_str(" => ");
            k += 2;
            continue;
        }
        let prev = if k > 0 { cc[k - 1] } else { ' ' };
        let next = cc.get(k + 1).copied().unwrap_or(' ');
        if c == '=' && !matches!(prev, '=' | '!' | '<' | '>') && next != '=' {
            out.push_str(" = ");
            k += 1;
            continue;
        }
        match c {
            ',' => out.push_str(", "),
            ':' => out.push_str(": "),
            '|' | '&' => {
                out.push(' ');
                out.push(c);
                out.push(' ');
            }
            _ => out.push(c),
        }
        k += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Visibility/export qualifiers documentation usually omits.
pub fn strip_modifiers(sig: &str) -> String {
    let mut s = sig.trim();
    loop {
        let before = s;
        for kw in ["export ", "default ", "declare ", "public "] {
            if let Some(r) = s.strip_prefix(kw) {
                s = r.trim_start();
            }
        }
        if s == before {
            return s.to_string();
        }
    }
}

fn snippet_path() -> RepoPath {
    RepoPath::from_relative_str("__doc__.ts").expect("valid path")
}

/// Signatures of the items declared in a documented code block.
pub fn documented(code: &str) -> Vec<DocSignature> {
    let idx = crate::index::index_snippet(&snippet_path(), code);
    let lines: Vec<&str> = code.lines().collect();
    idx.symbols
        .iter()
        .filter(|s| !s.is_test && s.signature.is_some())
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

/// `(name, normalized)` for one documented signature (a declaration, or a class or
/// interface member written on its own).
pub fn normalize_documented(text: &str) -> Option<(String, String)> {
    let first = |code: &str| {
        crate::index::index_snippet(&snippet_path(), code)
            .symbols
            .into_iter()
            .find(|s| s.signature.is_some() && !s.is_test)
            .map(|s| (s.name, s.signature.unwrap_or_default()))
    };
    let t = text.trim().trim_end_matches(';');
    if let Some(r) = first(&format!("{t};")) {
        return Some(r);
    }
    // A member signature: wrap it in an interface and take the member.
    crate::index::index_snippet(&snippet_path(), &format!("interface __Doc {{ {t}; }}"))
        .symbols
        .into_iter()
        .find(|s| s.path.len() >= 2 && s.signature.is_some())
        .map(|s| (s.name, s.signature.unwrap_or_default()))
}
