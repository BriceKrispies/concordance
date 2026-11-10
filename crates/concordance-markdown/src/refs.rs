//! Classification of code spans and links into [`Reference`] targets.

use crate::text::percent_decode;
use concordance_core::doc::{ReferenceStrength, ReferenceTarget};
use concordance_core::RepoPath;

const STD_NAMES: &[&str] = &[
    "Option",
    "Result",
    "Vec",
    "String",
    "Box",
    "Arc",
    "Rc",
    "HashMap",
    "HashSet",
    "BTreeMap",
    "Some",
    "None",
    "Ok",
    "Err",
    "Self",
    "Default",
    "Debug",
    "Clone",
    "Copy",
    "Send",
    "Sync",
    "Iterator",
    "From",
    "Into",
    // JavaScript / TypeScript built-ins.
    "Error",
    "RangeError",
    "TypeError",
    "SyntaxError",
    "ReferenceError",
    "Promise",
    "Array",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "Date",
    "Number",
    "Object",
    "Boolean",
    "Symbol",
    "BigInt",
    "JSON",
    "Math",
    "RegExp",
    "Uint8Array",
    "ArrayBuffer",
    "Record",
    "Partial",
    "Readonly",
    "Required",
    "Pick",
    "Omit",
    "ReturnType",
    "Awaited",
    "NaN",
    "Infinity",
    "Intl",
];

