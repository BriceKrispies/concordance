//! Word-level helpers for the lint rules. All offsets are byte offsets into the string
//! given, which is never altered in length.

/// `s` with every backticked span (backticks included) replaced by spaces of equal length.
pub fn mask_code(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for c in s.chars() {
        if c == '`' {
            inside = !inside;
            out.push(' ');
        } else if inside {
            out.extend(std::iter::repeat_n(' ', c.len_utf8()));
        } else {
            out.push(c);
        }
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Offsets of whole-word, case-insensitive occurrences of `phrase` (which may contain
/// spaces and hyphens) in `s`.
pub fn find_word(s: &str, phrase: &str) -> Vec<usize> {
    let hay = s.to_lowercase();
    let needle = phrase.to_lowercase();
    // Lowercasing can change byte lengths for exotic characters; fall back to no match
    // rather than reporting a wrong offset.
    if hay.len() != s.len() || needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = hay[from..].find(&needle) {
        let at = from + i;
        let end = at + needle.len();
        let before = hay[..at].chars().next_back();
        let after = hay[end..].chars().next();
        if !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char) {
            out.push(at);
        }
        from = at + needle.len().max(1);
    }
    out
}

/// Lowercase alphanumeric tokens with their byte offsets.
pub fn tokens(s: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if is_word_char(c) {
            start.get_or_insert(i);
        } else if let Some(st) = start.take() {
            out.push((st, s[st..i].to_lowercase()));
        }
    }
    if let Some(st) = start {
        out.push((st, s[st..].to_lowercase()));
    }
    out
}

const TIME_UNITS: &[&str] = &[
    "ns", "us", "\u{b5}s", "ms", "s", "sec", "secs", "second", "seconds", "min", "mins", "minute",
    "minutes", "hour", "hours", "day", "days",
];
const SIZE_UNITS: &[&str] = &[
    "b", "kb", "mb", "gb", "tb", "kib", "mib", "gib", "tib", "byte", "bytes", "percent",
];
const LIMIT_PHRASES: &[&str] = &[
    "at most",
    "at least",
    "no more than",
    "no fewer than",
    "no less than",
    "fewer than",
    "more than",
    "less than",
    "up to",
    "exceed",
    "exceeds",
    "exceeding",
];

/// The first time, size or count limit in `s` (already code-masked): a number with a unit
/// (`5 ms`, `10MB`, `90%`), or a number after `at most`, `no more than`, ... Returns the
/// byte offset and the phrase.
pub fn find_limit(s: &str) -> Option<(usize, String)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() || (i > 0 && (bytes[i - 1].is_ascii_alphanumeric())) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_' || bytes[i] == b',')
        {
            i += 1;
        }
        // A fractional part.
        if i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
        let num_end = i;
        let mut j = i;
        if j < bytes.len() && bytes[j] == b' ' {
            j += 1;
        }
        let unit: String = s[j..]
            .chars()
            .take_while(|c| c.is_alphabetic() || *c == '%' || *c == '\u{b5}')
            .collect();
        let lower = unit.to_lowercase();
        let tight = j == num_end;
        let has_unit = !unit.is_empty()
            && (unit == "%"
                || ((TIME_UNITS.contains(&lower.as_str()) || SIZE_UNITS.contains(&lower.as_str()))
                    // A single letter unit must touch the number (`5s`, `2 b` is prose).
                    && (lower.len() > 1 || tight)));
        if has_unit {
            let end = j + unit.len();
            return Some((start, s[start..end].to_string()));
        }
        // A limit phrase directly before the number.
        let before = s[..start].trim_end().to_lowercase();
        for p in LIMIT_PHRASES {
            if before.ends_with(p)
                && !before[..before.len() - p.len()]
                    .chars()
                    .next_back()
                    .is_some_and(is_word_char)
            {
                let phrase_start = before.len() - p.len();
                return Some((phrase_start, format!("{p} {}", &s[start..num_end])));
            }
        }
        i = num_end.max(i + 1);
    }
    None
}
