//! Text helpers: slugs, word tokenization, sentence splitting, percent-decoding.

/// GitHub-style heading anchor: lowercase, spaces -> `-`, drop punctuation except `-`/`_`.
pub fn slugify(heading: &str) -> String {
    let mut out = String::new();
    for c in heading.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c.is_whitespace() || c == '-' {
            out.push('-');
        } else if c == '_' {
            out.push('_');
        }
    }
    out
}

/// Lowercase words (split on non-alphanumerics).
pub fn words_lower(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

/// True if `phrase` occurs as consecutive words.
pub fn has_phrase(words: &[String], phrase: &[&str]) -> bool {
    find_phrase(words, phrase).is_some()
}

pub fn find_phrase(words: &[String], phrase: &[&str]) -> Option<usize> {
    if phrase.is_empty() || words.len() < phrase.len() {
        return None;
    }
    (0..=words.len() - phrase.len())
        .find(|&i| phrase.iter().enumerate().all(|(k, p)| words[i + k] == *p))
}

pub fn has_word(words: &[String], w: &str) -> bool {
    words.iter().any(|x| x == w)
}

/// `text` with the given byte ranges replaced by a single space.
pub fn mask_ranges(text: &str, ranges: &[(usize, usize)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for &(s, e) in ranges {
        if s < pos || e > text.len() || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
            continue;
        }
        out.push_str(&text[pos..s]);
        out.push(' ');
        pos = e;
    }
    out.push_str(&text[pos..]);
    out
}

const ABBREVIATIONS: &[&str] = &[
    "e.g", "i.e", "etc", "vs", "cf", "approx", "eg", "ie", "al", "no", "fig",
];

/// Split `text` into trimmed sentence byte ranges. Never splits inside `protected` ranges.
pub fn split_sentences(text: &str, protected: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if (b == b'.' || b == b'!' || b == b'?') && !protected.iter().any(|&(s, e)| i >= s && i < e)
        {
            let after = i + 1;
            let boundary = if after >= bytes.len() {
                true
            } else if bytes[after] == b' ' {
                let rest = text[after..].trim_start();
                match rest.chars().next() {
                    None => true,
                    Some(c) => c.is_uppercase() || c == '`',
                }
            } else {
                false
            };
            if boundary && !(b == b'.' && is_abbreviation(&text[start..i])) {
                push_trimmed(text, start, after, &mut out);
                start = after;
            }
        }
        i += 1;
    }
    push_trimmed(text, start, text.len(), &mut out);
    out
}

fn is_abbreviation(before: &str) -> bool {
    let last = before.rsplit(' ').next().unwrap_or("").to_lowercase();
    let last = last.trim_start_matches(['(', '"', '\'']);
    // Single initial ("J. Smith") or a known abbreviation.
    ABBREVIATIONS.contains(&last)
        || (last.chars().count() == 1 && last.chars().all(|c| c.is_alphabetic()))
}

fn push_trimmed(text: &str, s: usize, e: usize, out: &mut Vec<(usize, usize)>) {
    let slice = &text[s..e];
    let lead = slice.len() - slice.trim_start().len();
    let trimmed = slice.trim();
    if !trimmed.is_empty() {
        out.push((s + lead, s + lead + trimmed.len()));
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = (b[i + 1] as char).to_digit(16);
            let l = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_and_decode() {
        assert_eq!(slugify("Rate limits (v2)!"), "rate-limits-v2");
        assert_eq!(percent_decode("my%20file.md"), "my file.md");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn sentences_respect_abbreviations_and_code() {
        let t = "Use e.g. a cache. The `a. B` span stays. Done";
        let spans: Vec<&str> =
            split_sentences(t, &[(t.find('`').unwrap(), t.find("stays").unwrap() - 1)])
                .into_iter()
                .map(|(s, e)| &t[s..e])
                .collect();
        assert_eq!(
            spans,
            vec!["Use e.g. a cache.", "The `a. B` span stays.", "Done"]
        );
    }
}