const FILE_EXTS: &[&str] = &[
    "rs", "md", "mdx", "toml", "json", "yaml", "yml", "txt", "lock", "sh", "ps1", "py", "js", "ts",
    "tsx", "jsx", "mts", "cts", "mjs", "cjs", "cs", "csproj", "sln", "html", "css", "cfg",
    "ini", "csv", "proto", "sql",
];

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn strip_generics(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// `&mut crate::a::Foo<T>::bar()` -> `["crate","a","Foo","bar"]`.
fn clean_symbol(s: &str) -> Option<Vec<String>> {
    let mut s = s.trim();
    loop {
        let before = s;
        s = s.trim_start_matches('&').trim_start();
        if let Some(r) = s.strip_prefix("mut ") {
            s = r.trim_start();
        }
        if s == before {
            break;
        }
    }
    let head = s.split('(').next().unwrap_or(s);
    let head = strip_generics(head);
    let segs: Vec<String> = head
        .split("::")
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if segs.is_empty() || !segs.iter().all(|p| is_ident(p)) {
        return None;
    }
    Some(segs)
}

fn is_camel(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && s.chars().any(|c| c.is_ascii_lowercase())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `Mixer.setVolume` / `Codec.Header.parse`: a `.`-separated path whose first segment
/// is a type name (PascalCase). Member access on a variable (`mixer.setVolume`) is not
/// a path; its receiver is dropped by the caller.
fn dotted_type_path(s: &str) -> Option<Vec<String>> {
    if !s.contains('.') {
        return None;
    }
    let segs: Vec<&str> = s.split('.').collect();
    let first = segs.first()?;
    if STD_NAMES.contains(first) {
        return None; // `Math.min`, `Number.isNaN`, `Promise.all`: built-ins, not project symbols
    }
    (segs.len() >= 2
        && segs.iter().all(|x| is_ident(x))
        && first.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
    .then(|| segs.iter().map(|x| x.to_string()).collect())
}

fn is_snake(s: &str) -> bool {
    s.contains('_')
        && s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Split a trailing `#L10`, `#L10-L20` or `:10` line suffix off a path-like string.
fn split_line_suffix(s: &str) -> (&str, Option<u32>) {
    let leading_num = |r: &str| -> Option<u32> {
        let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    };
    if let Some(i) = s.rfind("#L") {
        if let Some(n) = leading_num(&s[i + 2..]) {
            return (&s[..i], Some(n));
        }
    }
    if let Some(i) = s.rfind(':') {
        let r = &s[i + 1..];
        if !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()) {
            if let Some(n) = leading_num(r) {
                return (&s[..i], Some(n));
            }
        }
    }
    (s, None)
}

fn has_known_ext(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty() && FILE_EXTS.contains(&ext.to_ascii_lowercase().as_str())
        }
        None => false,
    }
}

/// Classify the contents of an inline code span.
pub fn classify_code(code: &str) -> Option<(ReferenceTarget, ReferenceStrength)> {
    let s = code.trim();
    if s.is_empty() {
        return None;
    }
    let sym = |path: Vec<String>, st| Some((ReferenceTarget::Symbol { path }, st));

    if s.contains("::") {
        return clean_symbol(s).and_then(|p| sym(p, ReferenceStrength::Explicit));
    }
    for kw in ["fn", "struct", "enum", "trait", "mod"] {
        if let Some(rest) = s.strip_prefix(kw).and_then(|r| r.strip_prefix(' ')) {
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if is_ident(&name) {
                return sym(vec![name], ReferenceStrength::Explicit);
            }
        }
    }
    if s.ends_with(')') {
        if let Some(i) = s.find('(') {
            let head = s[..i].trim();
            if let Some(path) = dotted_type_path(head) {
                return sym(path, ReferenceStrength::Explicit);
            }
            if head.contains('.')
                && head
                    .split('.')
                    .next()
                    .is_some_and(|f| STD_NAMES.contains(&f))
            {
                return None; // a built-in's member (`Math.min(...)`)
            }
            let head = head.rsplit('.').next().unwrap_or(head);
            if is_ident(head) {
                return sym(vec![head.to_string()], ReferenceStrength::Explicit);
            }
        }
    }
    if !s.contains(char::is_whitespace) && !s.contains(['*', '?', '<', '>', '|']) {
        let (base, line) = split_line_suffix(s);
        let last = base.rsplit(['/', '\\']).next().unwrap_or(base);
        if has_known_ext(last) {
            return Some(match RepoPath::from_relative_str(base) {
                Some(path) => (
                    ReferenceTarget::File { path, line },
                    ReferenceStrength::Explicit,
                ),
                None => (
                    ReferenceTarget::OutsideRepo {
                        text: s.to_string(),
                    },
                    ReferenceStrength::Explicit,
                ),
            });
        }
    }
    let stripped = strip_generics(s.trim_start_matches('&').trim());
    let stripped = stripped.trim();
    if let Some(path) = dotted_type_path(stripped) {
        return sym(path, ReferenceStrength::Probable);
    }
    if is_camel(stripped) {
        if STD_NAMES.contains(&stripped) {
            return None;
        }
        return sym(vec![stripped.to_string()], ReferenceStrength::Probable);
    }
    if is_snake(stripped) {
        return sym(vec![stripped.to_string()], ReferenceStrength::Weak);
    }
    None
}

fn has_scheme(s: &str) -> bool {
    match s.find(':') {
        Some(i) if i >= 2 => {
            let scheme = &s[..i];
            scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        _ => false,
    }
}

/// Classify a link destination written in the document at `doc`.
pub fn classify_link(doc: &RepoPath, dest: &str) -> Option<ReferenceTarget> {
    let dest = dest.trim();
    if dest.is_empty() {
        return None;
    }
    if has_scheme(dest) || dest.starts_with("//") {
        return Some(ReferenceTarget::External {
            url: dest.to_string(),
        });
    }
    if let Some(frag) = dest.strip_prefix('#') {
        let anchor = percent_decode(frag);
        return Some(ReferenceTarget::Section {
            document: doc.clone(),
            anchor: if anchor.is_empty() {
                None
            } else {
                Some(anchor)
            },
        });
    }
    let (path_part, frag) = match dest.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (dest, None),
    };
    let path_part = percent_decode(path_part.split('?').next().unwrap_or(path_part));
    if path_part.is_empty() {
        return None;
    }
    let resolve = |p: &str| -> Option<RepoPath> {
        match p.strip_prefix('/') {
            Some(rooted) => RepoPath::from_relative_str(rooted),
            None => doc.join_relative(p),
        }
    };
    let lower = path_part.to_ascii_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdx") {
        return Some(match resolve(&path_part) {
            Some(document) => {
                let anchor = frag.map(percent_decode).filter(|a| !a.is_empty());
                ReferenceTarget::Section { document, anchor }
            }
            None => ReferenceTarget::OutsideRepo {
                text: dest.to_string(),
            },
        });
    }
    let line = frag.and_then(|f| {
        let r = f.strip_prefix('L')?;
        let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    });
    Some(match resolve(&path_part) {
        Some(path) => ReferenceTarget::File { path, line },
        None => ReferenceTarget::OutsideRepo {
            text: dest.to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ReferenceStrength::*;

    fn sym(code: &str) -> Option<(Vec<String>, ReferenceStrength)> {
        match classify_code(code) {
            Some((ReferenceTarget::Symbol { path }, s)) => Some((path, s)),
            _ => None,
        }
    }

    #[test]
    fn code_spans_classify_by_shape() {
        assert_eq!(
            sym("Foo::bar()").unwrap(),
            (vec!["Foo".into(), "bar".into()], Explicit)
        );
        assert_eq!(
            sym("&mut crate::a::B<T>").unwrap().0,
            vec!["crate", "a", "B"]
        );
        assert_eq!(sym("parse()").unwrap(), (vec!["parse".into()], Explicit));
        assert_eq!(
            sym("struct Limiter").unwrap(),
            (vec!["Limiter".into()], Explicit)
        );
        assert_eq!(sym("RateLimiter").unwrap().1, Probable);
        assert_eq!(sym("max_retries").unwrap().1, Weak);
        assert!(classify_code("Option<String>").is_none());
        assert!(classify_code("true").is_none());
        assert!(classify_code("cargo build").is_none());
        match classify_code("src/lib.rs#L10") {
            Some((ReferenceTarget::File { path, line }, Explicit)) => {
                assert_eq!(path.as_str(), "src/lib.rs");
                assert_eq!(line, Some(10));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn links_resolve_relative_to_document() {
        let doc = RepoPath::from_relative_str("docs/spec/a.md").unwrap();
        assert!(matches!(
            classify_link(&doc, "https://x.io"),
            Some(ReferenceTarget::External { .. })
        ));
        match classify_link(&doc, "../b%20c.md#top") {
            Some(ReferenceTarget::Section { document, anchor }) => {
                assert_eq!(document.as_str(), "docs/b c.md");
                assert_eq!(anchor.as_deref(), Some("top"));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            classify_link(&doc, "../../../x.md"),
            Some(ReferenceTarget::OutsideRepo { .. })
        ));
        match classify_link(&doc, "../../src/lib.rs#L7") {
            Some(ReferenceTarget::File { path, line }) => {
                assert_eq!(path.as_str(), "src/lib.rs");
                assert_eq!(line, Some(7));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            classify_link(&doc, "#intro"),
            Some(ReferenceTarget::Section {
                anchor: Some(_),
                ..
            })
        ));
    }
}
