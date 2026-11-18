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

/// Whether `s` has `no` followed by `ever` in the same sentence, with at most four words between
/// them (`no request ever`, `no user ever sees`). Returns the byte offset of `no`.
pub fn find_no_ever(s: &str) -> Option<usize> {
    for off in find_word(s, "no") {
        let sentence = s[off..].split(['.', ';', '!', '?']).next().unwrap_or("");
        let words: Vec<String> = tokens(sentence).into_iter().map(|(_, w)| w).collect();
        // words[0] is `no`; `ever` must come within the next five words.
        if words.iter().skip(1).take(5).any(|w| w == "ever") {
            return Some(off);
        }
    }
    None
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
    "ns",
    "us",
    "\u{b5}s",
    "ms",
    "s",
    "sec",
    "secs",
    "second",
    "seconds",
    "min",
    "mins",
    "minute",
    "minutes",
    "hour",
    "hours",
    "day",
    "days",
    "nanosecond",
    "nanoseconds",
    "microsecond",
    "microseconds",
    "millisecond",
    "milliseconds",
];
const SIZE_UNITS: &[&str] = &[
    "b",
    "kb",
    "mb",
    "gb",
    "tb",
    "kib",
    "mib",
    "gib",
    "tib",
    "byte",
    "bytes",
    "kilobyte",
    "kilobytes",
    "megabyte",
    "megabytes",
    "gigabyte",
    "gigabytes",
    "terabyte",
    "terabytes",
    "percent",
];
/// Number words. `a`/`an` are not among them: "a second copy" is not a limit.
const NUMBER_WORDS: &[&str] = &[
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
    "thirty",
    "forty",
    "fifty",
    "sixty",
    "seventy",
    "eighty",
    "ninety",
    "hundred",
    "thousand",
    "million",
    "billion",
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
/// (`5 ms`, `10MB`, `90%`, `five milliseconds`), or a number after `at most`, `no more than`, ...
/// (digits or number words). The word `one` after a phrase is not counted (`at least one retry`
/// is an existence, not a bound). Returns the byte offset and the phrase.
pub fn find_limit(s: &str) -> Option<(usize, String)> {
    match (find_limit_digits(s), find_limit_words(s)) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

/// A number word with a time or size unit, or a limit phrase directly before it.
fn find_limit_words(s: &str) -> Option<(usize, String)> {
    let toks = tokens(s);
    for (k, (off, w)) in toks.iter().enumerate() {
        if !NUMBER_WORDS.contains(&w.as_str()) {
            continue;
        }
        let end = off + w.len();
        if let Some((noff, nw)) = toks.get(k + 1) {
            let gap = &s[end..*noff];
            if gap.chars().all(|c| c == ' ')
                && (TIME_UNITS.contains(&nw.as_str()) || SIZE_UNITS.contains(&nw.as_str()))
            {
                return Some((*off, format!("{w} {nw}")));
            }
        }
        if w == "one" {
            continue;
        }
        let before = s[..*off].trim_end().to_lowercase();
        for p in LIMIT_PHRASES {
            if before.ends_with(p)
                && !before[..before.len() - p.len()]
                    .chars()
                    .next_back()
                    .is_some_and(is_word_char)
            {
                let phrase_start = before.len() - p.len();
                return Some((phrase_start, format!("{p} {w}")));
            }
        }
    }
    None
}

/// The digit form of [`find_limit`].
fn find_limit_digits(s: &str) -> Option<(usize, String)> {
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
