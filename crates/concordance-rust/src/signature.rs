//! Token normalisation and signature rendering. The single normal form shared by
//! `index_source` (real signatures) and `normalize_signature` (documented ones).

use quote::{quote, ToTokens};
use syn::{Item, Signature, Visibility};

/// Keywords after which a `(` / `[` keeps its leading space (`mut (a, b)`).
const SPACED_BEFORE_OPEN: &[&str] = &[
    "mut", "dyn", "impl", "in", "as", "const", "for", "where", "return", "if", "match", "while",
    "let", "else",
];

/// Whitespace-normalise a rendered token string. Collapses whitespace runs to one space,
/// removes spaces before `, ; : ) > ? . ]` and after `( < & ? . [ #`, around `::`, and
/// before `(`/`[`/`<` that directly follow an identifier. String and char literals are
/// copied verbatim. This is the only normaliser used for signatures.
pub fn normalize_tokens(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut pending = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            pending = true;
            i += 1;
            continue;
        }
        let end = literal_end(&chars, i);
        if pending && !out.is_empty() && wants_space(&out, c) {
            out.push(' ');
        }
        pending = false;
        // rustfmt adds trailing commas to multi-line lists; they carry no meaning.
        if matches!(c, ')' | ']' | '}' | '>') && out.ends_with(',') {
            out.pop();
            while out.ends_with(' ') {
                out.pop();
            }
        }
        out.extend(&chars[i..end]);
        i = end;
    }
    out
}

fn literal_end(chars: &[char], i: usize) -> usize {
    let n = chars.len();
    match chars[i] {
        '"' => {
            let mut j = i + 1;
            while j < n {
                if chars[j] == '\\' {
                    j += 2;
                    continue;
                }
                if chars[j] == '"' {
                    return j + 1;
                }
                j += 1;
            }
            n
        }
        '\'' => {
            if chars.get(i + 1) == Some(&'\\') {
                let mut k = i + 3;
                while k < n {
                    if chars[k] == '\'' {
                        return k + 1;
                    }
                    k += 1;
                }
                i + 1
            } else if chars.get(i + 2) == Some(&'\'') && chars.get(i + 1) != Some(&'\'') {
                i + 3
            } else {
                i + 1
            }
        }
        _ => i + 1,
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn last_word(out: &str) -> &str {
    let mut start = out.len();
    for (i, c) in out.char_indices().rev() {
        if is_word(c) {
            start = i;
        } else {
            break;
        }
    }
    &out[start..]
}

fn wants_space(out: &str, c: char) -> bool {
    let p = match out.chars().last() {
        Some(p) => p,
        None => return false,
    };
    if matches!(c, ',' | ';' | ':' | ')' | ']' | '?' | '.' | '>') {
        return false;
    }
    if matches!(p, '(' | '<' | '&' | '?' | '.' | '[' | '#') {
        return false;
    }
    if p == '!' && (is_word(c) || c == '(' || c == '[') {
        return false;
    }
    if p == ':' && out.ends_with("::") {
        return false;
    }
    if c == '<' && is_word(p) {
        return false;
    }
    if (c == '(' || c == '[') && is_word(p) && !SPACED_BEFORE_OPEN.contains(&last_word(out)) {
        return false;
    }
    true
}

/// Render any syntax node and normalise it.
pub(crate) fn tokens_string<T: ToTokens + ?Sized>(t: &T) -> String {
    normalize_tokens(&t.to_token_stream().to_string())
}

pub(crate) fn ident_name(i: &proc_macro2::Ident) -> String {
    let s = i.to_string();
    match s.strip_prefix("r#") {
        Some(r) => r.to_string(),
        None => s,
    }
}

/// `vis sig` of a function, normalised.
pub(crate) fn fn_header(vis: &Visibility, sig: &Signature) -> String {
    tokens_string(&quote!(#vis #sig))
}

/// `(name, normalised header)` for the item kinds that have a signature.
pub(crate) fn item_header(item: &Item) -> Option<(String, String)> {
    Some(match item {
        Item::Fn(f) => (ident_name(&f.sig.ident), fn_header(&f.vis, &f.sig)),
        Item::Struct(s) => {
            let (v, i, g, w) = (&s.vis, &s.ident, &s.generics, &s.generics.where_clause);
            (ident_name(i), tokens_string(&quote!(#v struct #i #g #w)))
        }
        Item::Enum(s) => {
            let (v, i, g, w) = (&s.vis, &s.ident, &s.generics, &s.generics.where_clause);
            (ident_name(i), tokens_string(&quote!(#v enum #i #g #w)))
        }
        Item::Union(s) => {
            let (v, i, g, w) = (&s.vis, &s.ident, &s.generics, &s.generics.where_clause);
            (ident_name(i), tokens_string(&quote!(#v union #i #g #w)))
        }
        Item::Trait(t) => {
            let (v, u, a, i, g) = (&t.vis, &t.unsafety, &t.auto_token, &t.ident, &t.generics);
            let (c, s, w) = (&t.colon_token, &t.supertraits, &t.generics.where_clause);
            (
                ident_name(i),
                tokens_string(&quote!(#v #u #a trait #i #g #c #s #w)),
            )
        }
        Item::Type(t) => {
            let (v, i, g, ty, w) = (
                &t.vis,
                &t.ident,
                &t.generics,
                &t.ty,
                &t.generics.where_clause,
            );
            (
                ident_name(i),
                tokens_string(&quote!(#v type #i #g = #ty #w)),
            )
        }
        Item::Const(c) => {
            let (v, i, ty) = (&c.vis, &c.ident, &c.ty);
            (ident_name(i), tokens_string(&quote!(#v const #i : #ty)))
        }
        Item::Static(c) => {
            let (v, m, i, ty) = (&c.vis, &c.mutability, &c.ident, &c.ty);
            (ident_name(i), tokens_string(&quote!(#v static #m #i : #ty)))
        }
        _ => return None,
    })
}

/// Parse a documented item signature (with or without body / trailing `;`) into
/// `(item name, normalised signature)`. Visibility is kept as written.
pub fn normalize_signature(text: &str) -> Option<(String, String)> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let base = t.trim_end_matches(';').trim_end();
    let mut candidates: Vec<String> = Vec::new();
    if base.ends_with('}') {
        candidates.push(base.to_string());
    }
    candidates.push(format!("{base} {{}}"));
    candidates.push(format!("{base};"));
    candidates.push(format!("{base} = 0;"));
    for c in candidates {
        if let Ok(item) = syn::parse_str::<Item>(&c) {
            if let Some(r) = item_header(&item) {
                return Some(r);
            }
        }
    }
    None
}

/// Remove a leading `pub`, `pub(crate)`, `pub(super)`, `pub(self)` or `pub(in ..)`.
pub fn strip_visibility(sig: &str) -> String {
    let t = sig.trim();
    let Some(rest) = t.strip_prefix("pub") else {
        return t.to_string();
    };
    if rest.starts_with(is_word) {
        return t.to_string();
    }
    let rest = rest.trim_start();
    if let Some(inner) = rest.strip_prefix('(') {
        if let Some(close) = inner.find(')') {
            let content = inner[..close].trim();
            if matches!(content, "crate" | "super" | "self")
                || content.starts_with("in ")
                || content.starts_with("in::")
            {
                return inner[close + 1..].trim_start().to_string();
            }
        }
    }
    rest.to_string()
}
