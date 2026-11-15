//! A small JSONC front end: comments and trailing commas are blanked out with spaces, so
//! the result is plain JSON of the same length and every byte keeps its line. `serde_json`
//! then parses it, and [`scan_top_level`] recovers where each top-level array element
//! starts (for line numbers) without a second parser.

use std::collections::BTreeMap;

/// Blank out `//` and `/* */` comments, a leading byte-order mark and trailing commas.
///
/// The output has the same byte length as the input. Newlines are preserved, so a byte
/// offset maps to the same line in the cleaned and the original text. String contents are
/// never touched: comment markers and commas inside strings (including after an escaped
/// quote) stay as they are.
pub fn clean(text: &str) -> Result<String, String> {
    let b = text.as_bytes();
    let mut out: Vec<u8> = b.to_vec();
    let mut i = 0;
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        out[..3].fill(b' ');
        i = 3;
    }
    // Pass 1: comments.
    while i < b.len() {
        match b[i] {
            b'"' => i = skip_string(b, i).ok_or_else(|| unterminated_string(b, i))?,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let start = i;
                out[i] = b' ';
                out[i + 1] = b' ';
                i += 2;
                loop {
                    if i + 1 >= b.len() {
                        return Err(format!(
                            "unterminated block comment starting at line {}",
                            line_at(b, start)
                        ));
                    }
                    if b[i] == b'*' && b[i + 1] == b'/' {
                        out[i] = b' ';
                        out[i + 1] = b' ';
                        i += 2;
                        break;
                    }
                    if b[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    // Pass 2: trailing commas, on the comment-free text.
    let mut i = 0;
    while i < out.len() {
        match out[i] {
            b'"' => i = skip_string(&out, i).ok_or_else(|| unterminated_string(&out, i))?,
            b',' => {
                let mut j = i + 1;
                while j < out.len() && out[j].is_ascii_whitespace() {
                    j += 1;
                }
                if matches!(out.get(j), Some(b']') | Some(b'}')) {
                    out[i] = b' ';
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    // Every byte written was ASCII and every byte of a comment was replaced, so the
    // buffer is still valid UTF-8.
    String::from_utf8(out).map_err(|e| e.to_string())
}

/// Index just past the string that opens at `start` (a `"`), or `None` if it never closes.
fn skip_string(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

fn unterminated_string(b: &[u8], at: usize) -> String {
    format!("unterminated string starting at line {}", line_at(b, at))
}

fn line_at(b: &[u8], offset: usize) -> usize {
    b[..offset.min(b.len())]
        .iter()
        .filter(|&&c| c == b'\n')
        .count()
        + 1
}

/// What [`scan_top_level`] found in a cleaned document whose root is an object.
#[derive(Debug, Default)]
pub struct TopLevel {
    /// Every key of the root object, in order.
    pub keys: Vec<String>,
    /// For each top-level key whose value is an array: the byte offset where each element
    /// starts, in order.
    pub elements: BTreeMap<String, Vec<usize>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Want {
    None,
    Key,
    Value,
    Element,
}

/// Walk cleaned JSON (see [`clean`]) and collect the root object's keys and the start
/// offset of each element of its array-valued keys. It is lenient about everything else:
/// the actual parse is `serde_json`'s job. `None` when the text is not an object or a
/// string never closes.
pub fn scan_top_level(clean: &str) -> Option<TopLevel> {
    let b = clean.as_bytes();
    let mut out = TopLevel::default();
    let mut depth = 0usize;
    let mut want = Want::None;
    let mut key: Option<String> = None;
    let mut in_array = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if depth == 0 {
            if c != b'{' {
                return None;
            }
            depth = 1;
            want = Want::Key;
            i += 1;
            continue;
        }
        // An element of a top-level array begins here.
        if depth == 2 && in_array && want == Want::Element && c != b']' {
            if let Some(k) = &key {
                out.elements.entry(k.clone()).or_default().push(i);
            }
            want = Want::None;
        }
        match c {
            b'"' => {
                let end = skip_string(b, i)?;
                if depth == 1 && want == Want::Key {
                    let raw = &clean[i..end];
                    let name = serde_json::from_str::<String>(raw)
                        .unwrap_or_else(|_| raw.trim_matches('"').to_string());
                    out.keys.push(name.clone());
                    key = Some(name);
                    want = Want::None;
                } else if depth == 1 && want == Want::Value {
                    want = Want::None;
                }
                i = end;
                continue;
            }
            b':' if depth == 1 => want = Want::Value,
            b',' if depth == 1 => want = Want::Key,
            b',' if depth == 2 && in_array => want = Want::Element,
            b'{' | b'[' => {
                if depth == 1 {
                    in_array = c == b'[' && want == Want::Value;
                    want = if in_array { Want::Element } else { Want::None };
                }
                depth += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                if depth == 1 {
                    in_array = false;
                    want = Want::None;
                }
                if depth == 0 {
                    break;
                }
            }
            _ => {
                if depth == 1 && want == Want::Value {
                    want = Want::None; // a scalar value
                }
            }
        }
        i += 1;
    }
    Some(out)
}

/// 1-based line of a byte offset.
#[cfg(test)]
pub fn line_of(text: &str, offset: usize) -> usize {
    line_at(text.as_bytes(), offset)
}

/// The byte offset where each line starts, for many offset-to-line lookups on one text
/// (`line_of` rescans from the start every time).
pub struct LineTable(Vec<usize>);

impl LineTable {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            text.bytes()
                .enumerate()
                .filter(|&(_, c)| c == b'\n')
                .map(|(i, _)| i + 1),
        );
        LineTable(starts)
    }

    /// 1-based line of a byte offset.
    pub fn line(&self, offset: usize) -> usize {
        self.0.partition_point(|&s| s <= offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_markers_inside_strings_survive() {
        let src = r#"{"u": "http://x // not a comment", "b": "/* nor this */"}"#;
        assert_eq!(clean(src).unwrap(), src);
    }

    #[test]
    fn crlf_line_endings_keep_line_numbers() {
        let src =
            "{\r\n  // c\r\n  \"a\": [\r\n    {\"id\": 1},\r\n    {\"id\": 2}\r\n  ]\r\n}\r\n";
        let c = clean(src).unwrap();
        let top = scan_top_level(&c).unwrap();
        let starts = &top.elements["a"];
        assert_eq!(starts.len(), 2);
        assert_eq!(line_of(&c, starts[0]), 4);
        assert_eq!(line_of(&c, starts[1]), 5);
    }
}
